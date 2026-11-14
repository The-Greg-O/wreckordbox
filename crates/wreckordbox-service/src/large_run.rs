//! A backup before a large Run (wbx-25, ADR 0004 story 39): a second way
//! back besides the Ledger.
//!
//! # Which Runs
//!
//! Covered: applying a projection of Energy and Prep colour
//! (`projection.apply`), of the Tag schema to `MyTags`
//! (`projection.mytags.apply`), and Review's accept of waiting Facts
//! (`review.accept`). Each calls [`before_write`] with the tracks of the
//! plan it is about to execute, after making that plan and before its
//! write, outside the Library's edit gate. A dry run never reaches it.
//!
//! Not covered: a Tag schema conversion (`schema.convert`) and bulk Fact
//! recording (`facts.record-many`), which write only the Sidecar, never a
//! track in the Library; every other bulk write an agent can make today
//! (`tracks.rate`, `mytags.assign`, `intake.set_tags`, playlist edits) is
//! not a planned Run and has no count before it writes.
//!
//! # The count cannot be gamed
//!
//! The count is of the plan the apply then writes, made by the apply
//! itself, never a dry run's: the write's own re-check under the edit gate
//! refuses, with nothing written, unless every planned track is exactly as
//! planned (Projection: row revisions and no Ledger entry since the plan;
//! Review: the listing still digests to the token the DJ was shown). So the
//! Run that writes is the one counted, and the backup, taken after the
//! plan, holds every planned track's value before the Run.
//!
//! # The threshold
//!
//! `[backup] above_tracks = N` in the projection policy file
//! (`projection-policy.toml`, the DJ's, beside the Tag schema): a backup is
//! taken before a Run that will change more than N tracks. With no file or
//! no section, [`DEFAULT_ABOVE`]. The value in force, the count and the
//! backup (or none) are put in the command of every Ledger entry of the
//! write, under `args.large_run` (`{tracks, above, backup:
//! {path, created_at, bytes} | null}`), before the write's intent is saved;
//! so the intent, the entry, and an unconfirmed entry after a crash all
//! name it. The answer carries the same object as `large_run`.
//!
//! # Failure
//!
//! The backup goes to the folder the window saves backups in
//! (`rbl_backup::default_destination`), which may be on a Stick: Stick
//! Protection's authorization is taken for it and re-checked before every
//! write. Library Protection on refuses before the backup. A backup that
//! fails or is refused (disk full, a protected or replaced Stick) stops the
//! Run before its intent: nothing is changed, no Run is made, and the
//! answer is the refusal (`run_backup_failed`, `stick_protected`,
//! `library_protected`).
//!
//! # A backup with no Run
//!
//! A crash, a refusal of the write's re-check, or a Ledger refusal after the
//! backup leaves a complete, ordinary backup and no change. It is kept: it
//! is listed by `wreckordbox backup list` and in Preferences › Backups like
//! any other, and deleted there by the DJ. Nothing sweeps it.
//!
//! # Locks
//!
//! Taken under the service's Library lock (one command at a time) and, for
//! `MyTags` and Review, the Fact coordination, which the apply already holds;
//! never under the edit gate. During the copy the analysis-file lock is
//! held (as the window's backup holds it) so the trees are not half
//! written; lock order: Stick, then the analysis lock, then nothing.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::library::OpenLibrary;
use crate::protocol::{codes as service_codes, ServiceError};
use crate::stems::Stems;

/// Codes this module answers.
pub mod codes {
    /// The backup before a large Run could not be made; nothing changed.
    pub const BACKUP_FAILED: &str = "run_backup_failed";
}

/// The threshold when the policy file names none.
pub const DEFAULT_ABOVE: usize = 200;

/// Test mode only: `disk_full` makes the backup's writes fail as on a full
/// disk once it has begun writing the archive.
pub const FAULT_ENV: &str = "WRECKORDBOX_RUN_BACKUP_FAULT";

/// The policy file's `[backup]` section.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Section {
    above_tracks: usize,
}

#[derive(Debug, Default, Deserialize)]
struct PolicyFile {
    backup: Option<Section>,
}

/// The backup a Run took first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Taken {
    pub path: PathBuf,
    /// Unix milliseconds.
    pub created_at: u64,
    pub bytes: u64,
    /// How long it took.
    pub took_ms: u64,
}

/// What a large-Run check decided, as recorded with the Run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checked {
    /// Tracks the Run will change.
    pub tracks: usize,
    /// The threshold in force.
    pub above: usize,
    pub backup: Option<Taken>,
}

thread_local! {
    static DECIDED: RefCell<Option<Checked>> = const { RefCell::new(None) };
}

/// The threshold in force: `[backup] above_tracks` of the policy file in
/// `sidecar_dir`, or [`DEFAULT_ABOVE`].
fn threshold(sidecar_dir: &Path) -> Result<usize, ServiceError> {
    let path = sidecar_dir.join(crate::projection::POLICY_FILE);
    let invalid = |why: String| {
        ServiceError::new(
            crate::projection::codes::POLICY_INVALID,
            format!(
                "{} cannot be used, so nothing was changed: {why}",
                path.display()
            ),
        )
    };
    let bytes = crate::projection::policy_bytes(sidecar_dir).map_err(|e| invalid(e.to_string()))?;
    let text = String::from_utf8(bytes).map_err(|e| invalid(e.to_string()))?;
    let file: PolicyFile = toml::from_str(&text).map_err(|e| {
        invalid(format!(
            "backup: above_tracks is a whole number of tracks ({e})"
        ))
    })?;
    Ok(file
        .backup
        .map_or(DEFAULT_ABOVE, |section| section.above_tracks))
}

/// Before the write of a Run that will change `tracks`: decides whether it
/// is large, takes the backup when it is, and records what was decided
/// with the command. An error stops the Run with nothing changed.
pub(crate) fn before_write(
    library: &OpenLibrary,
    stems: &Stems,
    tracks: impl IntoIterator<Item = String>,
) -> Result<(), ServiceError> {
    let count = tracks.into_iter().collect::<HashSet<_>>().len();
    let above = threshold(stems.sidecar_dir())?;
    let backup = if count > above {
        let taken = take(library, stems)?;
        hold("backed-up", library.for_backup().0);
        Some(taken)
    } else {
        None
    };
    let checked = Checked {
        tracks: count,
        above,
        backup,
    };
    let value = serde_json::to_value(&checked).unwrap_or(Value::Null);
    crate::ledger::note("large_run", value);
    DECIDED.with(|decided| *decided.borrow_mut() = Some(checked));
    Ok(())
}

/// A command's answer with what [`before_write`] decided in it (`large_run`
/// in a successful answer; a failure says the backup is kept). Call it on
/// every answer of a command that may call [`before_write`].
pub(crate) fn answered(result: Result<Value, ServiceError>) -> Result<Value, ServiceError> {
    let decided = DECIDED.with(|decided| decided.borrow_mut().take());
    let Some(decided) = decided else {
        return result;
    };
    match result {
        Ok(Value::Object(mut answer)) => {
            answer.insert("large_run".into(), json!(decided));
            Ok(Value::Object(answer))
        }
        Ok(other) => Ok(other),
        Err(mut error) => {
            if let Some(taken) = &decided.backup {
                error.message = format!(
                    "{} (A backup was taken before this Run and is kept: {}.)",
                    error.message,
                    taken.path.display()
                );
            }
            Err(error)
        }
    }
}

fn take(library: &OpenLibrary, stems: &Stems) -> Result<Taken, ServiceError> {
    let (state_dir, location, analysis_files) = library.for_backup();
    let state_dir = state_dir.to_path_buf();
    if crate::protection::is_on(&state_dir) {
        return Err(crate::protection::refusal());
    }
    let location = location.ok_or_else(|| {
        ServiceError::new(
            service_codes::LIBRARY_LOADING,
            rbl_library::NOT_LOADED.to_owned(),
        )
    })?;
    let destination = rbl_backup::default_destination(&state_dir);
    let started = Instant::now();
    // A backup folder on a Stick is a Stick write: refused unless the DJ
    // unlocked that very mount, and checked again before every write.
    let authorized =
        wreckordbox_stick_protection::authorize(&state_dir, &destination).map_err(|why| {
            let kind = match &why {
                wreckordbox_stick_protection::Refused::Protected { .. } => {
                    service_codes::STICK_PROTECTED
                }
                _ => codes::BACKUP_FAILED,
            };
            ServiceError::new(
                kind,
                format!(
                    "this Run would change more tracks than the backup threshold, and its backup could not be made: {why}; nothing was changed"
                ),
            )
        })?;
    hold("stick-held", &state_dir);
    let fault = rbl_db::test_mode()
        .then(|| std::env::var(FAULT_ENV).ok())
        .flatten();
    let writes = std::cell::Cell::new(0_u64);
    let failed = std::cell::Cell::new(false);
    let check = || -> std::io::Result<()> {
        authorized.recheck().map_err(|refused| {
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, refused.to_string())
        })?;
        writes.set(writes.get() + 1);
        if fault.as_deref() == Some("disk_full") && writes.get() > 3 && !failed.replace(true) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::StorageFull,
                "No space left on device (test fault)",
            ));
        }
        Ok(())
    };
    let sidecar_dir = stems.sidecar_dir();
    let request = rbl_backup::create::Request {
        location: &location,
        sidecar_dir: sidecar_dir.is_dir().then_some(sidecar_dir),
        sidecar: rbl_backup::sidecar_part::Options::saved(&state_dir),
        destination: &destination,
        scratch: &state_dir,
        check: &check,
        app_version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        at: &|point| hold(point, &state_dir),
    };
    let made = {
        let _files = analysis_files.lock();
        rbl_backup::create::write(&request)
    }
    .map_err(|error| {
        ServiceError::new(
            codes::BACKUP_FAILED,
            format!(
                "this Run would change more tracks than the backup threshold, and its backup to {} failed: {error}; nothing was changed",
                destination.display()
            ),
        )
    })?;
    drop(authorized);
    Ok(Taken {
        path: made.path,
        created_at: made.created_at,
        bytes: made.bytes,
        took_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

/// A folder in the state folder in which a test puts an empty file named
/// for a moment of the backup (`between-stores`, `stick-held`,
/// `backed-up`): a backup of that state folder reaching the moment makes
/// `<moment>.reached` there and waits until the test removes `<moment>`
/// (two minutes at most). Test mode only; scoped by state folder, so tests
/// in one process do not hold each other.
pub const HOLDS: &str = "test-run-backup-holds";

pub(crate) fn hold(point: &str, state_dir: &Path) {
    if !rbl_db::test_mode() {
        return;
    }
    let dir = state_dir.join(HOLDS);
    let armed = dir.join(point);
    if !armed.exists() {
        return;
    }
    let _ = std::fs::write(dir.join(format!("{point}.reached")), b"");
    let deadline = Instant::now() + std::time::Duration::from_secs(120);
    while armed.exists() && Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
