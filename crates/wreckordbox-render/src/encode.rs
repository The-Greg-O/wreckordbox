//! Turning rendered samples into a file.

use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};

use crate::Format;

/// One output file being written. The seam other formats (ticket #82) come
/// in behind: `render` only ever sees this.
pub(crate) trait Encoder {
    /// Appends interleaved stereo.
    fn write(&mut self, samples: &[f32]) -> io::Result<()>;

    /// Completes the file and flushes it to disk.
    fn finish(self: Box<Self>) -> io::Result<()>;
}

pub(crate) fn encoder(
    format: Format,
    file: File,
    sample_rate: u32,
) -> io::Result<Box<dyn Encoder>> {
    match format {
        Format::Wav => Ok(Box::new(Wav::new(file, sample_rate)?)),
        Format::Aiff => Ok(Box::new(Aiff::new(file, sample_rate)?)),
        Format::Mp3 => Ok(Box::new(Mp3::new(file, sample_rate)?)),
    }
}

const CHANNELS: u16 = 2;
const BYTES_PER_SAMPLE: u16 = 2;
const HEADER: u32 = 44;

/// 16-bit PCM stereo WAV, written as it arrives; the two lengths in the
/// header are filled in at the end.
struct Wav {
    out: BufWriter<File>,
    /// Bytes of sample data written.
    data: u64,
}

impl Wav {
    fn new(file: File, sample_rate: u32) -> io::Result<Self> {
        let mut out = BufWriter::new(file);
        let block_align = CHANNELS * BYTES_PER_SAMPLE;
        out.write_all(b"RIFF")?;
        out.write_all(&0_u32.to_le_bytes())?; // RIFF size, patched in `finish`
        out.write_all(b"WAVEfmt ")?;
        out.write_all(&16_u32.to_le_bytes())?;
        out.write_all(&1_u16.to_le_bytes())?; // PCM
        out.write_all(&CHANNELS.to_le_bytes())?;
        out.write_all(&sample_rate.to_le_bytes())?;
        out.write_all(&(sample_rate * u32::from(block_align)).to_le_bytes())?;
        out.write_all(&block_align.to_le_bytes())?;
        out.write_all(&(BYTES_PER_SAMPLE * 8).to_le_bytes())?;
        out.write_all(b"data")?;
        out.write_all(&0_u32.to_le_bytes())?; // data size, patched in `finish`
        Ok(Self { out, data: 0 })
    }
}

impl Encoder for Wav {
    fn write(&mut self, samples: &[f32]) -> io::Result<()> {
        for &sample in samples {
            self.out.write_all(&to_i16(sample).to_le_bytes())?;
        }
        self.data += samples.len() as u64 * u64::from(BYTES_PER_SAMPLE);
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> io::Result<()> {
        // A WAV's lengths are 32-bit: a little over six and a half hours of
        // 44.1 kHz stereo. Past that the file would lie about itself.
        let data = u32::try_from(self.data)
            .ok()
            .filter(|data| data.checked_add(HEADER - 8).is_some())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "too long for a WAV file")
            })?;
        self.out.seek(SeekFrom::Start(4))?;
        self.out.write_all(&(data + HEADER - 8).to_le_bytes())?;
        self.out.seek(SeekFrom::Start(u64::from(HEADER) - 4))?;
        self.out.write_all(&data.to_le_bytes())?;
        self.out.flush()?;
        self.out.get_ref().sync_all()
    }
}

/// Rounded to the nearest 16-bit step and clipped at full scale: a shifted
/// key can overshoot a source that was mastered to the limit.
#[allow(
    clippy::cast_possible_truncation,
    reason = "clamped to the range of an i16 first"
)]
fn to_i16(sample: f32) -> i16 {
    (sample * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16
}

/// 16-bit PCM stereo AIFF, written as it arrives; the lengths in the header
/// are filled in at the end.
struct Aiff {
    out: BufWriter<File>,
    /// Frames written.
    frames: u64,
}

/// Bytes before the first sample: `FORM` and `AIFF`, an 18-byte `COMM`
/// chunk, and the `SSND` chunk's header with its offset and block size.
const AIFF_HEADER: u32 = 54;

impl Aiff {
    fn new(file: File, sample_rate: u32) -> io::Result<Self> {
        let mut out = BufWriter::new(file);
        out.write_all(b"FORM")?;
        out.write_all(&0_u32.to_be_bytes())?; // FORM size, patched in `finish`
        out.write_all(b"AIFFCOMM")?;
        out.write_all(&18_u32.to_be_bytes())?;
        out.write_all(&CHANNELS.to_be_bytes())?;
        out.write_all(&0_u32.to_be_bytes())?; // frames, patched in `finish`
        out.write_all(&(BYTES_PER_SAMPLE * 8).to_be_bytes())?;
        out.write_all(&extended(sample_rate))?;
        out.write_all(b"SSND")?;
        out.write_all(&0_u32.to_be_bytes())?; // SSND size, patched in `finish`
        out.write_all(&0_u32.to_be_bytes())?; // offset
        out.write_all(&0_u32.to_be_bytes())?; // block size
        Ok(Self { out, frames: 0 })
    }
}

impl Encoder for Aiff {
    fn write(&mut self, samples: &[f32]) -> io::Result<()> {
        for &sample in samples {
            self.out.write_all(&to_i16(sample).to_be_bytes())?;
        }
        self.frames += (samples.len() / 2) as u64;
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> io::Result<()> {
        let too_long = || io::Error::new(io::ErrorKind::InvalidInput, "too long for an AIFF file");
        let frames = u32::try_from(self.frames).map_err(|_| too_long())?;
        let data = frames
            .checked_mul(u32::from(CHANNELS * BYTES_PER_SAMPLE))
            .filter(|data| data.checked_add(AIFF_HEADER).is_some())
            .ok_or_else(too_long)?;
        self.out.seek(SeekFrom::Start(4))?;
        self.out.write_all(&(data + AIFF_HEADER - 8).to_be_bytes())?;
        self.out.seek(SeekFrom::Start(22))?;
        self.out.write_all(&frames.to_be_bytes())?;
        self.out.seek(SeekFrom::Start(42))?;
        self.out.write_all(&(data + 8).to_be_bytes())?;
        self.out.flush()?;
        self.out.get_ref().sync_all()
    }
}

/// A sample rate as the 80-bit IEEE 754 extended float an AIFF header holds.
fn extended(rate: u32) -> [u8; 10] {
    let mut bytes = [0_u8; 10];
    if rate == 0 {
        return bytes;
    }
    let shift = 31 - rate.leading_zeros();
    #[allow(clippy::cast_possible_truncation, reason = "at most 16383 + 31")]
    let exponent = (16_383 + shift) as u16;
    bytes[..2].copy_from_slice(&exponent.to_be_bytes());
    bytes[2..].copy_from_slice(&(u64::from(rate) << (63 - shift)).to_be_bytes());
    bytes
}

/// Frames an MP3 written here puts ahead of its first sample on rekordbox's
/// timeline: the Info frame, which rekordbox's frame map plays as a frame of
/// silence (its channel mode is the audio's, see `rbl_core::mpeg`), LAME's
/// encoder delay (576) and the decoder's (529). Measured through
/// `rbl-audio` at 44.1 and 48 kHz: 2,257 frames.
fn mp3_ahead(sample_rate: u32) -> usize {
    // Samples per frame: MPEG-1 from 32 kHz up, MPEG-2 and 2.5 below.
    let frame = if sample_rate >= 32_000 { 1_152 } else { 576 };
    frame + 576 + 529
}

/// MP3 through LAME, at the export's settings (`rbl_audio::compatibility`):
/// 320 kbit/s constant, best quality, with an Info frame.
///
/// Lined up for rekordbox's timeline: a deck plays an MP3 with no gapless
/// trimming, so the Info frame (a frame of silence in rekordbox's frame map),
/// LAME's encoder delay and the decoder's delay all come before the first
/// sample given here. That many frames are dropped from the front, so the
/// render's audio sits where its source's did and the parent's grid fits.
/// What is dropped is the very start of the render, which for an MP3 parent
/// is its own lead and encoder delay.
struct Mp3 {
    out: BufWriter<File>,
    lame: mp3lame_encoder::Encoder,
    /// Frames still to drop from the front.
    skip: usize,
    left: Vec<f32>,
    right: Vec<f32>,
    bytes: Vec<u8>,
}

fn lame_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("MP3 encoder: {error}"))
}

impl Mp3 {
    fn new(file: File, sample_rate: u32) -> io::Result<Self> {
        use mp3lame_encoder::{Bitrate, Builder, Quality, VbrMode};
        let lame = Builder::new()
            .ok_or_else(|| lame_error("cannot start"))?
            .with_num_channels(2)
            .map_err(lame_error)?
            .with_sample_rate(sample_rate)
            .map_err(lame_error)?
            .with_brate(Bitrate::Kbps320)
            .map_err(lame_error)?
            .with_vbr_mode(VbrMode::Off)
            .map_err(lame_error)?
            .with_quality(Quality::Best)
            .map_err(lame_error)?
            .with_to_write_vbr_tag(true)
            .map_err(lame_error)?
            .build()
            .map_err(lame_error)?;
        Ok(Self {
            out: BufWriter::new(file),
            lame,
            skip: mp3_ahead(sample_rate),
            left: Vec::new(),
            right: Vec::new(),
            bytes: Vec::new(),
        })
    }
}

impl Encoder for Mp3 {
    fn write(&mut self, samples: &[f32]) -> io::Result<()> {
        let frames = samples.len() / 2;
        let skipped = self.skip.min(frames);
        self.skip -= skipped;
        self.left.clear();
        self.right.clear();
        for frame in samples[skipped * 2..].chunks_exact(2) {
            // The same 16-bit steps and clipping as the PCM formats.
            self.left.push(f32::from(to_i16(frame[0])) / 32_768.0);
            self.right.push(f32::from(to_i16(frame[1])) / 32_768.0);
        }
        if self.left.is_empty() {
            return Ok(());
        }
        self.bytes.clear();
        self.bytes
            .reserve(mp3lame_encoder::max_required_buffer_size(self.left.len()));
        self.lame
            .encode_to_vec(
                mp3lame_encoder::DualPcm {
                    left: &self.left,
                    right: &self.right,
                },
                &mut self.bytes,
            )
            .map_err(lame_error)?;
        self.out.write_all(&self.bytes)
    }

    fn finish(mut self: Box<Self>) -> io::Result<()> {
        self.bytes.clear();
        self.bytes.reserve(7_200);
        self.lame
            .flush_to_vec::<mp3lame_encoder::FlushGap>(&mut self.bytes)
            .map_err(lame_error)?;
        self.out.write_all(&self.bytes)?;
        // The Info frame reserved at the start, now with the length, delay
        // and padding.
        self.bytes.clear();
        self.bytes.reserve(self.lame.lame_tag_size());
        if self.lame.lame_tag_encode_to_vec(&mut self.bytes).is_some() {
            self.out
                .seek(SeekFrom::Start(self.lame.id3v2_tag_size() as u64))?;
            self.out.write_all(&self.bytes)?;
        }
        self.out.flush()?;
        self.out.get_ref().sync_all()
    }
}
