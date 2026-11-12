//! Where the audio to render comes from.

use std::path::Path;

use rbl_audio::MonoStream;

use crate::RenderError;

/// Decoded audio, handed over a block at a time.
///
/// The renderer's only view of its input, so that a whole track (here), a
/// sum of kept Stems (ticket #82) or generated samples all render the same
/// way. Always interleaved stereo `f32`; a mono file is its one channel
/// twice.
pub trait Source {
    /// Frames per second of what [`Source::next_block`] hands out.
    fn sample_rate(&self) -> u32;

    /// How many frames there will be in all, when known. Only used to say
    /// how far along a render is; a wrong or missing answer changes nothing
    /// in the file.
    fn frames_hint(&self) -> Option<u64>;

    /// The next block of interleaved stereo, of any length; `None` at the
    /// end.
    fn next_block(&mut self) -> Result<Option<&[f32]>, RenderError>;
}

/// A whole track, decoded from its file through `rbl-audio`.
///
/// The file is opened for reading and nothing else. It is decoded on
/// rekordbox's timeline (an MP3's lead is silence or dropped frames, as it is
/// for analysis), so a grid made for the file fits the render.
///
/// The frames are the 16-bit stereo ones `rbl-audio` hands out, which is the
/// depth this crate writes today.
pub struct TrackSource {
    stream: MonoStream,
    block: Vec<f32>,
}

impl TrackSource {
    pub fn open(path: &Path) -> Result<Self, RenderError> {
        Ok(Self {
            stream: MonoStream::open(path)?,
            block: Vec::new(),
        })
    }
}

impl Source for TrackSource {
    fn sample_rate(&self) -> u32 {
        self.stream.sample_rate()
    }

    fn frames_hint(&self) -> Option<u64> {
        self.stream.expected_frames()
    }

    fn next_block(&mut self) -> Result<Option<&[f32]>, RenderError> {
        let Some((_, stereo)) = self.stream.next_frames() else {
            return Ok(None);
        };
        self.block.clear();
        self.block.extend(
            stereo
                .iter()
                .flat_map(|frame| [from_i16(frame[0]), from_i16(frame[1])]),
        );
        Ok(Some(&self.block))
    }
}

/// The inverse of the encoder's rounding, so a 16-bit sample that is not
/// edited comes back out as itself.
fn from_i16(sample: i16) -> f32 {
    f32::from(sample) / 32_768.0
}

/// Samples already in memory: generated audio, or anything a caller mixed
/// itself.
pub struct MemorySource {
    samples: Vec<f32>,
    sample_rate: u32,
    at: usize,
}

impl MemorySource {
    /// `samples` is interleaved stereo; a trailing odd sample is ignored.
    #[must_use]
    pub fn new(mut samples: Vec<f32>, sample_rate: u32) -> Self {
        samples.truncate(samples.len() / 2 * 2);
        Self {
            samples,
            sample_rate,
            at: 0,
        }
    }
}

impl Source for MemorySource {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn frames_hint(&self) -> Option<u64> {
        Some((self.samples.len() / 2) as u64)
    }

    fn next_block(&mut self) -> Result<Option<&[f32]>, RenderError> {
        /// Samples per block: 4096 frames.
        const BLOCK: usize = 8192;
        let end = (self.at + BLOCK).min(self.samples.len());
        if end == self.at {
            return Ok(None);
        }
        let block = &self.samples[self.at..end];
        self.at = end;
        Ok(Some(block))
    }
}
