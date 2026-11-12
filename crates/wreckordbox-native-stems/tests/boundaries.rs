//! Where the writer must stop: another track's set, a name no file system
//! holds, anything that would take a read or a write off the Stick, and a
//! replacement that was interrupted half-way.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod deck;
mod support;

use std::os::unix::fs::symlink;
use std::path::Path;

use deck::Identity;
use support::Rig;
use wreckordbox_native_stems::naming::Unnameable;
use wreckordbox_native_stems::{write, NativeStemsError, Outcome};

fn written(rig: &Rig, track: u64) -> wreckordbox_native_stems::Written {
    let stems = rig.stems(&format!("set-{track}"), 40_000);
    match write(&rig.request(track, &stems)).unwrap() {
        Outcome::Written(written) => written,
        other => panic!("not written: {other:?}"),
    }
}

fn refused(rig: &Rig, track: u64) -> NativeStemsError {
    let stems = rig.stems(&format!("refused-{track}"), 40_000);
    write(&rig.request(track, &stems)).unwrap_err()
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Gives a track on the Stick another file name, on disk and in the
/// Stick's library, as a Stick written by another tool might hold it.
fn rename_on_stick(rig: &Rig, track: u64, name: &str) {
    let from = rig.track_file(track);
    std::fs::rename(&from, from.with_file_name(name)).unwrap();
    rig.hold_in_library(track, &format!("/Contents/Artist {track}/Single/{name}"));
}

// ---- another track's set ------------------------------------------------

#[test]
fn a_hashed_folder_that_holds_another_tracks_set_is_not_overwritten() {
    // Three tracks. The first is a file whose own name is what the layout
    // would call the second `Song.wav`'s hashed folder: no CRC collision
    // is needed, only a file named like one.
    let rig = Rig::spare(&[("Song-e40da05e.wav", 6), ("Song.wav", 7), ("Song.wav", 8)]);
    assert_eq!(rig.library_path(8), "Contents/Artist 8/Single/Song.wav");
    assert_eq!(written(&rig, 6).folder, "Song-e40da05e");
    assert_eq!(written(&rig, 7).folder, "Song");
    let taken = rig.root().join("STEMS/Song-e40da05e");
    let before = deck::snapshot(&taken);

    let error = refused(&rig, 8);

    assert!(
        matches!(&error, NativeStemsError::FolderTaken { folder } if folder == "Song-e40da05e"),
        "{error:?}"
    );
    assert_eq!(deck::snapshot(&taken), before);
    assert_eq!(
        names_in(&rig.root().join("STEMS")),
        ["Song", "Song-e40da05e"]
    );
    let first = deck::load(&[rig.root()], &rig.track_file(6)).unwrap();
    assert_eq!(
        (first.folder, first.identity, first.pads.len()),
        (taken, Identity::Proven, 4)
    );
}

#[test]
fn a_hashed_folder_with_no_record_is_not_taken_for_this_tracks() {
    // Under the track's own name a set with no record is the track's by
    // name. Under the hashed name it may as well be the set of a file
    // named `Song-e40da05e`, so it stays.
    let rig = Rig::spare(&[("Song.wav", 7), ("Song.wav", 8)]);
    written(&rig, 7);
    let unknown = rig.root().join("STEMS/Song-e40da05e");
    std::fs::create_dir(&unknown).unwrap();
    std::fs::write(unknown.join("vocals.ogg"), b"someone's").unwrap();

    let error = refused(&rig, 8);

    assert!(
        matches!(error, NativeStemsError::FolderTaken { .. }),
        "{error:?}"
    );
    assert_eq!(
        std::fs::read(unknown.join("vocals.ogg")).unwrap(),
        b"someone's"
    );
}

#[test]
fn this_tracks_own_hashed_set_is_replaced() {
    let rig = Rig::spare(&[("Song.wav", 7), ("Song.wav", 8)]);
    written(&rig, 7);
    assert!(!written(&rig, 8).replaced);

    let again = written(&rig, 8);

    assert!(again.replaced);
    assert_eq!(again.folder, "Song-e40da05e");
}

// ---- names at the file system's limit -----------------------------------

#[test]
fn a_file_name_at_the_limit_is_written_whatever_the_working_folders_are_called() {
    // 251 letters and `.wav`: 255 bytes, the most a name can be. A working
    // folder named after it would be longer than any file system holds.
    let rig = Rig::spare(&[("Long.wav", 7)]);
    let key = "a".repeat(251);
    rename_on_stick(&rig, 7, &format!("{key}.wav"));

    let first = written(&rig, 7);
    let again = written(&rig, 7);

    assert_eq!(first.folder, key);
    assert!(again.replaced);
    assert_eq!(names_in(&rig.root().join("STEMS")), [key]);
    let loaded = deck::load(&[rig.root()], &rig.track_file(7)).unwrap();
    assert_eq!((loaded.identity, loaded.pads.len()), (Identity::Proven, 4));
}

#[test]
fn a_hashed_folder_name_at_the_limit_is_written_and_one_over_is_skipped() {
    // `<key>-XXXXXXXX` is nine bytes longer than the key.
    for (letters, fits) in [(246, true), (247, false)] {
        let rig = Rig::spare(&[("One.wav", 7), ("Two.wav", 8)]);
        let name = format!("{}.wav", "b".repeat(letters));
        rename_on_stick(&rig, 7, &name);
        rename_on_stick(&rig, 8, &name);
        written(&rig, 7);
        let stems = rig.stems("second", 40_000);

        let outcome = write(&rig.request(8, &stems)).unwrap();

        match outcome {
            Outcome::Written(written) => {
                assert!(fits, "{letters}");
                assert_eq!(written.folder.len(), 255);
            }
            Outcome::Skipped(skipped) => {
                assert!(!fits, "{letters}");
                assert_eq!(skipped.reason, Unnameable::FolderTooLong);
                assert_eq!(names_in(&rig.root().join("STEMS")).len(), 1);
            }
        }
    }
}

#[test]
fn an_underscore_that_takes_the_folder_name_over_the_limit_is_skipped() {
    // 255 bytes with no extension and a space at the end: the folder would
    // be the name and `_`.
    let rig = Rig::spare(&[("Long.wav", 7)]);
    let name = format!("{} ", "c".repeat(254));
    rename_on_stick(&rig, 7, &name);
    let stems = rig.stems("set", 40_000);

    let outcome = write(&rig.request(7, &stems)).unwrap();

    let Outcome::Skipped(skipped) = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(skipped.reason, Unnameable::FolderTooLong);
    assert!(!rig.root().join("STEMS").exists());
}

// ---- writes stay on the Stick -------------------------------------------

#[test]
fn a_stems_folder_that_is_a_link_to_another_stick_is_refused() {
    // The marked Stick's STEMS points into a Stick that carries no marker.
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    let gig = Rig::unmarked(&[("Artist - Song.wav", 7)]);
    let theirs = gig.root().join("STEMS");
    std::fs::create_dir_all(theirs.join("Artist - Song")).unwrap();
    std::fs::write(theirs.join("Artist - Song/vocals.ogg"), b"theirs").unwrap();
    std::fs::create_dir_all(theirs.join(".wbx-00000000.part")).unwrap();
    symlink(&theirs, rig.root().join("STEMS")).unwrap();
    let (gig_before, spare_before) = (deck::snapshot(gig.root()), deck::snapshot(rig.root()));

    let error = refused(&rig, 7);

    assert!(
        matches!(error, NativeStemsError::OffStick { .. }),
        "{error:?}"
    );
    assert_eq!(deck::snapshot(gig.root()), gig_before);
    assert!(theirs.join(".wbx-00000000.part").is_dir());
    assert_eq!(deck::snapshot(rig.root()), spare_before);
}

#[test]
fn a_stems_folder_that_is_a_link_to_the_sticks_own_tracks_is_refused() {
    // STEMS -> Contents, with a folder there named like the track's key:
    // followed, the writer would swap that folder out and delete it.
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    let named_alike = rig.root().join("Contents/Artist - Song");
    std::fs::create_dir(&named_alike).unwrap();
    std::fs::write(named_alike.join("Another.wav"), b"a track").unwrap();
    symlink("Contents", rig.root().join("STEMS")).unwrap();
    let before = deck::snapshot(&rig.root().join("Contents"));

    let error = refused(&rig, 7);

    assert!(
        matches!(error, NativeStemsError::OffStick { .. }),
        "{error:?}"
    );
    assert_eq!(deck::snapshot(&rig.root().join("Contents")), before);
    assert!(named_alike.join("Another.wav").is_file());
}

#[test]
fn a_tracks_stems_folder_that_is_a_link_is_refused() {
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("vocals.ogg"), b"outside").unwrap();
    std::fs::create_dir(rig.root().join("STEMS")).unwrap();
    symlink(outside.path(), rig.root().join("STEMS/Artist - Song")).unwrap();
    let before = deck::snapshot(outside.path());

    let error = refused(&rig, 7);

    assert!(
        matches!(error, NativeStemsError::OffStick { .. }),
        "{error:?}"
    );
    assert_eq!(deck::snapshot(outside.path()), before);
    assert_eq!(names_in(&rig.root().join("STEMS")), ["Artist - Song"]);
}

#[test]
fn working_names_that_are_links_are_removed_not_followed() {
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("keep.txt"), b"outside").unwrap();
    let stems_dir = rig.root().join("STEMS");
    std::fs::create_dir(&stems_dir).unwrap();
    for name in [".wbx-11111111.part", ".wbx-22222222.old"] {
        symlink(outside.path(), stems_dir.join(name)).unwrap();
    }
    let before = deck::snapshot(outside.path());

    written(&rig, 7);

    assert_eq!(deck::snapshot(outside.path()), before);
}

// ---- reads stay on the Stick --------------------------------------------

#[test]
fn a_track_path_that_climbs_out_of_the_stick_is_refused() {
    let rig = Rig::spare(&[("Song.wav", 7)]);
    let beside = rig.source.path().join("from-7/Song.wav");
    assert!(beside.is_file());
    // From the Stick's root, up and into the folder beside it.
    let source_dir = rig.source.path().file_name().unwrap().to_str().unwrap();
    rig.hold_in_library(7, &format!("/../{source_dir}/from-7/Song.wav"));

    let error = refused(&rig, 7);

    assert!(
        matches!(error, NativeStemsError::OffStick { .. }),
        "{error:?}"
    );
    assert!(!rig.root().join("STEMS").exists());
}

#[test]
fn a_track_reached_through_a_link_out_of_the_stick_is_refused() {
    let rig = Rig::spare(&[("Song.wav", 7)]);
    symlink(
        rig.source.path().join("from-7"),
        rig.root().join("Contents/Elsewhere"),
    )
    .unwrap();
    rig.hold_in_library(7, "/Contents/Elsewhere/Song.wav");

    let error = refused(&rig, 7);

    assert!(
        matches!(error, NativeStemsError::OffStick { .. }),
        "{error:?}"
    );
    assert!(!rig.root().join("STEMS").exists());
}

#[test]
fn a_track_file_that_is_itself_a_link_is_refused() {
    let rig = Rig::spare(&[("Song.wav", 7)]);
    let file = rig.track_file(7);
    std::fs::remove_file(&file).unwrap();
    symlink(rig.source.path().join("from-7/Song.wav"), &file).unwrap();

    let error = refused(&rig, 7);

    assert!(
        matches!(error, NativeStemsError::OffStick { .. }),
        "{error:?}"
    );
    assert!(!rig.root().join("STEMS").exists());
}

// ---- a replacement that was interrupted ---------------------------------

/// Leaves the Stick as a replacement of `folder` stopped after the old set
/// was set aside: the set under a working name, and the note of where it
/// belongs.
fn set_aside(stems_dir: &Path, folder: &str) {
    std::fs::rename(stems_dir.join(folder), stems_dir.join(".wbx-0badc0de.old")).unwrap();
    std::fs::write(stems_dir.join(".wbx-0badc0de.dest"), folder).unwrap();
}

#[test]
fn a_set_that_was_set_aside_and_never_replaced_is_put_back() {
    let rig = Rig::spare(&[("Artist - Song.wav", 7), ("Other.wav", 8)]);
    written(&rig, 7);
    let stems_dir = rig.root().join("STEMS");
    let before = deck::snapshot(&stems_dir.join("Artist - Song"));
    set_aside(&stems_dir, "Artist - Song");
    // The half-made new set the same interruption left.
    std::fs::create_dir(stems_dir.join(".wbx-0badc0de.part")).unwrap();
    std::fs::write(stems_dir.join(".wbx-0badc0de.part/vocals.ogg"), b"half").unwrap();
    assert!(deck::load(&[rig.root()], &rig.track_file(7)).is_none());

    // The next write, for any track.
    written(&rig, 8);

    assert_eq!(names_in(&stems_dir), ["Artist - Song", "Other"]);
    assert_eq!(deck::snapshot(&stems_dir.join("Artist - Song")), before);
    let loaded = deck::load(&[rig.root()], &rig.track_file(7)).unwrap();
    assert_eq!((loaded.identity, loaded.pads.len()), (Identity::Proven, 4));
}

#[test]
fn a_set_that_was_set_aside_is_put_back_before_its_own_track_is_written_again() {
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    written(&rig, 7);
    let stems_dir = rig.root().join("STEMS");
    set_aside(&stems_dir, "Artist - Song");

    let again = written(&rig, 7);

    assert!(again.replaced);
    assert_eq!(names_in(&stems_dir), ["Artist - Song"]);
}

#[test]
fn a_set_aside_set_whose_replacement_arrived_is_cleared() {
    let rig = Rig::spare(&[("Artist - Song.wav", 7), ("Other.wav", 8)]);
    written(&rig, 7);
    let stems_dir = rig.root().join("STEMS");
    set_aside(&stems_dir, "Artist - Song");
    written(&rig, 7);
    // Interrupted after the new set was renamed in, before the old one
    // was removed.
    let old = stems_dir.join(".wbx-0badc0de.old");
    std::fs::create_dir(&old).unwrap();
    std::fs::write(old.join("vocals.ogg"), b"old").unwrap();
    std::fs::write(stems_dir.join(".wbx-0badc0de.dest"), "Artist - Song").unwrap();
    let current = deck::snapshot(&stems_dir.join("Artist - Song"));

    written(&rig, 8);

    assert_eq!(names_in(&stems_dir), ["Artist - Song", "Other"]);
    assert_eq!(deck::snapshot(&stems_dir.join("Artist - Song")), current);
}

#[test]
fn a_set_aside_folder_with_no_note_of_where_it_belongs_is_left_alone() {
    // It may be the only complete copy of something: not this writer's to
    // delete on a guess. Other tools' working folders are theirs.
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    let stems_dir = rig.root().join("STEMS");
    for kept in [".wbx-0badc0de.old", ".Other Song.old", ".azmods-1234"] {
        std::fs::create_dir_all(stems_dir.join(kept)).unwrap();
        std::fs::write(stems_dir.join(kept).join("vocals.ogg"), b"kept").unwrap();
    }

    written(&rig, 7);

    assert_eq!(
        names_in(&stems_dir),
        [
            ".Other Song.old",
            ".azmods-1234",
            ".wbx-0badc0de.old",
            "Artist - Song"
        ]
    );
}
