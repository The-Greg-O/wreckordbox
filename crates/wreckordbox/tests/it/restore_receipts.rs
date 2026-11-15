//! Restores and recoveries that wreckordbox could not record when they
//! were made (wbx-14.15): a `backup restore` of a Library that had no
//! Sidecar, and the receipts the standalone restore scripts and the
//! window's analysis-journal recovery hand off, recorded explicitly with
//! `backup receipts --record`. The real binary against a throwaway Library;
//! never the real Library or real backups.

use crate::common;

use std::path::{Path, PathBuf};

use common::{Run, Sandbox};
use rbl_backup::testing;
use wreckordbox_service::ledger_receipts::{self, Receipt};
use wreckordbox_sidecar::ledger::Action;
use wreckordbox_sidecar::{Sidecar, SourceKind};

fn sidecar_db(sandbox: &Sandbox) -> PathBuf {
    sandbox.sidecar_dir().join("sidecar.db")
}

fn count(sandbox: &Sandbox, sql: &str) -> i64 {
    rusqlite::Connection::open(sidecar_db(sandbox))
        .unwrap()
        .query_row(sql, [], |row| row.get(0))
        .unwrap()
}

fn restore(sandbox: &Sandbox, archive: &Path) -> Run {
    sandbox.cli(&[
        "--json",
        "backup",
        "restore",
        archive.to_str().unwrap(),
        "--yes",
    ])
}

/// A receipt the way the standalone restore script hands one off.
fn script_receipt(sandbox: &Sandbox, operation: &str) -> Receipt {
    Receipt {
        format: ledger_receipts::FORMAT,
        operation: operation.to_owned(),
        kind: ledger_receipts::RESTORE.to_owned(),
        source: ledger_receipts::RESTORE_SCRIPT.to_owned(),
        library: sandbox.master_db().to_path_buf(),
        at: 1_763_000_000_000,
        outcome: "restored".to_owned(),
        uncertainty: "The standalone script restored the Library outside wreckordbox.".to_owned(),
        details: serde_json::json!({ "backup": "rbexport-20261112-1100" }),
        reserved: None,
    }
}

/// Every file under `dir` with its bytes, for "nothing changed".
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if !path.to_string_lossy().ends_with("-shm") {
                out.push((path.clone(), std::fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a_library_only_restore_with_no_sidecar_yet_is_recorded_in_a_sidecar_it_makes() {
    let sandbox = Sandbox::new();
    testing::set_rating(sandbox.location(), 4);
    let archive = sandbox.state_dir().join("rbexport-20261112-1100.zip");
    testing::archive(sandbox.location(), &archive, None);
    testing::set_rating(sandbox.location(), 1);
    assert!(
        !sidecar_db(&sandbox).exists(),
        "no Sidecar before the restore"
    );

    let restored = restore(&sandbox, &archive);
    assert!(restored.status.success(), "{}", restored.stderr);
    assert_eq!(testing::rating(sandbox.location()), 4);
    let id = restored.json()["ledger_entry"]
        .as_i64()
        .unwrap_or_else(|| panic!("no entry: {}", restored.stdout));
    let entry = Sidecar::open(&sandbox.sidecar_dir())
        .unwrap()
        .ledger_entry(id)
        .unwrap()
        .unwrap();
    assert_eq!(
        (entry.action, entry.kind.as_str()),
        (Action::Write, "restore")
    );
    assert!(entry.label.contains("holds no Sidecar"), "{}", entry.label);
    assert!(!rbl_backup::journal::pending(&sandbox.state_dir()));
    // No Fact was made by it.
    assert_eq!(count(&sandbox, "SELECT count(*) FROM fact"), 0);
}

#[test]
fn a_restore_refused_by_library_protection_changes_nothing_and_records_nothing() {
    let sandbox = Sandbox::new();
    testing::set_rating(sandbox.location(), 4);
    let archive = sandbox.state_dir().join("rbexport-20261112-1100.zip");
    testing::archive(sandbox.location(), &archive, None);
    testing::set_rating(sandbox.location(), 1);
    sandbox.protect(true);
    let refused = restore(&sandbox, &archive);
    assert!(!refused.status.success());
    assert_eq!(refused.error_json()["error"]["code"], "library_protected");
    assert_eq!(testing::rating(sandbox.location()), 1);
    assert!(!sidecar_db(&sandbox).exists(), "no Sidecar and no entry");
    assert!(!ledger_receipts::folder(&sandbox.state_dir()).exists());
}

#[test]
fn listing_receipts_only_reads_and_recording_them_writes_one_tool_entry_each_once() {
    let sandbox = Sandbox::new();
    let receipt = script_receipt(&sandbox, "restore-script-20261112-1100-77");
    ledger_receipts::hand_off(&sandbox.state_dir(), &receipt).unwrap();
    let before = snapshot(&sandbox.state_dir());

    let listed = sandbox.cli(&["--json", "backup", "receipts"]);
    assert!(listed.status.success(), "{}", listed.stderr);
    let receipts = listed.json()["receipts"].as_array().unwrap().clone();
    assert_eq!(receipts.len(), 1, "{receipts:?}");
    assert_eq!(receipts[0]["receipt"]["operation"], receipt.operation);
    assert_eq!(receipts[0]["error"], serde_json::Value::Null);
    // A read: nothing recorded, created or removed.
    assert_eq!(snapshot(&sandbox.state_dir()), before);
    assert!(!sidecar_db(&sandbox).exists());

    let recorded = sandbox.cli(&["--json", "backup", "receipts", "--record"]);
    assert!(recorded.status.success(), "{}", recorded.stderr);
    let done = recorded.json()["recorded"][0].clone();
    let entry = Sidecar::open(&sandbox.sidecar_dir())
        .unwrap()
        .ledger_entry(done["entry"].as_i64().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(entry.kind, "restore");
    assert_eq!(entry.action, Action::Write);
    assert_eq!(
        (entry.source.kind, entry.source.name.as_str()),
        (SourceKind::Tool, "restore-script")
    );
    assert_eq!(entry.run.0, done["run"].as_i64().unwrap());
    let payload = entry.payload.unwrap();
    assert_eq!(payload["operation"], receipt.operation);
    assert_eq!(payload["outcome"], "restored");
    assert!(payload["uncertainty"].as_str().unwrap().contains("outside"));
    assert!(!entry.reversible);
    assert_eq!(count(&sandbox, "SELECT count(*) FROM fact"), 0);
    assert_eq!(count(&sandbox, "SELECT count(*) FROM ledger_intent"), 0);

    // Recorded once: the receipt is gone, and recording again adds nothing.
    let again = sandbox.cli(&["--json", "backup", "receipts", "--record"]);
    assert!(again.status.success(), "{}", again.stderr);
    assert_eq!(again.json()["recorded"].as_array().unwrap().len(), 0);
    assert_eq!(
        count(
            &sandbox,
            "SELECT count(*) FROM ledger WHERE kind = 'restore'"
        ),
        1
    );
    assert_eq!(count(&sandbox, "SELECT count(*) FROM run"), 1);
}

#[test]
fn a_receipt_cannot_claim_the_dj_and_another_librarys_is_left_alone() {
    let sandbox = Sandbox::new();
    let mut theirs = script_receipt(&sandbox, "restore-script-other");
    theirs.library = sandbox.state_dir().join("elsewhere/master.db");
    ledger_receipts::hand_off(&sandbox.state_dir(), &theirs).unwrap();
    // A file claiming the DJ is refused when it is written and when read.
    let mut dj = script_receipt(&sandbox, "restore-script-dj");
    dj.source = "dj:dj".to_owned();
    assert!(ledger_receipts::hand_off(&sandbox.state_dir(), &dj).is_err());
    let path = ledger_receipts::folder(&sandbox.state_dir()).join("restore-script-dj.json");
    std::fs::write(&path, serde_json::to_vec(&dj).unwrap()).unwrap();

    let recorded = sandbox.cli(&["--json", "backup", "receipts", "--record"]);
    assert!(!recorded.status.success(), "a kept receipt is reported");
    let error = recorded.error_json()["error"].clone();
    assert_eq!(error["code"], "restore_unrecorded", "{error}");
    assert!(path.exists(), "kept for the DJ to see");
    assert!(ledger_receipts::folder(&sandbox.state_dir())
        .join("restore-script-other.json")
        .exists());
    if sidecar_db(&sandbox).exists() {
        assert_eq!(count(&sandbox, "SELECT count(*) FROM ledger"), 0);
    }
}

#[test]
fn a_recording_that_fails_keeps_the_receipt_and_its_retry_uses_the_same_run_and_entry() {
    use std::os::unix::fs::PermissionsExt as _;
    let sandbox = Sandbox::new();
    // A Sidecar exists; its receipt folder cannot be written, so the
    // reservation cannot be saved and nothing is appended.
    Sidecar::open(&sandbox.sidecar_dir()).unwrap();
    let receipt = script_receipt(&sandbox, "restore-script-retry");
    let path = ledger_receipts::hand_off(&sandbox.state_dir(), &receipt).unwrap();
    let folder = ledger_receipts::folder(&sandbox.state_dir());
    let mode = |path: &Path, mode| {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap()
    };
    mode(&folder, 0o555);
    let failed = sandbox.cli(&["--json", "backup", "receipts", "--record"]);
    mode(&folder, 0o755);
    assert!(!failed.status.success());
    assert_eq!(failed.error_json()["error"]["code"], "restore_unrecorded");
    assert!(path.exists());
    assert_eq!(count(&sandbox, "SELECT count(*) FROM ledger"), 0);
    assert_eq!(
        count(&sandbox, "SELECT count(*) FROM run"),
        0,
        "no Run without its reservation"
    );

    let recorded = sandbox.cli(&["--json", "backup", "receipts", "--record"]);
    assert!(recorded.status.success(), "{}", recorded.stderr);
    assert_eq!(
        count(
            &sandbox,
            "SELECT count(*) FROM ledger WHERE kind = 'restore'"
        ),
        1
    );
    assert_eq!(count(&sandbox, "SELECT count(*) FROM run"), 1);
    assert!(!path.exists());

    // A reservation that was saved is reused exactly.
    let mut reserved = script_receipt(&sandbox, "restore-script-reserved");
    let sidecar = Sidecar::open(&sandbox.sidecar_dir()).unwrap();
    let run = sidecar
        .begin_owned_run(
            &wreckordbox_sidecar::Source::new(SourceKind::Tool, "restore-script"),
            "Restore a backup with its standalone script",
        )
        .unwrap();
    reserved.reserved = Some(ledger_receipts::Reserved {
        run: run.0,
        entry: None,
    });
    ledger_receipts::hand_off(&sandbox.state_dir(), &reserved).unwrap();
    let runs = count(&sandbox, "SELECT count(*) FROM run");
    let recorded = sandbox.cli(&["--json", "backup", "receipts", "--record"]);
    assert!(recorded.status.success(), "{}", recorded.stderr);
    assert_eq!(recorded.json()["recorded"][0]["run"], run.0);
    assert_eq!(
        count(&sandbox, "SELECT count(*) FROM run"),
        runs,
        "no new Run"
    );
}
