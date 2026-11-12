//! Audio made from a track's Stems: the kept ones summed.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use rbl_audio::MonoStream;

use crate::source::from_i16;
use crate::{RenderError, Source, TrackSource};

/// A track's four Stems, as the Sidecar holds them: one file per Stem, each
/// exactly the parent's length.
///
/// `frames` is that length as the Stems were made from the parent (the
/// Sidecar's `samples`); every file must hold exactly that many frames or
/// the render is refused. The four are the Sidecar's roles; `other` is
/// the melody.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stems {
    pub frames: u64,
    pub vocals: PathBuf,
    pub other: PathBuf,
    pub bass: PathBuf,
    pub drums: PathBuf,
}

/// Which Stems a render keeps. All four is no Stem edit at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "one independent choice per Stem, named as the Stems are"
)]
pub struct Kept {
    pub vocals: bool,
    pub other: bool,
    pub bass: bool,
    pub drums: bool,
}

impl Kept {
    /// Every Stem: the parent as it is.
    pub const ALL: Self = Self {
        vocals: true,
        other: true,
        bass: true,
        drums: true,
    };

    #[must_use]
    pub fn is_all(self) -> bool {
        self == Self::ALL
    }

    #[must_use]
    pub fn is_none(self) -> bool {
        !(self.vocals || self.other || self.bass || self.drums)
    }
}

impl Default for Kept {
    fn default() -> Self {
        Self::ALL
    }
}

impl Stems {
    /// The files of the kept Stems, in the order vocals, other, bass, drums.
    fn kept(&self, kept: Kept) -> Vec<&Path> {
        [
            (kept.vocals, &self.vocals),
            (kept.other, &self.other),
            (kept.bass, &self.bass),
            (kept.drums, &self.drums),
        ]
        .into_iter()
        .filter(|(keep, _)| *keep)
        .map(|(_, path)| path.as_path())
        .collect()
    }
}

/// What a Mutant's audio is made from: the parent track itself when every
/// Stem is kept, otherwise the sum of the kept Stems.
pub enum MutantSource {
    Track(TrackSource),
    Stems(StemSum),
}

impl MutantSource {
    /// The audio for a Mutant that keeps `kept` of its parent's Stems.
    ///
    /// Keeping all four is no Stem edit: the parent's own file is decoded and
    /// the Stems are not opened (nor needed). Otherwise the kept Stems are
    /// summed; with no Stems that is [`RenderError::StemsMissing`], and
    /// keeping none is [`RenderError::NothingKept`].
    pub fn open(parent: &Path, kept: Kept, stems: Option<&Stems>) -> Result<Self, RenderError> {
        if kept.is_all() {
            return Ok(Self::Track(TrackSource::open(parent)?));
        }
        let stems = stems.ok_or(RenderError::StemsMissing)?;
        Ok(Self::Stems(StemSum::open(stems, kept)?))
    }
}

impl Source for MutantSource {
    fn sample_rate(&self) -> u32 {
        match self {
            Self::Track(track) => track.sample_rate(),
            Self::Stems(stems) => stems.sample_rate(),
        }
    }

    fn frames_hint(&self) -> Option<u64> {
        match self {
            Self::Track(track) => track.frames_hint(),
            Self::Stems(stems) => stems.frames_hint(),
        }
    }

    fn next_block(&mut self) -> Result<Option<&[f32]>, RenderError> {
        match self {
            Self::Track(track) => track.next_block(),
            Self::Stems(stems) => stems.next_block(),
        }
    }
}

/// One kept Stem being read.
struct Reader {
    path: PathBuf,
    stream: MonoStream,
    /// Frames decoded and not yet summed.
    pending: VecDeque<[i16; 2]>,
    /// Frames decoded in all.
    decoded: u64,
    ended: bool,
}

/// The kept Stems of a set, summed sample for sample.
///
/// The sum is exact: each Stem's 16-bit samples are added as integers, so
/// one kept Stem renders unedited as itself and two as their exact sum. A
/// sum is not scaled: the Stems were separated from one mix, so together
/// they are about as loud as it, and any part of them less so; a peak past
/// full scale is clipped where the file is written, as for any render.
///
/// Every Stem must be exactly [`Stems::frames`] long and all at one rate:
/// a Stem that says otherwise in its header is refused when opened, and one
/// that turns out otherwise while it is read stops the render with
/// [`RenderError::StemLength`], so nothing is written.
pub struct StemSum {
    readers: Vec<Reader>,
    frames: u64,
    sample_rate: u32,
    block: Vec<f32>,
}

/// Frames summed per block at most.
const BLOCK: usize = 4096;

impl StemSum {
    pub fn open(stems: &Stems, kept: Kept) -> Result<Self, RenderError> {
        let paths = stems.kept(kept);
        if paths.is_empty() {
            return Err(RenderError::NothingKept);
        }
        let mut readers = Vec::with_capacity(paths.len());
        let mut sample_rate = None;
        for path in paths {
            let stream = MonoStream::open(path)?;
            if let Some(found) = stream.expected_frames().filter(|&n| n != stems.frames) {
                return Err(RenderError::StemLength {
                    stem: path.to_path_buf(),
                    expected: stems.frames,
                    found,
                });
            }
            let rate = stream.sample_rate();
            let expected = *sample_rate.get_or_insert(rate);
            if rate != expected {
                return Err(RenderError::StemRate {
                    stem: path.to_path_buf(),
                    expected,
                    found: rate,
                });
            }
            readers.push(Reader {
                path: path.to_path_buf(),
                stream,
                pending: VecDeque::new(),
                decoded: 0,
                ended: false,
            });
        }
        Ok(Self {
            readers,
            frames: stems.frames,
            sample_rate: sample_rate.unwrap_or(44_100),
            block: Vec::new(),
        })
    }

    /// Reads from `reader` until it has something pending or has ended, and
    /// holds it to the set's length.
    fn fill(reader: &mut Reader, frames: u64) -> Result<(), RenderError> {
        while reader.pending.is_empty() && !reader.ended {
            match reader.stream.try_next_frames()? {
                Some((_, stereo)) => {
                    reader.decoded += stereo.len() as u64;
                    reader.pending.extend(stereo.iter().copied());
                }
                None => reader.ended = true,
            }
            if reader.decoded > frames || (reader.ended && reader.decoded != frames) {
                return Err(RenderError::StemLength {
                    stem: reader.path.clone(),
                    expected: frames,
                    found: reader.decoded,
                });
            }
        }
        Ok(())
    }
}

impl Source for StemSum {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn frames_hint(&self) -> Option<u64> {
        Some(self.frames)
    }

    fn next_block(&mut self) -> Result<Option<&[f32]>, RenderError> {
        for reader in &mut self.readers {
            Self::fill(reader, self.frames)?;
        }
        // Every Stem is held to the same length, so they run out together:
        // the shortest pending run is what all of them can give.
        let n = self
            .readers
            .iter()
            .map(|reader| reader.pending.len())
            .min()
            .unwrap_or(0)
            .min(BLOCK);
        if n == 0 {
            return Ok(None);
        }
        self.block.clear();
        self.block.resize(n * 2, 0.0);
        let mut sums = vec![[0_i32; 2]; n];
        for reader in &mut self.readers {
            for (sum, frame) in sums.iter_mut().zip(reader.pending.drain(..n)) {
                sum[0] += i32::from(frame[0]);
                sum[1] += i32::from(frame[1]);
            }
        }
        for (out, sum) in self.block.chunks_exact_mut(2).zip(&sums) {
            out[0] = from_i32(sum[0]);
            out[1] = from_i32(sum[1]);
        }
        Ok(Some(&self.block))
    }
}

/// A sum of 16-bit samples on the same scale as [`from_i16`]: exact, since
/// four of them fit an `f32`'s 24 bits.
#[allow(
    clippy::cast_precision_loss,
    reason = "at most four 16-bit samples: 18 bits"
)]
fn from_i32(sum: i32) -> f32 {
    sum as f32 * from_i16(1)
}
