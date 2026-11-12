//! Loudness after ITU-R BS.1770 and EBU R128: the K-weighting filter, the
//! loudness of a stretch of audio, and the gated loudness of a whole track.
//!
//! The analyser keeps the K-weighted mean square of every 100 ms of audio
//! (the channels summed), which is all any of these needs. A stretch's
//! loudness is `-0.691 + 10 log10(mean of its blocks)` LUFS.

/// Seconds of audio in one kept block. The standards' 400 ms window with
/// 75 % overlap is four of these, stepped one at a time.
pub const BLOCK_SECONDS: f64 = 0.1;
/// Blocks in the 400 ms window the gate is applied to.
const GATE_BLOCKS: usize = 4;
/// Below this a window is silence and never counts (BS.1770's absolute gate).
pub const ABSOLUTE_GATE_LUFS: f64 = -70.0;
/// A window this far below the loudness of the rest does not count
/// (BS.1770's relative gate).
pub const RELATIVE_GATE_LU: f64 = 10.0;
/// What silence is reported as: it has no loudness, and JSON has no
/// minus infinity.
pub const SILENCE_LUFS: f64 = -100.0;

/// The loudness of a stretch whose K-weighted mean square is `mean_square`.
#[must_use]
pub fn lufs(mean_square: f64) -> f64 {
    if mean_square <= 0.0 {
        return SILENCE_LUFS;
    }
    (-0.691 + 10.0 * mean_square.log10()).max(SILENCE_LUFS)
}

/// The integrated loudness of a track from its 100 ms blocks, with the
/// standard's two gates: windows below -70 LUFS are left out, then windows
/// more than 10 LU below the loudness of what remains.
#[must_use]
pub fn integrated_lufs(blocks: &[f64]) -> f64 {
    let windows: Vec<f64> = if blocks.len() < GATE_BLOCKS {
        // Shorter than one window: what there is, as one.
        if blocks.is_empty() {
            Vec::new()
        } else {
            vec![blocks.iter().sum::<f64>() / blocks.len() as f64]
        }
    } else {
        blocks
            .windows(GATE_BLOCKS)
            .map(|window| window.iter().sum::<f64>() / GATE_BLOCKS as f64)
            .collect()
    };
    let mean_above = |gate_lufs: f64| {
        let (sum, count) = windows
            .iter()
            .filter(|mean_square| lufs(**mean_square) > gate_lufs)
            .fold((0.0, 0_usize), |(sum, count), mean_square| {
                (sum + mean_square, count + 1)
            });
        (count > 0).then(|| sum / count as f64)
    };
    let Some(ungated) = mean_above(ABSOLUTE_GATE_LUFS) else {
        return SILENCE_LUFS;
    };
    let relative_gate = (lufs(ungated) - RELATIVE_GATE_LU).max(ABSOLUTE_GATE_LUFS);
    mean_above(relative_gate).map_or(SILENCE_LUFS, lufs)
}

/// One second-order filter section, direct form I.
#[derive(Debug, Clone, Copy)]
struct Biquad {
    /// `b0 b1 b2 a1 a2`, with `a0` already divided out.
    c: [f64; 5],
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    fn new(c: [f64; 5]) -> Self {
        Self {
            c,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    #[inline]
    fn next(&mut self, x: f64) -> f64 {
        let [b0, b1, b2, a1, a2] = self.c;
        let y = b0 * x + b1 * self.x1 + b2 * self.x2 - a1 * self.y1 - a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// The K-weighting of one channel: a high shelf (the head's effect) and a
/// high-pass (the ear's insensitivity to the very low end), in series.
#[derive(Debug, Clone, Copy)]
pub struct KWeighting {
    shelf: Biquad,
    high_pass: Biquad,
}

impl KWeighting {
    /// The filter for audio at `sample_rate`.
    #[must_use]
    pub fn new(sample_rate: u32) -> Self {
        let [shelf, high_pass] = Self::coefficients(sample_rate);
        Self {
            shelf: Biquad::new(shelf),
            high_pass: Biquad::new(high_pass),
        }
    }

    /// The two sections' coefficients (`b0 b1 b2 a1 a2`) at `sample_rate`.
    ///
    /// BS.1770 tabulates them at 48 kHz only. These are the analogue
    /// prototypes behind that table, re-mapped to the rate asked for, which
    /// is how every R128 meter handles other rates; at 48 kHz they are the
    /// table.
    #[must_use]
    pub fn coefficients(sample_rate: u32) -> [[f64; 5]; 2] {
        let rate = f64::from(sample_rate.max(1));

        let (f0, gain_db, q) = (
            1_681.974_450_955_533,
            3.999_843_853_973_347,
            0.707_175_236_955_419_6,
        );
        let k = (std::f64::consts::PI * f0 / rate).tan();
        let vh = 10_f64.powf(gain_db / 20.0);
        let vb = vh.powf(0.499_666_774_154_541_6);
        let a0 = 1.0 + k / q + k * k;
        let shelf = [
            (vh + vb * k / q + k * k) / a0,
            2.0 * (k * k - vh) / a0,
            (vh - vb * k / q + k * k) / a0,
            2.0 * (k * k - 1.0) / a0,
            (1.0 - k / q + k * k) / a0,
        ];

        let (f0, q) = (38.135_470_876_024_44, 0.500_327_037_323_877_3);
        let k = (std::f64::consts::PI * f0 / rate).tan();
        let a0 = 1.0 + k / q + k * k;
        let high_pass = [
            1.0,
            -2.0,
            1.0,
            2.0 * (k * k - 1.0) / a0,
            (1.0 - k / q + k * k) / a0,
        ];

        [shelf, high_pass]
    }

    /// The next sample, weighted.
    #[inline]
    pub fn next(&mut self, sample: f64) -> f64 {
        self.high_pass.next(self.shelf.next(sample))
    }
}
