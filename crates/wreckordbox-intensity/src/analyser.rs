//! The streaming pass: audio in, a few numbers kept per tenth of a second
//! and per spectral frame, and the curves made from those at the end.
//!
//! Three timelines are kept, all far smaller than the audio:
//!
//! - **loudness blocks**: the K-weighted mean square of every 100 ms, the
//!   two channels summed, as BS.1770 measures; the gated integrated and
//!   the short-term loudness come from these;
//! - **spectral frames**: a Hann-windowed transform about 46 ms long every
//!   23 ms of the mono mix, from which come the low-end and high-end power
//!   and the spectral flux;
//! - **cells**: every [`CELL_SECONDS`] of audio, the loudness, the band
//!   levels, the summed flux and the onsets that fell in it. Cells are
//!   stored with the value ([`Cells`]); every curve, the per-bar one
//!   included, is made from them alone, so the bars can be made again for
//!   another beat grid without the audio ([`Intensity::with_bars`]).
//!
//! Nothing depends on the sample rate but the precision: the frame is the
//! same length in seconds at every rate, so the transform's bins are the
//! same width in hertz, and the flux is averaged over the same band of
//! hertz ([`FLUX_ABOVE_HZ`] to [`FLUX_BELOW_HZ`]).

use std::ops::Range;
use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

use crate::loudness::{integrated_lufs, lufs, KWeighting, BLOCK_SECONDS, SILENCE_LUFS};
use crate::{
    combine, round, BarCurve, Cells, Features, Intensity, Invalid, MeasuredAudio, Method,
    Summary, TimeCurve, CELL_SECONDS, HIGH_END_ABOVE_HZ, HIGH_END_BELOW_HZ, LOW_END_ABOVE_HZ,
    LOW_END_BELOW_HZ, SCALES, SILENCE_DB, STEP_SECONDS, WEIGHTS,
};

/// Seconds of audio in one spectral frame: 2048 samples at 44.1 kHz, and
/// as long in seconds at any other rate.
const FRAME_SECONDS: f64 = 2_048.0 / 44_100.0;
/// The flux is the rise of the spectrum between these, the same band at
/// every sample rate (and below the top of a 22.05 kHz file).
const FLUX_ABOVE_HZ: f64 = 20.0;
/// See [`FLUX_ABOVE_HZ`].
const FLUX_BELOW_HZ: f64 = 11_000.0;
/// How hard the magnitudes are compressed before the flux is taken:
/// `ln(1 + FLUX_COMPRESSION * magnitude)`, magnitude 1 being a full-scale
/// sine. The logarithm makes a rise count by its ratio, so the flux says
/// how much happens, and loudness is left to say how loud.
const FLUX_COMPRESSION: f64 = 1_000.0;
/// A flux peak is an onset when it is this many times the mean flux of
/// the second around it, plus [`ONSET_FLOOR`].
const ONSET_RATIO: f64 = 1.5;
/// The flux below which nothing is an onset: the jitter of a held note.
const ONSET_FLOOR: f64 = 0.02;
/// An onset is the highest flux within this many seconds either side, so
/// two onsets are never closer than this.
const ONSET_SPACING_SECONDS: f64 = 0.05;
/// The window R128's short-term loudness is measured over.
const SHORT_TERM_SECONDS: f64 = 3.0;
/// Cells in a second; [`CELL_SECONDS`] is its inverse.
const CELLS_PER_SECOND: u64 = 5;

/// What is kept of one spectral frame.
#[derive(Debug, Clone, Copy)]
struct Frame {
    /// The sample at its middle.
    centre: u64,
    /// Mean square of the audio below the low-end edge.
    low: f64,
    /// Mean square of the audio above the high-end edge.
    high: f64,
    /// The rise in compressed magnitude since the frame before, averaged
    /// over the bins of the flux band.
    flux: f64,
}

/// Measures Intensity from audio pushed a piece at a time.
pub struct Analyser {
    rate: u32,
    /// Sample frames pushed so far.
    pushed: u64,
    /// Whether any sample pushed was not a number.
    non_finite: bool,

    weighting: [KWeighting; 2],
    block_len: usize,
    in_block: usize,
    block_sum: f64,
    blocks: Vec<f64>,
    /// The K-weighted sum of squares of each cell so far, and its frames.
    cell_sums: Vec<f64>,
    cell_counts: Vec<u64>,
    /// Sample frames weighed so far.
    weighed: u64,

    frame_len: usize,
    hop: usize,
    window: Vec<f32>,
    low_bins: Range<usize>,
    high_bins: Range<usize>,
    flux_bins: Range<usize>,
    transform: Arc<dyn RealToComplex<f32>>,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    previous: Vec<f64>,
    has_previous: bool,
    /// Mono samples not yet used up by a frame; `pending[consumed..]` are
    /// live, and `pending[0]` is sample `pending_start` of the audio.
    pending: Vec<f32>,
    consumed: usize,
    pending_start: u64,
    frames: Vec<Frame>,
}

impl Analyser {
    /// For audio at `sample_rate`.
    #[must_use]
    pub fn new(sample_rate: u32) -> Self {
        let rate = sample_rate.max(1);
        // Even, so the hop is exactly half.
        let frame_len = (((f64::from(rate) * FRAME_SECONDS / 2.0).round() as usize) * 2).max(64);
        let window: Vec<f32> = (0..frame_len)
            .map(|i| {
                let phase = 2.0 * std::f64::consts::PI * i as f64 / frame_len as f64;
                (0.5 - 0.5 * phase.cos()) as f32
            })
            .collect();
        let bins = frame_len / 2 + 1;
        let bin_of =
            |hz: f64| ((hz * frame_len as f64 / f64::from(rate)).ceil() as usize).min(bins);
        let transform = RealFftPlanner::<f32>::new().plan_fft_forward(frame_len);
        Self {
            rate,
            pushed: 0,
            non_finite: false,
            weighting: [KWeighting::new(rate), KWeighting::new(rate)],
            block_len: ((f64::from(rate) * BLOCK_SECONDS).round() as usize).max(1),
            in_block: 0,
            block_sum: 0.0,
            blocks: Vec::new(),
            cell_sums: Vec::new(),
            cell_counts: Vec::new(),
            weighed: 0,
            frame_len,
            hop: frame_len / 2,
            low_bins: bin_of(LOW_END_ABOVE_HZ).max(1)..bin_of(LOW_END_BELOW_HZ),
            high_bins: bin_of(HIGH_END_ABOVE_HZ)..bin_of(HIGH_END_BELOW_HZ),
            flux_bins: bin_of(FLUX_ABOVE_HZ).max(1)..bin_of(FLUX_BELOW_HZ),
            input: transform.make_input_vec(),
            spectrum: transform.make_output_vec(),
            scratch: transform.make_scratch_vec(),
            transform,
            window,
            previous: vec![0.0; bins],
            has_previous: false,
            pending: Vec::new(),
            consumed: 0,
            pending_start: 0,
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
            let sample = if sample.is_finite() {
                f64::from(*sample)
            } else {
                0.0
            };
            self.weigh(sample, sample);
        }
        self.spectra(mono);
        self.pushed += mono.len() as u64;
    }

    fn weigh(&mut self, left: f64, right: f64) {
        let (left, right) = (self.weighting[0].next(left), self.weighting[1].next(right));
        let square = left * left + right * right;
        self.block_sum += square;
        self.in_block += 1;
        if self.in_block == self.block_len {
            self.blocks.push(self.block_sum / self.block_len as f64);
            self.block_sum = 0.0;
            self.in_block = 0;
        }
        let cell = self.cell_of(self.weighed);
        if self.cell_sums.len() <= cell {
            self.cell_sums.resize(cell + 1, 0.0);
            self.cell_counts.resize(cell + 1, 0);
        }
        self.cell_sums[cell] += square;
        self.cell_counts[cell] += 1;
        self.weighed += 1;
    }

    /// The cell sample `at` falls in.
    fn cell_of(&self, at: u64) -> usize {
        (at * CELLS_PER_SECOND / u64::from(self.rate)) as usize
    }

    fn spectra(&mut self, mono: &[f32]) {
        if mono.iter().any(|sample| !sample.is_finite()) {
            self.non_finite = true;
            self.pending
                .extend(mono.iter().map(|s| if s.is_finite() { *s } else { 0.0 }));
        } else {
            self.pending.extend_from_slice(mono);
        }
        while self.pending.len() - self.consumed >= self.frame_len {
            let start = self.consumed;
            self.consumed += self.hop;
            self.transform_at(start, None);
        }
        // Drop what every later frame is past, now and then, keeping a
        // frame's worth for the last frame of the file.
        if self.consumed > self.frame_len * 16 {
            let drop = self.consumed - self.frame_len;
            self.pending.drain(..drop);
            self.pending_start += drop as u64;
            self.consumed -= drop;
        }
    }

    /// Transforms the frame starting at `pending[start]`: a whole frame
    /// with the analysis window, or, for a file shorter than a frame, all
    /// of it with a window of its own length (`short`) and silence after.
    fn transform_at(&mut self, start: usize, short: Option<&[f32]>) {
        let window = short.unwrap_or(&self.window);
        for (at, slot) in self.input.iter_mut().enumerate() {
            *slot = window
                .get(at)
                .map_or(0.0, |weight| self.pending[start + at] * weight);
        }
        // One-sided power to the mean square of the audio in the frame,
        // `2 / (N * sum of window squared)`; magnitude to "1 is a
        // full-scale sine", `2 / sum of window`.
        let squares: f64 = window.iter().map(|w| f64::from(*w).powi(2)).sum();
        let sum: f64 = window.iter().map(|w| f64::from(*w)).sum();
        let power_norm = if squares > 0.0 {
            2.0 / (self.frame_len as f64 * squares)
        } else {
            0.0
        };
        let magnitude_norm = if sum > 0.0 { 2.0 / sum } else { 0.0 };
        let centre = self.pending_start + (start + window.len() / 2) as u64;
        // The lengths are the transform's own, so this cannot fail; a
        // frame that somehow did would count as silence.
        if self
            .transform
            .process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch)
            .is_err()
        {
            self.spectrum.fill(Complex::new(0.0, 0.0));
        }
        self.frame(centre, power_norm, magnitude_norm);
    }

    /// How many samples from the start the frames so far reach.
    fn covered(&self) -> u64 {
        let half = (self.frame_len / 2) as u64;
        self.frames.last().map_or(0, |last| last.centre + half)
    }

    /// Keeps what is wanted of the spectrum just computed.
    fn frame(&mut self, centre: u64, power_norm: f64, magnitude_norm: f64) {
        let power = |bins: &Range<usize>| -> f64 {
            self.spectrum.get(bins.clone()).map_or(0.0, |bins| {
                bins.iter()
                    .map(|bin| f64::from(bin.re).powi(2) + f64::from(bin.im).powi(2))
                    .sum()
            })
        };
        let low = power(&self.low_bins) * power_norm;
        let high = power(&self.high_bins) * power_norm;
        let mut rise = 0.0_f64;
        for at in self.flux_bins.clone() {
            let bin = self.spectrum[at];
            let magnitude = f64::from(bin.re).hypot(f64::from(bin.im));
            let compressed = (FLUX_COMPRESSION * magnitude * magnitude_norm).ln_1p();
            rise += (compressed - self.previous[at]).max(0.0);
            self.previous[at] = compressed;
        }
        // The first frame rises from nothing, which is the file starting,
        // not the music.
        let flux = if self.has_previous {
            rise / self.flux_bins.len().max(1) as f64
        } else {
            0.0
        };
        self.has_previous = true;
        self.frames.push(Frame {
            centre,
            low,
            high,
            flux,
        });
    }

    /// The Intensity of everything pushed. `bar_starts` is where each bar
    /// of the beat grid begins, in seconds and in order; empty for no grid.
    ///
    /// # Errors
    ///
    /// When nothing was pushed, when a sample pushed was not a number, or
    /// when a number of the result is not one.
    pub fn finish(mut self, bar_starts: &[f64]) -> Result<Intensity, Invalid> {
        if self.non_finite {
            return Err(Invalid::NonFinite);
        }
        if self.pushed == 0 {
            return Err(Invalid::Empty);
        }
        // The last block counts however short it is.
        if self.in_block > 0 {
            self.blocks.push(self.block_sum / self.in_block as f64);
        }
        // The samples after the last whole frame are in one more frame,
        // the file's last `frame_len` samples; a file shorter than a frame
        // is one frame of its own length.
        let length = (self.pushed - self.pending_start) as usize;
        if self.frames.is_empty() {
            let short: Vec<f32> = (0..length)
                .map(|i| {
                    let phase = 2.0 * std::f64::consts::PI * (i as f64 + 0.5) / length as f64;
                    (0.5 - 0.5 * phase.cos()) as f32
                })
                .collect();
            self.transform_at(0, Some(&short));
        } else if self.covered() < self.pushed {
            self.transform_at(length - self.frame_len, None);
        }

        let seconds = round(self.pushed as f64 / f64::from(self.rate), 3);
        let cells = self.cells();
        let mut intensity = Intensity {
            method: Method::current(),
            audio: MeasuredAudio {
                sample_rate: self.rate,
                seconds,
            },
            summary: Summary {
                intensity: 0.0,
                high: 0.0,
                low: 0.0,
                loudness_integrated_lufs: round(integrated_lufs(&self.blocks), 2),
                loudness_short_term_max_lufs: round(self.short_term_max(), 2),
                features: Features::splat(0.0),
            },
            curve: TimeCurve {
                offset_seconds: 0.0,
                step_seconds: STEP_SECONDS,
                intensity: Vec::new(),
                features: Features::splat(Vec::new()),
            },
            grid: None,
            bars: None,
            cells,
        };
        assemble(&mut intensity);
        let intensity = intensity.with_bars(bar_starts)?;
        Ok(intensity)
    }

    /// The cells, rounded as they are stored.
    fn cells(&self) -> Cells {
        let count = ((self.pushed * CELLS_PER_SECOND).div_ceil(u64::from(self.rate))).max(1) as usize;
        let mut low = vec![0.0_f64; count];
        let mut high = vec![0.0_f64; count];
        let mut flux = vec![0.0_f64; count];
        let mut onsets = vec![0_u32; count];
        let mut framed = vec![0_u32; count];
        let picked = self.onsets();
        for (at, frame) in self.frames.iter().enumerate() {
            let cell = self.cell_of(frame.centre).min(count - 1);
            low[cell] += frame.low;
            high[cell] += frame.high;
            flux[cell] += frame.flux;
            onsets[cell] += u32::from(picked[at]);
            framed[cell] += 1;
        }
        let level = |power: f64| {
            if power > 0.0 {
                round((10.0 * power.log10()).max(SILENCE_DB), 2)
            } else {
                SILENCE_DB
            }
        };
        let mut low_end = Vec::with_capacity(count);
        let mut high_end = Vec::with_capacity(count);
        for cell in 0..count {
            if framed[cell] == 0 {
                // No frame is centred in it (only ever a sliver at the
                // end): it sounds like the cell before.
                let before = cell.checked_sub(1);
                low_end.push(before.map_or(SILENCE_DB, |b| low_end[b]));
                high_end.push(before.map_or(SILENCE_DB, |b| high_end[b]));
            } else {
                let n = f64::from(framed[cell]);
                low_end.push(level(low[cell] / n));
                high_end.push(level(high[cell] / n));
            }
        }
        let loudness = (0..count)
            .map(|cell| {
                let (sum, n) = (
                    self.cell_sums.get(cell).copied().unwrap_or(0.0),
                    self.cell_counts.get(cell).copied().unwrap_or(0),
                );
                if n == 0 {
                    SILENCE_LUFS
                } else {
                    round(lufs(sum / n as f64), 2)
                }
            })
            .collect();
        Cells {
            step_seconds: CELL_SECONDS,
            loudness,
            low_end,
            high_end,
            flux: flux.into_iter().map(|value| round(value, 4)).collect(),
            onsets,
        }
    }

    /// Which frames are onsets: flux peaks that stand clear of the flux
    /// around them.
    fn onsets(&self) -> Vec<bool> {
        let per_second = f64::from(self.rate) / self.hop as f64;
        let spacing = ((ONSET_SPACING_SECONDS * per_second).round() as usize).max(1);
        let around = ((0.5 * per_second).round() as usize).max(1);
        let flux: Vec<f64> = self.frames.iter().map(|frame| frame.flux).collect();
        // Running sums, so the mean around each frame is two lookups.
        let mut sums = Vec::with_capacity(flux.len() + 1);
        sums.push(0.0_f64);
        for value in &flux {
            sums.push(sums.last().copied().unwrap_or(0.0) + value);
        }
        (0..flux.len())
            .map(|at| {
                let here = flux[at];
                let from = at.saturating_sub(around);
                let to = (at + around + 1).min(flux.len());
                let local = (sums[to] - sums[from]) / (to - from) as f64;
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

/// Fills in the time curve and the summary from the cells.
fn assemble(intensity: &mut Intensity) {
    let seconds = intensity.audio.seconds;
    let points = ((seconds / STEP_SECONDS - 1e-9).ceil() as usize).max(1);
    let spans: Vec<(f64, f64)> = (0..points)
        .map(|point| {
            (
                point as f64 * STEP_SECONDS,
                ((point + 1) as f64 * STEP_SECONDS).min(seconds),
            )
        })
        .collect();
    let cells = &intensity.cells;
    let (curve, features) = curve(cells, seconds, &spans);
    let mut ordered = curve.clone();
    ordered.sort_by(f64::total_cmp);
    let quarter = ordered.len().div_ceil(4).max(1).min(ordered.len());
    let whole = rounded(&span_features(cells, seconds, 0.0, seconds));
    intensity.summary.intensity = round(mean(&curve), 4);
    intensity.summary.high = round(
        mean(ordered.get(ordered.len() - quarter..).unwrap_or(&[])),
        4,
    );
    intensity.summary.low = round(mean(ordered.get(..quarter).unwrap_or(&[])), 4);
    intensity.summary.features = whole;
    intensity.curve.intensity = curve;
    intensity.curve.features = features;
}

/// One point per bar, from the cells: from each bar's start to the next
/// one's, the last bar as long as the one before it. Bars that begin after
/// the audio ends, or before it begins, are left out.
pub(crate) fn bars(cells: &Cells, seconds: f64, bar_starts: &[f64]) -> Option<BarCurve> {
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
    let (intensity, features) = curve(cells, seconds, &spans);
    Some(BarCurve {
        starts_seconds: starts.iter().map(|start| round(*start, 3)).collect(),
        end_seconds: round(end, 3),
        intensity,
        features,
    })
}

/// The combined Intensity and the features of each span, rounded as they
/// are stored.
fn curve(cells: &Cells, seconds: f64, spans: &[(f64, f64)]) -> (Vec<f64>, Features<Vec<f64>>) {
    let mut intensity = Vec::with_capacity(spans.len());
    let mut curves: Features<Vec<f64>> = Features::splat(Vec::with_capacity(spans.len()));
    for (from, to) in spans {
        // Combined from the rounded features, so the stored features give
        // back exactly the stored Intensity.
        let features = rounded(&span_features(cells, seconds, *from, *to));
        intensity.push(round(combine(&features, &WEIGHTS, &SCALES), 4));
        curves.loudness.push(features.loudness);
        curves.low_end.push(features.low_end);
        curves.high_end.push(features.high_end);
        curves.onset_strength.push(features.onset_strength);
        curves.onset_density.push(features.onset_density);
    }
    (intensity, curves)
}

/// The features of the audio from `from` to `to` seconds, each in its own
/// unit, from the cells: each cell counts by how much of it the span
/// covers. A span is cut to the audio's own length.
fn span_features(cells: &Cells, seconds: f64, from: f64, to: f64) -> Features<f64> {
    let step = cells.step_seconds;
    let (from, to) = (from.max(0.0), to.min(seconds));
    let mut covered = 0.0;
    let (mut loud, mut low, mut high, mut flux, mut onsets) = (0.0, 0.0, 0.0, 0.0, 0.0);
    if to > from && step > 0.0 {
        let first = (from / step).floor() as usize;
        for cell in first..cells.loudness.len() {
            let start = cell as f64 * step;
            if start >= to {
                break;
            }
            let end = ((cell + 1) as f64 * step).min(seconds);
            let overlap = end.min(to) - start.max(from);
            if overlap <= 0.0 || end <= start {
                continue;
            }
            let share = overlap / (end - start);
            covered += overlap;
            loud += overlap * lufs_power(cells.loudness[cell]);
            low += overlap * db_power(cells.low_end[cell]);
            high += overlap * db_power(cells.high_end[cell]);
            flux += share * cells.flux[cell];
            onsets += share * f64::from(cells.onsets[cell]);
        }
    }
    if covered <= 0.0 {
        return Features {
            loudness: SILENCE_LUFS,
            low_end: SILENCE_DB,
            high_end: SILENCE_DB,
            onset_strength: 0.0,
            onset_density: 0.0,
        };
    }
    let level = |power: f64| {
        if power > 0.0 {
            (10.0 * power.log10()).max(SILENCE_DB)
        } else {
            SILENCE_DB
        }
    };
    Features {
        loudness: lufs(loud / covered),
        low_end: level(low / covered),
        high_end: level(high / covered),
        onset_strength: flux / covered,
        onset_density: onsets / covered,
    }
}

/// The mean square a stored loudness stands for.
fn lufs_power(lufs: f64) -> f64 {
    if lufs <= SILENCE_LUFS {
        0.0
    } else {
        10_f64.powf((lufs + 0.691) / 10.0)
    }
}

/// The power a stored level stands for.
fn db_power(db: f64) -> f64 {
    if db <= SILENCE_DB {
        0.0
    } else {
        10_f64.powf(db / 10.0)
    }
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
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
