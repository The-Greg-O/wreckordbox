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
//! track in the Library. Every other write an agent can make
//! (`tracks.rate`, `mytags.assign`, `intake.set_tags`, playlist edits) is
//! counted when it joins a named Run (below); made as a Run of its own it
//! is one command's small change and is not counted.
//!
//! # The count cannot be gamed
//!
//! The count is of the plan the apply then writes, made by the apply
//! itself, never a dry run's: the write's own re-check under the edit gate
//! refuses, with nothing written, unless every planned track is exactly as
//! planned (Projection: row revisions and no Ledger entry since the plan;
//! Review: the listing still digests to the token the DJ was shown, or each
//! chosen row to its own). So the Run that writes is the one counted.
//!
//! A Run joined by several commands (`--run`) is counted whole: the tracks
//! its earlier `track` and `track_tags` entries changed, with this
//! command's plan. Its total is not known in advance, so the backup is
//! taken before the command that takes it over the threshold, not before
//! its first write; later commands of the Run name that backup
//! (`run_backup`) and take none. Any command can cross it: every write of
//! a command that joined a Run, whatever the command (`tracks.rate`, a My
//! Tag edit), is counted the same way where the service's writes enter
//! the edit path ([`before_joined_write`], from `OpenLibrary`), before it
//! is made. A write that cannot say before it is made which tracks it
//! changes (a My Tag renamed, merged or deleted, tracks removed) counts as
//! over the threshold (`uncounted`). A command that is a Run of its own,
//! and the DJ's edits in the window, are counted only as above.
//!
//! # The threshold
//!
//! `[backup] above_tracks = N` in the projection policy file
//! (`projection-policy.toml`, the DJ's, beside the Tag schema): a backup is
//! taken before a Run that will change more than N tracks. With no file or
//! no section, [`DEFAULT_ABOVE`]. What was decided ([`Checked`]) is put in
//! the command of every Ledger entry of the write, under `args.large_run`,
//! before the write's intent is saved; so the intent, the entry, and an
//! unconfirmed entry after a crash all name it. The answer carries it too.
//!
//! # One moment
//!
//! [`take`] stages both stores and the trees under the edit gate and the
//! analysis-file lock (`rbl_backup::create::stage`), on the Library's own
//! file system, hard-linking the large files (a link that cannot be made
//! refuses the backup: no byte copy under the gate). Of Stem audio it
//! stages only the files the Sidecar snapshot's Facts name, so a
//! separation publishing meanwhile, which holds neither lock, adds nothing.
//! It remembers where the Library (`localUpdateCount`, the index
//! generation) and the Sidecar (the Ledger's order position; the newest
//! Run and how many have ended, which a separation's Run moves) stood.
//! With neither lock held it writes the archive from the staged copies.
//! Before the write, under the edit gate, [`bound`] checks all still stand
//! there; if not, the backup is removed and taken again: [`ATTEMPTS`]
//! backups at most, each checked, then the Run is refused. The gate is
//! then held through the write.
//!
//! # Failure
//!
//! The backup goes to the folder the window saves backups in
//! (`rbl_backup::default_destination`), which may be on a Stick: a protected
//! Stick is refused before anything is staged, and the authorization is
//! taken for the archive and re-checked before every write. A folder inside
//! the analysis or artwork tree or the Sidecar is refused. Library
//! Protection on refuses before the backup. A backup that fails or is
//! refused stops the Run before its intent: nothing is changed, no Run is
//! made, and the answer is the refusal (`run_backup_failed`,
//! `stick_protected`, `library_protected`).
//!
//! # A backup with no Run
//!
//! A crash, or a refusal of the write's re-check, after the backup leaves a
//! complete, ordinary backup and no change. It is kept: it is listed by
//! `wreckordbox backup list` and in Preferences › Backups like any other,
//! and deleted there by the DJ. Nothing sweeps it.
//!
//! # Locks
//!
//! One order for every backup writer, the window's too: the edit gate, then
//! a Stick, then the analysis-file lock. Here the Stick is never held with
//! the other two: staging holds the gate and the analysis lock, publishing
//! holds only the Stick. Taken under the service's Library lock (one
//! command at a time) and, for `MyTags` and Review, the Fact coordination,
//! which the apply already holds (taken before the Library lock): both are
//! held for the whole backup. The backup itself never takes the Fact
//! coordination: background Stem publication is excluded by the snapshot
//! rule and the Run mark above, not by a lock.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::library::OpenLibrary;
use crate::protocol::{codes as service_codes, ServiceError};
use crate::stems::Stems;
use wreckordbox_stick_write::selection::LibrarySource as _;

/// Codes this module answers.
pub mod codes {
    /// The backup before a large Run could not be made; nothing changed.
    pub const BACKUP_FAILED: &str = "run_backup_failed";
}

/// The threshold when the policy file names none.
pub const DEFAULT_ABOVE: usize = 200;

/// Test mode only: `disk_full` makes the backup's writes fail as on a full
/// disk once it has begun writing the archive; `link_fails` makes every
/// hard link of staging fail, as across file systems.
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
    /// How long it took, all told.
    pub took_ms: u64,
    /// How long the Library's edit gate (and the analysis-file lock) was
    /// held: the staging of both stores and the trees.
    pub gate_ms: u64,
    /// How long the backup folder's Stick (if any) was held: the archive.
    pub publish_ms: u64,
}

/// What a large-Run check decided, as recorded with the Run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checked {
    /// Tracks this command will change.
    pub tracks: usize,
    /// Tracks the Run will have changed with this command: the ones its
    /// earlier writes changed and this command's.
    pub run_tracks: usize,
    /// The threshold in force.
    pub above: usize,
    /// The backup taken before this command.
    pub backup: Option<Taken>,
    /// The backup taken before an earlier command of the same Run, the one
    /// that took the Run over the threshold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_backup: Option<PathBuf>,
    /// This write joining a Run could not say before it was made which
    /// tracks it changes (a My Tag renamed or deleted, tracks removed):
    /// counted as over the threshold, so the Run's backup comes first.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub uncounted: bool,
}

/// The tracks a write will change, as known before it is made.
#[derive(Debug, Clone)]
pub(crate) enum Planned {
    Tracks(Vec<String>),
    /// Not known before the write.
    Uncounted,
}

impl Planned {
    /// What a write through the edit path that re-reads `touched` (and
    /// names `named`) will change: the tracks it names, none for the
    /// playlist tree or the Tag List alone, else not known.
    pub(crate) fn of(touched: &rbl_library::Touched, named: &[String]) -> Self {
        use rbl_library::Touched;
        let mut tracks = named.to_vec();
        match touched {
            Touched::Tracks if named.is_empty() => return Self::Uncounted,
            Touched::Playlists | Touched::TagList | Touched::Tracks => {}
            Touched::Metadata(ids) | Touched::Histories(ids) => tracks.extend(ids.iter().cloned()),
            Touched::Cues(id) => tracks.push(id.clone()),
        }
        Self::Tracks(tracks)
    }
}

/// Where the Library and the Sidecar stood when a backup was staged.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Mark {
    update_count: Option<i64>,
    generation: u32,
    ledger: i64,
    /// The newest Run and how many have ended: a Run begun or ended by
    /// background work (a separation) writes no Ledger entry.
    runs: (i64, i64),
}

struct Pending {
    checked: Checked,
    mark: Option<Mark>,
    /// What was counted, for a backup taken again.
    planned: Planned,
    /// Decided by a planned Run's command ([`before_write`]) for all its
    /// writes, not by its write ([`before_joined_write`]).
    by_command: bool,
}

thread_local! {
    static DECIDED: RefCell<Option<Pending>> = const { RefCell::new(None) };
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

/// The tracks the earlier writes of the Run this command joined changed,
/// and the backup one of its commands took first, if any. Nothing for a
/// Run of the command's own: it has no earlier writes.
fn run_so_far(stems: &Stems) -> Result<(HashSet<String>, Option<PathBuf>), ServiceError> {
    let mut tracks = HashSet::new();
    let mut backup = None;
    let Some(run) = crate::ledger::joined_run() else {
        return Ok((tracks, backup));
    };
    let entries = stems.with_sidecar_wait(|open| {
        open.map_or(Ok(Vec::new()), |sidecar| {
            sidecar
                .run_entries(run, usize::MAX >> 1, 0)
                .map(|(entries, _)| entries)
        })
    })?;
    let entries =
        entries.map_err(|e| ServiceError::new(service_codes::SERVICE_FAILED, e.to_string()))?;
    for entry in entries {
        if let Some(path) = entry
            .command
            .as_ref()
            .and_then(|command| command["args"]["large_run"]["backup"]["path"].as_str())
        {
            backup = Some(PathBuf::from(path));
        }
        let Some(payload) = &entry.payload else {
            continue;
        };
        for list in ["edits", "tags"] {
            for item in payload[list].as_array().into_iter().flatten() {
                if let Some(track) = item["track"].as_str() {
                    tracks.insert(track.to_owned());
                }
            }
        }
    }
    Ok((tracks, backup))
}

/// Before the write of a Run that will change `tracks`: counts the Run with
/// its earlier writes, takes the backup when this command takes it over
/// the threshold, and records what was decided with the command. Outside
/// the edit gate; [`bound`] then checks the backup is still of the moment
/// before the write. An error stops the Run with nothing changed.
pub(crate) fn before_write(
    library: &OpenLibrary,
    stems: &Stems,
    tracks: impl IntoIterator<Item = String>,
) -> Result<(), ServiceError> {
    decide(
        library,
        stems,
        Planned::Tracks(tracks.into_iter().collect()),
        true,
    )
}

/// Before every write through the service's edit path of a command that
/// joined a named Run (wbx-25, review 2): the same count as
/// [`before_write`], of the Run so far and this write's tracks, and the
/// backup when this write takes the Run over the threshold. So a Run can
/// cross it by any command (`tracks.rate`, a My Tag edit), not only the
/// planned ones. The edit gate, held, for the write (taken first, then
/// the write takes it again), or `None` when the command joined no Run
/// or is a planned one that decided already.
pub(crate) fn before_joined_write(
    library: &OpenLibrary,
    planned: Planned,
) -> Result<Option<parking_lot::ReentrantMutexGuard<'_, ()>>, ServiceError> {
    let Some(stems) = crate::ledger::joined_sidecar() else {
        return Ok(None);
    };
    let decided = DECIDED.with(|decided| decided.borrow().as_ref().is_some_and(|p| p.by_command));
    if decided {
        return Ok(None);
    }
    decide(library, &stems, planned, false)?;
    checked_gate(library, &stems).map(Some)
}

fn decide(
    library: &OpenLibrary,
    stems: &Stems,
    planned: Planned,
    by_command: bool,
) -> Result<(), ServiceError> {
    let mine: HashSet<String> = match &planned {
        Planned::Tracks(tracks) => tracks.iter().cloned().collect(),
        Planned::Uncounted => HashSet::new(),
    };
    let uncounted = matches!(planned, Planned::Uncounted);
    let above = threshold(stems.sidecar_dir())?;
    let (mut run, run_backup) = run_so_far(stems)?;
    run.extend(mine.iter().cloned());
    let (backup, mark) = if (uncounted || run.len() > above) && run_backup.is_none() {
        let (taken, mark) = take(library, stems)?;
        hold("backed-up", library.for_backup().0);
        (Some(taken), Some(mark))
    } else {
        (None, None)
    };
    let checked = Checked {
        tracks: mine.len(),
        run_tracks: run.len(),
        above,
        backup,
        run_backup,
        uncounted,
    };
    let value = serde_json::to_value(&checked).unwrap_or(Value::Null);
    crate::ledger::note("large_run", value);
    DECIDED.with(|decided| {
        *decided.borrow_mut() = Some(Pending {
            checked,
            mark,
            planned,
            by_command,
        });
    });
    Ok(())
}

/// How many backups are taken, each checked, before the Run is refused.
const ATTEMPTS: usize = 3;

/// The edit gate, held, for the write of a Run [`before_write`] decided:
/// when it took a backup, the Library and the Sidecar are checked to be
/// where they stood when it was staged. If anything changed in between,
/// the backup is no longer of the moment before the Run: it is removed and
/// taken again. [`ATTEMPTS`] backups at most, each checked; when the last
/// is no longer of the moment either, it is removed too and the Run is
/// refused with nothing changed.
pub(crate) fn bound<'a>(
    library: &'a OpenLibrary,
    stems: &Stems,
    _tracks: &[String],
) -> Result<parking_lot::ReentrantMutexGuard<'a, ()>, ServiceError> {
    checked_gate(library, stems)
}

fn checked_gate<'a>(
    library: &'a OpenLibrary,
    stems: &Stems,
) -> Result<parking_lot::ReentrantMutexGuard<'a, ()>, ServiceError> {
    for attempt in 1..=ATTEMPTS {
        let guard = library.edit_gate().lock();
        let mark = DECIDED.with(|decided| decided.borrow().as_ref().and_then(|p| p.mark.clone()));
        let Some(mark) = mark else {
            return Ok(guard);
        };
        if mark_now(library, stems)? == mark {
            return Ok(guard);
        }
        drop(guard);
        let Some(stale) = DECIDED.with(|decided| decided.borrow_mut().take()) else {
            break;
        };
        if let Some(taken) = &stale.checked.backup {
            discard(library, &taken.path);
        }
        if attempt == ATTEMPTS {
            break;
        }
        decide(library, stems, stale.planned, stale.by_command)?;
    }
    Err(ServiceError::new(
        codes::BACKUP_FAILED,
        "the Library kept changing while this Run's backup was taken, so the backup could not be of the moment before it; nothing was changed: try again",
    ))
}

/// Where the Library and the Sidecar stand now. Under the edit gate.
fn mark_now(library: &OpenLibrary, stems: &Stems) -> Result<Mark, ServiceError> {
    let update_count = library.with_db(|db| {
        Ok(db
            .connection()
            .query_row(
                "SELECT int_1 FROM agentRegistry WHERE registry_id = 'localUpdateCount'",
                [],
                |r| r.get::<_, Option<i64>>(0),
            )
            .ok()
            .flatten())
    })?;
    let (ledger, runs) = sidecar_mark(stems)?
        .map_err(|e| ServiceError::new(service_codes::SERVICE_FAILED, e.to_string()))?;
    Ok(Mark {
        update_count,
        generation: library.generation(),
        ledger,
        runs,
    })
}

/// The Ledger's order position, and the newest Run and how many ended.
type SidecarMark = (i64, (i64, i64));

/// Where the Sidecar stands: its Ledger order position, and its Runs.
fn sidecar_mark(stems: &Stems) -> Result<wreckordbox_sidecar::Result<SidecarMark>, ServiceError> {
    stems.with_sidecar_wait(|open| {
        open.map_or(Ok((0, (0, 0))), |sidecar| {
            Ok((sidecar.ledger_position()?, sidecar.runs_position()?))
        })
    })
}

/// Removes a backup taken for this Run that is no longer of the moment
/// before it, when its folder may still be written; else it is kept.
fn discard(library: &OpenLibrary, path: &Path) {
    let state_dir = library.for_backup().0;
    let Some(folder) = path.parent() else { return };
    if let Ok(authorized) = wreckordbox_stick_protection::authorize(state_dir, folder) {
        let check = || {
            authorized.recheck().map_err(|refused| {
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, refused.to_string())
            })
        };
        if let Err(error) = rbl_backup::create::discard(path, &check) {
            tracing::warn!(%error, path = %path.display(), "a backup taken again was left");
        }
    }
}

/// A command's answer with what [`before_write`] decided in it (`large_run`
/// in a successful answer; a failure says the backup is kept). Call it on
/// every answer of a command that may call [`before_write`].
pub(crate) fn answered(result: Result<Value, ServiceError>) -> Result<Value, ServiceError> {
    let decided = DECIDED.with(|decided| decided.borrow_mut().take());
    let Some(Pending { checked, .. }) = decided else {
        return result;
    };
    match result {
        Ok(Value::Object(mut answer)) => {
            answer.insert("large_run".into(), json!(checked));
            Ok(Value::Object(answer))
        }
        Ok(other) => Ok(other),
        Err(mut error) => {
            if let Some(taken) = &checked.backup {
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

fn failed(destination: &Path, error: impl std::fmt::Display) -> ServiceError {
    ServiceError::new(
        codes::BACKUP_FAILED,
        format!(
            "this Run would change more tracks than the backup threshold, and its backup to {} failed: {error}; nothing was changed",
            destination.display()
        ),
    )
}

/// Takes the backup: both stores and the trees staged under the edit gate
/// and the analysis-file lock, then (with neither held) the archive
/// published from the staged copies under the folder's Stick
/// authorization. Lock order, as every backup writer's: the edit gate,
/// then a Stick, then the analysis-file lock; here the Stick is never held
/// with the other two.
fn take(library: &OpenLibrary, stems: &Stems) -> Result<(Taken, Mark), ServiceError> {
    let started = Instant::now();
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
    let sidecar_dir = stems.sidecar_dir();
    let sidecar_dir = sidecar_dir.is_dir().then_some(sidecar_dir);
    rbl_backup::create::refuse_overlap(&location, Some(stems.sidecar_dir()), &destination)
        .map_err(|e| failed(&destination, e))?;
    // A protected Stick is refused before anything is staged; the
    // authorization itself is taken for the archive, after the gate.
    if let Some(stick) = wreckordbox_stick_protection::stick_root(&destination) {
        if wreckordbox_stick_protection::status(&state_dir, &stick).protected {
            return Err(stick_refused(&wreckordbox_stick_protection::refusal(
                &stick,
            )));
        }
    }
    let gated = Instant::now();
    let (staged, mark) = {
        let _gate = library.edit_gate().lock();
        let _files = analysis_files.lock();
        // Before the Sidecar is read: a Fact recorded, or a Run begun or
        // ended, meanwhile moves it.
        let (ledger, runs) = sidecar_mark(stems)?.map_err(|e| failed(&destination, e))?;
        let staged = rbl_backup::create::stage(&rbl_backup::create::Stage {
            location: &location,
            sidecar_dir,
            sidecar: rbl_backup::sidecar_part::Options::saved(&state_dir),
            scratch: &state_dir,
            link_fault: fault().as_deref() == Some("link_fails"),
            at: &|point| hold(point, &state_dir),
        })
        .map_err(|e| failed(&destination, e))?;
        let mark = Mark {
            update_count: staged.update_count,
            generation: library.generation(),
            ledger,
            runs,
        };
        (staged, mark)
    };
    let gate_ms = millis(gated);
    let published = Instant::now();
    let authorized = wreckordbox_stick_protection::authorize(&state_dir, &destination)
        .map_err(|why| stick_refused(&why.to_string()))?;
    hold("stick-held", &state_dir);
    let fault = fault();
    let writes = std::cell::Cell::new(0_u64);
    let failed_once = std::cell::Cell::new(false);
    let check = || -> std::io::Result<()> {
        authorized.recheck().map_err(|refused| {
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, refused.to_string())
        })?;
        writes.set(writes.get() + 1);
        if fault.as_deref() == Some("disk_full") && writes.get() > 3 && !failed_once.replace(true) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::StorageFull,
                "No space left on device (test fault)",
            ));
        }
        Ok(())
    };
    let made = rbl_backup::create::publish(
        &staged,
        &rbl_backup::create::Publish {
            destination: &destination,
            check: &check,
            app_version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        },
    )
    .map_err(|e| failed(&destination, e))?;
    drop(authorized);
    let publish_ms = millis(published);
    Ok((
        Taken {
            path: made.path,
            created_at: made.created_at,
            bytes: made.bytes,
            took_ms: millis(started),
            gate_ms,
            publish_ms,
        },
        mark,
    ))
}

/// The test fault asked for ([`FAULT_ENV`]), in test mode only.
fn fault() -> Option<String> {
    rbl_db::test_mode()
        .then(|| std::env::var(FAULT_ENV).ok())
        .flatten()
}

fn stick_refused(why: &str) -> ServiceError {
    ServiceError::new(
        service_codes::STICK_PROTECTED,
        format!(
            "this Run would change more tracks than the backup threshold, and its backup could not be made: {why}; nothing was changed"
        ),
    )
}

fn millis(since: Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// A folder in the state folder in which a test puts an empty file named
/// for a moment of the backup (`sidecar-snapshotted`, `between-stores`,
/// `stick-held`, `backed-up`): a backup of that state folder reaching the
/// moment makes `<moment>.reached` there and waits until the test removes
/// `<moment>` (released for good) or `<moment>.reached` (released this
/// once; the next backup to reach it is held again), two minutes at most. Test mode only; scoped by state folder, so tests
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
    let reached = dir.join(format!("{point}.reached"));
    let _ = std::fs::write(&reached, b"");
    let deadline = Instant::now() + std::time::Duration::from_secs(120);
    while armed.exists() && reached.exists() && Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
