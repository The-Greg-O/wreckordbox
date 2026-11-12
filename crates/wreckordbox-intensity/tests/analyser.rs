//! Intensity on generated signals whose answer is known by construction.
#![allow(clippy::pedantic, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{joined, loud_busy, loud_busy_at, quiet_sparse, RATE};
use wreckordbox_intensity::{Analyser, Intensity};

fn measure(samples: &[f32]) -> Intensity {
    let mut analyser = Analyser::new(RATE);
    analyser.push_mono(samples);
    analyser.finish(&[])
}

/// The time of the curve's `point`th value, in seconds.
fn time_of(intensity: &Intensity, point: usize) -> f64 {
    intensity.curve.offset_seconds + point as f64 * intensity.curve.step_seconds
}

#[test]
fn the_curve_is_low_through_the_quiet_sparse_stretch_and_high_through_the_loud_busy_one() {
    let intensity = measure(&joined(&[quiet_sparse(12.0), loud_busy(12.0)]));

    let curve = &intensity.curve.intensity;
    assert_eq!(curve.len(), 24, "one point a second");
    let (quiet, loud): (Vec<(usize, f64)>, Vec<(usize, f64)>) =
        curve.iter().copied().enumerate().partition(|(point, _)| time_of(&intensity, *point) < 12.0);
    let highest_quiet = quiet.iter().map(|(_, v)| *v).fold(0.0, f64::max);
    let lowest_loud = loud.iter().map(|(_, v)| *v).fold(1.0, f64::min);
    assert!(highest_quiet < lowest_loud, "every loud point is above every quiet one: {curve:?}");
    // Well apart once clear of the change, not merely ordered.
    let settled_quiet = curve[..10].iter().copied().fold(0.0, f64::max);
    let settled_loud = curve[14..].iter().copied().fold(1.0, f64::min);
    assert!(settled_loud - settled_quiet > 0.3, "{curve:?}");
    assert!(curve.iter().all(|v| (0.0..=1.0).contains(v)), "{curve:?}");
}

#[test]
fn a_louder_busier_track_has_a_higher_summary_than_a_quieter_sparser_one() {
    let busy = measure(&loud_busy(10.0));
    let sparse = measure(&quiet_sparse(10.0));
    assert!(busy.summary.intensity > sparse.summary.intensity + 0.3, "{:?} {:?}", busy.summary, sparse.summary);
    // Each feature says so by itself, in its own unit.
    let (b, s) = (busy.summary.features, sparse.summary.features);
    assert!(b.loudness > s.loudness + 10.0);
    assert!(b.low_end > s.low_end + 10.0);
    assert!(b.high_end > s.high_end + 10.0);
    assert!(b.onset_strength > s.onset_strength * 5.0);
    assert!(b.onset_density > s.onset_density * 5.0);
    assert!(busy.summary.loudness_integrated_lufs > sparse.summary.loudness_integrated_lufs + 10.0);
}

#[test]
fn the_same_pattern_turned_down_measures_lower_and_only_its_levels_change() {
    let full = measure(&loud_busy_at(10.0, 128.0, 1.0));
    let down = measure(&loud_busy_at(10.0, 128.0, 0.25));
    assert!(full.summary.intensity > down.summary.intensity);
    // A quarter of the amplitude is 12 dB.
    let fall = full.summary.features.loudness - down.summary.features.loudness;
    assert!((fall - 12.04).abs() < 0.1, "{fall}");
    // The same things happen as often.
    assert_eq!(full.summary.features.onset_density, down.summary.features.onset_density);
}

#[test]
fn onset_density_counts_the_hits() {
    // Sixteenths at 120 BPM are eight a second.
    let intensity = measure(&loud_busy_at(10.0, 120.0, 1.0));
    let density = intensity.summary.features.onset_density;
    assert!((density - 8.0).abs() < 0.5, "{density}");
    // A held note has none.
    let held: Vec<f32> = (0..RATE * 5).map(|i| (f64::from(i) * 2.0 * std::f64::consts::PI * 220.0 / f64::from(RATE)).sin() as f32 * 0.5).collect();
    let intensity = measure(&held);
    assert_eq!(intensity.summary.features.onset_density, 0.0);
    assert!(intensity.summary.features.onset_strength < 0.05, "{:?}", intensity.summary);
}

#[test]
fn low_end_and_high_end_are_the_levels_of_what_is_in_those_bands() {
    let tone = |hz: f64| -> Vec<f32> {
        (0..RATE * 4).map(|i| (f64::from(i) * 2.0 * std::f64::consts::PI * hz / f64::from(RATE)).sin() as f32 * 0.5).collect()
    };
    // A sine of amplitude 0.5 has a mean square of 0.125: -9.03 dB.
    let bass = measure(&tone(60.0)).summary.features;
    assert!((bass.low_end + 9.03).abs() < 0.3, "{bass:?}");
    assert!(bass.high_end < -80.0, "{bass:?}");
    let treble = measure(&tone(8_000.0)).summary.features;
    assert!((treble.high_end + 9.03).abs() < 0.3, "{treble:?}");
    assert!(treble.low_end < -80.0, "{treble:?}");
    let middle = measure(&tone(1_000.0)).summary.features;
    assert!(middle.low_end < -80.0 && middle.high_end < -80.0, "{middle:?}");
}

#[test]
fn the_same_audio_measures_the_same_however_it_is_cut_up() {
    let audio = joined(&[quiet_sparse(4.0), loud_busy(5.0)]);
    let whole = measure(&audio);
    let mut analyser = Analyser::new(RATE);
    for piece in audio.chunks(1_152) {
        analyser.push_mono(piece);
    }
    let pieces = analyser.finish(&[]);
    assert_eq!(serde_json::to_string(&whole).unwrap(), serde_json::to_string(&pieces).unwrap());
}

#[test]
fn a_beat_grid_gives_one_point_per_bar_and_no_grid_gives_only_the_time_curve() {
    let audio = joined(&[quiet_sparse(7.5), loud_busy(7.5)]);
    let without = measure(&audio);
    assert!(without.bars.is_none());
    assert_eq!(without.curve.intensity.len(), 15);

    // 128 BPM: a bar is 1.875 s. The grid runs on past the audio's end.
    let starts: Vec<f64> = (0..10).map(|bar| f64::from(bar) * 1.875).collect();
    let mut analyser = Analyser::new(RATE);
    analyser.push_mono(&audio);
    let with = analyser.finish(&starts);
    let bars = with.bars.as_ref().unwrap();
    assert_eq!(bars.starts_seconds, starts[..8].to_vec(), "the bars that begin inside the audio");
    assert_eq!(bars.end_seconds, 15.0);
    assert_eq!(bars.intensity.len(), 8);
    assert_eq!(bars.features.loudness.len(), 8);
    // The change is on a bar line: four quiet bars, four loud ones.
    let highest_quiet = bars.intensity[..4].iter().copied().fold(0.0, f64::max);
    let lowest_loud = bars.intensity[4..].iter().copied().fold(1.0, f64::min);
    assert!(lowest_loud - highest_quiet > 0.3, "{:?}", bars.intensity);
    // The time curve is what it was.
    assert_eq!(with.curve, without.curve);
    assert_eq!(with.summary, without.summary);
}

#[test]
fn the_stored_features_and_the_recorded_weights_give_back_the_stored_curve() {
    let intensity = measure(&joined(&[quiet_sparse(3.0), loud_busy(3.0)]));
    let (method, curve) = (&intensity.method, &intensity.curve);
    assert_eq!(method.version, wreckordbox_intensity::METHOD_VERSION);
    assert_eq!(method.features, ["loudness", "low_end", "high_end", "onset_strength", "onset_density"]);
    for point in 0..curve.intensity.len() {
        let features = wreckordbox_intensity::Features {
            loudness: curve.features.loudness[point],
            low_end: curve.features.low_end[point],
            high_end: curve.features.high_end[point],
            onset_strength: curve.features.onset_strength[point],
            onset_density: curve.features.onset_density[point],
        };
        let again = wreckordbox_intensity::combine(&features, &method.weights, &method.scales);
        assert!((again - curve.intensity[point]).abs() < 1e-4, "point {point}");
    }
}

#[test]
fn silence_measures_zero() {
    let intensity = measure(&vec![0.0; RATE as usize * 3]);
    assert_eq!(intensity.summary.intensity, 0.0);
    assert_eq!(intensity.curve.intensity, vec![0.0; 3]);
}
