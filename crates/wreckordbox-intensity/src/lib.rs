//! Intensity: a Measurement of how hard a track hits and how that changes
//! over its length.
//!
//! It is computed from decoded audio alone, in one streaming pass, from
//! five well-understood features:
//!
//! | feature | what it is | unit |
//! |---|---|---|
//! | `loudness` | K-weighted loudness after ITU-R BS.1770 / EBU R128 ([`loudness`]) | LUFS |
//! | `low_end` | level of the audio below [`LOW_END_BELOW_HZ`] | dB re full scale |
//! | `high_end` | level of the audio above [`HIGH_END_ABOVE_HZ`] | dB re full scale |
//! | `onset_strength` | spectral flux: how much the spectrum rises, per second | flux per second |
//! | `onset_density` | onsets picked from the flux, per second | onsets per second |
//!
//! Each feature is mapped onto 0 to 1 by a fixed straight line
//! ([`SCALES`]) and the five are added with fixed weights ([`WEIGHTS`]).
//! Nothing is fitted to data and nothing is learned: fitting Intensity to
//! the DJ's Energy is calibration, a separate step, which is why the value
//! keeps every feature's own curve beside the combined one. Calibration can
//! then weight them differently without the audio being read again.
//!
//! The value has a summary, a curve on a fixed time grid
//! ([`STEP_SECONDS`]) and, when the track has a beat grid, a curve with one
//! point per bar. [`METHOD_VERSION`] changes whenever a number here would
//! come out differently for the same audio.
//!
//! The same audio gives the same value: there is no randomness, no clock
//! and no threading inside a measurement, and stored numbers are rounded.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "sample counts and times convert between usize and f64 throughout"
)]

mod analyser;
pub mod loudness;

use std::path::Path;

use serde::{Deserialize, Serialize};

pub use analyser::Analyser;

/// The name recorded as what produced a value, and the name of the tool
/// that is its source.
pub const METHOD_NAME: &str = "wreckordbox-intensity";
/// The version of the method. Bump it when the same audio would measure
/// differently: a stored value of another version is measured again.
pub const METHOD_VERSION: u32 = 1;

/// Seconds between points of the time curve.
pub const STEP_SECONDS: f64 = 1.0;
/// The low end is what lies below this: the kick's body and the sub.
pub const LOW_END_BELOW_HZ: f64 = 150.0;
/// Nothing below this counts as low end: it is rumble and offset.
pub const LOW_END_ABOVE_HZ: f64 = 20.0;
/// The high end is what lies above this: hats, cymbals, air.
pub const HIGH_END_ABOVE_HZ: f64 = 5_000.0;
/// What a level with no energy at all is reported as, in dB.
pub const SILENCE_DB: f64 = -100.0;

/// One number per feature, in the order the features are always listed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Features<T> {
    pub loudness: T,
    pub low_end: T,
    pub high_end: T,
    pub onset_strength: T,
    pub onset_density: T,
}

/// The features' names, in order.
pub const FEATURES: [&str; 5] = ["loudness", "low_end", "high_end", "onset_strength", "onset_density"];

/// The straight line that maps a feature onto 0 to 1: `floor` and below is
/// 0, `ceiling` and above is 1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Scale {
    pub floor: f64,
    pub ceiling: f64,
}

impl Scale {
    /// Where `value` falls between the floor and the ceiling, 0 to 1.
    #[must_use]
    pub fn normalise(&self, value: f64) -> f64 {
        ((value - self.floor) / (self.ceiling - self.floor)).clamp(0.0, 1.0)
    }
}

/// How much each feature counts in the combined Intensity. They add up
/// to 1, so Intensity is 0 to 1.
///
/// Chosen by reasoning, not fitted: loudness carries most, because a
/// stretch that is louder hits harder whatever else is true; how many
/// things happen per second comes next; the weight of the low end, the
/// size of each hit and the brightness share the rest.
pub const WEIGHTS: Features<f64> =
    Features { loudness: 0.40, low_end: 0.15, high_end: 0.10, onset_strength: 0.15, onset_density: 0.20 };

/// Where each feature's 0 and 1 lie.
///
/// - `loudness`: -40 LUFS is a faint pad under a breakdown; -5 LUFS is as
///   loud as a club master gets over a second.
/// - `low_end`, `high_end`: the band's level in dB re full scale, from
///   barely there to dominating the mix.
/// - `onset_strength`: flux per second, from a held note (0) to a dense
///   drum pattern.
/// - `onset_density`: onsets per second, from none to sixteenths at
///   120 BPM (8 a second).
pub const SCALES: Features<Scale> = Features {
    loudness: Scale { floor: -40.0, ceiling: -5.0 },
    low_end: Scale { floor: -50.0, ceiling: -10.0 },
    high_end: Scale { floor: -60.0, ceiling: -20.0 },
    onset_strength: Scale { floor: 0.0, ceiling: 10.0 },
    onset_density: Scale { floor: 0.0, ceiling: 8.0 },
};

/// The one place the features become Intensity: each mapped onto 0 to 1 by
/// its scale, multiplied by its weight, and added.
#[must_use]
pub fn combine(features: &Features<f64>, weights: &Features<f64>, scales: &Features<Scale>) -> f64 {
    weights.loudness * scales.loudness.normalise(features.loudness)
        + weights.low_end * scales.low_end.normalise(features.low_end)
        + weights.high_end * scales.high_end.normalise(features.high_end)
        + weights.onset_strength * scales.onset_strength.normalise(features.onset_strength)
        + weights.onset_density * scales.onset_density.normalise(features.onset_density)
}

/// What produced a value: enough to tell whether it is current, and to
/// redo the combination from the stored features.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Method {
    pub name: String,
    pub version: u32,
    /// The features measured, in order.
    pub features: Vec<String>,
    pub weights: Features<f64>,
    pub scales: Features<Scale>,
}

impl Method {
    /// The method this build measures with.
    #[must_use]
    pub fn current() -> Self {
        Self {
            name: METHOD_NAME.to_owned(),
            version: METHOD_VERSION,
            features: FEATURES.iter().map(|name| (*name).to_owned()).collect(),
            weights: WEIGHTS,
            scales: SCALES,
        }
    }
}

/// The audio a value was measured from, as it was decoded.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MeasuredAudio {
    pub sample_rate: u32,
    pub seconds: f64,
}

/// The whole track in a few numbers.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    /// The summary value: the mean of the time curve, 0 to 1.
    pub intensity: f64,
    /// The mean of the highest quarter of the time curve: how hard the
    /// track hits where it hits hardest.
    pub high: f64,
    /// The mean of the lowest quarter of the time curve.
    pub low: f64,
    /// Integrated loudness of the whole track, gated as BS.1770 says.
    pub loudness_integrated_lufs: f64,
    /// The loudest three seconds (R128's maximum short-term loudness).
    pub loudness_short_term_max_lufs: f64,
    /// Each feature over the whole track, in its own unit; `loudness` is
    /// the ungated loudness of all of it.
    pub features: Features<f64>,
}

/// Intensity on a fixed time grid: point `i` is the audio from
/// `offset_seconds + i * step_seconds` for `step_seconds`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeCurve {
    pub offset_seconds: f64,
    pub step_seconds: f64,
    /// The combined Intensity of each point, 0 to 1.
    pub intensity: Vec<f64>,
    /// Each feature of each point, in its own unit, before any scale or
    /// weight. `loudness` is the loudness of that point's own audio.
    pub features: Features<Vec<f64>>,
}

/// Intensity per bar of the beat grid: bar `i` runs from
/// `starts_seconds[i]` to `starts_seconds[i + 1]`, the last to
/// `end_seconds`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BarCurve {
    pub starts_seconds: Vec<f64>,
    pub end_seconds: f64,
    pub intensity: Vec<f64>,
    pub features: Features<Vec<f64>>,
}

/// A track's Intensity: the value of the Measurement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Intensity {
    pub method: Method,
    pub audio: MeasuredAudio,
    pub summary: Summary,
    pub curve: TimeCurve,
    /// One point per bar, when the track had a beat grid; `null` when not.
    pub bars: Option<BarCurve>,
}

/// Measures an audio file, decoding it once, a packet at a time, through
/// the decoder the tempo, key and waveform analysis reads with; so a time
/// here is a time on the beat grid's timeline, and no more than a packet
/// of audio is held.
///
/// `bar_starts` is where each bar of the track's beat grid begins, in
/// seconds; empty for a track with no grid. `stop` is asked between
/// packets; when it answers true the measurement is abandoned and `None`
/// is answered.
///
/// # Errors
///
/// When the file cannot be opened or decodes to nothing.
pub fn measure_file(
    path: &Path,
    bar_starts: &[f64],
    stop: &dyn Fn() -> bool,
) -> Result<Option<Intensity>, rbl_audio::AudioError> {
    let mut stream = rbl_audio::MonoStream::open(path)?;
    let mut analyser: Option<Analyser> = None;
    // One packet, copied out so the stream can be asked its rate, which is
    // only certain once a packet has been decoded.
    let (mut mono, mut stereo) = (Vec::new(), Vec::new());
    loop {
        if stop() {
            return Ok(None);
        }
        match stream.next_frames() {
            Some((packet, frames)) => {
                mono.clear();
                mono.extend_from_slice(packet);
                stereo.clear();
                stereo.extend_from_slice(frames);
            }
            None => break,
        }
        let rate = stream.sample_rate();
        analyser.get_or_insert_with(|| Analyser::new(rate)).push(&mono, &stereo);
    }
    match analyser {
        Some(analyser) if analyser.frames() > 0 => Ok(Some(analyser.finish(bar_starts))),
        _ => Err(rbl_audio::AudioError::Unsupported("decoded no samples".into())),
    }
}
