//! The Ledger receipt an analysis-journal recovery hands off (wbx-14.15).
//!
//! A file journal (`file_journal`) names no Ledger intent, so when recovery
//! publishes or puts back an interrupted edit's files, nothing else would
//! record that it did. Before it changes a file, recovery hands off a
//! receipt under its own source, `tool:wreckordbox-recovery` (never the
//! source of the edit it finishes, which the journal does not know), named
//! by the journal: handing it off again, after a recovery that stopped,
//! replaces the same receipt, and a journal whose files are already in
//! place changes nothing and hands off none. `wreckordbox backup receipts
//! --record` records it.
use std::path::{Path, PathBuf};

use wreckordbox_service::ledger_receipts::{self, Receipt};

use crate::error::{AppError, AppResult};

/// How many file paths a receipt names; the count is always given.
const MAX_FILES: usize = 16;

fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_millis()),
    )
    .unwrap_or(i64::MAX)
}

/// A path as the receipt names it: relative to the Library's share folder
/// when it is inside it, else only its file name.
fn shown(location: &rbl_db::LibraryLocation, path: &Path) -> String {
    path.strip_prefix(&location.share_root).map_or_else(
        |_| {
            path.file_name()
                .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
        },
        |relative| relative.to_string_lossy().into_owned(),
    )
}

/// Hands off the receipt for recovering the journal in `journal`, which
/// will publish (`published`) or put back its files; `changes` are the
/// files that will change, each with whether it exists afterwards.
pub fn hand_off(
    root: &Path,
    location: &rbl_db::LibraryLocation,
    journal: &Path,
    track: &str,
    published: bool,
    changes: &[(PathBuf, bool)],
) -> AppResult<()> {
    if changes.is_empty() {
        return Ok(());
    }
    let name = journal
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let outcome = if published {
        "published"
    } else {
        "rolled_back"
    };
    let files: Vec<_> = changes
        .iter()
        .take(MAX_FILES)
        .map(|(path, exists)| serde_json::json!({ "path": shown(location, path), "exists_after": exists }))
        .collect();
    let receipt = Receipt {
        format: ledger_receipts::FORMAT,
        operation: format!("analysis-journal-{name}"),
        kind: ledger_receipts::JOURNAL_RECOVERY.to_owned(),
        source: ledger_receipts::RECOVERY.to_owned(),
        library: location.master_db.clone(),
        at: now_ms(),
        outcome: outcome.to_owned(),
        uncertainty: format!(
            "The journal names no Ledger intent, so the interrupted edit's own source and Run are not known from it; any intent that edit began is settled separately. Recovery {} the files from the journal, judged from the Library row's state.",
            if published { "published" } else { "put back" }
        ),
        details: serde_json::json!({
            "track": track.chars().take(64).collect::<String>(),
            "files": files,
            "file_count": changes.len(),
        }),
        reserved: None,
    };
    ledger_receipts::hand_off(root, &receipt).map(|_| ()).map_err(|e| {
        AppError::internal(format!(
            "Analysis recovery: the Ledger receipt for recovering {} could not be written, so nothing was recovered: {e}",
            journal.display()
        ))
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use crate::file_journal::{recover, FileJournal};
    use wreckordbox_service::ledger_receipts;

    struct Rig {
        _root: tempfile::TempDir,
        location: rbl_db::LibraryLocation,
        state: std::path::PathBuf,
        dat: std::path::PathBuf,
        track: String,
    }

    /// A fixture Library whose track's DAT is under its share folder.
    fn rig() -> Rig {
        let root = tempfile::tempdir().unwrap();
        let location =
            rbl_db::fixture::build(root.path(), rbl_db::fixture::Shape::default()).unwrap();
        let dat = location.share_root.join("PIONEER/USBANLZ/aa/ANLZ0000.DAT");
        std::fs::create_dir_all(dat.parent().unwrap()).unwrap();
        std::fs::write(&dat, b"old dat").unwrap();
        Rig {
            state: root.path().join("backups"),
            location,
            dat,
            track: rbl_db::fixture::track_id(0),
            _root: root,
        }
    }

    fn receipts(rig: &Rig) -> Vec<ledger_receipts::Receipt> {
        ledger_receipts::list(&rig.state, &rig.location.master_db)
            .unwrap()
            .into_iter()
            .map(|p| {
                assert_eq!(p.error, None);
                p.receipt.unwrap()
            })
            .collect()
    }

    /// An interrupted edit: files published, the row never written.
    fn interrupted(rig: &Rig) -> std::path::PathBuf {
        let journal = FileJournal::prepare(
            &rig.state,
            &rig.location,
            &rig.track,
            12345,
            None,
            true,
            &[
                (rig.dat.clone(), b"new dat".to_vec()),
                (rig.dat.with_extension("EXT"), b"new ext".to_vec()),
            ],
        )
        .unwrap();
        journal.publish().unwrap();
        let dir = std::fs::read_dir(rig.state.join("analysis-journal"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        drop(journal);
        dir
    }

    fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }

    #[test]
    fn recovering_a_journal_hands_off_one_receipt_under_the_recovery_tool_with_bounded_targets() {
        let rig = rig();
        let dir = interrupted(&rig);
        recover(&rig.state, &rig.location).unwrap();
        assert_eq!(std::fs::read(&rig.dat).unwrap(), b"old dat");

        let found = receipts(&rig);
        assert_eq!(found.len(), 1, "{found:?}");
        let receipt = &found[0];
        assert_eq!(receipt.kind, "analysis_journal_recovery");
        assert_eq!(receipt.source, "wreckordbox-recovery");
        assert_eq!(receipt.outcome, "rolled_back");
        assert!(
            receipt.uncertainty.contains("names no Ledger intent"),
            "{}",
            receipt.uncertainty
        );
        assert_eq!(
            receipt.operation,
            format!(
                "analysis-journal-{}",
                dir.file_name().unwrap().to_string_lossy()
            )
        );
        assert_eq!(receipt.details["track"], rig.track);
        assert_eq!(receipt.details["file_count"], 2);
        assert_eq!(
            receipt.details["files"][0]["path"],
            "PIONEER/USBANLZ/aa/ANLZ0000.DAT"
        );
        assert_eq!(receipt.details["files"][1]["exists_after"], false);

        // Recorded once, as the recovery tool's, and never twice.
        let sidecar = rig.state.join("sidecar");
        let done = ledger_receipts::record(&rig.state, &rig.location.master_db, &sidecar).unwrap();
        assert_eq!(done.recorded.len(), 1);
        let store = wreckordbox_sidecar::Sidecar::open(&sidecar).unwrap();
        let entry = store.ledger_entry(done.recorded[0].entry).unwrap().unwrap();
        assert_eq!(entry.kind, "analysis_journal_recovery");
        assert_eq!(
            (entry.source.kind, entry.source.name.as_str()),
            (
                wreckordbox_sidecar::SourceKind::Tool,
                "wreckordbox-recovery"
            )
        );
        assert!(entry.label.contains("rolled back"), "{}", entry.label);
        assert!(!entry.reversible);
        assert_eq!(store.runs(10, 0).unwrap().1, 1);
    }

    #[test]
    fn a_recovery_that_stopped_after_installing_replays_without_a_second_receipt() {
        let rig = rig();
        let dir = interrupted(&rig);
        let kept = rig.state.join("kept-journal");
        copy_dir(&dir, &kept);
        recover(&rig.state, &rig.location).unwrap();
        let first = receipts(&rig);
        assert_eq!(first.len(), 1);
        // As if it stopped before removing the journal: run it again.
        copy_dir(&kept, &dir);
        recover(&rig.state, &rig.location).unwrap();
        assert_eq!(receipts(&rig), first, "the same receipt, no other");
        assert!(!dir.exists());
    }

    #[test]
    fn a_recovery_refused_for_an_external_change_changes_nothing_and_hands_off_nothing() {
        let rig = rig();
        let dir = interrupted(&rig);
        std::fs::write(&rig.dat, b"external").unwrap();
        assert!(recover(&rig.state, &rig.location).is_err());
        assert_eq!(std::fs::read(&rig.dat).unwrap(), b"external");
        assert!(dir.exists());
        assert!(receipts(&rig).is_empty());
    }

    #[test]
    fn a_receipt_that_cannot_be_handed_off_stops_recovery_before_any_file_changes() {
        use std::os::unix::fs::PermissionsExt as _;
        let rig = rig();
        let dir = interrupted(&rig);
        let folder = ledger_receipts::folder(&rig.state);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = recover(&rig.state, &rig.location);
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(result.is_err());
        assert_eq!(
            std::fs::read(&rig.dat).unwrap(),
            b"new dat",
            "left as the interrupted edit left it"
        );
        assert!(dir.exists(), "the journal is kept for the next attempt");
    }
}
