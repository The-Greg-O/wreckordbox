//! The streaming pass: audio in, a few numbers kept per tenth of a second
//! and per spectral frame, and the curves made from those at the end.
//!
//! Two timelines are kept, both far smaller than the audio:
//!
//! - **loudness blocks**: the K-weighted mean square of every 100 ms, the
//!   two channels summed, as BS.1770 measures;
//! - **spectral frames**: a Hann-windowed transform about 46 ms long every
//!   23 ms of the mono mix, from which come the low-end and high-end power
//!   and the spectral flux.
//!
//! A point of a curve (a second, or a bar) is made from the blocks and
//! frames whose centres fall inside it.

use std::ops::Range;
use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

use crate::loudness::{integrated_lufs, lufs, KWeighting, BLOCK_SECONDS};
use crate::{
    combine, BarCurve, Features, Intensity, MeasuredAudio, Method, Summary, TimeCurve,
    HIGH_END_ABOVE_HZ, LOW_END_ABOVE_HZ, LOW_END_BELOW_HZ, SCALES, SILENCE_DB, STEP_SECONDS,
    WEIGHTS,
};

/// Seconds of audio in one spectral frame, before rounding up to a power
/// of two samples: 2048 samples at 44.1 and 48 kHz.
const FRAME_SECONDS: f64 = 0.04;
/// How hard the magnitudes are compressed before the flux is taken:
/// `ln(1 + FLUX_COMPRESSION * magnitude)`, magnitude 1 being a full-scale
/// sine. The logarithm makes a rise count by its ratio, so the flux says
/// how much happens, and loudness is left to say how loud.
const FLUX_COMPRESSION: f32 = 1_000.0;
/// A flux peak is an onset when it is this many times the mean flux of
/// the second around it, plus [`ONSET_FLOOR`].
const ONSET_RATIO: f32 = 1.5;
/// The flux below which nothing is an onset: the jitter of a held note.
const ONSET_FLOOR: f32 = 0.02;
/// An onset is the highest flux within this many seconds either side, so
/// two onsets are never closer than this.
const ONSET_SPACING_SECONDS: f64 = 0.05;
/// The window R128's short-term loudness is measured over.
const SHORT_TERM_SECONDS: f64 = 3.0;

/// What is kept of one spectral frame.
#[derive(Debug, Clone, Copy)]
struct Frame {
    /// Mean square of the audio below the low-end edge.
    low: f32,
    /// Mean square of the audio above the high-end edge.
    high: f32,
    /// The rise in compressed magnitude since the frame before, averaged
    /// over the bins.
    flux: f32,
}

/// Measures Intensity from audio pushed a piece at a time.
pub struct Analyser {
    rate: u32,
    /// Sample frames pushed so far.
    pushed: u64,

    weighting: [KWeighting; 2],
    block_len: usize,
    in_block: usize,
    block_sum: f64,
    blocks: Vec<f64>,

    frame_len: usize,
    hop: usize,
    window: Vec<f32>,
    /// One-sided power to mean square: `2 / (N * sum of window squared)`.
    power_norm: f32,
    /// Magnitude to "1 is a full-scale sine": `2 / sum of window`.
    magnitude_norm: f32,
    low_bins: Range<usize>,
    high_bins: Range<usize>,
    transform: Arc<dyn RealToComplex<f32>>,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    previous: Vec<f32>,
    has_previous: bool,
    /// Mono samples not yet used up by a frame; `pending[consumed..]` are
    /// live.
    pending: Vec<f32>,
    consumed: usize,
    frames: Vec<Frame>,
}

impl Analyser {
    /// For audio at `sample_rate`.
    #[must_use]
    pub fn new(sample_rate: u32) -> Self {
        let rate = sample_rate.max(1);
        let frame_len = ((f64::from(rate) * FRAME_SECONDS) as usize)
            .next_power_of_two()
            .max(64);
        let window: Vec<f32> = (0..frame_len)
            .map(|i| {
                let phase = 2.0 * std::f64::consts::PI * i as f64 / frame_len as f64;
                (0.5 - 0.5 * phase.cos()) as f32
            })
            .collect();
        let window_sum: f32 = window.iter().sum();
        let window_squares: f32 = window.iter().map(|w| w * w).sum();
        let bins = frame_len / 2 + 1;
        let bin_of =
            |hz: f64| ((hz * frame_len as f64 / f64::from(rate)).ceil() as usize).min(bins);
        let transform = RealFftPlanner::<f32>::new().plan_fft_forward(frame_len);
        Self {
            rate,
            pushed: 0,
            weighting: [KWeighting::new(rate), KWeighting::new(rate)],
            block_len: ((f64::from(rate) * BLOCK_SECONDS).round() as usize).max(1),
            in_block: 0,
            block_sum: 0.0,
            blocks: Vec::new(),
            frame_len,
            hop: frame_len / 2,
            power_norm: 2.0 / (frame_len as f32 * window_squares),
            magnitude_norm: 2.0 / window_sum,
            low_bins: bin_of(LOW_END_ABOVE_HZ).max(1)..bin_of(LOW_END_BELOW_HZ),
            high_bins: bin_of(HIGH_END_ABOVE_HZ)..bins,
            input: transform.make_input_vec(),
            spectrum: transform.make_output_vec(),
            scratch: transform.make_scratch_vec(),
            transform,
            window,
            previous: vec![0.0; bins],
            has_previous: false,
            pending: Vec::new(),
            consumed: 0,
            frames: Vec::new(),
        }
    }

    /// Sample frames pushed so far.
    #[must_use]
    pub fn frames(&self) -> u64 {
        self.pushed
    }

    /// The next stretch of audio: the mono mix, and the same sample frames
    /// as 16-bit stereo (one channel twice for a mono file), which is how
    /// the analysis decoder hands audio out. Loudness is measured from the
    /// two channels, as the standard says; the spectrum from the mix.
    pub fn push(&mut self, mono: &[f32], stereo: &[[i16; 2]]) {
        for [left, right] in stereo {
            self.weigh(f64::from(*left) / 32_768.0, f64::from(*right) / 32_768.0);
        }
        self.spectra(mono);
        self.pushed += mono.len() as u64;
    }

    /// The next stretch of audio that has one channel, heard from both
    /// speakers.
    pub fn push_mono(&mut self, mono: &[f32]) {
        for sample in mono {
            self.weigh(f64::from(*sample), f64::from(*sample));
        }
        self.spectra(mono);
        self.pushed += mono.len() as u64;
    }

    fn weigh(&mut self, left: f64, right: f64) {
        let (left, right) = (self.weighting[0].next(left), self.weighting[1].next(right));
        self.block_sum += left * left + right * right;
        self.in_block += 1;
        if self.in_block == self.block_len {
            self.blocks.push(self.block_sum / self.block_len as f64);
            self.block_sum = 0.0;
            self.in_block = 0;
        }
    }

    fn spectra(&mut self, mono: &[f32]) {
        self.pending.extend_from_slice(mono);
        while self.pending.len() - self.consumed >= self.frame_len {
            let start = self.consumed;
            for ((slot, sample), weight) in self
                .input
                .iter_mut()
                .zip(&self.pending[start..start + self.frame_len])
                .zip(&self.window)
            {
                *slot = sample * weight;
            }
            self.consumed += self.hop;
            // The lengths are the transform's own, so this cannot fail; a
            // frame that somehow did would count as silence.
            if self
                .transform
                .process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch)
                .is_err()
            {
                self.spectrum.fill(Complex::new(0.0, 0.0));
            }
            self.frame();
        }
        // Drop what every later frame is past, now and then.
        if self.consumed > self.frame_len * 16 {
            self.pending.drain(..self.consumed);
            self.consumed = 0;
        }
    }

    /// Keeps what is wanted of the spectrum just computed.
    fn frame(&mut self) {
        let power = |bins: &Range<usize>| -> f32 {
            self.spectrum
                .get(bins.clone())
                .map_or(0.0, |bins| bins.iter().map(Complex::norm_sqr).sum())
        };
        let low = power(&self.low_bins) * self.power_norm;
        let high = power(&self.high_bins) * self.power_norm;
        let mut rise = 0.0_f32;
        // Bin 0 is the offset, not sound.
        for (bin, previous) in self.spectrum.iter().zip(self.previous.iter_mut()).skip(1) {
            let compressed = (FLUX_COMPRESSION * bin.norm() * self.magnitude_norm).ln_1p();
            rise += (compressed - *previous).max(0.0);
            *previous = compressed;
        }
        // The first frame rises from nothing, which is the file starting,
        // not the music.
        let flux = if self.has_previous {
            rise / (self.previous.len() - 1).max(1) as f32
        } else {
            0.0
        };
        self.has_previous = true;
        self.frames.push(Frame { low, high, flux });
    }

    /// The Intensity of everything pushed. `bar_starts` is where each bar
    /// of the beat grid begins, in seconds and in order; empty for no grid.
    #[must_use]
    pub fn finish(mut self, bar_starts: &[f64]) -> Intensity {
        // A last block at least half full counts; less is dropped.
        if self.in_block * 2 >= self.block_len {
            self.blocks.push(self.block_sum / self.in_block as f64);
        }
        let seconds = self.pushed as f64 / f64::from(self.rate);
        let onsets = self.onsets();
        let view = View {
            analyser: &self,
            onsets: &onsets,
        };

        let points = ((seconds / STEP_SECONDS).round() as usize).max(usize::from(self.pushed > 0));
        let spans: Vec<(f64, f64)> = (0..points)
            .map(|point| {
                (
                    point as f64 * STEP_SECONDS,
                    (point + 1) as f64 * STEP_SECONDS,
                )
            })
            .collect();
        let (intensity, features) = view.curve(&spans);

        let mut ordered = intensity.clone();
        ordered.sort_by(f64::total_cmp);
        let quarter = ordered.len().div_ceil(4).max(1).min(ordered.len());
        let whole = view.features(0.0, seconds.max(STEP_SECONDS));
        let summary = Summary {
            intensity: round(mean(&intensity), 4),
            high: round(
                mean(ordered.get(ordered.len() - quarter..).unwrap_or(&[])),
                4,
            ),
            low: round(mean(ordered.get(..quarter).unwrap_or(&[])), 4),
            loudness_integrated_lufs: round(integrated_lufs(&self.blocks), 2),
            loudness_short_term_max_lufs: round(self.short_term_max(), 2),
            features: rounded(&whole),
        };

        Intensity {
            method: Method::current(),
            audio: MeasuredAudio {
                sample_rate: self.rate,
                seconds: round(seconds, 3),
            },
            summary,
            curve: TimeCurve {
                offset_seconds: 0.0,
                step_seconds: STEP_SECONDS,
                intensity,
                features,
            },
            bars: view.bars(bar_starts, seconds),
        }
    }

    /// Which frames are onsets: flux peaks that stand clear of the flux
    /// around them.
    fn onsets(&self) -> Vec<bool> {
        let per_second = f64::from(self.rate) / self.hop as f64;
        let spacing = ((ONSET_SPACING_SECONDS * per_second).round() as usize).max(1);
        let around = ((0.5 * per_second).round() as usize).max(1);
        let flux: Vec<f32> = self.frames.iter().map(|frame| frame.flux).collect();
        // Running sums, so the mean around each frame is two lookups.
        let mut sums = Vec::with_capacity(flux.len() + 1);
        sums.push(0.0_f64);
        for value in &flux {
            sums.push(sums.last().copied().unwrap_or(0.0) + f64::from(*value));
        }
        (0..flux.len())
            .map(|at| {
                let here = flux[at];
                let from = at.saturating_sub(around);
                let to = (at + around + 1).min(flux.len());
                let local = ((sums[to] - sums[from]) / (to - from) as f64) as f32;
                if here <= ONSET_FLOOR + ONSET_RATIO * local {
                    return false;
                }
                let before = &flux[at.saturating_sub(spacing)..at];
                let after = &flux[at + 1..(at + spacing + 1).min(flux.len())];
                // The first of equal peaks is the onset.
                before.iter().all(|other| *other < here) && after.iter().all(|other| *other <= here)
            })
            .collect()
    }

    /// The loudest three seconds, stepped a block at a time.
    fn short_term_max(&self) -> f64 {
        let window = ((SHORT_TERM_SECONDS / BLOCK_SECONDS).round() as usize).max(1);
        if self.blocks.len() < window {
            return lufs(mean(&self.blocks));
        }
        self.blocks
            .windows(window)
            .map(|blocks| lufs(mean(blocks)))
            .fold(f64::NEG_INFINITY, f64::max)
    }
}

/// The finished timelines, read a span at a time.
struct View<'a> {
    analyser: &'a Analyser,
    onsets: &'a [bool],
}

impl View<'_> {
    /// The features of the audio from `from` to `to` seconds, each in its
    /// own unit.
    fn features(&self, from: f64, to: f64) -> Features<f64> {
        let analyser = self.analyser;
        let rate = f64::from(analyser.rate);
        let length = (to - from).max(f64::EPSILON);

        let block = analyser.block_len as f64 / rate;
        let blocks =
            &analyser.blocks[centred_in(from, to, block / 2.0, block, analyser.blocks.len())];

        let hop = analyser.hop as f64 / rate;
        let first_centre = analyser.frame_len as f64 / 2.0 / rate;
        let range = centred_in(from, to, first_centre, hop, analyser.frames.len());
        let frames = &analyser.frames[range.clone()];
        let level = |power: f64| {
            if power > 0.0 {
                (10.0 * power.log10()).max(SILENCE_DB)
            } else {
                SILENCE_DB
            }
        };
        let mean_of = |pick: fn(&Frame) -> f32| {
            if frames.is_empty() {
                0.0
            } else {
                frames
                    .iter()
                    .map(|frame| f64::from(pick(frame)))
                    .sum::<f64>()
                    / frames.len() as f64
            }
        };

        Features {
            loudness: lufs(mean(blocks)),
            low_end: level(mean_of(|frame| frame.low)),
            high_end: level(mean_of(|frame| frame.high)),
            onset_strength: frames
                .iter()
                .map(|frame| f64::from(frame.flux))
                .sum::<f64>()
                / length,
            onset_density: self.onsets[range].iter().filter(|onset| **onset).count() as f64
                / length,
        }
    }

    /// The combined Intensity and the features of each span, rounded as
    /// they are stored.
    fn curve(&self, spans: &[(f64, f64)]) -> (Vec<f64>, Features<Vec<f64>>) {
        let mut intensity = Vec::with_capacity(spans.len());
        let mut curves: Features<Vec<f64>> = Features {
            loudness: Vec::with_capacity(spans.len()),
            low_end: Vec::with_capacity(spans.len()),
            high_end: Vec::with_capacity(spans.len()),
            onset_strength: Vec::with_capacity(spans.len()),
            onset_density: Vec::with_capacity(spans.len()),
        };
        for (from, to) in spans {
            // Combined from the rounded features, so the stored features
            // give back exactly the stored Intensity.
            let features = rounded(&self.features(*from, *to));
            intensity.push(round(combine(&features, &WEIGHTS, &SCALES), 4));
            curves.loudness.push(features.loudness);
            curves.low_end.push(features.low_end);
            curves.high_end.push(features.high_end);
            curves.onset_strength.push(features.onset_strength);
            curves.onset_density.push(features.onset_density);
        }
        (intensity, curves)
    }

    /// One point per bar: from each bar's start to the next one's, the last
    /// bar as long as the one before it. Bars that begin after the audio
    /// ends, or before it begins, are left out.
    fn bars(&self, bar_starts: &[f64], seconds: f64) -> Option<BarCurve> {
        let inside = |start: &f64| start.is_finite() && *start >= 0.0 && *start < seconds;
        let starts: Vec<f64> = bar_starts.iter().copied().filter(inside).collect();
        let last_at = bar_starts.iter().rposition(inside)?;
        let last = bar_starts[last_at];
        let last_length = match (bar_starts.get(last_at + 1), last_at.checked_sub(1)) {
            // The grid's own next bar, when it has one past the audio.
            (Some(next), _) if next.is_finite() && *next > last => next - last,
            (_, Some(before)) => last - bar_starts[before],
            _ => seconds - last,
        };
        let end = (last + last_length).min(seconds).max(last);
        let spans: Vec<(f64, f64)> = starts
            .iter()
            .enumerate()
            .map(|(at, start)| (*start, starts.get(at + 1).copied().unwrap_or(end)))
            .collect();
        let (intensity, features) = self.curve(&spans);
        Some(BarCurve {
            starts_seconds: starts.iter().map(|start| round(*start, 3)).collect(),
            end_seconds: round(end, 3),
            intensity,
            features,
        })
    }
}

/// Which of `len` things, the first centred at `first_centre` seconds and
/// each `step` after the last, have their centre in `from..to`.
fn centred_in(from: f64, to: f64, first_centre: f64, step: f64, len: usize) -> Range<usize> {
    let index =
        |seconds: f64| (((seconds - first_centre) / step).ceil().max(0.0) as usize).min(len);
    let start = index(from);
    start..index(to).max(start)
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

/// `value` to `places` decimal places: what is stored, so the same audio
/// stores the same text.
fn round(value: f64, places: i32) -> f64 {
    let scale = 10_f64.powi(places);
    let rounded = (value * scale).round() / scale;
    // Minus zero would print differently from zero.
    if rounded == 0.0 {
        0.0
    } else {
        rounded
    }
}

fn rounded(features: &Features<f64>) -> Features<f64> {
    Features {
        loudness: round(features.loudness, 2),
        low_end: round(features.low_end, 2),
        high_end: round(features.high_end, 2),
        onset_strength: round(features.onset_strength, 4),
        onset_density: round(features.onset_density, 4),
    }
}
