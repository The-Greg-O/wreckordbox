//! Exact length, for the Stems the earlier fixtures avoided: short ones,
//! silent ones and bare tones, which encode into very little.
//!
//! Two things are read back for each file and kept apart. What the file
//! *states* is its length by the Ogg container alone (the last page's
//! granule position, no codec involved): that is the writer's to get
//! right. What the test reader's decoder *hands back* is another matter:
//! see `the_test_readers_decoder_...` below.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod deck;
mod support;

use support::Rig;
use wreckordbox_native_stems::{write, Outcome, FILE_NAMES};

fn silent(_: u64) -> i16 {
    0
}

/// A bare 440 Hz tone from the first frame: no noise, no silence.
fn tone(frame: u64) -> i16 {
    ((frame as f64 / 44_100.0 * 440.0 * std::f64::consts::TAU).sin() * 16_000.0) as i16
}

/// The tone with noise over it, as the other tests' Stems have.
fn noisy(frame: u64) -> i16 {
    let hiss = (frame.wrapping_mul(2_654_435_761) >> 7) as i16 / 8;
    tone(frame).saturating_add(hiss)
}

type Fixture = (&'static str, fn(u64) -> i16);
const FIXTURES: [Fixture; 3] = [("silent", silent), ("tone", tone), ("noisy", noisy)];

/// Lengths around Vorbis's block sizes (256 and 2048 at this rate, so
/// steps of 128 and 1024), a tenth of a second, a second, and one that is
/// a multiple of nothing.
const LENGTHS: [u64; 12] = [
    1, 63, 64, 127, 128, 129, 1_023, 1_024, 2_049, 4_410, 44_100, 61_237,
];

/// Writes four Stems of one fixture and length and answers the folder.
fn written(rig: &Rig, kind: &str, sample: fn(u64) -> i16, frames: u64) -> std::path::PathBuf {
    let stems = rig.stems_of(&format!("{kind}-{frames}"), frames, sample);
    match write(&rig.request(7, &stems)).unwrap() {
        Outcome::Written(written) => {
            assert_eq!(written.frames, frames);
            written.dir
        }
        other => panic!("not written: {other:?}"),
    }
}

#[test]
fn every_file_states_exactly_its_sources_length_however_short_or_quiet() {
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    for (kind, sample) in FIXTURES {
        for frames in LENGTHS {
            let dir = written(&rig, kind, sample, frames);
            for name in FILE_NAMES {
                assert_eq!(
                    deck::stated_frames(&dir.join(name)),
                    Some(frames),
                    "{kind}, {frames} frames, {name}"
                );
            }
        }
    }
}

#[test]
fn a_long_silent_stem_states_and_decodes_to_exactly_its_length() {
    // A track with no vocals has a vocals Stem of digital silence. An Ogg
    // page holds at most 255 packets, so minutes of silence still span
    // many pages. Ten seconds here.
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    let frames = 441_013;

    let dir = written(&rig, "silent", silent, frames);

    for name in FILE_NAMES {
        let file = dir.join(name);
        assert_eq!(deck::stated_frames(&file), Some(frames), "{name}");
        assert!(audio_pages(&file) > 1, "{name}");
        assert_eq!(deck::decode(&file).unwrap().frames, frames, "{name}");
    }
}

/// The pages of a file that end a packet of audio: every page after the
/// three headers' (which state position 0).
fn audio_pages(file: &std::path::Path) -> usize {
    deck::ogg_pages(file)
        .unwrap()
        .iter()
        .filter(|page| page.granule.is_some_and(|granule| granule > 0))
        .count()
}

#[test]
fn the_test_readers_decoder_hands_back_the_stated_length_once_audio_spans_two_pages() {
    // The reader's decoder (lewton) cuts the last packet to the stated
    // length only when an earlier page told it where it was; a stream
    // whose audio fits one page comes back whole blocks long. That is the
    // decoder's, not the file's: the file states the right length either
    // way (the test above), and this test pins the difference down so a
    // change in either is seen.
    let rig = Rig::spare(&[("Artist - Song.wav", 7)]);
    let mut one_page = 0;
    let mut several = 0;
    let mut overlong = 0;
    for (kind, sample) in FIXTURES {
        for frames in LENGTHS {
            let dir = written(&rig, kind, sample, frames);
            for name in FILE_NAMES {
                let file = dir.join(name);
                let decoded = deck::decode(&file).unwrap().frames;
                if audio_pages(&file) > 1 {
                    several += 1;
                    assert_eq!(decoded, frames, "{kind}, {frames} frames, {name}");
                } else {
                    one_page += 1;
                    assert!(decoded >= frames, "{kind}, {frames} frames, {name}");
                    overlong += usize::from(decoded > frames);
                }
            }
        }
    }
    assert!(one_page > 0 && several > 0, "{one_page} and {several}");
    // The decoder's shortfall is real, and only ever on one-page streams.
    assert!(overlong > 0, "{overlong} of {one_page}");
}
