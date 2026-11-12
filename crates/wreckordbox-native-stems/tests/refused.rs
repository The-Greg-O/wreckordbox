//! What `write` refuses, and that a refusal leaves the Stick as it was.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod deck;
mod support;

use support::Rig;
use wreckordbox_native_stems::{write, NativeStemsError, Request};

#[test]
fn a_stick_without_the_spare_stick_marker_is_refused_before_anything_is_written() {
    let rig = Rig::unmarked(&[("Artist - Song.wav", 7)]);
    let stems = rig.stems("set", 30_000);
    let before = deck::snapshot(rig.root());

    let error = write(&rig.request(7, &stems)).unwrap_err();

    assert!(
        matches!(error, NativeStemsError::NotSpareStick { .. }),
        "{error:?}"
    );
    assert!(error.to_string().contains(".rbxport-disposable"), "{error}");
    assert_eq!(
        deck::snapshot(rig.root()),
        before,
        "the Stick was written to"
    );
    assert!(!rig.root().join("STEMS").exists());
}

#[test]
fn a_folder_named_like_the_marker_does_not_make_a_spare_stick() {
    let rig = Rig::unmarked(&[("Artist - Song.wav", 7)]);
    std::fs::create_dir(rig.root().join(".rbxport-disposable")).unwrap();
    let stems = rig.stems("set", 30_000);

    let error = write(&rig.request(7, &stems)).unwrap_err();

    assert!(
        matches!(error, NativeStemsError::NotSpareStick { .. }),
        "{error:?}"
    );
    assert!(!rig.root().join("STEMS").exists());
}

#[test]
fn a_track_with_no_export_on_the_stick_is_refused() {
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    let stems = rig.stems("set", 30_000);

    let error = write(&rig.request(8, &stems)).unwrap_err();

    assert!(
        matches!(error, NativeStemsError::TrackNotOnStick { track_id: 8 }),
        "{error:?}"
    );
    assert!(!rig.root().join("STEMS").exists());
}

#[test]
fn a_spare_stick_with_no_exported_library_is_refused() {
    let stick = tempfile::tempdir().unwrap();
    std::fs::write(stick.path().join(".rbxport-disposable"), b"").unwrap();
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    let stems = rig.stems("set", 30_000);

    let error = write(&Request {
        stick: stick.path(),
        ..rig.request(7, &stems)
    })
    .unwrap_err();

    assert!(
        matches!(error, NativeStemsError::NoLibrary { .. }),
        "{error:?}"
    );
    assert!(!stick.path().join("STEMS").exists());
}

#[test]
fn a_stick_exported_from_another_library_does_not_hold_this_librarys_track() {
    // The Stick's library names the Library each track came from. A track
    // with the same identifier in another Library is another track.
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    let stems = rig.stems("set", 30_000);
    rig.exported_from_library(1_000_000_001);
    let from = rig.exported_from(7);
    assert_eq!(from, 1_000_000_001);

    let other = write(&Request {
        library_id: from + 1,
        ..rig.request(7, &stems)
    })
    .unwrap_err();
    assert!(
        matches!(other, NativeStemsError::TrackNotOnStick { track_id: 7 }),
        "{other:?}"
    );

    write(&Request {
        library_id: from,
        ..rig.request(7, &stems)
    })
    .unwrap();
    assert!(deck::load(&[rig.root()], &rig.track_file(7)).is_some());
}
