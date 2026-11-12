//! Rendering from Stems: the kept ones summed, the muted ones absent, and
//! Stems that do not fit their parent refused.
//!
//! The Stems are generated: four tones, one per Stem, written as 16-bit WAV
//! the way the Sidecar stores them. Every render is read back through
//! `rbl-audio`.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod support;

use std::path::Path;

use support::{channel, level_at, read, StemBench, AMPLITUDE, HZ, RATE};
use wreckordbox_render::{
    render, Destination, Edits, Kept, MutantSource, Proceed, RenderError, Rendered,
};

const VOCALS_AND_BASS: Kept = Kept {
    vocals: true,
    other: false,
    bass: true,
    drums: false,
};

fn render_to(source: &mut MutantSource, edits: Edits, out: &Path) -> Result<Rendered, RenderError> {
    render(source, edits, &Destination::wav(out), &mut |_| Proceed::Continue)
}

#[test]
fn kept_stems_sum_to_their_samples_and_a_muted_stem_is_absent() {
    let bench = StemBench::new(RATE as usize);
    let parent = bench.parent();
    let out = bench.path("out.wav");

    let mut source = MutantSource::open(&parent, VOCALS_AND_BASS, Some(&bench.stems())).unwrap();
    let rendered = render_to(&mut source, Edits::default(), &out).unwrap();

    let [vocals, _, bass, _] = bench.samples();
    let expected: Vec<[i16; 2]> = vocals
        .iter()
        .zip(&bass)
        .map(|(v, b)| [v[0] + b[0], v[1] + b[1]])
        .collect();
    let (frames, rate) = read(&out);
    assert_eq!(rate, RATE);
    assert_eq!(rendered.frames, bench.frames as u64);
    assert_eq!(frames, expected, "the kept Stems, summed sample for sample");

    let left = channel(&frames, 0);
    for (hz, kept) in HZ.into_iter().zip([true, false, true, false]) {
        let level = level_at(&left, hz);
        if kept {
            assert!((level - AMPLITUDE).abs() < 0.01, "{hz} Hz kept at {level}");
        } else {
            assert!(level < 1e-4, "{hz} Hz muted, yet measures {level}");
        }
    }
}
