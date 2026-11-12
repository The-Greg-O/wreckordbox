//! The naming rules as they come out on a Stick: the folder is named from
//! the path the Stick's exported library holds, and the deck (the
//! independent reader) finds it from that same path.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod deck;
mod support;

use support::Rig;
use wreckordbox_native_stems::naming::Unnameable;
use wreckordbox_native_stems::{write, Outcome};

fn written(rig: &Rig, track: u64) -> wreckordbox_native_stems::Written {
    let stems = rig.stems(&format!("set-{track}"), 20_000);
    match write(&rig.request(track, &stems)).unwrap() {
        Outcome::Written(written) => written,
        other => panic!("not written: {other:?}"),
    }
}

#[test]
fn a_name_ending_in_a_space_or_a_dot_goes_in_a_folder_with_an_underscore() {
    let rig = Rig::spare(&[("Intro .wav", 1), ("Song..wav", 2)]);
    assert!(rig.library_path(1).ends_with("/Intro .wav"));
    assert!(rig.library_path(2).ends_with("/Song..wav"));

    assert_eq!(written(&rig, 1).folder, "Intro _");
    assert_eq!(written(&rig, 2).folder, "Song._");

    for (track, folder) in [(1, "Intro _"), (2, "Song._")] {
        let loaded = deck::load(&[rig.root()], &rig.track_file(track)).unwrap();
        assert_eq!(loaded.folder, rig.root().join("STEMS").join(folder));
        assert_eq!(loaded.rank, 1);
        assert_eq!(loaded.pads.len(), 4);
    }
}

#[test]
fn a_name_outside_ascii_is_kept_byte_for_byte_as_the_library_spells_it() {
    let name = "Ünï — 曲.wav";
    let rig = Rig::spare(&[(name, 6)]);
    let in_library = rig.library_path(6);
    let key = deck::track_key(&in_library);

    let written = written(&rig, 6);

    assert_eq!(written.folder.as_bytes(), key.as_bytes());
    assert_eq!(written.track_path, in_library);
    let loaded = deck::load(&[rig.root()], &rig.track_file(6)).unwrap();
    assert_eq!(loaded.rank, 0);
    assert_eq!(loaded.pads.len(), 4);
}

#[test]
fn the_folder_is_named_from_the_library_not_from_the_file_on_disk() {
    // A Stick whose file system holds the file under another spelling
    // than its library does: the library says precomposed "é", the
    // directory listing says "e" and a combining accent. The deck derives
    // the key from its library, so the folder must carry the library's
    // bytes, and its identity record must still be of the file on disk.
    let rig = Rig::spare(&[("Café.wav", 3)]);
    let in_library = rig.library_path(3);
    assert!(in_library.ends_with("/Café.wav"));
    let on_disk = rig.track_file(3).with_file_name("Cafe\u{301}.wav");
    std::fs::rename(rig.track_file(3), &on_disk).unwrap();

    let written = written(&rig, 3);

    assert_eq!(written.folder, "Café");
    assert_eq!(written.folder.as_bytes(), "Caf\u{e9}".as_bytes());
    assert!(rig.root().join("STEMS/Caf\u{e9}").is_dir());
    assert_eq!(
        deck::identity(&rig.root().join("STEMS/Caf\u{e9}"), &on_disk),
        deck::Identity::Proven
    );
}

#[test]
fn a_track_whose_name_no_folder_can_carry_is_skipped_and_reported_not_failed() {
    let rig = Rig::spare(&[(".hidden.wav", 4), (" .wav", 5), ("Fine.wav", 6)]);
    let stems = rig.stems("set", 20_000);

    let dotted = write(&rig.request(4, &stems)).unwrap();
    let blank = write(&rig.request(5, &stems)).unwrap();

    let Outcome::Skipped(dotted) = dotted else {
        panic!("{dotted:?}")
    };
    assert_eq!(dotted.reason, Unnameable::StartsWithDot);
    assert_eq!(dotted.file_name, ".hidden.wav");
    assert_eq!(dotted.track_path, rig.library_path(4));
    let Outcome::Skipped(blank) = blank else {
        panic!("{blank:?}")
    };
    assert_eq!(blank.reason, Unnameable::Empty);
    // Nothing was made for either, and the deck is not even looking yet.
    assert!(!deck::stems_active(&[rig.root()]));
    assert!(deck::load(&[rig.root()], &rig.track_file(4)).is_none());

    // The next track of the same run is written all the same.
    assert!(matches!(
        write(&rig.request(6, &stems)).unwrap(),
        Outcome::Written(_)
    ));
    assert!(deck::load(&[rig.root()], &rig.track_file(6)).is_some());
    assert!(deck::load(&[rig.root()], &rig.track_file(4)).is_none());
}

#[test]
fn a_file_name_longer_than_the_deck_keeps_is_skipped() {
    // 90 three-byte characters: a name FAT can store (255 UTF-16 units)
    // and the deck's 256-byte buffer cannot. This machine's file system
    // cannot store it either, so only the Stick's library holds it.
    let rig = Rig::spare(&[("Long.wav", 9)]);
    let name = format!("{}.wav", "曲".repeat(90));
    rig.hold_in_library(9, &format!("/Contents/Artist 9/Single/{name}"));
    let stems = rig.stems("set", 20_000);

    let outcome = write(&rig.request(9, &stems)).unwrap();

    let Outcome::Skipped(skipped) = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(skipped.reason, Unnameable::TooLong);
    assert_eq!(skipped.file_name, name);
    assert!(!rig.root().join("STEMS").exists());
}

#[test]
fn a_name_another_tool_put_on_the_stick_follows_the_same_rules() {
    // The export pipeline here renames what a Stick cannot store; a Stick
    // written elsewhere may hold such names in its library all the same.
    let rig = Rig::spare(&[("Device.wav", 1), ("Colon.wav", 2)]);
    let device = rig.track_file(1).with_file_name("CON.wav");
    std::fs::rename(rig.track_file(1), &device).unwrap();
    rig.hold_in_library(1, "/Contents/Artist 1/Single/CON.wav");
    rig.hold_in_library(2, "/Contents/Artist 2/Single/What?.wav");

    assert_eq!(written(&rig, 1).folder, "CON_");
    let loaded = deck::load(&[rig.root()], &device).unwrap();
    assert_eq!((loaded.rank, loaded.pads.len()), (1, 4));

    let stems = rig.stems("set", 20_000);
    let outcome = write(&rig.request(2, &stems)).unwrap();
    let Outcome::Skipped(skipped) = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(skipped.reason, Unnameable::Character('?'));
}

#[test]
fn an_existing_stems_folder_in_another_case_is_used_not_doubled() {
    // "`STEMS`: a directory at the root of the USB, name matched ignoring
    // ASCII case." On FAT a second folder differing only in case cannot
    // be made, so the one there is the one to write into.
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    std::fs::create_dir(rig.root().join("Stems")).unwrap();

    let written = written(&rig, 7);

    assert_eq!(written.dir, rig.root().join("Stems/Artist - Song"));
    assert!(!rig.root().join("STEMS").exists());
    assert!(deck::load(&[rig.root()], &rig.track_file(7)).is_some());
}
