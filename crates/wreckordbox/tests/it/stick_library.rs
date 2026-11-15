//! Stick Protection (wbx-45.11, round 4 A4) for a Library or a Sidecar kept
//! on a Stick, through the real `wreckordbox` command and a service it
//! started: with Library Protection off, nothing is written to a Library or
//! a Sidecar on a protected Stick until the DJ unlocks that Stick; a Library
//! on an ordinary folder is unaffected.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::common;

use common::Sandbox;

fn rating(sandbox: &Sandbox, id: &str) -> serde_json::Value {
    let run = sandbox.cli(&["--json", "tracks", "list"]);
    assert!(run.status.success(), "{}", run.stderr);
    run.json()["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .expect(&run.stdout)["rating"]
        .clone()
}

/// The folder holding the fixture Library made to stand in for a mounted
/// Stick (the sandbox runs with `RBXPORT_TEST=1`).
fn on_a_stick(sandbox: &Sandbox) -> String {
    let library = sandbox.master_db().parent().unwrap().to_path_buf();
    std::fs::write(
        library.join(wreckordbox_stick_protection::STAND_IN_MARKER),
        b"",
    )
    .unwrap();
    library.to_str().unwrap().to_owned()
}

#[test]
fn a_library_on_a_protected_stick_is_not_edited_until_the_stick_is_unlocked() {
    let sandbox = Sandbox::new();
    let stick = on_a_stick(&sandbox);
    let before = std::fs::read(sandbox.master_db()).unwrap();

    let refused = sandbox.cli(&["--json", "tracks", "rate", "10003", "4"]);
    assert!(!refused.status.success(), "{}", refused.stdout);
    let error = refused.error_json();
    assert_eq!(error["error"]["code"], "stick_protected", "{error}");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unlock"),
        "{error}"
    );
    assert_eq!(rating(&sandbox, "10003"), 0);
    assert_eq!(
        std::fs::read(sandbox.master_db()).unwrap(),
        before,
        "not a byte of the Library changed"
    );

    let unlocked = sandbox.cli(&["stick", "unlock", &stick, "--name", "library"]);
    assert!(unlocked.status.success(), "{}", unlocked.stderr);
    let rated = sandbox.cli(&["--json", "tracks", "rate", "10003", "4"]);
    assert!(rated.status.success(), "{}", rated.stderr);
    assert_eq!(rating(&sandbox, "10003"), 4);

    assert!(sandbox.cli(&["stick", "lock", &stick]).status.success());
    let again = sandbox.cli(&["--json", "tracks", "rate", "10003", "2"]);
    assert_eq!(again.error_json()["error"]["code"], "stick_protected");
    assert_eq!(rating(&sandbox, "10003"), 4);
}

#[test]
fn a_sidecar_on_a_stick_records_no_fact_locked_or_unlocked() {
    // A Library hosted on a Stick is read-only for what the Sidecar records
    // (wbx-8yh): before wbx-8yh an unlock let the Fact through.
    let sandbox = Sandbox::new();
    let sidecar = sandbox.sidecar_dir();
    std::fs::create_dir_all(&sidecar).unwrap();
    std::fs::write(
        sidecar.join(wreckordbox_stick_protection::STAND_IN_MARKER),
        b"",
    )
    .unwrap();
    let record = [
        "--json", "facts", "record", "10003", "mood", "Dark", "--source", "dj",
    ];

    let refused = sandbox.cli(&record);
    assert!(!refused.status.success(), "{}", refused.stdout);
    assert_eq!(
        refused.error_json()["error"]["code"],
        "sidecar_on_stick",
        "{}",
        refused.stdout
    );
    assert!(
        !sidecar.join("sidecar.db").exists(),
        "the Sidecar was not even made"
    );

    let name = sidecar.file_name().unwrap().to_str().unwrap().to_owned();
    assert!(sandbox
        .cli(&[
            "stick",
            "unlock",
            sidecar.to_str().unwrap(),
            "--name",
            &name
        ])
        .status
        .success());
    let again = sandbox.cli(&record);
    assert!(!again.status.success(), "{}", again.stdout);
    assert_eq!(again.error_json()["error"]["code"], "sidecar_on_stick");
    assert!(!sidecar.join("sidecar.db").exists(), "still not made");
}

#[test]
fn an_edit_of_a_library_whose_sidecar_is_on_a_stick_is_kept_only_while_it_is_unlocked() {
    // Its Ledger entry is the edit's own record: written on the Stick only
    // while the DJ has it unlocked, so a locked Stick refuses the edit.
    let sandbox = Sandbox::new();
    let sidecar = sandbox.sidecar_dir();
    std::fs::create_dir_all(&sidecar).unwrap();
    std::fs::write(
        sidecar.join(wreckordbox_stick_protection::STAND_IN_MARKER),
        b"",
    )
    .unwrap();
    let refused = sandbox.cli(&["--json", "tracks", "rate", "10003", "4"]);
    assert_eq!(
        refused.error_json()["error"]["code"],
        "stick_protected",
        "{}",
        refused.stdout
    );
    assert_eq!(rating(&sandbox, "10003"), 0);

    let name = sidecar.file_name().unwrap().to_str().unwrap().to_owned();
    assert!(sandbox
        .cli(&[
            "stick",
            "unlock",
            sidecar.to_str().unwrap(),
            "--name",
            &name
        ])
        .status
        .success());
    let rated = sandbox.cli(&["--json", "tracks", "rate", "10003", "4"]);
    assert!(rated.status.success(), "{}", rated.stderr);
    assert_eq!(rating(&sandbox, "10003"), 4);
}

#[test]
fn a_library_on_an_ordinary_folder_needs_nothing_unlocked() {
    let sandbox = Sandbox::new();
    let rated = sandbox.cli(&["--json", "tracks", "rate", "10003", "3"]);
    assert!(rated.status.success(), "{}", rated.stderr);
    assert_eq!(rating(&sandbox, "10003"), 3);
}
