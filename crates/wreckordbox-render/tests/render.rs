//! The renderer, measured on generated audio.
//!
//! The same measurements as the player's stretcher tests
//! (`rbl-deck/tests/rubberband.rs`), taken on a rendered file instead of a
//! stretcher's output, and held to the same gate: a pitch within **3 cents**.
//! The frequency is read from interpolated zero crossings rather than a count
//! of them, because a count over one second is itself only good to 4 cents.
//!
//! Every render is read back through `rbl-audio`, not through this crate, so
//! a file that only this crate could read would fail here.
// A build without Rubber Band renders no edits, so the measuring helpers
// have nothing to measure there.
#![cfg_attr(not(feature = "rubberband"), allow(dead_code))]
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::path::{Path, PathBuf};

use wreckordbox_render::{
    render, Destination, Edits, MemorySource, Proceed, Progress, RenderError, TrackSource,
};

const RATE: u32 = 44_100;

/// The player's stretcher tests' gate.
const CENTS: f64 = 3.0;

/// Interleaved stereo: a sine on both channels.
fn tone(hz: f64, secs: f64, amplitude: f64) -> Vec<f32> {
    let frames = (secs * f64::from(RATE)) as usize;
    (0..frames)
        .flat_map(|i| {
            let value = (amplitude
                * (i as f64 / f64::from(RATE) * hz * std::f64::consts::TAU).sin())
                as f32;
            [value, value]
        })
        .collect()
}

fn silence(secs: f64) -> Vec<f32> {
    vec![0.0; (secs * f64::from(RATE)) as usize * 2]
}

/// A 16-bit PCM stereo WAV written by hand, so the source owes nothing to the
/// code under test.
fn write_wav(path: &Path, samples: &[f32]) {
    let data = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * 4).to_le_bytes());
    bytes.extend_from_slice(&4_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for &sample in samples {
        bytes.extend_from_slice(&((sample * 32_767.0).round() as i16).to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

/// A file's frames as 16-bit stereo and its rate, read through `rbl-audio`.
fn read(path: &Path) -> (Vec<[i16; 2]>, u32) {
    let mut stream = rbl_audio::MonoStream::open(path).unwrap();
    let mut frames = Vec::new();
    while let Some((_, stereo)) = stream.next_frames() {
        frames.extend_from_slice(stereo);
    }
    (frames, stream.sample_rate())
}

fn left(frames: &[[i16; 2]]) -> Vec<f64> {
    frames
        .iter()
        .map(|frame| f64::from(frame[0]) / 32_768.0)
        .collect()
}

/// Frequency over the settled middle half, from the first and last rising
/// zero crossings, each placed between samples by linear interpolation.
fn hz_of(signal: &[f64]) -> f64 {
    let middle = &signal[signal.len() / 4..signal.len() - signal.len() / 4];
    let crossings: Vec<f64> = middle
        .windows(2)
        .enumerate()
        .filter(|(_, w)| w[0] <= 0.0 && w[1] > 0.0)
        .map(|(i, w)| i as f64 + w[0] / (w[0] - w[1]))
        .collect();
    assert!(crossings.len() > 2, "no tone to measure");
    (crossings.len() - 1) as f64 * f64::from(RATE) / (crossings[crossings.len() - 1] - crossings[0])
}

fn cents(got: f64, want: f64) -> f64 {
    1200.0 * (got / want).log2()
}

/// Where a tone burst starts and ends, in seconds: the first and last 64-frame
/// window whose level is over half the burst's own.
fn burst(signal: &[f64], amplitude: f64) -> (f64, f64) {
    const WINDOW: usize = 64;
    let gate = amplitude / 2.0_f64.sqrt() / 2.0;
    let loud: Vec<usize> = signal
        .chunks(WINDOW)
        .enumerate()
        .filter(|(_, w)| (w.iter().map(|x| x * x).sum::<f64>() / w.len() as f64).sqrt() > gate)
        .map(|(i, _)| i * WINDOW)
        .collect();
    let secs = |frame: usize| frame as f64 / f64::from(RATE);
    (secs(loud[0]), secs(loud[loud.len() - 1] + WINDOW))
}

struct Bench {
    dir: tempfile::TempDir,
}

impl Bench {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn source(&self, samples: &[f32]) -> PathBuf {
        let path = self.dir.path().join("source.wav");
        write_wav(&path, samples);
        path
    }

    fn out(&self) -> PathBuf {
        self.dir.path().join("out.wav")
    }

    /// Renders a file with no interest in progress, and reads the result.
    fn render(&self, source: &Path, edits: Edits) -> (Vec<[i16; 2]>, u32) {
        let out = self.out();
        let _ = std::fs::remove_file(&out);
        let mut track = TrackSource::open(source).unwrap();
        let rendered = render(&mut track, edits, &Destination::wav(&out), &mut |_| {
            Proceed::Continue
        })
        .unwrap();
        let (frames, rate) = read(&out);
        assert_eq!(
            rendered.frames,
            frames.len() as u64,
            "the report and the file disagree on length"
        );
        assert_eq!(rendered.sample_rate, rate);
        (frames, rate)
    }

    /// Every file in the bench's folder, by name.
    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }
}

fn key(semitones: i8) -> Edits {
    Edits {
        key_shift: semitones,
        ..Edits::default()
    }
}

/// An edit that puts the stretcher in the path, where the build has one that
/// can render: progress, cancelling and what happens to files are the same
/// with it or without.
fn an_edit() -> Edits {
    if cfg!(feature = "rubberband") {
        tempo(1.05)
    } else {
        Edits::default()
    }
}

fn tempo(ratio: f64) -> Edits {
    Edits {
        tempo: ratio,
        ..Edits::default()
    }
}

#[test]
fn with_no_edits_the_render_matches_the_source() {
    // A tone with a quieter one on top, so no two frames in a row repeat, and
    // a hard start and stop, which is where a misaligned render shows.
    let bench = Bench::new();
    let mut samples = tone(440.0, 1.5, 0.6);
    for (sample, other) in samples.iter_mut().zip(tone(1234.5, 1.5, 0.3)) {
        *sample += other;
    }
    let source = bench.source(&samples);
    let (want, _) = read(&source);

    let (got, rate) = bench.render(&source, Edits::default());

    assert_eq!(rate, RATE);
    assert_eq!(got.len(), want.len(), "the length changed");
    assert!(got == want, "a frame changed");
}

#[cfg(feature = "rubberband")]
#[test]
fn a_key_shift_lands_on_the_interval_and_keeps_the_length() {
    let bench = Bench::new();
    let source = bench.source(&tone(440.0, 2.0, 0.5));
    for semitones in [-6_i8, -2, -1, 1, 2, 6] {
        let (frames, _) = bench.render(&source, key(semitones));

        let want = 440.0 * 2.0_f64.powf(f64::from(semitones) / 12.0);
        let hz = hz_of(&left(&frames));
        let off = cents(hz, want);
        assert!(
            off.abs() < CENTS,
            "{semitones} semitones: wanted {want:.2} Hz, got {hz:.2}, {off:.2} cents out"
        );
        assert_eq!(
            frames.len(),
            2 * RATE as usize,
            "{semitones} semitones changed the length"
        );
    }
}

#[cfg(feature = "rubberband")]
#[test]
fn a_tempo_change_gives_the_expected_length_at_the_same_pitch() {
    let bench = Bench::new();
    let source = bench.source(&tone(440.0, 2.0, 0.5));
    for ratio in [0.84, 0.94, 1.06, 1.16] {
        let (frames, _) = bench.render(&source, tempo(ratio));

        // Six percent fast is six percent more of the source per second, so
        // the file is that much shorter: exactly, to the frame.
        let want = (2.0 * f64::from(RATE) / ratio).round() as usize;
        assert_eq!(frames.len(), want, "at {ratio}x");

        let hz = hz_of(&left(&frames));
        let off = cents(hz, 440.0);
        assert!(
            off.abs() < CENTS,
            "at {ratio}x the tone came out at {hz:.2} Hz, {off:.2} cents out"
        );
    }
}

#[cfg(feature = "rubberband")]
#[test]
fn key_and_tempo_combine() {
    let bench = Bench::new();
    let source = bench.source(&tone(440.0, 2.0, 0.5));
    for (semitones, ratio) in [(3_i8, 1.08), (-4, 0.9)] {
        let (frames, _) = bench.render(
            &source,
            Edits {
                key_shift: semitones,
                tempo: ratio,
            },
        );

        assert_eq!(
            frames.len(),
            (2.0 * f64::from(RATE) / ratio).round() as usize,
            "{semitones} at {ratio}x"
        );
        let want = 440.0 * 2.0_f64.powf(f64::from(semitones) / 12.0);
        let hz = hz_of(&left(&frames));
        let off = cents(hz, want);
        assert!(
            off.abs() < CENTS,
            "{semitones} at {ratio}x: wanted {want:.2} Hz, got {hz:.2}, {off:.2} cents out"
        );
    }
}

/// How far a rendered event may sit from where it belongs. Measured at under
/// 2 ms on every case below, against a 1.5 ms measuring window; the latency
/// this guards against is 20 to 50 ms.
const ALIGNMENT_SECS: f64 = 0.005;

/// The level of the `secs` at one end of a signal, as a root mean square.
fn level(signal: &[f64], secs: f64, at_end: bool) -> f64 {
    let frames = (secs * f64::from(RATE)) as usize;
    let part = if at_end {
        &signal[signal.len() - frames..]
    } else {
        &signal[..frames]
    };
    (part.iter().map(|x| x * x).sum::<f64>() / part.len() as f64).sqrt()
}

#[cfg(feature = "rubberband")]
#[test]
fn audio_that_runs_to_the_ends_of_the_source_runs_to_the_ends_of_the_render() {
    // A tone from the first frame to the last. Whatever is still inside the
    // stretcher when the source runs out has to come out, and the stretcher's
    // warm-up must not eat the first moments: the first and last 20 ms of the
    // render are tone, not silence. Not at the tone's full level: the
    // stretcher's window sees silence outside the file, so the outermost
    // 20 ms come up short of it: measured at 0.68 to 0.70 of the level for a
    // tempo change and 0.42 with a key shift three semitones down, against
    // nothing at all when the tail is not flushed. Gated at a quarter.
    let bench = Bench::new();
    let source = bench.source(&tone(440.0, 2.0, 0.5));
    let nominal = 0.5 / 2.0_f64.sqrt();

    let cases = [
        tempo(1.1),
        tempo(0.9),
        key(2),
        Edits {
            key_shift: -3,
            tempo: 1.2,
        },
    ];
    for edits in cases {
        let (frames, _) = bench.render(&source, edits);
        let signal = left(&frames);
        let (start, end) = (level(&signal, 0.02, false), level(&signal, 0.02, true));
        assert!(
            start > nominal * 0.25 && end > nominal * 0.25,
            "{edits:?}: the first 20 ms are at {start:.3} and the last at {end:.3}, \
             against {nominal:.3} for the tone",
        );
    }
}

#[cfg(feature = "rubberband")]
#[test]
fn an_edited_render_starts_and_ends_where_it_should() {
    // A second of silence, a second of tone, a second of silence: the
    // stretcher's latency, uncorrected, would move the burst late by tens of
    // milliseconds, and its tail would be cut off the end of the file.
    let bench = Bench::new();
    let mut samples = silence(1.0);
    samples.extend(tone(1000.0, 1.0, 0.5));
    samples.extend(silence(1.0));
    let source = bench.source(&samples);

    let cases = [
        tempo(1.25),
        tempo(0.8),
        key(2),
        key(-5),
        Edits {
            key_shift: 3,
            tempo: 1.1,
        },
    ];
    for edits in cases {
        let (frames, _) = bench.render(&source, edits);
        let (from, to) = burst(&left(&frames), 0.5);
        let (want_from, want_to) = (1.0 / edits.tempo, 2.0 / edits.tempo);
        assert!(
            (from - want_from).abs() < ALIGNMENT_SECS && (to - want_to).abs() < ALIGNMENT_SECS,
            "{edits:?}: the burst ran {from:.4} to {to:.4} s, wanted {want_from:.4} to {want_to:.4}",
        );
    }
}

#[test]
fn progress_is_reported_while_rendering() {
    let bench = Bench::new();
    let source = bench.source(&tone(440.0, 4.0, 0.5));
    let mut track = TrackSource::open(&source).unwrap();
    let mut seen: Vec<Progress> = Vec::new();
    let mut existed_midway = false;
    let out = bench.out();

    render(
        &mut track,
        an_edit(),
        &Destination::wav(&out),
        &mut |progress| {
            existed_midway |= out.exists();
            seen.push(progress);
            Proceed::Continue
        },
    )
    .unwrap();

    let total = 4 * u64::from(RATE);
    assert!(
        seen.len() >= 4,
        "only {} reports for four seconds",
        seen.len()
    );
    assert!(
        seen.windows(2).all(|w| w[0].frames_done < w[1].frames_done),
        "progress went backwards"
    );
    assert!(seen.iter().all(|p| p.frames_total == Some(total)));
    assert_eq!(
        seen.last().unwrap().frames_done,
        total,
        "the last report is the whole source"
    );
    assert!(
        !existed_midway,
        "the destination appeared before the render was complete"
    );
}

#[test]
fn a_cancelled_render_leaves_no_file() {
    let bench = Bench::new();
    let source = bench.source(&tone(440.0, 4.0, 0.5));
    let mut track = TrackSource::open(&source).unwrap();

    let result = render(
        &mut track,
        an_edit(),
        &Destination::wav(bench.out()),
        &mut |progress| {
            if progress.frames_done >= u64::from(RATE) {
                Proceed::Cancel
            } else {
                Proceed::Continue
            }
        },
    );

    assert!(matches!(result, Err(RenderError::Cancelled)), "{result:?}");
    assert_eq!(bench.files(), ["source.wav"], "something was left behind");
}

#[test]
fn the_source_file_is_never_modified() {
    let bench = Bench::new();
    let source = bench.source(&tone(440.0, 1.0, 0.5));
    let before = (
        std::fs::read(&source).unwrap(),
        std::fs::metadata(&source).unwrap().modified().unwrap(),
    );
    let untouched = || {
        let now = (
            std::fs::read(&source).unwrap(),
            std::fs::metadata(&source).unwrap().modified().unwrap(),
        );
        assert!(now == before, "the source changed");
    };

    // A complete render, an edited one, and a cancelled one.
    bench.render(&source, Edits::default());
    untouched();
    bench.render(&source, an_edit());
    untouched();
    std::fs::remove_file(bench.out()).unwrap();
    let mut track = TrackSource::open(&source).unwrap();
    let _ = render(
        &mut track,
        an_edit(),
        &Destination::wav(bench.out()),
        &mut |_| Proceed::Cancel,
    );
    untouched();

    // And the one that would do the damage: the source named as the
    // destination. Anything already at a destination is refused.
    let mut track = TrackSource::open(&source).unwrap();
    let result = render(
        &mut track,
        an_edit(),
        &Destination::wav(&source),
        &mut |_| Proceed::Continue,
    );
    assert!(
        matches!(result, Err(RenderError::DestinationExists(_))),
        "{result:?}"
    );
    untouched();
    assert_eq!(bench.files(), ["source.wav"]);
}

#[test]
fn a_file_that_appears_at_the_destination_during_a_render_is_not_replaced() {
    // Two renders to one place, or anything else writing there meanwhile:
    // whoever got there first keeps their file.
    let bench = Bench::new();
    let source = bench.source(&tone(440.0, 2.0, 0.5));
    let mut track = TrackSource::open(&source).unwrap();
    let out = bench.out();

    let result = render(&mut track, an_edit(), &Destination::wav(&out), &mut |_| {
        std::fs::write(&out, b"someone else's").unwrap();
        Proceed::Continue
    });

    assert!(
        matches!(result, Err(RenderError::DestinationExists(_))),
        "{result:?}"
    );
    assert_eq!(std::fs::read(&out).unwrap(), b"someone else's");
    assert_eq!(bench.files(), ["out.wav", "source.wav"]);
}

#[test]
fn edits_outside_what_the_stretcher_holds_are_refused() {
    let bench = Bench::new();
    let source = bench.source(&tone(440.0, 0.5, 0.5));
    for edits in [tempo(0.4), tempo(2.5), tempo(f64::NAN), key(13), key(-13)] {
        let mut track = TrackSource::open(&source).unwrap();
        let result = render(
            &mut track,
            edits,
            &Destination::wav(bench.out()),
            &mut |_| Proceed::Continue,
        );
        let want = if edits.key_shift == 0 {
            matches!(result, Err(RenderError::TempoOutOfRange))
        } else {
            matches!(result, Err(RenderError::KeyShiftOutOfRange))
        };
        assert!(want, "{edits:?} gave {result:?}");
    }
    assert_eq!(bench.files(), ["source.wav"]);
}

#[cfg(not(feature = "rubberband"))]
#[test]
fn a_build_without_rubber_band_refuses_a_tempo_change() {
    // Its stretcher cannot hold a pitch: a 440 Hz tone at 0.94x came back
    // 107 cents flat. A render that is in the wrong key is refused, not
    // written.
    let bench = Bench::new();
    let source = bench.source(&tone(440.0, 0.5, 0.5));
    for ratio in [0.94, 1.06, 0.75, 1.5] {
        let mut track = TrackSource::open(&source).unwrap();
        let result = render(
            &mut track,
            tempo(ratio),
            &Destination::wav(bench.out()),
            &mut |_| Proceed::Continue,
        );
        assert!(
            matches!(result, Err(RenderError::TempoUnavailable)),
            "at {ratio}x: {result:?}"
        );
    }
    assert_eq!(bench.files(), ["source.wav"]);
}

#[cfg(not(feature = "rubberband"))]
#[test]
fn a_build_without_rubber_band_refuses_a_key_shift() {
    let bench = Bench::new();
    let source = bench.source(&tone(440.0, 0.5, 0.5));
    let mut track = TrackSource::open(&source).unwrap();
    let result = render(
        &mut track,
        key(1),
        &Destination::wav(bench.out()),
        &mut |_| Proceed::Continue,
    );
    assert!(
        matches!(result, Err(RenderError::KeyShiftUnavailable)),
        "{result:?}"
    );
    assert_eq!(bench.files(), ["source.wav"]);
}

#[test]
fn audio_that_is_not_a_file_renders_the_same_way() {
    // The seam a sum of Stems comes in through: samples with no file behind
    // them.
    let bench = Bench::new();
    let samples = tone(440.0, 1.0, 0.5);
    let mut source = MemorySource::new(samples.clone(), RATE);

    let rendered = render(
        &mut source,
        Edits::default(),
        &Destination::wav(bench.out()),
        &mut |_| Proceed::Continue,
    )
    .unwrap();

    assert_eq!(rendered.frames, u64::from(RATE));
    let (frames, rate) = read(&bench.out());
    assert_eq!(rate, RATE);
    let want: Vec<[i16; 2]> = samples
        .chunks_exact(2)
        .map(|f| {
            [
                (f[0] * 32_768.0).round() as i16,
                (f[1] * 32_768.0).round() as i16,
            ]
        })
        .collect();
    assert!(frames == want, "the samples changed");
}

#[test]
fn a_source_with_no_audio_writes_nothing() {
    let bench = Bench::new();
    let mut source = MemorySource::new(Vec::new(), RATE);
    let result = render(
        &mut source,
        Edits::default(),
        &Destination::wav(bench.out()),
        &mut |_| Proceed::Continue,
    );
    assert!(matches!(result, Err(RenderError::Empty)), "{result:?}");
    assert!(bench.files().is_empty());
}
