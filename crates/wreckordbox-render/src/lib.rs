//! The offline renderer behind Mutants.
//!
//! One function, [`render`]: a [`Source`] of decoded audio, the [`Edits`]
//! (a key shift and a tempo), a [`Destination`], and a callback that hears
//! how far along it is and may cancel. See the README beside this crate.

mod encode;
mod source;

use std::path::{Path, PathBuf};

pub use source::{MemorySource, Source, TrackSource};

/// What to do to the audio. The default is nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Edits {
    /// Semitones up (positive) or down, with the tempo unchanged.
    pub key_shift: i8,
    /// Speed, with the pitch unchanged: 1.0 is the source's own, 1.06 is six
    /// percent fast and so a file that much shorter.
    pub tempo: f64,
}

impl Default for Edits {
    fn default() -> Self {
        Self {
            key_shift: 0,
            tempo: 1.0,
        }
    }
}

/// The file format written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Format {
    /// 16-bit PCM stereo at the source's sample rate.
    Wav,
}

/// Where the render goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    pub path: PathBuf,
    pub format: Format,
}

impl Destination {
    pub fn wav(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            format: Format::Wav,
        }
    }
}

/// How far along a render is, in frames of the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub frames_done: u64,
    /// The source's length when it knows it.
    pub frames_total: Option<u64>,
}

/// What the progress callback answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proceed {
    Continue,
    /// Stop, and leave nothing at the destination.
    Cancel,
}

/// What was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rendered {
    pub frames: u64,
    pub sample_rate: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("the render was cancelled")]
    Cancelled,
    #[error("something is already at the destination: {0}")]
    DestinationExists(PathBuf),
    #[error("the key shift is outside what the stretcher holds")]
    KeyShiftOutOfRange,
    #[error("this build cannot shift a key (it has no Rubber Band)")]
    KeyShiftUnavailable,
    #[error("the tempo is outside what the stretcher holds")]
    TempoOutOfRange,
    #[error("the source holds no audio")]
    Empty,
    #[error("the stretcher failed: {0}")]
    Stretch(rbl_deck::OfflineError),
    #[error(transparent)]
    Source(#[from] rbl_audio::AudioError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// The widest key shift, in semitones either way: an octave, which is the
/// deck's own limit. What a Mutant may ask for is narrower and is the
/// caller's rule.
pub const MAX_KEY_SHIFT: i8 = 12;

/// The slowest and fastest tempo: the deck's own limits.
pub const MIN_TEMPO: f64 = rbl_deck::MIN_RATIO as f64;
pub const MAX_TEMPO: f64 = rbl_deck::MAX_RATIO as f64;

/// Renders `source` with `edits` to `destination`.
///
/// `progress` is called about twice for every second of source audio and
/// once more when the source is exhausted; answering [`Proceed::Cancel`]
/// stops the render with [`RenderError::Cancelled`].
///
/// The destination either holds the complete file or does not exist: the
/// audio is written beside it under a temporary name and moved into place
/// last, and on any error or a cancel the temporary file is removed. A
/// destination that already exists is refused, so no existing file, a source
/// least of all, is ever replaced.
///
/// With no edits the source's samples are written as they are. Otherwise
/// they go through the deck's stretcher, whole-file ([`rbl_deck::Offline`]).
pub fn render(
    source: &mut dyn Source,
    edits: Edits,
    destination: &Destination,
    progress: &mut dyn FnMut(Progress) -> Proceed,
) -> Result<Rendered, RenderError> {
    if !(-MAX_KEY_SHIFT..=MAX_KEY_SHIFT).contains(&edits.key_shift) {
        return Err(RenderError::KeyShiftOutOfRange);
    }
    if !edits.tempo.is_finite() || !(MIN_TEMPO..=MAX_TEMPO).contains(&edits.tempo) {
        return Err(RenderError::TempoOutOfRange);
    }
    if destination.path.try_exists()? {
        return Err(RenderError::DestinationExists(destination.path.clone()));
    }
    let sample_rate = source.sample_rate();
    let mut stretch = stretcher(sample_rate, edits)?;

    let part = Part::beside(&destination.path)?;
    let mut encoder = encode::encoder(destination.format, part.create()?, sample_rate)?;

    let total = source.frames_hint();
    // About twice a second of source audio: often enough to draw, and a
    // cancel is heard within milliseconds of real time.
    let step = u64::from(sample_rate / 2).max(1);
    let mut done = 0_u64;
    let mut reported = 0_u64;
    let mut written = 0_u64;
    let mut stretched: Vec<f32> = Vec::new();

    while let Some(block) = source.next_block()? {
        done += (block.len() / 2) as u64;
        if let Some(stretch) = stretch.as_mut() {
            stretched.clear();
            stretch
                .process(block, &mut stretched)
                .map_err(RenderError::Stretch)?;
            encoder.write(&stretched)?;
            written += (stretched.len() / 2) as u64;
        } else {
            encoder.write(block)?;
            written += (block.len() / 2) as u64;
        }
        if done - reported >= step {
            reported = done;
            if progress(Progress {
                frames_done: done,
                frames_total: total,
            }) == Proceed::Cancel
            {
                return Err(RenderError::Cancelled);
            }
        }
    }
    if done == 0 {
        return Err(RenderError::Empty);
    }
    if let Some(stretch) = stretch.as_mut() {
        stretched.clear();
        stretch
            .finish(&mut stretched)
            .map_err(RenderError::Stretch)?;
        encoder.write(&stretched)?;
        written += (stretched.len() / 2) as u64;
    }
    // The last word, with the length the source turned out to have: a hint
    // can be wrong, and a caller drawing a bar wants it to reach the end.
    if done != reported
        && progress(Progress {
            frames_done: done,
            frames_total: Some(done),
        }) == Proceed::Cancel
    {
        return Err(RenderError::Cancelled);
    }
    encoder.finish()?;
    part.commit(&destination.path)?;
    Ok(Rendered {
        frames: written,
        sample_rate,
    })
}

/// The stretcher for these edits, or none when there is nothing to do: a
/// deck plays an unedited track straight too.
fn stretcher(sample_rate: u32, edits: Edits) -> Result<Option<rbl_deck::Offline>, RenderError> {
    #[allow(clippy::float_cmp, reason = "exactly 1.0 is what 'no tempo change' is")]
    if edits.key_shift == 0 && edits.tempo == 1.0 {
        return Ok(None);
    }
    let pitch = 2.0_f64.powf(f64::from(edits.key_shift) / 12.0);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the stretcher's own precision"
    )]
    let (tempo, pitch) = (
        edits.tempo as f32,
        if edits.key_shift == 0 {
            1.0
        } else {
            pitch as f32
        },
    );
    match rbl_deck::Offline::new(sample_rate, tempo, pitch) {
        Ok(stretch) => Ok(Some(stretch)),
        Err(rbl_deck::OfflineError::PitchUnavailable) => Err(RenderError::KeyShiftUnavailable),
        Err(rbl_deck::OfflineError::TempoOutOfRange) => Err(RenderError::TempoOutOfRange),
        Err(rbl_deck::OfflineError::PitchOutOfRange) => Err(RenderError::KeyShiftOutOfRange),
        Err(other) => Err(RenderError::Stretch(other)),
    }
}

/// The file being written, under a name that is not the destination's.
/// Dropped without [`Part::commit`], it is removed.
struct Part {
    path: PathBuf,
    committed: bool,
}

impl Part {
    /// In the destination's own folder, so moving it into place is a rename
    /// and never a copy.
    fn beside(destination: &Path) -> Result<Self, RenderError> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let name = destination.file_name().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "the destination has no file name",
            )
        })?;
        let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut part = std::ffi::OsString::from(".");
        part.push(name);
        part.push(format!(".{}-{unique}.part", std::process::id()));
        Ok(Self {
            path: destination.with_file_name(part),
            committed: false,
        })
    }

    fn create(&self) -> std::io::Result<std::fs::File> {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.path)
    }

    fn commit(mut self, destination: &Path) -> Result<(), RenderError> {
        // A render takes a while, and a rename replaces whatever arrived
        // meanwhile. A hard link does not: it fails if the name is taken, in
        // one step, and the temporary name is then dropped as usual.
        match std::fs::hard_link(&self.path, destination) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(RenderError::DestinationExists(destination.to_path_buf()));
            }
            // A filesystem without hard links (FAT, exFAT): ask again and
            // rename, which leaves a moment between the two.
            Err(_) => {}
        }
        if destination.try_exists()? {
            return Err(RenderError::DestinationExists(destination.to_path_buf()));
        }
        std::fs::rename(&self.path, destination)?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for Part {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
