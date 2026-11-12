//! The stretcher driven over a whole file instead of in real time.
//!
//! A deck pulls a block when the device asks for one and never sees the start
//! or the end of anything. Rendering a file wants the opposite: every input
//! frame accounted for, the output starting where the input starts and ending
//! where it ends. This is that path, and it is deliberately the *same engine
//! in the same mode* as the deck's key lock ([`crate::deck`]'s `key_lock`:
//! Rubber Band R3 in its real-time mode by default, WSOLA in a build without
//! it), so that a key or tempo heard on a deck and one rendered to a file are
//! the same sound. Rubber Band's own offline mode studies the file first and
//! places transients differently; it would be a second sound to keep in step.
//!
//! What the real-time use leaves out and this adds:
//!
//! - **The start.** A stretcher's window is centred, so its first output is
//!   the window filling. The backend says how much silence to feed first
//!   ([`Stretcher::start_pad`]) and how much output to throw away
//!   ([`Stretcher::start_delay`]); after that the first frame out is the
//!   first frame in.
//! - **The end.** The last input is still inside the window when the file
//!   runs out. [`Offline::finish`] feeds silence until it has come out.
//! - **The length.** The output is exactly `round(input / tempo)` frames. A
//!   real-time stretcher is only approximately that long, so the tail is cut
//!   or, if the backend comes up short, filled with silence.
//!
//! Nothing here runs in the audio thread; it allocates as it pleases.

use crate::stretch::{Stretcher, MAX_RATIO, MIN_RATIO};

const CHANNELS: usize = 2;

/// Frames pulled from the stretcher at a time.
const PULL: usize = 4096;

/// How many times in a row the stretcher may neither take input nor give
/// output before the render is called stuck. It should never happen once.
const PATIENCE: u32 = 64;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OfflineError {
    /// The tempo is outside what the stretcher holds
    /// ([`MIN_RATIO`] to [`MAX_RATIO`]), or not a number.
    #[error("the tempo ratio is outside what the stretcher holds")]
    TempoOutOfRange,
    /// The pitch scale is outside an octave either way, or not a number.
    #[error("the pitch scale is outside what the stretcher holds")]
    PitchOutOfRange,
    /// A pitch change was asked of a build whose stretcher cannot hold one
    /// (no Rubber Band: see `LICENSING.md`).
    #[error("this build's stretcher cannot shift pitch")]
    PitchUnavailable,
    /// The stretcher stopped taking input and giving output.
    #[error("the stretcher stalled")]
    Stalled,
}

/// One whole-file pass through the deck's stretcher.
///
/// Feed interleaved stereo with [`Offline::process`] in blocks of any size,
/// then call [`Offline::finish`] once. Output is appended to the caller's
/// buffer as it becomes available.
pub struct Offline {
    stretcher: Box<dyn Stretcher>,
    /// Input frames per output frame, as the stretcher holds it.
    tempo: f64,
    /// Real input frames fed so far.
    fed: u64,
    /// Output frames handed to the caller so far.
    emitted: u64,
    /// Output frames still to throw away: the start delay.
    to_drop: usize,
    /// Silence still to feed ahead of the first real frame.
    to_pad: usize,
    block: Vec<f32>,
}

impl Offline {
    /// The stretcher a deck at `sample_rate` would use, set to `tempo`
    /// (1.06 is six percent fast, so the output is that much shorter) and
    /// `pitch_scale` (a frequency multiplier; 1.0 leaves the pitch alone).
    pub fn new(sample_rate: u32, tempo: f32, pitch_scale: f32) -> Result<Self, OfflineError> {
        Self::with(crate::deck::key_lock(sample_rate), tempo, pitch_scale)
    }

    /// The same, over a stretcher of the caller's choosing.
    pub fn with(
        mut stretcher: Box<dyn Stretcher>,
        tempo: f32,
        pitch_scale: f32,
    ) -> Result<Self, OfflineError> {
        if !tempo.is_finite() || !(MIN_RATIO..=MAX_RATIO).contains(&tempo) {
            return Err(OfflineError::TempoOutOfRange);
        }
        if !pitch_scale.is_finite() || !(0.5..=2.0).contains(&pitch_scale) {
            return Err(OfflineError::PitchOutOfRange);
        }
        #[allow(clippy::float_cmp, reason = "exactly 1.0 is what 'no pitch change' is")]
        let shifting = pitch_scale != 1.0;
        if shifting && !stretcher.shifts_pitch() {
            return Err(OfflineError::PitchUnavailable);
        }
        stretcher.reset();
        stretcher.set_ratio(tempo);
        if shifting {
            stretcher.set_pitch_scale(pitch_scale);
        }
        // Read after the ratio and pitch are set: both answers depend on them.
        let to_pad = stretcher.start_pad();
        let to_drop = stretcher.start_delay();
        Ok(Self {
            tempo: f64::from(stretcher.ratio()),
            stretcher,
            fed: 0,
            emitted: 0,
            to_drop,
            to_pad,
            block: vec![0.0; PULL * CHANNELS],
        })
    }

    /// Output frames the input fed so far amounts to: `round(fed / tempo)`.
    #[must_use]
    pub fn expected_frames(&self) -> u64 {
        expected(self.fed, self.tempo)
    }

    /// Output frames handed over so far.
    #[must_use]
    pub fn emitted_frames(&self) -> u64 {
        self.emitted
    }

    /// Feeds interleaved stereo and appends whatever output is ready to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) -> Result<(), OfflineError> {
        if self.to_pad > 0 {
            let silence = vec![0.0; self.to_pad * CHANNELS];
            self.to_pad = 0;
            self.push(&silence, 0, out)?;
        }
        let frames = (input.len() / CHANNELS) as u64;
        self.push(input, frames, out)
    }

    /// Flushes the tail: appends what is left so that everything appended
    /// since the start is exactly [`Offline::expected_frames`] long.
    pub fn finish(&mut self, out: &mut Vec<f32>) -> Result<(), OfflineError> {
        let want = self.expected_frames();
        let silence = vec![0.0; PULL * CHANNELS];
        // The tail still inside the stretcher is its latency and no more, so
        // this much silence is far more than it takes to push it out. Past
        // it the backend has come up short and the rest is silence.
        let mut budget = self.to_drop + self.block.len() * 16;
        while self.emitted < want && budget > 0 {
            budget = budget.saturating_sub(PULL);
            self.push(&silence, 0, out)?;
        }
        if self.emitted < want {
            let short = usize::try_from(want - self.emitted).unwrap_or(0);
            out.resize(out.len() + short * CHANNELS, 0.0);
            self.emitted = want;
        }
        Ok(())
    }

    /// Feeds `input`, of which `counted` frames are the file's own, draining
    /// as it goes.
    fn push(
        &mut self,
        input: &[f32],
        counted: u64,
        out: &mut Vec<f32>,
    ) -> Result<(), OfflineError> {
        // Counted up front so the cap in `drain` sees the whole block.
        self.fed += counted;
        let mut rest = input;
        let mut idle = 0;
        while !rest.is_empty() {
            let wanted = self.stretcher.wants().min(rest.len() / CHANNELS);
            let took = if wanted == 0 {
                0
            } else {
                self.stretcher.feed(&rest[..wanted * CHANNELS])
            };
            rest = &rest[took * CHANNELS..];
            let gave = self.drain(out);
            if took == 0 && gave == 0 {
                idle += 1;
                if idle > PATIENCE {
                    return Err(OfflineError::Stalled);
                }
            } else {
                idle = 0;
            }
        }
        Ok(())
    }

    /// Pulls everything ready, drops the start delay, and appends the rest up
    /// to the expected length. Returns the frames pulled from the stretcher.
    fn drain(&mut self, out: &mut Vec<f32>) -> usize {
        let mut pulled = 0;
        loop {
            let got = self.stretcher.pull(&mut self.block);
            if got == 0 {
                return pulled;
            }
            pulled += got;
            let drop = self.to_drop.min(got);
            self.to_drop -= drop;
            let room = usize::try_from(expected(self.fed, self.tempo) - self.emitted)
                .unwrap_or(usize::MAX);
            let keep = (got - drop).min(room);
            out.extend_from_slice(&self.block[drop * CHANNELS..(drop + keep) * CHANNELS]);
            self.emitted += keep as u64;
        }
    }
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "frame counts of any real file are far inside what an f64 holds exactly"
)]
fn expected(fed: u64, tempo: f64) -> u64 {
    (fed as f64 / tempo).round() as u64
}
