//! Stick Protection (wbx-45.11): every Stick protected by default, unlocked
//! only by the DJ naming it, the unlock bound to that one mount of the
//! volume, and every writer holding the Stick's execution lock.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use wreckordbox_stick_protection::{
    authorize, classify, exec_lock, lock, status, try_exec_lock, unlock, Refused, UnlockError, Volume, FILE,
    SPARE_MARKER, STAND_IN_MARKER,
};

/// A temporary folder standing in for a mounted Stick named `name` (honoured
/// because the tests run with `RBXPORT_TEST=1`).
fn stick(parent: &Path, name: &str, spare: bool) -> PathBuf {
    assert_eq!(std::env::var("RBXPORT_TEST").as_deref(), Ok("1"), "tests run in test mode");
    let root = parent.join(name);
    std::fs::create_dir_all(root.join("PIONEER")).unwrap();
    std::fs::write(root.join(STAND_IN_MARKER), b"").unwrap();
    if spare {
        std::fs::write(root.join(SPARE_MARKER), b"").unwrap();
    }
    root
}

#[test]
fn every_stick_is_protected_until_unlocked() {
    let (state, sticks) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let gig = stick(sticks.path(), "GIG", false);
    let now = status(state.path(), &gig);
    assert!(now.protected);
    assert!(!now.spare);
    assert_eq!(now.name, "GIG");
}

#[test]
fn a_spare_stick_unlocks_with_a_confirmation_and_a_gig_stick_needs_its_name() {
    let (state, sticks) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let spare = stick(sticks.path(), "SPARE", true);
    assert!(!unlock(state.path(), &spare, None).unwrap().protected);

    let gig = stick(sticks.path(), "GIG", false);
    assert_eq!(unlock(state.path(), &gig, None).unwrap_err(), UnlockError::NameRequired { name: "GIG".into() });
    assert_eq!(unlock(state.path(), &gig, Some("gig")).unwrap_err(), UnlockError::NameMismatch { name: "GIG".into() });
    assert!(status(state.path(), &gig).protected, "a wrong name unlocks nothing");
    assert!(!unlock(state.path(), &gig, Some(" GIG ")).unwrap().protected);
    // One Stick's unlock is not another's.
    assert!(status(state.path(), &stick(sticks.path(), "OTHER", true)).protected);
}

#[test]
fn a_folder_inside_a_stick_is_that_stick_and_cannot_be_unlocked_on_its_own() {
    let (state, sticks) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let gig = stick(sticks.path(), "GIG", false);
    let inside = gig.join("PIONEER");
    assert_eq!(classify(&inside.join("not yet/x.xml")), Volume::Stick { root: gig.canonicalize().unwrap() });
    // Unlocked through a folder inside: the Stick itself is what is unlocked.
    assert_eq!(unlock(state.path(), &inside, Some("PIONEER")).unwrap_err(), UnlockError::NameMismatch { name: "GIG".into() });
    unlock(state.path(), &inside, Some("GIG")).unwrap();
    assert!(!status(state.path(), &gig).protected);
    // Locked at the root: the folder inside is protected with it.
    lock(state.path(), &gig).unwrap();
    assert!(status(state.path(), &inside).protected);
    assert!(authorize(state.path(), &inside).is_err());
}

#[test]
fn an_unlock_ends_when_the_dj_locks_it_or_the_stick_is_removed_or_put_back() {
    let (state, sticks) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let gig = stick(sticks.path(), "GIG", false);
    unlock(state.path(), &gig, Some("GIG")).unwrap();
    assert!(lock(state.path(), &gig).unwrap().protected);

    unlock(state.path(), &gig, Some("GIG")).unwrap();
    std::fs::remove_dir_all(&gig).unwrap();
    assert!(status(state.path(), &gig).protected, "removed");
    let back = stick(sticks.path(), "GIG", false);
    assert!(status(state.path(), &back).protected, "re-inserted");
}

#[test]
fn an_unlock_recorded_in_another_boot_is_never_taken_for_this_one() {
    let (state, sticks) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let spare = stick(sticks.path(), "SPARE", true);
    unlock(state.path(), &spare, None).unwrap();
    let record = std::fs::read_to_string(state.path().join(FILE)).unwrap();
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
    let boot = boot.trim();
    assert!(record.contains(boot), "the record names this boot: {record}");
    // The same mount tuple, as a later boot might recreate it.
    std::fs::write(state.path().join(FILE), record.replace(boot, "00000000-0000-0000-0000-000000000000")).unwrap();
    assert!(status(state.path(), &spare).protected);
}

#[test]
fn missing_unreadable_or_corrupt_state_means_protected() {
    let (state, sticks) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let spare = stick(sticks.path(), "SPARE", true);
    unlock(state.path(), &spare, None).unwrap();
    std::fs::write(state.path().join(FILE), b"{ not json").unwrap();
    assert!(status(state.path(), &spare).protected, "corrupt");
    std::fs::remove_file(state.path().join(FILE)).unwrap();
    std::fs::create_dir(state.path().join(FILE)).unwrap();
    assert!(status(state.path(), &spare).protected, "unreadable");
}

#[test]
fn an_ordinary_folder_is_not_a_stick_and_needs_nothing() {
    let (state, folders) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let ordinary = folders.path().join("Exports");
    std::fs::create_dir_all(&ordinary).unwrap();
    assert_eq!(classify(&ordinary.join("set.m3u8")), Volume::Ordinary);
    assert!(authorize(state.path(), &ordinary).unwrap().stick().is_none());
    assert_eq!(unlock(state.path(), &ordinary, None).unwrap_err(), UnlockError::NotAStick);
    // A path that only looks like it is on a Stick: `..` resolved first.
    let gig = stick(folders.path(), "GIG", false);
    assert_eq!(classify(&gig.join("../Exports/set.m3u8")), Volume::Ordinary);
}

#[test]
fn a_writer_holds_the_sticks_execution_lock_and_locking_waits_for_it_to_finish() {
    let (state, sticks) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let spare = stick(sticks.path(), "SPARE", true);
    unlock(state.path(), &spare, None).unwrap();
    let writing = authorize(state.path(), &spare).unwrap();
    assert!(try_exec_lock(state.path(), &spare).unwrap().is_none(), "held by the writer");

    let (done, locked) = mpsc::channel();
    let (dir, root) = (state.path().to_path_buf(), spare.clone());
    let locker = std::thread::spawn(move || {
        let now = lock(&dir, &root).unwrap();
        done.send(now.protected).unwrap();
    });
    assert!(locked.recv_timeout(Duration::from_millis(400)).is_err(), "not acknowledged while a write runs");
    writing.recheck().unwrap();
    drop(writing);
    assert_eq!(locked.recv_timeout(Duration::from_secs(10)), Ok(true), "acknowledged once it is drained");
    locker.join().unwrap();
    assert!(matches!(authorize(state.path(), &spare), Err(Refused::Protected { .. })));
}

#[test]
fn a_writer_that_finds_its_stick_replaced_stops() {
    let (state, sticks) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let spare = stick(sticks.path(), "SPARE", true);
    unlock(state.path(), &spare, None).unwrap();
    let writing = authorize(state.path(), &spare).unwrap();
    std::fs::remove_dir_all(&spare).unwrap();
    let again = stick(sticks.path(), "SPARE", true);
    assert!(matches!(writing.recheck(), Err(Refused::Replaced { .. })));
    drop(writing);
    assert!(status(state.path(), &again).protected);
}

#[test]
fn the_execution_lock_is_kept_in_the_state_folder_not_on_the_stick() {
    let (state, sticks) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let spare = stick(sticks.path(), "SPARE", true);
    let before: Vec<_> = std::fs::read_dir(&spare).unwrap().map(|e| e.unwrap().file_name()).collect();
    let held = exec_lock(state.path(), &spare).unwrap();
    let after: Vec<_> = std::fs::read_dir(&spare).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(before, after);
    assert!(std::fs::read_dir(state.path().join("stick-locks")).unwrap().count() > 0);
    drop(held);
}
