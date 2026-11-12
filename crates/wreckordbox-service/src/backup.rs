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
    /// to record it in.
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

/// What the Library's lock file says while a restore holds it.
pub const RESTORING: &str = "restoring a backup";

/// The Library owned for a restore: its ownership lock, the one every
/// service (by itself or hosted by the window) holds for as long as it has
/// the Library, with [`RESTORING`] written in it. Released when dropped.
pub struct Owned {
    _lock: std::fs::File,
}

impl Owned {
    /// Takes the Library for a restore. Refused ([`codes::RESTORE_REFUSED`])
    /// while a service, the window or another restore has it.
    pub fn take(library: &LibraryLocation) -> Result<Self, ServiceError> {
        let lock = crate::server::take_lock(&library.master_db, Path::new(RESTORING)).map_err(|error| {
            if error.code == codes::SERVICE_ALREADY_RUNNING {
                ServiceError::new(
                    codes::RESTORE_REFUSED,
                    format!(
                        "the Library is in use: {}. Quit the wreckordbox window, or stop the Library service with `wreckordbox stop`, then restore.",
                        error.message
                    ),
                )
            } else {
                error
            }
        })?;
        Ok(Self { _lock: lock })
    }
}

/// Whether a restore owns the Library now: its lock is held, and what the
/// holder wrote in it is [`RESTORING`]. A window that cannot host the
/// service asks this to know it must wait rather than read.
#[must_use]
pub fn restoring(library: &LibraryLocation) -> bool {
    use std::io::Read as _;
    let Ok(mut file) = std::fs::File::open(crate::lock_path(&library.master_db)) else {
        return false;
    };
    let mut note = String::new();
    if file.read_to_string(&mut note).is_err() || note.lines().nth(1) != Some(RESTORING) {
        return false;
    }
    // Held, by someone: a shared try fails while the holder has it.
    match file.try_lock_shared() {
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(_) => true,
    }
}

/// Finishes or rolls back a restore that stopped midway, if there is one,
/// and writes the Ledger entry a finished restore still owes. The caller
/// must own the Library (hold its lock) and must not have opened the
/// Library or the Sidecar yet. Until both are done, the journal stays and
/// the Library is not opened ([`codes::RESTORE_UNFINISHED`]).
pub(crate) fn recover(recovery_dir: &Path, library: &LibraryLocation) -> Result<(), ServiceError> {
    if !rbl_backup::journal::pending(recovery_dir) {
        return Ok(());
    }
    rbl_backup::journal::recover(recovery_dir, library).map_err(|e| {
        ServiceError::new(
            codes::RESTORE_UNFINISHED,
            format!("a restore of the Library stopped midway and could not be finished: {e}"),
        )
    })?;
    record_owed(recovery_dir).map(|_| ())
}

/// The record a restore owes the Ledger, kept in its journal
/// (`rbl_backup::restore::restore_owing`) from before anything is replaced
/// until it is written.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Owed {
    /// The backup's file name.
    backup: String,
    /// When the backup was made, Unix milliseconds.
    created_at: u64,
    /// Whether the Sidecar came back with the Library.
    sidecar: bool,
    /// The Sidecar's folder the entry is written in, checked when the
    /// restore began; `None` when there was none.
    folder: Option<PathBuf>,
    /// The entry's identifier, reserved by an intent once the files are
    /// in place: appending under it again changes nothing.
    #[serde(default)]
    id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Owing {
    ledger: Owed,
}

/// Writes the Ledger entry a finished restore owes, if it owes one, and
/// lets go of its journal. Safe to repeat: the entry's identifier is
/// reserved and saved before it is appended.
fn record_owed(recovery_dir: &Path) -> Result<Option<i64>, ServiceError> {
    let unrecorded = |why: &dyn std::fmt::Display| {
        ServiceError::new(
            codes::RESTORE_UNRECORDED,
            format!(
                "the backup was restored, but the Ledger entry recording it could not be written: {why}. The Library stays closed to edits until it is; open wreckordbox again to retry"
            ),
        )
    };
    let Some(owed) = rbl_backup::journal::owed(recovery_dir).map_err(|e| unrecorded(&e))? else {
        return Ok(None);
    };
    let Ok(Owing { mut ledger }) = serde_json::from_value::<Owing>(owed) else {
        // Owed by another program: not ours to write.
        return Err(unrecorded(
            &"the journal owes something this wreckordbox does not know",
        ));
    };
    let id = match ledger.folder.clone().as_deref() {
        Some(folder)
            if folder
                .join(rbl_backup::sidecar_part::DATABASE_FILE)
                .is_file() =>
        {
            Some(write_entry(recovery_dir, folder, &mut ledger).map_err(|e| unrecorded(&e))?)
        }
        // No Sidecar to record it in.
        _ => None,
    };
    rbl_backup::journal::discharge(recovery_dir).map_err(|e| unrecorded(&e))?;
    Ok(id)
}

fn write_entry(recovery_dir: &Path, folder: &Path, ledger: &mut Owed) -> Result<i64, String> {
    use wreckordbox_sidecar::ledger::{Action, Intent, NewEntry, Store};
    let sidecar = wreckordbox_sidecar::Sidecar::open(folder).map_err(|e| e.to_string())?;
    let source = wreckordbox_sidecar::Source::new(wreckordbox_sidecar::SourceKind::Dj, "dj");
    let command =
        serde_json::json!({ "name": "backup.restore", "args": { "backup": ledger.backup } });
    let id = if let Some(id) = ledger.id {
        id
    } else {
        {
            let at = i64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |since| since.as_millis()),
            )
            .unwrap_or(i64::MAX);
            let id = sidecar
                .begin_intent(
                    &Intent {
                        source: &source,
                        run: None,
                        description: "Restore a backup",
                        command: Some(&command),
                        at,
                    },
                    &[],
                )
                .map_err(|e| e.to_string())?;
            ledger.id = Some(id);
            rbl_backup::journal::update_owed(
                recovery_dir,
                serde_json::json!(Owing {
                    ledger: ledger.clone()
                }),
            )
            .map_err(|e| e.to_string())?;
            id
        }
    };
    if sidecar
        .ledger_entry(id)
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Ok(id);
    }
    let label = if ledger.sidecar {
        format!(
            "The Library and the Sidecar were restored from the backup {}.",
            ledger.backup
        )
    } else {
        format!(
            "The Library was restored from the backup {}, which holds no Sidecar. Entries before this one may describe changes the restored Library does not have.",
            ledger.backup
        )
    };
    let payload = serde_json::json!({ "backup": ledger.backup, "created_at": ledger.created_at, "sidecar": ledger.sidecar });
    let run = sidecar
        .begin_owned_run(&source, "Restore a backup")
        .map_err(|e| e.to_string())?;
    sidecar
        .append(&NewEntry {
            id: Some(id),
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
        })
        .map_err(|e| e.to_string())?;
    sidecar
        .finish_run(run, "restored")
        .map_err(|e| e.to_string())?;
    Ok(id)
}

/// Restores the whole of the backup at `archive` as the Library at
/// `library`, and its Sidecar with it when the backup holds one, and
/// records it in the Ledger.
///
/// Refused, with nothing changed, while a service or the window owns the
/// Library ([`codes::RESTORE_REFUSED`]) and while Library Protection is on
/// or has never been recorded ([`codes::LIBRARY_PROTECTED`]): it is checked
/// under the lock before anything is staged and again before anything is
/// replaced. The Library's lock is held from before anything is read until
/// the restore has finished, or been put back, and been recorded. The
/// Ledger entry is owed in the restore's journal from before the first file
/// is replaced, so a restore that stops after its files are in place is
/// recorded by the next owner; a failure to record it fails the restore
/// ([`codes::RESTORE_UNRECORDED`]) and keeps the journal.
pub fn restore(
    library: &LibraryLocation,
    config: &Config,
    archive: &Path,
    progress: &mut Progress<'_>,
) -> Result<Restored, ServiceError> {
    let _owned = Owned::take(library)?;
    restore_owned(library, config, archive, progress)
}

/// [`restore`], for a caller already holding [`Owned`].
pub fn restore_owned(
    library: &LibraryLocation,
    config: &Config,
    archive: &Path,
    progress: &mut Progress<'_>,
) -> Result<Restored, ServiceError> {
    let protected = || crate::protection::is_on(&config.recovery_dir);
    if protected() {
        return Err(protection_refusal());
    }
    // An earlier restore is finished or put back, and recorded, first.
    recover(&config.recovery_dir, library)?;
    let contents =
        rbl_backup::sidecar_part::Contents::read(archive).map_err(|e| from_backup(&e))?;
    let manifest = rbl_backup::manifest::Manifest::read(archive).map_err(|e| from_backup(&e))?;
    let folder = rbl_backup::sidecar_part::Destination::of(library)
        .ok()
        .map(|destination| destination.path);
    let backup = archive
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let owed = Owing {
        ledger: Owed {
            backup,
            created_at: manifest.created_at,
            sidecar: contents.is_some(),
            folder: folder.clone(),
            id: None,
        },
    };
    rbl_backup::restore::restore_owing(
        &Request {
            archive,
            parts: Parts::ALL,
            location: library,
            state_dir: &config.recovery_dir,
            sizes: None,
        },
        Some(serde_json::json!(owed)),
        // The lock keeps every other owner out; Library Protection is the
        // DJ's, and can be turned while this runs.
        &|| {
            if protected() {
                Err(rbl_backup::Error::Refused(
                    crate::protection::MESSAGE.to_owned(),
                ))
            } else {
                Ok(())
            }
        },
        &mut |phase, done, total, item| {
            progress(phase_name(phase), done, total, item);
            Ok(())
        },
    )
    .map_err(|e| {
        if protected() {
            protection_refusal()
        } else {
            from_backup(&e)
        }
    })?;
    let ledger_entry = record_owed(&config.recovery_dir)?;
    Ok(Restored {
        archive: archive.to_path_buf(),
        library: library.master_db.clone(),
        sidecar: contents.as_ref().and(folder),
        stems: contents.is_some_and(|contents| contents.stems),
        ledger_entry,
    })
}

fn protection_refusal() -> ServiceError {
    ServiceError::new(
        codes::LIBRARY_PROTECTED,
        format!(
            "{} Restoring a backup replaces the Library, so it is refused too; --yes confirms the replacement, it does not turn protection off.",
            crate::protection::MESSAGE
        ),
    )
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
        let contents = rbl_backup::sidecar_part::Contents::read(&path)
            .ok()
            .flatten();
        backups.push(Backup {
            bytes: entry.metadata().map_or(0, |meta| meta.len()),
            path,
            created_at: manifest.created_at,
            sidecar: contents.is_some(),
            stems: contents.is_some_and(|contents| contents.stems),
        });
    }
    backups.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.path.cmp(&a.path))
    });
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
        rbl_backup::Error::Refused(message) => {
            ServiceError::new(codes::RESTORE_REFUSED, message.clone())
        }
        rbl_backup::Error::Cancelled => {
            ServiceError::new(codes::RESTORE_REFUSED, error.to_string())
        }
        other => ServiceError::new(codes::RESTORE_FAILED, other.to_string()),
    }
}
