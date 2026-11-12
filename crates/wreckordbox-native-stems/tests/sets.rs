//! A set as a whole: its identity record, two songs of one name, a set
//! made again, what an interrupted write leaves, and the rest of the Stick.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod deck;
mod support;

use deck::Identity;
use rbl_export::device_library::{self, Format};
use support::Rig;
use wreckordbox_native_stems::{write, NativeStemsError, Outcome, Request};

fn written(rig: &Rig, track: u64, frames: u64) -> wreckordbox_native_stems::Written {
    let stems = rig.stems(&format!("set-{track}-{frames}"), frames);
    match write(&rig.request(track, &stems)).unwrap() {
        Outcome::Written(written) => written,
        other => panic!("not written: {other:?}"),
    }
}

// Sets whose length is read back are 40,000 frames or more: a shorter one
// fits a single Ogg page, which decoders do not trim to the stream's
// length (see `support::write_tone`).

fn names_in(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn the_set_says_which_file_it_was_made_from_and_the_deck_takes_it_as_proven() {
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    // A track file with known bytes, longer than the fingerprint's three
    // regions together, so its fingerprint can be checked against the
    // layout's reference implementation (run in Python over these bytes).
    let bytes: Vec<u8> = (0..200_001_u32).map(|i| (i * 7 % 251) as u8).collect();
    std::fs::write(rig.track_file(7), &bytes).unwrap();

    let written = written(&rig, 7, 20_000);

    let text = std::fs::read_to_string(written.dir.join("source.json")).unwrap();
    assert!(
        text.ends_with("}\n") && text.lines().count() == 1,
        "{text:?}"
    );
    let record = deck::read_source_record(&written.dir).expect("the deck can read the record");
    assert_eq!(
        record.file.as_deref(),
        Some("Contents/Artist 7/Single/Artist - Song.wav")
    );
    assert_eq!(record.size, Some(200_001));
    assert_eq!(record.fp.as_deref(), Some("fnv1a64:2e8bc8fed7764fc0"));
    let mtime = std::fs::metadata(rig.track_file(7))
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert_eq!(record.mtime, Some(mtime as i64));
    let loaded = deck::load(&[rig.root()], &rig.track_file(7)).unwrap();
    assert_eq!(loaded.identity, Identity::Proven);
}

#[test]
fn a_small_track_file_is_fingerprinted_with_its_regions_overlapping() {
    // 3007 bytes of 0x07: what the rig exports for track 7. Reference
    // value from the layout's Python.
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    assert_eq!(std::fs::read(rig.track_file(7)).unwrap(), vec![7_u8; 3007]);

    let written = written(&rig, 7, 20_000);

    let record = deck::read_source_record(&written.dir).unwrap();
    assert_eq!(record.fp.as_deref(), Some("fnv1a64:dea03f5f7764989c"));
}

#[test]
fn a_path_outside_ascii_is_escaped_the_way_the_deck_reads_it() {
    let name = "Ünï — 曲 𝄞.wav";
    let rig = Rig::spare(&[(name, 6)]);
    let in_library = rig.library_path(6);
    assert!(in_library.ends_with(name), "{in_library}");

    let written = written(&rig, 6, 20_000);

    // As Python's default json.dumps writes it.
    let text = std::fs::read_to_string(written.dir.join("source.json")).unwrap();
    assert!(
        text.starts_with(
            "{\"file\": \"Contents/Artist 6/Single/\\u00dcn\\u00ef \\u2014 \\u66f2 \\ud834\\udd1e.wav\", \"size\": "
        ),
        "{text}"
    );
    assert!(text.is_ascii());
    let record = deck::read_source_record(&written.dir).unwrap();
    assert_eq!(record.file.as_deref(), Some(in_library.as_str()));
    assert_eq!(
        deck::load(&[rig.root()], &rig.track_file(6))
            .unwrap()
            .identity,
        Identity::Proven
    );
}

#[test]
fn two_songs_with_one_file_name_each_get_their_own_set() {
    let rig = Rig::spare(&[("Song.wav", 7), ("Song.wav", 8)]);
    assert_ne!(rig.library_path(7), rig.library_path(8));

    let first = written(&rig, 7, 40_000);
    let second = written(&rig, 8, 48_000);

    assert_eq!(first.folder, "Song");
    assert!(!second.replaced);
    // CRC-32 of "Contents/Artist 8/Single/Song.wav", from Python's zlib.
    assert_eq!(rig.library_path(8), "Contents/Artist 8/Single/Song.wav");
    assert_eq!(second.folder, "Song-e40da05e");
    let (one, two) = (
        deck::load(&[rig.root()], &rig.track_file(7)).unwrap(),
        deck::load(&[rig.root()], &rig.track_file(8)).unwrap(),
    );
    assert_eq!(
        (one.folder.clone(), one.rank, one.identity),
        (first.dir, 0, Identity::Proven)
    );
    assert_eq!(
        (two.folder.clone(), two.rank, two.identity),
        (second.dir, 2, Identity::Proven)
    );
    // Each plays its own audio: the two sets differ in length.
    assert_eq!((one.frames(), two.frames()), (40_000, 48_000));

    // Making the second song's set again replaces its own folder.
    let again = written(&rig, 8, 52_000);
    assert_eq!(
        (again.folder.as_str(), again.replaced),
        ("Song-e40da05e", true)
    );
    assert_eq!(
        names_in(&rig.root().join("STEMS")),
        ["Song", "Song-e40da05e"]
    );
    assert_eq!(
        deck::load(&[rig.root()], &rig.track_file(7))
            .unwrap()
            .frames(),
        40_000
    );
}

#[test]
fn a_set_without_a_record_is_this_tracks_by_name_and_is_replaced() {
    // A set some other writer left with no source.json: the deck loads it
    // for this track by name, so a new set takes its place.
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    let theirs = rig.root().join("STEMS/artist - song");
    std::fs::create_dir_all(&theirs).unwrap();
    std::fs::write(theirs.join("vocals.ogg"), b"not vorbis").unwrap();

    let written = written(&rig, 7, 20_000);

    assert!(written.replaced);
    assert_eq!(names_in(&rig.root().join("STEMS")), ["Artist - Song"]);
    let loaded = deck::load(&[rig.root()], &rig.track_file(7)).unwrap();
    assert_eq!((loaded.pads.len(), loaded.identity), (4, Identity::Proven));
}

#[test]
fn making_a_set_again_swaps_the_whole_folder_and_the_decks_cache_goes_with_it() {
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    let first = written(&rig, 7, 20_000);
    assert!(!first.replaced);
    // What the deck leaves in the folder after decoding a set.
    std::fs::write(first.dir.join("azstem.env"), b"AZENV1\n").unwrap();

    let second = written(&rig, 7, 44_000);

    assert!(second.replaced);
    assert_eq!(second.dir, first.dir);
    assert_eq!(
        names_in(&second.dir),
        [
            "bass.ogg",
            "drums.ogg",
            "other.ogg",
            "source.json",
            "vocals.ogg"
        ]
    );
    assert_eq!(names_in(&rig.root().join("STEMS")), ["Artist - Song"]);
    assert_eq!(
        deck::load(&[rig.root()], &rig.track_file(7))
            .unwrap()
            .frames(),
        44_000
    );
}

#[test]
fn what_an_interrupted_write_left_is_cleared_and_other_dot_folders_are_left_alone() {
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    let stems_dir = rig.root().join("STEMS");
    for leftover in [".Other Song.part", ".Other Song.old", ".azmods-1234"] {
        std::fs::create_dir_all(stems_dir.join(leftover)).unwrap();
        std::fs::write(stems_dir.join(leftover).join("vocals.ogg"), b"half").unwrap();
    }

    written(&rig, 7, 20_000);

    // The AZ-MODS app's own working folder is its to clear.
    assert_eq!(names_in(&stems_dir), [".azmods-1234", "Artist - Song"]);
}

#[test]
fn stems_of_unequal_length_are_refused_and_nothing_appears_for_the_deck() {
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    let stems = rig.stems("set", 20_000);
    support::write_tone(&stems.drums, 19_999, 2, 220.0);

    let error = write(&rig.request(7, &stems)).unwrap_err();

    assert!(
        matches!(
            error,
            NativeStemsError::StemLength {
                frames: 19_999,
                expected: 20_000,
                ..
            }
        ),
        "{error:?}"
    );
    assert_eq!(names_in(&rig.root().join("STEMS")), Vec::<String>::new());
    assert!(deck::load(&[rig.root()], &rig.track_file(7)).is_none());
}

#[test]
fn stems_that_are_not_as_long_as_their_parent_are_refused() {
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    let stems = rig.stems("set", 20_000);

    let error = write(&Request {
        frames: Some(20_001),
        ..rig.request(7, &stems)
    })
    .unwrap_err();

    assert!(
        matches!(error, NativeStemsError::StemLength { .. }),
        "{error:?}"
    );
    assert!(deck::load(&[rig.root()], &rig.track_file(7)).is_none());
}

#[test]
fn a_failed_remake_leaves_the_earlier_set_playing() {
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    written(&rig, 7, 40_000);
    let stems = rig.stems("bad", 20_000);
    std::fs::write(&stems.other, b"not audio at all").unwrap();

    let error = write(&rig.request(7, &stems)).unwrap_err();

    assert!(matches!(error, NativeStemsError::Audio { .. }), "{error:?}");
    assert_eq!(names_in(&rig.root().join("STEMS")), ["Artist - Song"]);
    let loaded = deck::load(&[rig.root()], &rig.track_file(7)).unwrap();
    assert_eq!((loaded.pads.len(), loaded.frames()), (4, 40_000));
}

#[test]
fn stems_at_another_sample_rate_are_refused_rather_than_written_for_the_deck_to_resample() {
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    let stems = rig.stems("set", 20_000);
    support::write_tone_at(&stems.vocals, 20_000, 2, 880.0, 48_000);

    let error = write(&rig.request(7, &stems)).unwrap_err();

    assert!(matches!(error, NativeStemsError::Audio { .. }), "{error:?}");
    assert!(error.to_string().contains("48000"), "{error}");
    assert!(deck::load(&[rig.root()], &rig.track_file(7)).is_none());
}

#[test]
fn a_track_whose_file_is_gone_from_the_stick_is_refused() {
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    std::fs::remove_file(rig.track_file(7)).unwrap();
    let stems = rig.stems("set", 20_000);

    let error = write(&rig.request(7, &stems)).unwrap_err();

    assert!(
        matches!(
            error,
            NativeStemsError::TrackFileMissing { track_id: 7, .. }
        ),
        "{error:?}"
    );
    assert!(!rig.root().join("STEMS").exists());
}

#[test]
fn the_sticks_exported_library_reads_back_the_same_and_nothing_else_on_it_is_touched() {
    let rig = Rig::spare(&[("Artist - Song.wav", 7), ("Other.wav", 8)]);
    let libraries = |rig: &Rig| {
        [Format::DeviceLibrary, Format::OneLibrary]
            .map(|f| device_library::read(rig.root(), f).unwrap())
    };
    let (before, files_before) = (libraries(&rig), deck::snapshot(rig.root()));
    assert!(rbl_export::verify(rig.root()).unwrap().is_ok());

    let written = written(&rig, 7, 20_000);

    // Upstream's readers and its own check of an export still pass.
    assert_eq!(libraries(&rig), before);
    assert_eq!(before[1].tracks.len(), 2);
    let report = rbl_export::verify(rig.root()).unwrap();
    assert!(
        report.is_ok(),
        "{}",
        rbl_export::verification_failure(&report)
    );
    // Byte for byte, everything that was on the Stick is as it was, and
    // the only additions are the set's five files.
    let mut files_after = deck::snapshot(rig.root());
    let folder = std::path::Path::new("STEMS").join(&written.folder);
    for name in [
        "vocals.ogg",
        "other.ogg",
        "bass.ogg",
        "drums.ogg",
        "source.json",
    ] {
        assert!(files_after.remove(&folder.join(name)).is_some(), "{name}");
    }
    assert!(
        files_after == files_before,
        "something else on the Stick changed"
    );
    assert!(files_before.keys().any(|p| p.starts_with("PIONEER")));
    assert!(files_before.keys().any(|p| p.starts_with("Contents")));
}
