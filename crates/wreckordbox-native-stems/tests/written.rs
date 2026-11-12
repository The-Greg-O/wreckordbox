//! Native stems written to a temporary folder standing in for a Stick, and
//! read back by the independent reader in `deck`.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod deck;
mod support;

use support::Rig;
use wreckordbox_native_stems::{write, Outcome};

#[test]
fn the_deck_loads_four_stems_for_a_track_whose_native_stems_were_written() {
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    let stems = rig.stems("set", 30_000);

    let outcome = write(&rig.request(7, &stems)).unwrap();

    let Outcome::Written(written) = outcome else {
        panic!("not written: {outcome:?}");
    };
    assert_eq!(written.folder, "Artist - Song");
    let loaded = deck::load(&[rig.root()], &rig.track_file(7)).expect("the deck finds a folder");
    assert_eq!(loaded.folder, rig.root().join("STEMS/Artist - Song"));
    assert_eq!(loaded.rank, 0);
    assert_eq!(
        loaded.pads.keys().copied().collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );
    let names: Vec<&str> = loaded.pads.values().map(|d| d.file_name.as_str()).collect();
    assert_eq!(names, ["vocals.ogg", "other.ogg", "bass.ogg", "drums.ogg"]);
    assert!(deck::counts_as_made(&loaded.folder));
}

#[test]
fn each_stem_is_stereo_vorbis_at_44100_hz_exactly_as_long_as_its_source_and_aligned_with_it() {
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    // Not a multiple of any Vorbis block size.
    let frames = 61_237;
    let stems = rig.stems("set", frames);

    write(&rig.request(7, &stems)).unwrap();

    let loaded = deck::load(&[rig.root()], &rig.track_file(7)).unwrap();
    assert_eq!(loaded.pads.len(), 4);
    for (pad, stem) in &loaded.pads {
        assert_eq!(stem.sample_rate, 44_100, "pad {pad}");
        assert_eq!(stem.channels, 2, "pad {pad}");
        // Vorbis records a stream's length to the sample, so nothing is
        // allowed for: no padding at either end.
        assert_eq!(stem.frames, frames, "pad {pad}");
        // The source is silent up to ONSET and a tone from there on. A
        // lossy codec may smear a little energy ahead of an onset (one
        // short block, 128 frames at this rate, is the most Vorbis can),
        // so the Stem is quiet until just before it and loud just after.
        assert!(
            stem.peak(0, support::ONSET - 256) < 200,
            "pad {pad} starts early: {}",
            stem.peak(0, support::ONSET - 256)
        );
        assert!(
            stem.peak(support::ONSET, support::ONSET + 512) > 8_000,
            "pad {pad} starts late: {}",
            stem.peak(support::ONSET, support::ONSET + 512)
        );
    }
    assert_eq!(loaded.frames(), frames);
}

#[test]
fn a_mono_stem_is_written_to_both_sides() {
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    let stems = rig.stems("set", 30_000);
    support::write_tone(&stems.bass, 30_000, 1, 110.0);

    write(&rig.request(7, &stems)).unwrap();

    let loaded = deck::load(&[rig.root()], &rig.track_file(7)).unwrap();
    let bass = &loaded.pads[&3];
    assert_eq!((bass.channels, bass.frames), (2, 30_000));
    let (mut left, mut right) = (0_i16, 0_i16);
    for frame in bass.samples.chunks_exact(2) {
        left = left.max(frame[0].saturating_abs());
        right = right.max(frame[1].saturating_abs());
    }
    assert!(left > 8_000 && right > 8_000, "left {left}, right {right}");
}
