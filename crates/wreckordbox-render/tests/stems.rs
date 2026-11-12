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

#[test]
fn keeping_all_four_stems_is_no_stem_edit_and_renders_the_parent() {
    let bench = StemBench::new(RATE as usize / 2);
    // A parent that is not the sum of its Stems, so the render says which
    // it was made from.
    let parent = bench.path("parent.wav");
    let tone = support::sine(330.0, bench.frames + 100, 0.3);
    support::write_wav(&parent, &tone, &tone);
    // Stems that could not be rendered: one is the wrong length.
    let mut stems = bench.stems();
    stems.frames += 1;

    for stems in [Some(&stems), None] {
        let out = bench.path("out.wav");
        let _ = std::fs::remove_file(&out);
        let mut source = MutantSource::open(&parent, Kept::ALL, stems).unwrap();
        render_to(&mut source, Edits::default(), &out).unwrap();
        assert_eq!(read(&out).0, read(&parent).0, "the parent's own audio");
    }
}

#[test]
fn a_stem_edit_needs_stems_and_something_kept() {
    let bench = StemBench::new(1_000);
    let parent = bench.parent();
    assert!(matches!(
        MutantSource::open(&parent, VOCALS_AND_BASS, None),
        Err(RenderError::StemsMissing)
    ));
    let none = Kept {
        vocals: false,
        other: false,
        bass: false,
        drums: false,
    };
    assert!(matches!(
        MutantSource::open(&parent, none, Some(&bench.stems())),
        Err(RenderError::NothingKept)
    ));
}

/// Renders the bench's vocals and bass to `out`, expecting a refusal, and
/// checks nothing at all was left in the output folder.
fn refused(stems: &wreckordbox_render::Stems, parent: &Path) -> RenderError {
    let outdir = tempfile::tempdir().unwrap();
    let out = outdir.path().join("out.wav");
    let error = MutantSource::open(parent, VOCALS_AND_BASS, Some(stems))
        .and_then(|mut source| render_to(&mut source, Edits::default(), &out))
        .unwrap_err();
    assert_eq!(std::fs::read_dir(outdir.path()).unwrap().count(), 0, "nothing written");
    error
}

#[test]
fn stems_whose_length_does_not_match_their_parent_are_refused() {
    let bench = StemBench::new(RATE as usize / 2);
    let parent = bench.parent();

    // The parent is longer than its Stems.
    let mut longer = bench.stems();
    longer.frames += 1;
    assert!(matches!(
        refused(&longer, &parent),
        RenderError::StemLength { expected, found, .. }
            if expected == bench.frames as u64 + 1 && found == bench.frames as u64
    ));

    // The parent is shorter.
    let mut shorter = bench.stems();
    shorter.frames -= 1;
    assert!(matches!(refused(&shorter, &parent), RenderError::StemLength { .. }));

    // One kept Stem is shorter than the others.
    let short = support::sine(HZ[2], bench.frames - 300, AMPLITUDE);
    support::write_wav(&bench.path("bass.wav"), &short, &short);
    assert!(matches!(
        refused(&bench.stems(), &parent),
        RenderError::StemLength { ref stem, found, .. }
            if stem.ends_with("bass.wav") && found == bench.frames as u64 - 300
    ));
}

#[test]
fn a_stem_cut_short_is_refused_not_rendered_shorter() {
    let bench = StemBench::new(RATE as usize / 2);
    let parent = bench.parent();
    // The header still declares the whole Stem; the data stops halfway.
    let path = bench.path("vocals.wav");
    let bytes = std::fs::read(&path).unwrap();
    std::fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
    assert!(matches!(
        refused(&bench.stems(), &parent),
        RenderError::Source(rbl_audio::AudioError::Truncated { .. }) | RenderError::StemLength { .. }
    ));
}

#[test]
fn stems_at_different_rates_are_refused() {
    let bench = StemBench::new(1_000);
    let parent = bench.parent();
    let samples = support::sine(HZ[2], bench.frames, AMPLITUDE);
    support::write_wav_rate(&bench.path("bass.wav"), &samples, &samples, 48_000);
    assert!(matches!(
        refused(&bench.stems(), &parent),
        RenderError::StemRate { expected: 44_100, found: 48_000, .. }
    ));
}

/// Frequency over the settled middle half, from interpolated rising zero
/// crossings (as `tests/render.rs` measures it).
#[cfg(feature = "rubberband")]
fn hz_of(signal: &[f64]) -> f64 {
    let middle = &signal[signal.len() / 4..signal.len() - signal.len() / 4];
    let crossings: Vec<f64> = middle
        .windows(2)
        .enumerate()
        .filter(|(_, w)| w[0] <= 0.0 && w[1] > 0.0)
        .map(|(i, w)| i as f64 + w[0] / (w[0] - w[1]))
        .collect();
    (crossings.len() - 1) as f64 * f64::from(RATE) / (crossings[crossings.len() - 1] - crossings[0])
}

#[cfg(feature = "rubberband")]
#[test]
fn a_stem_edit_combines_with_key_shift_and_tempo() {
    let bench = StemBench::new(RATE as usize);
    let parent = bench.parent();
    let out = bench.path("out.wav");
    let vocals_only = Kept {
        vocals: true,
        other: false,
        bass: false,
        drums: false,
    };
    let edits = Edits {
        key_shift: 2,
        tempo: 1.05,
    };
    let mut source = MutantSource::open(&parent, vocals_only, Some(&bench.stems())).unwrap();
    render_to(&mut source, edits, &out).unwrap();

    let (frames, _) = read(&out);
    assert_eq!(frames.len(), (bench.frames as f64 / 1.05).round() as usize);
    let left = channel(&frames, 0);
    let want = HZ[0] * 2.0_f64.powf(2.0 / 12.0);
    let got = hz_of(&left);
    let cents = 1200.0 * (got / want).log2();
    assert!(cents.abs() < 3.0, "{got} Hz, {cents} cents from {want}");
    let settled = &left[left.len() / 4..left.len() * 3 / 4];
    for hz in &HZ[1..] {
        for shifted in [*hz, hz * 2.0_f64.powf(2.0 / 12.0)] {
            let level = level_at(settled, shifted);
            assert!(level < 0.01, "muted {hz} Hz measures {level} at {shifted} Hz");
        }
    }
}
