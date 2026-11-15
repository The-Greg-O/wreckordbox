//! Ledger receipts: the durable record a Library write owes the Ledger
//! when the program that made it could not write the entry itself (wbx-14.15).
//!
//! Two writers hand one off:
//!
//! - the standalone restore scripts in every backup, when they restore a
//!   Library-only backup by themselves (no `wreckordbox` command to hand it
//!   to, or a Library path given by hand). Source `tool:restore-script`.
//! - the window's recovery of an interrupted analysis edit from its file
//!   journal ([`JOURNAL_RECOVERY`]), whose journal names no Ledger intent.
//!   Source `tool:wreckordbox-recovery`.
//!
//! A receipt is one JSON file in `<recovery dir>/ledger-receipts/`, named
//! by its operation identity. It is written before the files it describes
//! are changed, so a writer that stops midway still leaves it (its
//! `outcome` then says `begun`). Nothing records a receipt by itself:
//! `wreckordbox backup receipts` lists them without changing anything, and
//! `wreckordbox backup receipts --record` writes each as a named Ledger
//! entry, explicitly. Recording reserves its Run and entry in the receipt
//! before appending, so a recording that fails or stops is retried under
//! the same Run and entry identifier: never a second Run or entry.
//!
//! A receipt's source is always a tool, never the DJ, whatever the file
//! says: a file on disk cannot claim the DJ's authority. The entry is an
//! [`Action::Write`](wreckordbox_sidecar::ledger::Action::Write) with no
//! values: it records no Fact and gives Projection no ownership.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{codes, ServiceError};

/// The folder in the recovery directory receipts are kept in.
pub const FOLDER: &str = "ledger-receipts";
/// The version of a receipt's shape.
pub const FORMAT: u32 = 1;
/// The source name of a standalone restore script.
pub const RESTORE_SCRIPT: &str = "restore-script";
/// The source name of wreckordbox's own interrupted-edit recovery.
pub const RECOVERY: &str = "wreckordbox-recovery";
/// The entry kind of an analysis-journal recovery.
pub const JOURNAL_RECOVERY: &str = "analysis_journal_recovery";
/// The entry kind of a restore (the one `backup restore` writes too).
pub const RESTORE: &str = crate::backup::RESTORED;
/// The command named on a recorded receipt's entry.
pub const RECORD_COMMAND: &str = "backup.receipts.record";

/// The largest receipt file read, and the largest `details` kept.
const MAX_FILE: u64 = 64 * 1024;
const MAX_DETAILS: usize = 16 * 1024;

/// What a receipt says.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    pub format: u32,
    /// The operation's identity, unique per write: also the file's name.
    pub operation: String,
    /// The entry kind: [`RESTORE`] or [`JOURNAL_RECOVERY`].
    pub kind: String,
    /// The tool that made the write (its source kind is always `tool`).
    pub source: String,
    /// The Library's `master.db`.
    pub library: PathBuf,
    /// When the write began, Unix milliseconds.
    pub at: i64,
    /// How far it got: `begun`, `restored`, `failed`, `published` or
    /// `rolled_back`.
    pub outcome: String,
    /// Why the Ledger cannot know more than this receipt says.
    pub uncertainty: String,
    /// Bounded metadata about the write's targets.
    #[serde(default)]
    pub details: Value,
    /// The Run and entry reserved for it once recording began.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reserved: Option<Reserved>,
}

/// The Run and the entry a recording reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reserved {
    pub run: i64,
    #[serde(default)]
    pub entry: Option<i64>,
}

/// One receipt in the folder, as `backup receipts` lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    pub path: PathBuf,
    /// The receipt, when it could be read.
    pub receipt: Option<Receipt>,
    /// Why it cannot be recorded, when it cannot.
    pub error: Option<String>,
}

/// One receipt recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedReceipt {
    pub operation: String,
    pub run: i64,
    pub entry: i64,
}

/// What `backup receipts --record` did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Recorded {
    pub recorded: Vec<RecordedReceipt>,
    /// Receipts kept because they could not be recorded, with why.
    pub unrecorded: Vec<Pending>,
}

/// The receipt folder in `recovery_dir`.
#[must_use]
pub fn folder(recovery_dir: &Path) -> PathBuf {
    recovery_dir.join(FOLDER)
}

fn plain_word(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && !value.starts_with('.')
}

impl Receipt {
    /// Why this receipt cannot be recorded, if it cannot.
    fn problem(&self) -> Option<String> {
        if self.format != FORMAT {
            return Some(format!("receipt format {} is not known", self.format));
        }
        if !plain_word(&self.operation, 128) {
            return Some("the operation identity is not a plain name".to_owned());
        }
        if self.kind != RESTORE && self.kind != JOURNAL_RECOVERY {
            return Some(format!(
                "the kind {:?} is not one a receipt records",
                self.kind
            ));
        }
        if !plain_word(&self.source, 64) || self.source.contains('.') {
            return Some("the source is not a plain tool name".to_owned());
        }
        if self.uncertainty.trim().is_empty() {
            return Some("the receipt says nothing of what is uncertain".to_owned());
        }
        if self.details.to_string().len() > MAX_DETAILS {
            return Some("the receipt's details are too large".to_owned());
        }
        None
    }
}

fn path_of(recovery_dir: &Path, operation: &str) -> PathBuf {
    folder(recovery_dir).join(format!("{operation}.json"))
}

fn sync_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

fn save(path: &Path, receipt: &Receipt) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let temporary = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
        std::process::id()
    ));
    {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(receipt).map_err(std::io::Error::other)?)?;
        file.sync_all()?;
    }
    std::fs::rename(&temporary, path)?;
    sync_dir(dir)
}

/// Hands a receipt off, durably, before the write it describes changes
/// anything. Handing off the same operation again replaces what it says
/// but keeps a reservation already made, so it is never recorded twice.
///
/// # Errors
/// When the receipt is not one that could be recorded, or cannot be
/// written. The writer must then not make its write.
pub fn hand_off(recovery_dir: &Path, receipt: &Receipt) -> std::io::Result<PathBuf> {
    if let Some(problem) = receipt.problem() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            problem,
        ));
    }
    let path = path_of(recovery_dir, &receipt.operation);
    let mut receipt = receipt.clone();
    if let Some(Ok(earlier)) = read(&path) {
        receipt.reserved = receipt.reserved.or(earlier.reserved);
    }
    save(&path, &receipt)?;
    Ok(path)
}

fn read(path: &Path) -> Option<Result<Receipt, String>> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => return Some(Err(e.to_string())),
    };
    if !meta.file_type().is_file() {
        return Some(Err("not a plain file".to_owned()));
    }
    if meta.len() > MAX_FILE {
        return Some(Err("larger than a receipt can be".to_owned()));
    }
    Some(
        std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|e| e.to_string())),
    )
}

fn same_library(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

/// The receipts waiting for the Library at `master_db`, and every receipt
/// that cannot be read, oldest first. Only reads: nothing is created,
/// recorded or removed.
///
/// # Errors
/// When the folder exists but cannot be read.
pub fn list(recovery_dir: &Path, master_db: &Path) -> Result<Vec<Pending>, ServiceError> {
    let dir = folder(recovery_dir);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(ServiceError::new(
                codes::SERVICE_FAILED,
                format!(
                    "could not read the Ledger receipts in {}: {e}",
                    dir.display()
                ),
            ))
        }
    };
    let mut pending = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let stem = name.trim_end_matches(".json");
        match read(&path) {
            None => {}
            Some(Err(error)) => pending.push(Pending {
                path,
                receipt: None,
                error: Some(error),
            }),
            Some(Ok(receipt)) => {
                if !same_library(&receipt.library, master_db) {
                    continue;
                }
                let error = receipt.problem().or_else(|| {
                    (receipt.operation != stem)
                        .then(|| "the file's name is not the receipt's operation".to_owned())
                });
                pending.push(Pending {
                    path,
                    receipt: Some(receipt),
                    error,
                });
            }
        }
    }
    pending.sort_by(|a, b| {
        let at = |p: &Pending| p.receipt.as_ref().map_or(i64::MIN, |r| r.at);
        at(a).cmp(&at(b)).then_with(|| a.path.cmp(&b.path))
    });
    Ok(pending)
}

fn label(receipt: &Receipt) -> String {
    let what = match (receipt.kind.as_str(), receipt.outcome.as_str()) {
        (JOURNAL_RECOVERY, "published") => {
            "An interrupted analysis edit was finished from its journal: its files were published"
        }
        (JOURNAL_RECOVERY, _) => {
            "An interrupted analysis edit was rolled back from its journal: its files were put back"
        }
        (_, "restored") => "The Library was restored from a backup by its standalone restore script",
        (_, "failed") => {
            "A standalone restore script began restoring the Library from a backup and failed; the files it replaced were set aside"
        }
        _ => "A standalone restore script began restoring the Library from a backup; whether it finished is not known",
    };
    format!("{what}. {}", receipt.uncertainty.trim())
}

/// Records every waiting receipt of the Library at `master_db` in the
/// Sidecar in `sidecar`, as a named entry in a Run of its own, and removes
/// each receipt once its entry is in place. A receipt that cannot be
/// recorded is kept, with its reservation, and listed as unrecorded.
///
/// The caller must own the Library (hold its lock): the Sidecar is opened
/// here.
///
/// # Errors
/// When the receipts cannot be listed or the Sidecar cannot be opened.
pub fn record(
    recovery_dir: &Path,
    master_db: &Path,
    sidecar: &Path,
) -> Result<Recorded, ServiceError> {
    let pending = list(recovery_dir, master_db)?;
    let mut done = Recorded::default();
    if pending.is_empty() {
        return Ok(done);
    }
    let store = wreckordbox_sidecar::Sidecar::open(sidecar).map_err(|e| {
        ServiceError::new(
            codes::RESTORE_UNRECORDED,
            format!("the Sidecar the receipts are recorded in could not be opened: {e}"),
        )
    })?;
    for item in pending {
        let (Some(receipt), None) = (&item.receipt, &item.error) else {
            done.unrecorded.push(item);
            continue;
        };
        match record_one(&store, &item.path, receipt) {
            Ok(recorded) => done.recorded.push(recorded),
            Err(error) => done.unrecorded.push(Pending {
                error: Some(error),
                ..item
            }),
        }
    }
    Ok(done)
}

fn record_one(
    store: &wreckordbox_sidecar::Sidecar,
    path: &Path,
    receipt: &Receipt,
) -> Result<RecordedReceipt, String> {
    use wreckordbox_sidecar::ledger::{Action, Intent, NewEntry, Store};
    use wreckordbox_sidecar::{RunId, Source, SourceKind};
    let source = Source::new(SourceKind::Tool, receipt.source.as_str());
    let description = if receipt.kind == JOURNAL_RECOVERY {
        "Recover an interrupted analysis edit"
    } else {
        "Restore a backup with its standalone script"
    };
    let command = serde_json::json!({
        "name": RECORD_COMMAND,
        "args": { "operation": receipt.operation, "kind": receipt.kind },
    });
    let mut receipt = receipt.clone();
    let run = if let Some(reserved) = receipt.reserved {
        RunId(reserved.run)
    } else {
        // The reservation must be savable before a Run is made for it.
        save(path, &receipt).map_err(|e| e.to_string())?;
        let run = store
            .begin_owned_run(&source, description)
            .map_err(|e| e.to_string())?;
        receipt.reserved = Some(Reserved {
            run: run.0,
            entry: None,
        });
        save(path, &receipt).map_err(|e| e.to_string())?;
        run
    };
    let id = if let Some(id) = receipt.reserved.and_then(|r| r.entry) {
        id
    } else {
        let id = store
            .begin_intent(
                &Intent {
                    source: &source,
                    run: Some(run),
                    description,
                    command: Some(&command),
                    at: receipt.at,
                },
                &[],
            )
            .map_err(|e| e.to_string())?;
        receipt.reserved = Some(Reserved {
            run: run.0,
            entry: Some(id),
        });
        save(path, &receipt).map_err(|e| e.to_string())?;
        id
    };
    if store.ledger_entry(id).map_err(|e| e.to_string())?.is_none() {
        let payload = serde_json::json!({
            "operation": receipt.operation,
            "outcome": receipt.outcome,
            "uncertainty": receipt.uncertainty,
            "library": receipt.library,
            "details": receipt.details,
        });
        let label = label(&receipt);
        store
            .append(&NewEntry {
                id: Some(id),
                run,
                store: Store::Library,
                source: &source,
                at: Some(receipt.at),
                action: Action::Write,
                kind: &receipt.kind,
                label: &label,
                command: Some(&command),
                reverses: None,
                part_of: None,
                reversible: false,
                format: FORMAT,
                payload: Some(&payload),
            })
            .map_err(|e| e.to_string())?;
    }
    store
        .finish_run(run, &receipt.outcome)
        .map_err(|e| e.to_string())?;
    std::fs::remove_file(path).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        sync_dir(dir).map_err(|e| e.to_string())?;
    }
    Ok(RecordedReceipt {
        operation: receipt.operation,
        run: run.0,
        entry: id,
    })
}

/// [`record`] for the Library at `library`, in its own Sidecar, owning the
/// Library for as long as it takes: refused ([`codes::RESTORE_REFUSED`])
/// while a service, the window or a restore has it.
///
/// # Errors
/// As [`record`], and when the Library cannot be owned or its Sidecar's
/// folder is not usable.
pub fn record_owned(
    library: &rbl_db::LibraryLocation,
    config: &crate::Config,
) -> Result<Recorded, ServiceError> {
    let _owned = crate::backup::Owned::take(library)?;
    let sidecar = rbl_backup::sidecar_part::Destination::of(library)
        .map_err(|e| ServiceError::new(codes::RESTORE_UNRECORDED, e.to_string()))?
        .path;
    record(&config.recovery_dir, &library.master_db, &sidecar)
}
