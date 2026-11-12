//! Backups from outside the window: `wreckordbox backup restore` and
//! `wreckordbox backup list`, and the recovery every owner of the Library
//! runs before it opens it.
//!
//! A restore replaces the Library's files and the Sidecar's while holding
//! the Library's ownership lock, the one a service (by itself or hosted by
//! the window) holds for as long as it has the Library. So a restore is
//! refused while either runs, neither can start until the restore has
//! finished, and a restore that stopped midway is finished or rolled back
//! by the next owner before it opens either store. What a restore checks,
//! and its journal, are `rbl_backup::restore`'s.
//!
//! Making a backup is the window's (Preferences › Backups); there is no
//! `backup create` here yet.

use std::path::{Path, PathBuf};

use rbl_backup::restore::{Parts, Phase, Request};
use rbl_db::LibraryLocation;
use serde::{Deserialize, Serialize};

use crate::{codes, Config, ServiceError};

/// What `backup restore` put back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Restored {
    /// The backup restored.
    pub archive: PathBuf,
    /// The Library's `master.db`.
    pub library: PathBuf,
    /// The Sidecar's folder, when the backup held a Sidecar and it was
    /// restored with the Library. `None` for a backup made before backups
    /// carried the Sidecar, which leaves the Sidecar as it was.
    pub sidecar: Option<PathBuf>,
    /// Whether Stem audio was in the backup. Without it, a track whose Stem
    /// audio is not on this machine needs separating again.
    pub stems: bool,
    /// The Ledger entry that records the restore, when there is a Sidecar
    /// to record it in. `None` also when it could not be written; the
    /// restore stands either way.
    pub ledger_entry: Option<i64>,
}

/// The kind of the Ledger entry a restore writes.
pub const RESTORED: &str = "restore";

/// One backup of this Library, as `backup list` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backup {
    pub path: PathBuf,
    /// Unix milliseconds.
    pub created_at: u64,
    /// The archive's size on disk.
    pub bytes: u64,
    /// Whether it holds the Sidecar.
    pub sidecar: bool,
    /// Whether it holds Stem audio.
    pub stems: bool,
}

/// The phase, bytes unpacked so far, bytes expected, and what is being
/// worked on.
pub type Progress<'a> = dyn FnMut(&str, u64, u64, &str) + 'a;

/// Finishes or rolls back a restore that stopped midway, if there is one.
/// The caller must own the Library (hold its lock) and must not have opened
/// the Library or the Sidecar yet.
pub(crate) fn recover(recovery_dir: &Path, library: &LibraryLocation) -> Result<(), ServiceError> {
    if !rbl_backup::journal::pending(recovery_dir) {
        return Ok(());
    }
    rbl_backup::journal::recover(recovery_dir, library).map_err(|e| {
        ServiceError::new(
            codes::RESTORE_UNFINISHED,
            format!("a restore of the Library stopped midway and could not be finished: {e}"),
        )
    })
}

/// Restores the whole of the backup at `archive` as the Library at
/// `library`, and its Sidecar with it when the backup holds one.
///
/// Refused, with nothing changed, while a service or the window owns the
/// Library ([`codes::RESTORE_REFUSED`]); the Library's lock is held from
/// before anything is read until the restore has finished or been put back.
pub fn restore(
    library: &LibraryLocation,
    config: &Config,
    archive: &Path,
    progress: &mut Progress<'_>,
) -> Result<Restored, ServiceError> {
    let refused = |message: String| ServiceError::new(codes::RESTORE_REFUSED, message);
    let _owned = crate::server::take_lock(&library.master_db, Path::new("restoring a backup"))
        .map_err(|error| {
            if error.code == codes::SERVICE_ALREADY_RUNNING {
                refused(format!(
                    "the Library is in use: {}. Quit the wreckordbox window, or stop the Library service with `wreckordbox stop`, then restore.",
                    error.message
                ))
            } else {
                error
            }
        })?;
    let contents = rbl_backup::sidecar_part::Contents::read(archive).map_err(|e| from_backup(&e))?;
    let manifest = rbl_backup::manifest::Manifest::read(archive).map_err(|e| from_backup(&e))?;
    rbl_backup::restore::restore(
        &Request {
            archive,
            parts: Parts::ALL,
            location: library,
            state_dir: &config.recovery_dir,
            sizes: None,
        },
        // The lock is held throughout: nothing else can have the Library.
        &|| Ok(()),
        &mut |phase, done, total, item| {
            progress(phase_name(phase), done, total, item);
            Ok(())
        },
    )
    .map_err(|e| from_backup(&e))?;
    let sidecar = match &contents {
        Some(_) => Some(
            rbl_backup::sidecar_part::Destination::of(library)
                .map_err(|e| from_backup(&e))?
                .path,
        ),
        None => None,
    };
    // Still holding the lock: the next owner reads the Ledger with the
    // restore in it.
    let ledger_entry = note_restore(library, archive, manifest.created_at, sidecar.is_some());
    Ok(Restored {
        archive: archive.to_path_buf(),
        library: library.master_db.clone(),
        sidecar,
        stems: contents.is_some_and(|contents| contents.stems),
        ledger_entry,
    })
}

/// Writes, in the Sidecar the Library now has, a Ledger entry saying the
/// Library was replaced by a backup's, in a Run of its own.
///
/// The Ledger and the Library must not disagree silently after a restore.
/// When the Sidecar came back with the Library, its entries and intents are
/// those the backup was taken with: a backup reads the Sidecar before the
/// Library, so every entry in it is of a change the restored Library has,
/// and an intent still open there becomes an unconfirmed entry, as after
/// any stop. When the backup held no Sidecar (one made before backups
/// carried it), the Ledger kept here describes changes the restored Library
/// may not have; this entry says so, after every one of them, and the
/// conflict check of Undo and Unwind refuses to take back what is not
/// there. Nothing is written when there is no Sidecar; a failure is logged
/// and leaves the restore as it is.
fn note_restore(library: &LibraryLocation, archive: &Path, created_at: u64, with_sidecar: bool) -> Option<i64> {
    use wreckordbox_sidecar::ledger::{Action, NewEntry, Store};
    let folder = rbl_backup::sidecar_part::Destination::of(library).ok()?.path;
    if !folder.join(rbl_backup::sidecar_part::DATABASE_FILE).is_file() {
        return None;
    }
    let name = archive.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let label = if with_sidecar {
        format!("The Library and the Sidecar were restored from the backup {name}.")
    } else {
        format!(
            "The Library was restored from the backup {name}, which holds no Sidecar. Entries before this one may describe changes the restored Library does not have."
        )
    };
    let payload = serde_json::json!({ "backup": name, "created_at": created_at, "sidecar": with_sidecar });
    let command = serde_json::json!({ "name": "backup.restore", "args": { "backup": name } });
    let written = (|| -> wreckordbox_sidecar::Result<i64> {
        let sidecar = wreckordbox_sidecar::Sidecar::open(&folder)?;
        let source = wreckordbox_sidecar::Source::new(wreckordbox_sidecar::SourceKind::Dj, "dj");
        let run = sidecar.begin_owned_run(&source, "Restore a backup")?;
        let id = sidecar.append(&NewEntry {
            id: None,
            run,
            store: Store::Library,
            source: &source,
            at: None,
            action: Action::Write,
            kind: RESTORED,
            label: &label,
            command: Some(&command),
            reverses: None,
            part_of: None,
            reversible: false,
            format: 1,
            payload: Some(&payload),
        })?;
        sidecar.finish_run(run, "restored")?;
        Ok(id)
    })();
    match written {
        Ok(id) => Some(id),
        Err(error) => {
            tracing::warn!(%error, "the restore could not be recorded in the Ledger");
            None
        }
    }
}

/// The backups of `library` in the backup folder the window saves new ones
/// in, newest first.
pub fn list(library: &LibraryLocation, config: &Config) -> Result<Vec<Backup>, ServiceError> {
    let folder = rbl_backup::default_destination(&config.recovery_dir);
    let entries = match std::fs::read_dir(&folder) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(ServiceError::new(
                codes::SERVICE_FAILED,
                format!("could not read the backup folder {}: {e}", folder.display()),
            ))
        }
    };
    let mut backups = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "zip") {
            continue;
        }
        // Another Library's backup, or no backup of ours.
        let Ok(manifest) = rbl_backup::manifest::Manifest::read(&path) else {
            continue;
        };
        if !manifest.belongs_to(library) {
            continue;
        }
        let contents = rbl_backup::sidecar_part::Contents::read(&path).ok().flatten();
        backups.push(Backup {
            bytes: entry.metadata().map(|meta| meta.len()).unwrap_or(0),
            path,
            created_at: manifest.created_at,
            sidecar: contents.is_some(),
            stems: contents.is_some_and(|contents| contents.stems),
        });
    }
    backups.sort_by(|a, b| b.created_at.cmp(&a.created_at).then_with(|| b.path.cmp(&a.path)));
    Ok(backups)
}

fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Preparing => "preparing",
        Phase::Unpacking => "unpacking",
        Phase::Validating => "validating",
        Phase::Replacing => "replacing",
    }
}

fn from_backup(error: &rbl_backup::Error) -> ServiceError {
    match error {
        rbl_backup::Error::Refused(message) => ServiceError::new(codes::RESTORE_REFUSED, message.clone()),
        rbl_backup::Error::Cancelled => ServiceError::new(codes::RESTORE_REFUSED, error.to_string()),
        other => ServiceError::new(codes::RESTORE_FAILED, other.to_string()),
    }
}
