//! The standalone restore scripts and wreckordbox's Ledger (wbx-14.15): a
//! Library-only backup goes to `wreckordbox backup restore` when it can,
//! and is otherwise restored with a receipt handed off before anything is
//! replaced; Library Protection refuses it with nothing changed. Bash
//! against throwaway folders; never a real Library or backup.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use wreckordbox_service::ledger_receipts;

use super::{shell, SHELL_NAME};

struct Rig {
    dir: tempfile::TempDir,
}

impl Rig {
    /// An extracted Library-only backup and a live Library to restore it
    /// over, with its archive beside the folder when `zip`.
    fn new(zip: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let rig = Self { dir };
        let extracted = rig.extracted();
        fs::create_dir_all(extracted.join("analysis")).unwrap();
        fs::create_dir_all(extracted.join("artwork")).unwrap();
        fs::write(extracted.join("master.db"), b"saved database").unwrap();
        fs::create_dir_all(rig.live().join("share/PIONEER/USBANLZ")).unwrap();
        fs::write(rig.master_db(), b"current database").unwrap();
        if zip {
            fs::write(rig.dir.path().join("rbexport-20261111-1100.zip"), b"zip").unwrap();
        }
        fs::write(
            extracted.join(SHELL_NAME),
            shell(&rig.master_db(), &rig.state()),
        )
        .unwrap();
        rig
    }
    fn extracted(&self) -> PathBuf {
        self.dir.path().join("rbexport-20261111-1100")
    }
    fn live(&self) -> PathBuf {
        self.dir.path().join("live")
    }
    fn master_db(&self) -> PathBuf {
        self.live().join("master.db")
    }
    fn state(&self) -> PathBuf {
        self.dir.path().join("state")
    }
    fn receipts(&self) -> Vec<ledger_receipts::Pending> {
        ledger_receipts::list(&self.state(), &self.master_db()).unwrap()
    }
    /// Runs the script answering "yes", with a `wreckordbox` stub that
    /// writes its arguments to `calls` when `stub`, and `args` after it.
    fn run(&self, stub: bool, args: &[&str]) -> Output {
        let mut command = Command::new("/bin/bash");
        command
            .arg(self.extracted().join(SHELL_NAME))
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.dir.path());
        let bin = self.dir.path().join("wreckordbox-stub");
        if stub {
            fs::write(&bin, "#!/bin/sh\necho \"$@\" > \"$CALLS\"\n").unwrap();
            fs::set_permissions(&bin, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        }
        command
            .env("WRECKORDBOX_BIN", &bin)
            .env("CALLS", self.dir.path().join("calls"));
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"yes\n").unwrap();
        child.wait_with_output().unwrap()
    }
    fn calls(&self) -> Option<String> {
        fs::read_to_string(self.dir.path().join("calls")).ok()
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn a_library_only_backup_goes_to_wreckordbox_backup_restore_when_it_can_be_found() {
    let rig = Rig::new(true);
    let output = rig.run(true, &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    let archive = rig.dir.path().join("rbexport-20261111-1100.zip");
    assert_eq!(
        rig.calls().unwrap().trim(),
        format!(
            "backup restore {} --yes",
            archive.canonicalize().unwrap().display()
        )
    );
    // The recorded owner restores it; the script changed nothing itself.
    assert_eq!(fs::read(rig.master_db()).unwrap(), b"current database");
    assert!(rig.receipts().is_empty());
}

#[test]
fn a_library_only_restore_by_the_script_hands_off_a_receipt_recorded_once_as_the_tools() {
    // A library path given by hand is not wreckordbox's to restore.
    let rig = Rig::new(true);
    let output = rig.run(true, &[rig.master_db().to_str().unwrap()]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(rig.calls().is_none(), "not handed to wreckordbox");
    assert_eq!(fs::read(rig.master_db()).unwrap(), b"saved database");

    let receipts = rig.receipts();
    assert_eq!(receipts.len(), 1, "{receipts:?}");
    assert_eq!(receipts[0].error, None);
    let receipt = receipts[0].receipt.clone().unwrap();
    assert_eq!(receipt.kind, "restore");
    assert_eq!(receipt.source, "restore-script");
    assert_eq!(receipt.outcome, "restored");
    assert!(receipt.operation.starts_with("restore-script-"));
    assert_eq!(receipt.details["backup"], "rbexport-20261111-1100");
    assert_eq!(receipt.details["script"], SHELL_NAME);
    let rollback = PathBuf::from(receipt.details["rollback"].as_str().unwrap());
    assert_eq!(
        fs::read(rollback.join("database/master.db")).unwrap(),
        b"current database"
    );

    let sidecar = rig.dir.path().join("sidecar");
    let done = ledger_receipts::record(&rig.state(), &rig.master_db(), &sidecar).unwrap();
    assert!(done.unrecorded.is_empty(), "{:?}", done.unrecorded);
    assert_eq!(done.recorded.len(), 1);
    let store = wreckordbox_sidecar::Sidecar::open(&sidecar).unwrap();
    let entry = store.ledger_entry(done.recorded[0].entry).unwrap().unwrap();
    assert_eq!(entry.kind, "restore");
    assert_eq!(entry.source.kind, wreckordbox_sidecar::SourceKind::Tool);
    assert_eq!(entry.source.name, "restore-script");
    assert_eq!(entry.payload.unwrap()["operation"], receipt.operation);
    assert!(rig.receipts().is_empty());
    let again = ledger_receipts::record(&rig.state(), &rig.master_db(), &sidecar).unwrap();
    assert!(again.recorded.is_empty());
}

#[test]
fn a_restore_the_script_could_not_finish_leaves_a_receipt_saying_so() {
    let rig = Rig::new(false);
    // The analysis folder cannot be moved aside.
    let blocked = rig.live().join("share/PIONEER");
    std::fs::set_permissions(
        &blocked,
        std::os::unix::fs::PermissionsExt::from_mode(0o555),
    )
    .unwrap();
    let output = rig.run(false, &[]);
    std::fs::set_permissions(
        &blocked,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    assert!(!output.status.success());
    let receipt = rig.receipts()[0].receipt.clone().unwrap();
    assert_eq!(receipt.outcome, "failed");
}

#[test]
fn library_protection_refuses_the_script_with_nothing_changed_and_no_receipt() {
    let rig = Rig::new(false);
    fs::create_dir_all(rig.state()).unwrap();
    for protection in [r#"{"protect_library":true,"revision":3}"#, "not json"] {
        fs::write(rig.state().join("library-protection.json"), protection).unwrap();
        let output = rig.run(false, &[]);
        assert!(!output.status.success());
        assert!(
            stderr(&output).contains("Library Protection is on"),
            "{}",
            stderr(&output)
        );
        assert_eq!(fs::read(rig.master_db()).unwrap(), b"current database");
        let names: Vec<_> = fs::read_dir(rig.live())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 2, "no rollback folder: {names:?}");
        assert!(!ledger_receipts::folder(&rig.state()).exists());
    }
    // Turned off, it restores.
    fs::write(
        rig.state().join("library-protection.json"),
        r#"{"protect_library": false, "revision": 4}"#,
    )
    .unwrap();
    let output = rig.run(false, &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(fs::read(rig.master_db()).unwrap(), b"saved database");
    assert_eq!(rig.receipts().len(), 1);
}

#[test]
fn the_powershell_script_delegates_checks_protection_and_hands_off_a_receipt_before_replacing() {
    let script = super::powershell(Path::new("/tmp/master.db"), Path::new("/tmp/state"));
    let delegate = script.find("!$GivenLibrary -and $Wreckordbox").unwrap();
    let protection = script.find("library-protection.json").unwrap();
    let receipt = script.find("Write-Receipt 'begun'").unwrap();
    let replace = script
        .find("Replace-File (Join-Path $SourceDir 'master.db')")
        .unwrap();
    assert!(delegate < protection && protection < receipt && receipt < replace);
    assert!(script.contains("source = 'restore-script'"));
    assert!(script.contains("Write-Receipt 'failed'"));
    assert!(script.contains("Write-Receipt 'restored'"));
}
