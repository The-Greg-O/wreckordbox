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
