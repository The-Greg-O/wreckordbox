//! Rendering from Stems: the kept ones summed, the muted ones absent, and
//! Stems that do not fit their parent refused.
//!
//! The Stems are generated: four Stems, each a tone of its own on the left
//! and another on the right, written as 16-bit WAV the way the Sidecar
//! stores them. Every render is read back through `rbl-audio`.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod support;

use std::path::Path;

use support::{
    channel, level_at, read, sine, StemBench, AMPLITUDE, AMPLITUDE_RIGHT, HZ, HZ_RIGHT, RATE,
};
use wreckordbox_render::{
    render, Destination, Edits, Format, Kept, MutantSource, Proceed, RenderError, Rendered,
    StemSum, Source,
};

const VOCALS_AND_BASS: Kept = Kept {
    vocals: true,
    other: false,
    bass: true,
    drums: false,
};

fn kept_of(mask: [bool; 4]) -> Kept {
    Kept {
        vocals: mask[0],
        other: mask[1],
        bass: mask[2],
        drums: mask[3],
    }
}

fn render_to(source: &mut dyn Source, edits: Edits, out: &Path) -> Result<Rendered, RenderError> {
    render(source, edits, &Destination::wav(out), &mut |_| {
        Proceed::Continue
    })
}

#[test]
fn every_kept_combination_sums_exactly_on_each_side_and_muted_stems_are_absent() {
    let bench = StemBench::new(RATE as usize);
    let parent = bench.parent();
    // Every non-empty choice short of all four (which is the parent itself).
    for bits in 1_u8..15 {
        let mask: [bool; 4] = std::array::from_fn(|i| bits & (1 << i) != 0);
        let out = bench.path(&format!("out-{bits}.wav"));
        let mut source = MutantSource::open(&parent, kept_of(mask), Some(&bench.stems())).unwrap();
        let rendered = render_to(&mut source, Edits::default(), &out).unwrap();

        let (frames, rate) = read(&out);
        assert_eq!(rate, RATE);
        assert_eq!(rendered.frames, bench.frames as u64);
        assert_eq!(frames, bench.sum(mask), "{mask:?}: the kept Stems, summed sample for sample");

        // Left and right are each their own Stems' side: a swap or a mix of
        // the two would put a right-hand tone on the left.
        let (left, right) = (channel(&frames, 0), channel(&frames, 1));
        for i in 0..4 {
            for (signal, hz, amplitude, side) in [
                (&left, HZ[i], AMPLITUDE, "left"),
                (&right, HZ_RIGHT[i], AMPLITUDE_RIGHT, "right"),
            ] {
                let level = level_at(signal, hz);
                if mask[i] {
                    assert!(
                        (level - amplitude).abs() < 0.01,
                        "{mask:?}: {hz} Hz kept on the {side} at {level}"
                    );
                } else {
                    assert!(level < 1e-4, "{mask:?}: {hz} Hz muted, yet {level} on the {side}");
                }
            }
            assert!(level_at(&left, HZ_RIGHT[i]) < 1e-4, "{mask:?}: a right tone on the left");
            assert!(level_at(&right, HZ[i]) < 1e-4, "{mask:?}: a left tone on the right");
        }
    }
}

#[test]
fn opposite_stems_cancel_to_silence() {
    let frames = 10_000;
    let up = sine(440.0, frames, 0.5);
    let down: Vec<i16> = up.iter().map(|s| -s).collect();
    let quiet = vec![0_i16; frames];
    let bench = StemBench::with(
        frames,
        [
            (up.clone(), down.clone()),
            (down, up),
            (quiet.clone(), quiet.clone()),
            (quiet.clone(), quiet),
        ],
    );
    let parent = bench.parent();
    let out = bench.path("out.wav");
    let kept = kept_of([true, true, false, false]);
    let mut source = MutantSource::open(&parent, kept, Some(&bench.stems())).unwrap();
    render_to(&mut source, Edits::default(), &out).unwrap();
    let (rendered, _) = read(&out);
    assert_eq!(rendered.len(), frames);
    assert!(rendered.iter().all(|f| *f == [0, 0]), "not silence");
}

#[test]
fn a_sum_past_full_scale_saturates_and_never_wraps() {
    // Two Stems at nine tenths of full scale in phase on the left and in
    // opposite phases on the right: the left sum reaches 1.8, the right 0.
    let frames = 10_000;
    let loud = sine(100.0, frames, 0.9);
    let inverted: Vec<i16> = loud.iter().map(|s| -s).collect();
    let quiet = vec![0_i16; frames];
    let bench = StemBench::with(
        frames,
        [
            (loud.clone(), loud.clone()),
            (loud.clone(), inverted),
            (quiet.clone(), quiet.clone()),
            (quiet.clone(), quiet),
        ],
    );
    let parent = bench.parent();
    let out = bench.path("out.wav");
    let mask = [true, true, false, false];
    let mut source = MutantSource::open(&parent, kept_of(mask), Some(&bench.stems())).unwrap();
    render_to(&mut source, Edits::default(), &out).unwrap();
    let (rendered, _) = read(&out);
    assert_eq!(rendered, bench.sum(mask), "clipped at full scale");
    assert!(rendered.iter().any(|f| f[0] == i16::MAX));
    assert!(rendered.iter().any(|f| f[0] == i16::MIN));
    // A wrapped sum would flip sign at the peaks: every left sample has the
    // sign of the Stem it was made from.
    for (got, stem) in rendered.iter().zip(&loud) {
        assert!(i32::from(got[0]) * i32::from(*stem) >= 0, "{got:?} from {stem}");
    }
    assert!(rendered.iter().all(|f| f[1] == 0));
}

#[test]
fn all_four_stems_summed_directly_give_back_their_parent() {
    // Tolerance: none. The generated parent is the integer sum of its Stems
    // (in range on both sides), so the sum must equal it to the sample;
    // real Stems match their mix only to the separation's error, which is
    // why keeping all four renders the parent instead.
    let bench = StemBench::new(RATE as usize / 2);
    let parent = bench.parent();
    let out = bench.path("out.wav");
    let mut sum = StemSum::open(&parent, &bench.stems(), Kept::ALL).unwrap();
    render_to(&mut sum, Edits::default(), &out).unwrap();
    let (rendered, _) = read(&out);
    let (want, _) = read(&parent);
    let worst = rendered
        .iter()
        .zip(&want)
        .flat_map(|(a, b)| [(a[0] - b[0]).abs(), (a[1] - b[1]).abs()])
        .max()
        .unwrap();
    assert_eq!(rendered.len(), want.len());
    assert_eq!(worst, 0, "{worst} steps from the parent");
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
    assert!(matches!(
        MutantSource::open(&parent, kept_of([false; 4]), Some(&bench.stems())),
        Err(RenderError::NothingKept)
    ));
}

/// Renders vocals and bass of `stems` against `parent` to a fresh folder,
/// expecting a refusal, and checks nothing at all was left in the folder.
fn refused(stems: &wreckordbox_render::Stems, parent: &Path) -> RenderError {
    let outdir = tempfile::tempdir().unwrap();
    let out = outdir.path().join("out.wav");
    let error = MutantSource::open(parent, VOCALS_AND_BASS, Some(stems))
        .and_then(|mut source| render_to(&mut source, Edits::default(), &out))
        .unwrap_err();
    assert_eq!(
        std::fs::read_dir(outdir.path()).unwrap().count(),
        0,
        "nothing written"
    );
    error
}

fn renders(stems: &wreckordbox_render::Stems, parent: &Path) {
    let outdir = tempfile::tempdir().unwrap();
    let out = outdir.path().join("out.wav");
    let mut source = MutantSource::open(parent, VOCALS_AND_BASS, Some(stems)).unwrap();
    let rendered = render_to(&mut source, Edits::default(), &out).unwrap();
    assert_eq!(rendered.frames, stems.frames);
    assert_eq!(rendered.sample_rate, 44_100);
}

#[test]
fn stems_are_held_to_the_length_of_the_actual_parent_file() {
    // A consistent set: every Stem is `frames` long and says so. Only the
    // parent on disk differs.
    let bench = StemBench::new(RATE as usize / 2);
    let stems = bench.stems();
    let frames = bench.frames;

    renders(&stems, &bench.parent_of("same.wav", frames, RATE));
    for (name, length) in [
        ("one-short.wav", frames - 1),
        ("much-shorter.wav", frames / 2),
        ("one-long.wav", frames + 1),
        ("twice.wav", frames * 2),
    ] {
        let parent = bench.parent_of(name, length, RATE);
        assert!(
            matches!(
                refused(&stems, &parent),
                RenderError::StemsDoNotFitParent { stems: s, parent_frames, parent_rate: 44_100, .. }
                    if s == frames as u64 && parent_frames == length as u64
            ),
            "{name}"
        );
    }
    // No parent at all is no parent to fit.
    assert!(matches!(
        refused(&stems, &bench.path("gone.wav")),
        RenderError::Source(_)
    ));
}

#[test]
fn a_parent_at_another_rate_fits_when_it_lasts_as_long() {
    // Half a second of Stems at 44.1 kHz is 24,000 frames at 48 kHz and
    // 11,025 at 22.05 kHz.
    let bench = StemBench::new(RATE as usize / 2);
    let stems = bench.stems();
    renders(&stems, &bench.parent_of("48k.wav", 24_000, 48_000));
    renders(&stems, &bench.parent_of("22k.wav", 11_025, 22_050));
    // The same frame count at a higher rate is a shorter track; at a lower
    // one a longer one.
    for (name, length, rate) in [
        ("48k-same-frames.wav", bench.frames, 48_000),
        ("48k-short.wav", 23_998, 48_000),
        ("48k-long.wav", 24_002, 48_000),
        ("22k-same-frames.wav", bench.frames, 22_050),
    ] {
        let parent = bench.parent_of(name, length, rate);
        assert!(
            matches!(
                refused(&stems, &parent),
                RenderError::StemsDoNotFitParent { parent_rate, .. } if parent_rate == rate
            ),
            "{name}"
        );
    }
}

#[test]
fn stems_not_at_44_1_khz_are_refused_even_with_a_parent_that_fits() {
    // A set made at 48 kHz of a 48 kHz parent, consistent in itself: the
    // Stem contract is 44.1 kHz.
    let frames = 24_000;
    let bench = StemBench::new(frames);
    for name in support::NAMES {
        let tone = sine(440.0, frames, 0.2);
        support::write_wav_rate(&bench.path(&format!("{name}.wav")), &tone, &tone, 48_000);
    }
    let parent = bench.parent_of("parent.wav", frames, 48_000);
    assert!(matches!(
        refused(&bench.stems(), &parent),
        RenderError::StemRate {
            expected: 44_100,
            found: 48_000,
            ..
        }
    ));
}

#[test]
fn a_lossy_parent_is_refused_until_its_stems_can_be_lined_up() {
    // Generated MP3 and AAC parents: their decoded length carries encoder
    // delay and padding, so the Stems' place on them is not known.
    let bench = StemBench::new(RATE as usize / 2);
    let wav = bench.parent();
    let mp3 = bench.path("parent.mp3");
    let mut track = wreckordbox_render::TrackSource::open(&wav).unwrap();
    render(
        &mut track,
        Edits::default(),
        &Destination::new(&mp3, Format::Mp3),
        &mut |_| Proceed::Continue,
    )
    .unwrap();
    let m4a =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../rbl-audio/tests/fixtures/tone-faststart.m4a");
    for parent in [mp3.as_path(), m4a.as_path()] {
        assert!(
            matches!(
                refused(&bench.stems(), parent),
                RenderError::StemAlignmentUnknown { .. }
            ),
            "{parent:?}"
        );
    }
}

#[test]
fn one_kept_stem_of_another_length_than_the_set_is_refused() {
    let bench = StemBench::new(RATE as usize / 2);
    let parent = bench.parent();
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
        RenderError::Source(rbl_audio::AudioError::Truncated { .. })
            | RenderError::StemLength { .. }
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
    let vocals_only = kept_of([true, false, false, false]);
    let edits = Edits {
        key_shift: 2,
        tempo: 1.05,
    };
    let mut source = MutantSource::open(&parent, vocals_only, Some(&bench.stems())).unwrap();
    render_to(&mut source, edits, &out).unwrap();

    let (frames, _) = read(&out);
    assert_eq!(frames.len(), (bench.frames as f64 / 1.05).round() as usize);
    let shift = 2.0_f64.powf(2.0 / 12.0);
    for (side, hz, others) in [(0, HZ[0], &HZ[1..]), (1, HZ_RIGHT[0], &HZ_RIGHT[1..])] {
        let signal = channel(&frames, side);
        let want = hz * shift;
        let got = hz_of(&signal);
        let cents = 1200.0 * (got / want).log2();
        assert!(cents.abs() < 3.0, "side {side}: {got} Hz, {cents} cents from {want}");
        let settled = &signal[signal.len() / 4..signal.len() * 3 / 4];
        for hz in others {
            for shifted in [*hz, hz * shift] {
                let level = level_at(settled, shifted);
                assert!(level < 0.01, "muted {hz} Hz measures {level} at {shifted} Hz");
            }
        }
    }
}
