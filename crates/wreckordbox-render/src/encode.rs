//! Turning rendered samples into a file.

use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};

use crate::Format;

/// What an encoder put in the file around the frames it was given, in
/// frames: nothing for PCM.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Coding {
    pub delay: u64,
    pub padding: u64,
}

/// One output file being written. The seam other formats (ticket #82) come
/// in behind: `render` only ever sees this.
pub(crate) trait Encoder {
    /// Appends interleaved stereo. Every frame given is encoded.
    fn write(&mut self, samples: &[f32]) -> io::Result<()>;

    /// Completes the file and flushes it to disk.
    fn finish(self: Box<Self>) -> io::Result<Coding>;
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

    fn finish(mut self: Box<Self>) -> io::Result<Coding> {
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
        self.out.get_ref().sync_all()?;
        Ok(Coding::default())
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

    fn finish(mut self: Box<Self>) -> io::Result<Coding> {
        let too_long = || io::Error::new(io::ErrorKind::InvalidInput, "too long for an AIFF file");
        let frames = u32::try_from(self.frames).map_err(|_| too_long())?;
        let data = frames
            .checked_mul(u32::from(CHANNELS * BYTES_PER_SAMPLE))
            .filter(|data| data.checked_add(AIFF_HEADER).is_some())
            .ok_or_else(too_long)?;
        self.out.seek(SeekFrom::Start(4))?;
        self.out
            .write_all(&(data + AIFF_HEADER - 8).to_be_bytes())?;
        self.out.seek(SeekFrom::Start(22))?;
        self.out.write_all(&frames.to_be_bytes())?;
        self.out.seek(SeekFrom::Start(42))?;
        self.out.write_all(&(data + 8).to_be_bytes())?;
        self.out.flush()?;
        self.out.get_ref().sync_all()?;
        Ok(Coding::default())
    }
}

/// A sample rate as the 80-bit IEEE 754 extended float an AIFF header holds.
fn extended(rate: u32) -> [u8; 10] {
    let mut bytes = [0_u8; 10];
    if rate == 0 {
        return bytes;
    }
    let shift = rate.ilog2();
    #[allow(clippy::cast_possible_truncation, reason = "at most 16383 + 31")]
    let exponent = (16_383 + shift) as u16;
    bytes[..2].copy_from_slice(&exponent.to_be_bytes());
    bytes[2..].copy_from_slice(&(u64::from(rate) << (63 - shift)).to_be_bytes());
    bytes
}

/// MP3 through LAME, at the export's settings (`rbl_audio::compatibility`):
/// 320 kbit/s constant, best quality, with an Info frame carrying the LAME
/// tag.
///
/// Every frame given is encoded; nothing is dropped or moved to make up for
/// the encoder's delay. LAME puts its delay (576 frames) ahead of the first
/// frame and pads the last MPEG frame; both are written in the LAME tag,
/// read back from it, and reported beside the frame count. Where a player,
/// a deck or rekordbox then places the first frame (whether it honours the
/// tag's delay, plays the Info frame as silence, and so where a beat grid
/// lands) has not been checked on hardware: see the README.
struct Mp3 {
    out: BufWriter<File>,
    lame: mp3lame_encoder::Encoder,
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
            left: Vec::new(),
            right: Vec::new(),
            bytes: Vec::new(),
        })
    }
}

impl Encoder for Mp3 {
    fn write(&mut self, samples: &[f32]) -> io::Result<()> {
        self.left.clear();
        self.right.clear();
        for frame in samples.chunks_exact(2) {
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

    fn finish(mut self: Box<Self>) -> io::Result<Coding> {
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
        if self.lame.lame_tag_encode_to_vec(&mut self.bytes).is_none() {
            return Err(lame_error("no LAME tag to write"));
        }
        let coding = lame_tag_coding(&self.bytes)
            .ok_or_else(|| lame_error("the LAME tag has no delay and padding"))?;
        self.out
            .seek(SeekFrom::Start(self.lame.id3v2_tag_size() as u64))?;
        self.out.write_all(&self.bytes)?;
        self.out.flush()?;
        self.out.get_ref().sync_all()?;
        Ok(coding)
    }
}

/// The encoder delay and padding a LAME tag records: the Info (or Xing)
/// header's optional fields, then the LAME extension, whose bytes 21 to 23
/// hold the two as 12-bit numbers.
fn lame_tag_coding(frame: &[u8]) -> Option<Coding> {
    let at = frame
        .windows(4)
        .position(|w| w == b"Info" || w == b"Xing")?;
    let flags = u32::from_be_bytes(frame.get(at + 4..at + 8)?.try_into().ok()?);
    let mut lame = at + 8;
    for (flag, size) in [(1, 4), (2, 4), (4, 100), (8, 4)] {
        if flags & flag != 0 {
            lame += size;
        }
    }
    if frame.get(lame..lame + 4)? != b"LAME" {
        return None;
    }
    let bytes = frame.get(lame + 21..lame + 24)?;
    let delay = (u64::from(bytes[0]) << 4) | (u64::from(bytes[1]) >> 4);
    let padding = (u64::from(bytes[1] & 0x0F) << 8) | u64::from(bytes[2]);
    Some(Coding { delay, padding })
}
