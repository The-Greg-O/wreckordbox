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
//! point per bar. Beside them it keeps the finer [`Cells`] every curve is
//! made from, and which beat grid the bars were made for ([`grid_of`]),
//! so that a grid added or corrected later gets its bars from the stored
//! value, without the audio ([`Intensity::with_bars`]).
//! [`METHOD_VERSION`] changes whenever a number here would come out
//! differently for the same audio.
//!
//! Only mono and stereo audio is measured; a file of more channels is
//! refused ([`MeasureError::Channels`]). A mono file is measured as it is
//! heard, from both speakers. Nothing depends on the sample rate: the same
//! band-limited audio measures the same at 44.1, 48, 88.2 or 96 kHz.
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
pub const METHOD_VERSION: u32 = 2;

/// Seconds between points of the time curve.
pub const STEP_SECONDS: f64 = 1.0;
/// The low end is what lies below this: the kick's body and the sub.
pub const LOW_END_BELOW_HZ: f64 = 150.0;
/// Nothing below this counts as low end: it is rumble and offset.
pub const LOW_END_ABOVE_HZ: f64 = 20.0;
/// The high end is what lies above this: hats, cymbals, air.
pub const HIGH_END_ABOVE_HZ: f64 = 5_000.0;
/// ... and below this, so a file at a higher sample rate does not add
/// what nobody hears.
pub const HIGH_END_BELOW_HZ: f64 = 20_000.0;
/// Seconds of audio in one [`Cells`] entry.
pub const CELL_SECONDS: f64 = 0.2;
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

impl<T: Clone> Features<T> {
    /// The same value for every feature.
    #[must_use]
    pub fn splat(value: T) -> Self {
        Self {
            loudness: value.clone(),
            low_end: value.clone(),
            high_end: value.clone(),
            onset_strength: value.clone(),
            onset_density: value,
        }
    }
}

/// The features' names, in order.
pub const FEATURES: [&str; 5] = [
    "loudness",
    "low_end",
    "high_end",
    "onset_strength",
    "onset_density",
];

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
pub const WEIGHTS: Features<f64> = Features {
    loudness: 0.40,
    low_end: 0.15,
    high_end: 0.10,
    onset_strength: 0.15,
    onset_density: 0.20,
};

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
    loudness: Scale {
        floor: -40.0,
        ceiling: -5.0,
    },
    low_end: Scale {
        floor: -50.0,
        ceiling: -10.0,
    },
    high_end: Scale {
        floor: -60.0,
        ceiling: -20.0,
    },
    onset_strength: Scale {
        floor: 0.0,
        ceiling: 10.0,
    },
    onset_density: Scale {
        floor: 0.0,
        ceiling: 8.0,
    },
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

/// The audio every [`CELL_SECONDS`], what every curve is made from: cell
/// `i` is the audio from `i * step_seconds` (the last one is cut to the
/// audio's end). Rounded as stored, so a curve made from the stored cells
/// is the stored curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cells {
    pub step_seconds: f64,
    /// K-weighted loudness of the cell, LUFS (silence -100).
    pub loudness: Vec<f64>,
    /// Mean level below [`LOW_END_BELOW_HZ`], dB re full scale.
    pub low_end: Vec<f64>,
    /// Mean level above [`HIGH_END_ABOVE_HZ`], dB re full scale.
    pub high_end: Vec<f64>,
    /// Spectral flux summed over the cell.
    pub flux: Vec<f64>,
    /// Onsets in the cell.
    pub onsets: Vec<u32>,
}

/// A track's Intensity: the value of the Measurement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Intensity {
    pub method: Method,
    pub audio: MeasuredAudio,
    pub summary: Summary,
    pub curve: TimeCurve,
    /// Which beat grid `bars` was made for ([`grid_of`] its bar lines);
    /// `null` when it was made with none.
    pub grid: Option<String>,
    /// One point per bar, when the track had a beat grid; `null` when not.
    pub bars: Option<BarCurve>,
    /// The finer timeline the curves are made from.
    pub cells: Cells,
}

impl Intensity {
    /// The same Intensity with the bars of another beat grid, made from the
    /// stored cells: exactly what measuring the audio again with that grid
    /// would give, without the audio.
    ///
    /// # Errors
    ///
    /// When the value fails its [`check`](Self::check).
    pub fn with_bars(&self, bar_starts: &[f64]) -> Result<Self, Invalid> {
        let mut again = self.clone();
        again.grid = grid_of(bar_starts);
        again.bars = analyser::bars(&self.cells, self.audio.seconds, bar_starts);
        again.check()?;
        Ok(again)
    }

    /// Whether every number is a number and in its range, and the curves
    /// are as long as each other: what a value must be to be recorded, and
    /// to be reused.
    ///
    /// # Errors
    ///
    /// Says what is wrong.
    pub fn check(&self) -> Result<(), Invalid> {
        let bad = |what: &str| Err(Invalid::Result(what.to_owned()));
        let finite = |values: &[f64]| values.iter().all(|value| value.is_finite());
        let unit = |values: &[f64]| values.iter().all(|value| (0.0..=1.0).contains(value));
        let features = |f: &Features<Vec<f64>>, len: usize| {
            [
                &f.loudness,
                &f.low_end,
                &f.high_end,
                &f.onset_strength,
                &f.onset_density,
            ]
            .iter()
            .all(|values| values.len() == len && finite(values))
        };
        let summary = &self.summary;
        let s = &summary.features;
        if !finite(&[
            self.audio.seconds,
            summary.loudness_integrated_lufs,
            summary.loudness_short_term_max_lufs,
            s.loudness,
            s.low_end,
            s.high_end,
            s.onset_strength,
            s.onset_density,
        ]) || !unit(&[summary.intensity, summary.high, summary.low])
            || self.audio.seconds <= 0.0
        {
            return bad("the summary");
        }
        let curve = &self.curve;
        if curve.intensity.is_empty()
            || !unit(&curve.intensity)
            || !features(&curve.features, curve.intensity.len())
        {
            return bad("the time curve");
        }
        if let Some(bars) = &self.bars {
            let n = bars.intensity.len();
            if n == 0
                || bars.starts_seconds.len() != n
                || !finite(&bars.starts_seconds)
                || !finite(&[bars.end_seconds])
                || !unit(&bars.intensity)
                || !features(&bars.features, n)
            {
                return bad("the bars");
            }
        }
        let cells = &self.cells;
        let n = cells.loudness.len();
        if n == 0
            || cells.step_seconds.is_nan()
            || cells.step_seconds <= 0.0
            || [&cells.low_end, &cells.high_end, &cells.flux]
                .iter()
                .any(|values| values.len() != n || !finite(values))
            || cells.onsets.len() != n
            || !finite(&cells.loudness)
        {
            return bad("the cells");
        }
        Ok(())
    }
}

/// Names a beat grid by its bar lines: `g1:` and a hash of where each bar
/// begins, to the microsecond. `None` for no grid.
#[must_use]
pub fn grid_of(bar_starts: &[f64]) -> Option<String> {
    if bar_starts.is_empty() {
        return None;
    }
    // FNV-1a, 64 bits: stable across builds and machines.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for start in bar_starts {
        let micros = if start.is_finite() {
            (start * 1e6).round() as i64
        } else {
            i64::MIN
        };
        for byte in micros.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    Some(format!("g1:{hash:016x}"))
}

/// Why audio could not become a value, or a value is not one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Invalid {
    #[error("the audio holds samples that are not numbers (NaN or infinite)")]
    NonFinite,
    #[error("the audio decoded to no samples")]
    Empty,
    #[error("the measurement is not valid: {0} has a number out of range or not a number")]
    Result(String),
}

/// Why a file could not be measured.
#[derive(Debug, thiserror::Error)]
pub enum MeasureError {
    #[error(transparent)]
    Audio(#[from] rbl_audio::AudioError),
    #[error("the audio has {0} channels; Intensity measures mono and stereo files only")]
    Channels(u16),
    #[error(transparent)]
    Invalid(#[from] Invalid),
}

/// `value` to `places` decimal places: what is stored, so the same audio
/// stores the same text.
pub(crate) fn round(value: f64, places: i32) -> f64 {
    let scale = 10_f64.powi(places);
    let rounded = (value * scale).round() / scale;
    // Minus zero would print differently from zero.
    if rounded == 0.0 {
        0.0
    } else {
        rounded
    }
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
/// When the file cannot be opened, has more than two channels, decodes to
/// nothing, or holds samples that are not numbers.
pub fn measure_file(
    path: &Path,
    bar_starts: &[f64],
    stop: &dyn Fn() -> bool,
) -> Result<Option<Intensity>, MeasureError> {
    let mut stream = rbl_audio::MonoStream::open(path)?;
    let channels_ok = |stream: &rbl_audio::MonoStream| match stream.source_channels() {
        1 | 2 => Ok(()),
        other => Err(MeasureError::Channels(other)),
    };
    channels_ok(&stream)?;
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
        // Said per packet, by the packet.
        channels_ok(&stream)?;
        let rate = stream.sample_rate();
        analyser
            .get_or_insert_with(|| Analyser::new(rate))
            .push(&mono, &stereo);
    }
    let analyser = analyser.ok_or(Invalid::Empty)?;
    Ok(Some(analyser.finish(bar_starts)?))
}
