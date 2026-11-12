//! The ramp at either end of a sound.
//!
//! **Transport moves no longer use it** (wbx-45.16). The DJ's decision of
//! 2026-11-12 was "I don't want a short fade in and out, I want an immediate
//! start and stop", so play, pause, cue, a seek and a beat jump run the deck's
//! ramp over `TRANSPORT_FADE_FRAMES`, which is zero: the first frame of the
//! track is heard at its own level and the frame after a stop is silence. A
//! drag still ramps over `FADE_FRAMES`. What follows explains why the fade was
//! there, which is what to weigh if a short one is ever wanted back.
//!
//! Audio does not start and stop at zero of its own accord. Press play and the
//! first sample handed to the device is wherever the waveform happened to be —
//! on a kick that is most of full scale, and a step from silence to full scale
//! is a click. Pause, seek, cue, jump a beat: each one is the same step, taken
//! in the other direction or in both. The fix is the same every time, so it
//! lives here rather than in each of them: a gain that walks to where it is
//! asked for instead of jumping, so that audio is only ever started or stopped
//! where the gain is already zero. Nothing in the deck cuts — not a pause, not
//! a seek, not the end of a file, not even the decode thread failing to keep
//! up, which holds its last frame and fades that instead.

/// Frames a start or a stop is faded over: about two milliseconds at 44.1 kHz.
///
/// Short enough that a stop still reads as a stop — two milliseconds is under
/// a tenth of the shortest thing anybody hears as a note — and long enough to
/// have no edge in it. Measured in frames rather than seconds because the
/// callback counts frames, and a device at 48 kHz fading over the same count
/// is 1.8 ms, which is the same thing.
pub const FADE_FRAMES: u16 = 88;

/// Frames a deck's start and stop are faded over: none. Play, pause, cue, a
/// seek, a beat jump and the end of the track are immediate, by the DJ's
/// decision (see the top of this file). One millisecond, which is what
/// hardware players do, would be 44.
pub const TRANSPORT_FADE_FRAMES: u16 = 0;

/// A gain that moves towards what it is asked for, one step a frame.
///
/// Not a filter and not a curve: a straight line, because it is short enough
/// that the shape of it cannot be heard and a line is exact about reaching
/// zero. Reaching zero is the whole point — audio is only ever started or cut
/// where this reads zero.
///
/// Counted in frames rather than accumulated as a float. Eighty-eight
/// additions of one eighty-eighth land six ten-millionths short of zero, and
/// six ten-millionths of a kick drum multiplied back in at the end of a fade
/// is the click, quieter. A count divides exactly.
#[derive(Debug, Clone, Copy)]
pub struct Ramp {
    /// Frames into the fade, from 0 (silent) to `len` (open).
    at: u16,
    /// Steps from silent to open: at least one. One is no fade at all: the
    /// first step up is full level and the first step down is silence.
    len: u16,
}

impl Default for Ramp {
    fn default() -> Self {
        Self::silent()
    }
}

impl Ramp {
    /// A ramp at zero, which is where everything that is not sounding sits,
    /// fading over `FADE_FRAMES`.
    pub const fn silent() -> Self {
        Self::silent_over(FADE_FRAMES)
    }

    /// A ramp at zero that fades over `frames`; zero is no fade.
    pub const fn silent_over(frames: u16) -> Self {
        Self { at: 0, len: if frames == 0 { 1 } else { frames } }
    }

    /// Frames from the last of the sound to silence, after which nothing is
    /// heard: 0 for a ramp that does not fade.
    pub fn tail(self) -> usize {
        usize::from(self.len - 1)
    }

    /// Whether the next step down reaches silence, so the frame it would be
    /// applied to would not be heard at all and need not be taken.
    pub fn closing_now(self) -> bool {
        self.at <= 1
    }

    /// Exactly 0 while silent and exactly 1 while open, by construction.
    pub fn gain(self) -> f32 {
        f32::from(self.at) / f32::from(self.len)
    }

    /// Whether the sound has gone: nothing is being heard through this.
    pub fn silent_now(self) -> bool {
        self.at == 0
    }

    /// Whether the sound is all the way up, so nothing is being faded.
    #[cfg_attr(not(test), allow(dead_code, reason = "read by the fade tests"))]
    pub fn open(self) -> bool {
        self.at >= self.len
    }

    /// Moves one frame, up if `open` and down if not, and returns the gain to
    /// multiply that frame by.
    pub fn step(&mut self, open: bool) -> f32 {
        if open {
            self.at = self.at.saturating_add(1).min(self.len);
        } else {
            self.at = self.at.saturating_sub(1);
        }
        self.gain()
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "the ramp reaches exactly zero and one, and that is the point")]
mod tests {
    use super::*;

    /// The gain a frame is multiplied by, `frames` into a fade in.
    fn after(frames: usize, open: bool) -> Ramp {
        let mut ramp = Ramp::silent();
        for _ in 0..frames {
            ramp.step(open);
        }
        ramp
    }

    #[test]
    fn a_fade_in_starts_at_zero_and_reaches_exactly_full() {
        let mut ramp = Ramp::silent();
        assert_eq!(ramp.gain(), 0.0);
        assert_eq!(ramp.step(true), 1.0 / f32::from(FADE_FRAMES));
        let open = after(usize::from(FADE_FRAMES), true);
        assert!(open.open());
        assert_eq!(open.gain(), 1.0);
    }

    #[test]
    fn a_fade_out_ends_at_exactly_zero() {
        // Not "close to zero": the last sample of a stop is multiplied by this,
        // and anything left over is the click, quieter.
        let mut ramp = after(200, true);
        for _ in 0..FADE_FRAMES {
            ramp.step(false);
        }
        assert_eq!(ramp.gain(), 0.0);
        assert!(ramp.silent_now());
    }

    #[test]
    fn it_takes_exactly_two_milliseconds_and_a_step_at_a_time() {
        let mut ramp = Ramp::silent();
        let mut last = 0.0;
        let mut frames = 0_u16;
        while !ramp.open() {
            let gain = ramp.step(true);
            assert!(gain - last <= 1.0 / f32::from(FADE_FRAMES) + 1e-6, "stepped by {}", gain - last);
            last = gain;
            frames += 1;
            assert!(frames <= FADE_FRAMES, "still fading after {frames} frames");
        }
        assert_eq!(frames, FADE_FRAMES);
    }

    #[test]
    #[allow(clippy::float_cmp, reason = "no fade means exactly full and exactly silent")]
    fn a_ramp_over_no_frames_opens_and_closes_in_one_step() {
        let mut ramp = Ramp::silent_over(TRANSPORT_FADE_FRAMES);
        assert_eq!(ramp.step(true), 1.0, "the first frame is at full level");
        assert!(ramp.closing_now());
        assert_eq!(ramp.step(false), 0.0, "the first frame after a stop is silence");
        assert_eq!(ramp.tail(), 0);
    }

    #[test]
    fn a_ramp_held_down_stays_at_zero_rather_than_going_under() {
        let mut ramp = Ramp::silent();
        for _ in 0..1_000 {
            assert_eq!(ramp.step(false), 0.0);
        }
    }
}
