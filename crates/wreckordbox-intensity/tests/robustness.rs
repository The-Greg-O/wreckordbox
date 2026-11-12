//! What the measurement must not get wrong at its edges: the end of a
//! file, a short file, another sample rate, a channel layout, numbers that
//! are not numbers, and a beat grid that changes after the measuring.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod common;

use std::f64::consts::PI;

use common::{joined, loud_busy, quiet_sparse, write_wav, RATE};
use wreckordbox_intensity::{measure_file, Analyser, Intensity, MeasureError};

fn measure(samples: &[f32]) -> Intensity {
    let mut analyser = Analyser::new(RATE);
    analyser.push_mono(samples);
    analyser.finish(&[]).unwrap()
}

fn never() -> bool {
    false
}

#[test]
fn a_sound_in_the_last_part_of_a_second_is_measured() {
    // 1.4 s, silent until 1.05 s, then loud.
    let mut audio = vec![0.0_f32; (1.05 * f64::from(RATE)) as usize];
    let tail = loud_busy(0.35);
    audio.extend_from_slice(&tail);
    let intensity = measure(&audio);
    assert_eq!(intensity.curve.intensity.len(), 2, "{:?}", intensity.curve);
    assert!(intensity.curve.intensity[1] > 0.2, "{:?}", intensity.curve);
    assert!(intensity.summary.intensity > 0.1, "{:?}", intensity.summary);
    assert!(intensity.curve.features.loudness[1] > -30.0);
}

#[test]
fn a_very_short_loud_clip_is_not_silence() {
    // Ten milliseconds of loud noise: shorter than a loudness block and
    // than a spectral frame.
    let mut noise: u32 = 7;
    let clip: Vec<f32> = (0..RATE / 100)
        .map(|_| {
            noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((noise >> 8) as f32 / 8_388_608.0 - 1.0) * 0.8
        })
        .collect();
    let intensity = measure(&clip);
    assert_eq!(intensity.curve.intensity.len(), 1);
    assert!(intensity.summary.intensity > 0.1, "{:?}", intensity.summary);
    assert!(intensity.summary.features.loudness > -20.0);
    assert!(intensity.summary.features.high_end > -40.0);
    assert!(intensity.summary.loudness_integrated_lufs > -20.0);
}

/// Band-limited pattern at `rate`: a decaying 60 Hz kick on each beat at
/// 120 BPM, a 1 kHz blip on every eighth and a 7 kHz shimmer on the
/// off-beats, under a faint 220 Hz note.
fn pattern_at(rate: u32, seconds: f64) -> Vec<f32> {
    let n = (seconds * f64::from(rate)) as usize;
    let mut out = vec![0.0_f32; n];
    let t = |i: usize| i as f64 / f64::from(rate);
    for (i, sample) in out.iter_mut().enumerate() {
        *sample += (0.02 * (2.0 * PI * 220.0 * t(i)).sin()) as f32;
    }
    let mut hit = |at: f64, length: f64, hz: f64, amplitude: f64| {
        let start = (at * f64::from(rate)) as usize;
        let len = (length * f64::from(rate)) as usize;
        for i in 0..len {
            let Some(sample) = out.get_mut(start + i) else {
                break;
            };
            let decay = 1.0 - i as f64 / len as f64;
            *sample += (amplitude * decay * decay * (2.0 * PI * hz * t(i)).sin()) as f32;
        }
    };
    let mut eighth = 0;
    loop {
        let at = f64::from(eighth) * 0.25;
        if at >= seconds {
            break;
        }
        if eighth % 2 == 0 {
            hit(at, 0.15, 60.0, 0.6);
        } else {
            hit(at, 0.05, 7_000.0, 0.15);
        }
        hit(at, 0.03, 1_000.0, 0.3);
        eighth += 1;
    }
    out
}

#[test]
fn the_same_band_limited_audio_measures_the_same_at_any_sample_rate() {
    let at = |rate: u32| {
        let mut analyser = Analyser::new(rate);
        analyser.push_mono(&pattern_at(rate, 8.0));
        analyser.finish(&[]).unwrap()
    };
    let base = at(44_100);
    assert!(
        base.summary.features.onset_density > 3.0,
        "{:?}",
        base.summary
    );
    for rate in [48_000, 88_200, 96_000] {
        let other = at(rate);
        let (a, b) = (base.summary.features, other.summary.features);
        assert!((a.loudness - b.loudness).abs() < 0.3, "{rate}: {a:?} {b:?}");
        assert!((a.low_end - b.low_end).abs() < 0.5, "{rate}: {a:?} {b:?}");
        assert!((a.high_end - b.high_end).abs() < 0.5, "{rate}: {a:?} {b:?}");
        assert!(
            (a.onset_strength - b.onset_strength).abs() < 0.1 * a.onset_strength,
            "{rate}: {a:?} {b:?}"
        );
        assert!(
            (a.onset_density - b.onset_density).abs() < 0.3,
            "{rate}: {a:?} {b:?}"
        );
        assert!(
            (base.summary.intensity - other.summary.intensity).abs() < 0.02,
            "{rate}: {:?} {:?}",
            base.summary,
            other.summary
        );
        assert_eq!(base.curve.intensity.len(), other.curve.intensity.len());
    }
}

/// `frames` mono samples as interleaved frames of `channels`, the audio in
/// channel `into` only, or in every channel when `None`.
fn spread(mono: &[f32], channels: u16, into: Option<u16>) -> Vec<f32> {
    let mut out = Vec::with_capacity(mono.len() * usize::from(channels));
    for sample in mono {
        for channel in 0..channels {
            out.push(if into.is_none_or(|only| only == channel) {
                *sample
            } else {
                0.0
            });
        }
    }
    out
}

#[test]
fn a_file_of_more_than_two_channels_is_refused_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("surround.wav");
    // Audio only in the centre channel of six.
    write_wav(&path, RATE, 6, &spread(&loud_busy(3.0), 6, Some(2)), false);
    let error = measure_file(&path, &[], &never).unwrap_err();
    assert!(matches!(error, MeasureError::Channels(6)), "{error:?}");
    assert!(error.to_string().contains("6 channels"), "{error}");
}

#[test]
fn mono_and_stereo_files_measure_as_they_are_heard() {
    let dir = tempfile::tempdir().unwrap();
    let audio = joined(&[quiet_sparse(2.0), loud_busy(4.0)]);
    let file = |name: &str, channels: u16, into: Option<u16>| {
        let path = dir.path().join(name);
        write_wav(
            &path,
            RATE,
            channels,
            &spread(&audio, channels, into),
            false,
        );
        measure_file(&path, &[], &never).unwrap().unwrap()
    };
    let mono = file("mono.wav", 1, None);
    let both = file("both.wav", 2, None);
    let left = file("left.wav", 2, Some(0));
    // A mono file is heard from both speakers: as loud as the same audio in
    // both channels of a stereo file.
    assert_eq!(mono.summary, both.summary);
    // The same audio in one speaker only is 3 dB quieter.
    let fall = both.summary.loudness_integrated_lufs - left.summary.loudness_integrated_lufs;
    assert!((fall - 3.01).abs() < 0.1, "{fall}");
    // The memory of the mix (its spectrum) is the mono downmix.
    assert!(left.summary.features.onset_density > 1.0);
}

#[test]
fn samples_that_are_not_numbers_are_refused() {
    let mut audio = loud_busy(2.0);
    audio[RATE as usize] = f32::NAN;
    let mut analyser = Analyser::new(RATE);
    analyser.push_mono(&audio);
    assert!(analyser.finish(&[]).is_err());

    let mut infinite = loud_busy(2.0);
    infinite[100] = f32::INFINITY;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("float.wav");
    write_wav(&path, RATE, 1, &infinite, true);
    let error = measure_file(&path, &[], &never).unwrap_err();
    assert!(matches!(error, MeasureError::Invalid(_)), "{error:?}");

    // A huge but finite outlier does not overflow into infinity.
    let mut outlier = loud_busy(2.0);
    outlier[100] = 1.0e20;
    let mut analyser = Analyser::new(RATE);
    analyser.push_mono(&outlier);
    let intensity = analyser.finish(&[]).unwrap();
    assert!(intensity.check().is_ok());
    let text = serde_json::to_string(&intensity).unwrap();
    // Only `grid` and `bars` may be null: no number became one.
    assert_eq!(text.matches("null").count(), 2, "{text}");
}

#[test]
fn a_value_with_a_number_that_is_not_one_fails_its_check() {
    let mut intensity = measure(&loud_busy(2.0));
    assert!(intensity.check().is_ok());
    intensity.curve.features.low_end[1] = f64::NAN;
    assert!(intensity.check().is_err());
    let mut intensity = measure(&loud_busy(2.0));
    intensity.summary.intensity = 1.5;
    assert!(intensity.check().is_err());
}

#[test]
fn bars_rebuilt_from_the_stored_value_are_the_bars_measured_with_that_grid() {
    let audio = joined(&[quiet_sparse(7.5), loud_busy(7.5)]);
    let first: Vec<f64> = (0..10).map(|bar| f64::from(bar) * 1.875).collect();
    let moved: Vec<f64> = (0..10).map(|bar| 0.4 + f64::from(bar) * 1.875).collect();
    let measured_with = |starts: &[f64]| {
        let mut analyser = Analyser::new(RATE);
        analyser.push_mono(&audio);
        analyser.finish(starts).unwrap()
    };
    let without = measured_with(&[]);
    assert_eq!(without.grid, None);
    let with_first = measured_with(&first);
    let with_moved = measured_with(&moved);
    assert!(with_first.grid.is_some());
    assert_ne!(with_first.grid, with_moved.grid);
    assert_ne!(with_first.bars, with_moved.bars);

    // From the stored value alone, through JSON as the Sidecar keeps it.
    let stored: Intensity =
        serde_json::from_str(&serde_json::to_string(&without).unwrap()).unwrap();
    assert_eq!(stored.with_bars(&first).unwrap(), with_first);
    assert_eq!(stored.with_bars(&moved).unwrap(), with_moved);
    assert_eq!(with_first.with_bars(&moved).unwrap(), with_moved);
    assert_eq!(with_first.with_bars(&[]).unwrap(), without);
    // The grid's identity is a function of the bar lines alone.
    assert_eq!(wreckordbox_intensity::grid_of(&first), with_first.grid);
    assert_eq!(wreckordbox_intensity::grid_of(&[]), None);
}
