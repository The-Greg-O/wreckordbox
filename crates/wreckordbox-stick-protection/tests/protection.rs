//! Stick Protection (wbx-45.11): every Stick protected by default, unlocked
//! only by the DJ naming it, the unlock ending with the mount.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use wreckordbox_stick_protection::{
    lock, status, stick_root_of, unlock, UnlockError, FILE, SPARE_MARKER, STAND_IN_MARKER,
};

/// A temporary folder standing in for a mounted Stick named `name`.
fn stick(parent: &Path, name: &str, spare: bool) -> std::path::PathBuf {
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
fn an_unlock_ends_when_the_dj_locks_it_or_the_stick_is_removed_or_put_back() {
    let (state, sticks) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let gig = stick(sticks.path(), "GIG", false);
    unlock(state.path(), &gig, Some("GIG")).unwrap();
    assert!(lock(state.path(), &gig).unwrap().protected);

    unlock(state.path(), &gig, Some("GIG")).unwrap();
    // Pulled out and put back: another mount of the same name.
    std::fs::remove_dir_all(&gig).unwrap();
    assert!(status(state.path(), &gig).protected, "removed");
    let back = stick(sticks.path(), "GIG", false);
    assert!(status(state.path(), &back).protected, "re-inserted");
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
fn a_destination_is_a_stick_only_when_it_is_on_a_mounted_one() {
    let sticks = tempfile::tempdir().unwrap();
    let mounted = sticks.path().join("MOUNTED");
    std::fs::create_dir_all(mounted.join("Music")).unwrap();
    let stand_in = stick(sticks.path(), "STAND", false);
    let ordinary = sticks.path().join("Exports");
    std::fs::create_dir_all(&ordinary).unwrap();
    let mounts = [mounted.clone()];

    assert_eq!(stick_root_of(&mounted.join("Music/set.m3u8"), &mounts), Some(mounted.canonicalize().unwrap()));
    assert_eq!(stick_root_of(&mounted, &mounts), Some(mounted.canonicalize().unwrap()));
    assert_eq!(stick_root_of(&stand_in.join("PIONEER/x.xml"), &mounts), Some(stand_in.canonicalize().unwrap()));
    assert_eq!(stick_root_of(&ordinary.join("set.m3u8"), &mounts), None);
    // A file not written yet, in a folder not made yet, on the Stick.
    assert_eq!(stick_root_of(&mounted.join("New/Deeper/loop.wav"), &mounts), Some(mounted.canonicalize().unwrap()));
    // A path that only looks like it: `..` resolved first.
    assert_eq!(stick_root_of(&mounted.join("../Exports/set.m3u8"), &mounts), None);
}
