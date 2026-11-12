//! The offline renderer behind Mutants.
//!
//! One function, [`render`]: a [`Source`] of decoded audio, the [`Edits`]
//! (a key shift and a tempo), a [`Destination`], and a callback that hears
//! how far along it is and may cancel. See the README beside this crate.

mod encode;
#[doc(hidden)]
pub mod publish;
mod source;
mod stems;

use std::path::{Path, PathBuf};

pub use source::{MemorySource, Source, TrackSource};
pub use stems::{Kept, MutantSource, StemSum, Stems};

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

/// The file format written. Always 16-bit stereo at the source's rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Format {
    /// PCM WAV.
    Wav,
    /// PCM AIFF (big-endian).
    Aiff,
    /// MPEG-1 Layer III at 320 kbit/s constant, with a LAME Info frame: the
    /// export's own MP3 settings. Only when asked for by name: never a
    /// default. Lossy, and not exact in length: the encoder's delay and
    /// padding surround the frames (both reported in [`Rendered`]), and
    /// where a deck places the audio, and so a beat grid, is not known
    /// without one (see the README).
    Mp3,
}

impl Format {
    /// The default format for a Mutant of `parent`: always lossless, so the
    /// render is exact to the frame. AIFF for an AIFF parent, WAV for every
    /// other, an MP3 parent included (MP3 output is [`Format::Mp3`], asked
    /// for by name). By the parent file's extension, as the Library and the
    /// export name formats.
    #[must_use]
    pub fn for_parent(parent: &Path) -> Self {
        let extension = parent
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match extension.as_str() {
            "aif" | "aiff" => Self::Aiff,
            _ => Self::Wav,
        }
    }

    /// Whether a file in this format decodes to exactly the frames rendered,
    /// sample for sample: WAV and AIFF.
    #[must_use]
    pub fn is_lossless(self) -> bool {
        matches!(self, Self::Wav | Self::Aiff)
    }

    /// The extension a file in this format is given.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Aiff => "aiff",
            Self::Mp3 => "mp3",
        }
    }
}

/// Where the render goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    pub path: PathBuf,
    pub format: Format,
}

impl Destination {
    pub fn new(path: impl Into<PathBuf>, format: Format) -> Self {
        Self {
            path: path.into(),
            format,
        }
    }

    pub fn wav(path: impl Into<PathBuf>) -> Self {
        Self::new(path, Format::Wav)
    }

    pub fn aiff(path: impl Into<PathBuf>) -> Self {
        Self::new(path, Format::Aiff)
    }

    pub fn mp3(path: impl Into<PathBuf>) -> Self {
        Self::new(path, Format::Mp3)
    }

    /// `name` plus the extension of [`Format::for_parent`], in that format:
    /// `/work/m3` for an AIFF parent is `/work/m3.aiff`.
    #[must_use]
    pub fn for_parent(parent: &Path, name: &Path) -> Self {
        let format = Format::for_parent(parent);
        let mut path = name.as_os_str().to_owned();
        path.push(".");
        path.push(format.extension());
        Self::new(path, format)
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
    /// The frames the render made, every one of them in the file: for no
    /// edits the source's length, for a tempo `round(source / tempo)`. A WAV
    /// or AIFF decodes to exactly this many.
    pub frames: u64,
    pub sample_rate: u32,
    /// Frames the encoder put ahead of the first rendered frame: 0 for WAV
    /// and AIFF, LAME's delay (576) for MP3, as its LAME tag records it.
    pub encoder_delay: u64,
    /// Frames the encoder put after the last one to fill its last block: 0
    /// for WAV and AIFF; for MP3 as the LAME tag records it.
    pub padding: u64,
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
    #[error("this build cannot change a tempo and hold the pitch (it has no Rubber Band)")]
    TempoUnavailable,
    #[error("this filesystem cannot put a file in place without replacing: {0}")]
    NoReplaceUnsupported(PathBuf),
    #[error("the source holds no audio")]
    Empty,
    #[error("a Stem edit needs the track's Stems, and there are none")]
    StemsMissing,
    #[error("no Stem is kept, so there is nothing to render")]
    NothingKept,
    #[error("the Stem {stem} is {found} frames long, not its parent's {expected}")]
    StemLength {
        stem: PathBuf,
        expected: u64,
        found: u64,
    },
    #[error("the Stem {stem} is at {found} Hz, not {expected} Hz like the others")]
    StemRate {
        stem: PathBuf,
        expected: u32,
        found: u32,
    },
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
    render_on(&publish::Real, source, edits, destination, progress)
}

/// [`render`] over a filesystem of the caller's choosing: the seam the tests
/// use to stand in for a filesystem that cannot link or rename without
/// replacing, and to put a competing file in the way at an exact moment.
#[doc(hidden)]
pub fn render_on(
    files: &dyn publish::Filesystem,
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

    let (part, file) = publish::Part::create(files, &destination.path)?;
    let mut encoder = encode::encoder(destination.format, file, sample_rate)?;

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
    let coding = encoder.finish()?;
    part.commit(&destination.path)?;
    Ok(Rendered {
        frames: written,
        sample_rate,
        encoder_delay: coding.delay,
        padding: coding.padding,
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
        Err(rbl_deck::OfflineError::TempoUnavailable) => Err(RenderError::TempoUnavailable),
        Err(rbl_deck::OfflineError::TempoOutOfRange) => Err(RenderError::TempoOutOfRange),
        Err(rbl_deck::OfflineError::PitchOutOfRange) => Err(RenderError::KeyShiftOutOfRange),
        Err(other) => Err(RenderError::Stretch(other)),
    }
}
