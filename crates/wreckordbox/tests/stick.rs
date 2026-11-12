//! `wreckordbox stick` (wbx-45.11): Stick Protection from a terminal, kept
//! with the window's. A temporary folder stands in for every Stick.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::Sandbox;

#[test]
fn a_gig_stick_unlocks_only_with_its_name_and_a_spare_stick_with_a_confirmation() {
    let sandbox = Sandbox::new();
    let sticks = tempfile::tempdir().unwrap();
    let gig = sticks.path().join("FRIDAY");
    std::fs::create_dir(&gig).unwrap();
    let gig = gig.to_str().unwrap();

    let status = sandbox.cli(&["--json", "stick", "status", gig]);
    assert!(status.status.success(), "{}", status.stderr);
    assert_eq!(status.json()["protected"], true, "protected by default");
    assert_eq!(status.json()["spare"], false);

    let refused = sandbox.cli(&["stick", "unlock", gig]);
    assert!(!refused.status.success());
    assert!(refused.stderr.contains("type its name, FRIDAY"), "{}", refused.stderr);
    let wrong = sandbox.cli(&["stick", "unlock", gig, "--name", "friday"]);
    assert!(!wrong.status.success());
    assert_eq!(sandbox.cli(&["--json", "stick", "status", gig]).json()["protected"], true);

    let unlocked = sandbox.cli(&["stick", "unlock", gig, "--name", "FRIDAY"]);
    assert!(unlocked.status.success(), "{}", unlocked.stderr);
    assert!(unlocked.stdout.contains("FRIDAY (Gig stick) is unlocked"), "{}", unlocked.stdout);
    let locked = sandbox.cli(&["stick", "lock", gig]);
    assert!(locked.stdout.contains("is protected"), "{}", locked.stdout);

    let spare = sticks.path().join("SPARE");
    std::fs::create_dir(&spare).unwrap();
    std::fs::write(spare.join(".rbxport-disposable"), b"").unwrap();
    let spare = spare.to_str().unwrap();
    let unlocked = sandbox.cli(&["--json", "stick", "unlock", spare]);
    assert!(unlocked.status.success(), "{}", unlocked.stderr);
    assert_eq!(unlocked.json()["protected"], false);
}

#[test]
fn no_stick_there_is_said() {
    let sandbox = Sandbox::new();
    let gone = sandbox.cli(&["--json", "stick", "status", "/nonexistent/STICK"]);
    assert!(!gone.status.success());
    assert_eq!(gone.error_json()["error"]["code"], "stick_not_found");
}
