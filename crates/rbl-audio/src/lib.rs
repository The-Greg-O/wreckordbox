//! Decoding for analysis.
//!
//! Analysis wants one mono signal at a known rate, not the file's own layout.
//! Decoding is the expensive step, so it happens once and every later stage
//! reads the same buffer.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "sample counts and durations convert between usize and f64 throughout decoding"
)]

pub mod aifc;
pub mod compatibility;

use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("unsupported or unreadable audio: {0}")]
    Unsupported(String),
    #[error("no audio track in the file")]
    NoTrack,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The file ended before the audio its container declares. Only reported
    /// by [`MonoStream::try_next_frames`], and only for a container whose
    /// declared length is exact.
    #[error("the audio ends after {decoded} of the {expected} frames the file declares")]
    Truncated { expected: u64, decoded: u64 },
}

pub type Result<T> = std::result::Result<T, AudioError>;

/// Mono audio at a known sample rate.
#[derive(Debug, Clone)]
pub struct Audio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    /// Channels in the source, before the downmix.
    pub source_channels: u16,
}

impl Audio {
    pub fn duration_secs(&self) -> f64 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.samples.len() as f64 / f64::from(self.sample_rate)
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

/// Decodes a file to mono `f32`.
///
/// `max_secs` bounds the work: tempo and key are stable well before a whole
/// long mix is decoded, and an unbounded decode is how a 2-hour file becomes a
/// memory problem. A caller that needs the rest of the file as well, without
/// holding it, reads it from a [`MonoStream`] instead.
pub fn decode_mono(path: &Path, max_secs: Option<f64>) -> Result<Audio> {
    let mut stream = MonoStream::open(path)?;
    let cap = max_secs.map(|s| (s * f64::from(stream.sample_rate())) as usize);
    let mut samples: Vec<f32> = Vec::with_capacity(cap.unwrap_or(0).min(1 << 24));
    while let Some(chunk) = stream.next_chunk() {
        samples.extend_from_slice(chunk);
        if let Some(cap) = cap {
            if samples.len() >= cap {
                samples.truncate(cap);
                break;
            }
        }
    }

    if samples.is_empty() {
        return Err(AudioError::Unsupported("decoded no samples".into()));
    }

    Ok(Audio { samples, sample_rate: stream.sample_rate(), source_channels: stream.source_channels() })
}

/// One packet's frames: downmixed, and the same frames as 16-bit stereo.
pub type Frames<'a> = (&'a [f32], &'a [[i16; 2]]);

/// A file decoded to mono `f32` one packet at a time.
///
/// What [`decode_mono`] collects into one buffer, for a caller that wants
/// to keep only part of it: the analysis keeps the first half hour for the
/// tempo and key and only folds the rest into the waveform, so a two-hour
/// mix is drawn to its end without its samples all being held at once.
pub struct MonoStream {
    format: Box<dyn symphonia::core::formats::FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track_id: u32,
    sample_rate: u32,
    source_channels: u16,
    buffer: Option<SampleBuffer<f32>>,
    mono: Vec<f32>,
    /// The same frames as 16-bit stereo: see [`MonoStream::next_frames`].
    stereo: Vec<[i16; 2]>,
    /// What is left of the file's lead on rekordbox's timeline: frames of
    /// silence still to hand out when positive, decoded frames still to drop
    /// when negative. See [`rekordbox_lead`].
    lead: i64,
    /// How many frames the container says the file holds, lead included,
    /// when it says.
    expected: Option<u64>,
    /// When `expected` is a count of what the file must hold, how many
    /// frames short of it the audio may still end: `Some(0)` for PCM and
    /// FLAC, a frame for an MP3 whose tag counts its frames. `None` when it
    /// is an estimate (an MP3's from its size).
    slack: Option<u64>,
    /// Frames handed out so far, lead included.
    handed: u64,
    /// The most frames the file's sound holds, when the decoder may read
    /// past it: an AIFF's `COMM` count (see `aifc::declared_frames`).
    limit: Option<u64>,
}

/// Source-rate frames rekordbox's timeline for a file runs ahead of what
/// symphonia decodes from it, zero for anything but an MP3 that rekordbox
/// maps differently. Every time analysis writes (beats, cues, waveform
/// columns) and every time read back for audio is on rekordbox's timeline,
/// so decoding for either starts this much later. See [`rbl_core::mpeg`].
fn rekordbox_lead(path: &Path, codec: symphonia::core::codecs::CodecType) -> i64 {
    if codec != symphonia::core::codecs::CODEC_TYPE_MP3 {
        return 0;
    }
    std::fs::File::open(path).map_or(0, |mut file| rbl_core::mpeg::rekordbox_lead_frames(&mut file))
}

impl MonoStream {
    /// Opens a file and its first audio track.
    pub fn open(path: &Path) -> Result<Self> {
        let (source, aifc) = aifc::open(path)?;
        Self::over(source, aifc, path.extension().and_then(|e| e.to_str()), Some(path))
    }

    /// [`MonoStream::open`] over bytes from anywhere. `path` is the file the
    /// bytes are from, when there is one: an MP3's lead is read from it.
    fn over(
        source: Box<dyn symphonia::core::io::MediaSource>,
        aifc: aifc::Aifc,
        extension: Option<&str>,
        path: Option<&Path>,
    ) -> Result<Self> {
        let stream = MediaSourceStream::new(source, symphonia::core::io::MediaSourceStreamOptions::default());

        let mut hint = Hint::new();
        if let Some(ext) = extension {
            hint.with_extension(ext);
        }

        let probed = symphonia::default::get_probe()
            .format(&hint, stream, &FormatOptions::default(), &MetadataOptions::default())
            .map_err(|e| AudioError::Unsupported(aifc.describe(e)))?;
        let format = probed.format;

        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
            .ok_or(AudioError::NoTrack)?;
        let track_id = track.id;
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|e| AudioError::Unsupported(e.to_string()))?;
        let sample_rate = track.codec_params.sample_rate.unwrap_or(44_100);
        let source_channels = track
            .codec_params
            .channels
            .map_or(2, |c| u16::try_from(c.count()).unwrap_or(2));
        let lead = path.map_or(0, |path| rekordbox_lead(path, track.codec_params.codec));
        // An AIFF's own header, where symphonia's count is off (see
        // `aifc::declared_frames`); the container's count otherwise.
        let limit = path.and_then(aifc::declared_frames);
        let expected = limit
            .or_else(|| track.codec_params.n_frames.map(|frames| frames.saturating_add_signed(lead)));
        let slack = slack(&track.codec_params, path);
        Ok(Self {
            format,
            decoder,
            track_id,
            sample_rate,
            source_channels,
            buffer: None,
            mono: Vec::new(),
            stereo: Vec::new(),
            lead,
            expected,
            slack,
            handed: 0,
            limit,
        })
    }

    /// How many samples the stream is expected to hand out in all, from the
    /// container's frame count; `None` when the container does not declare
    /// one. An estimate: a damaged or truncated file can end sooner.
    pub fn expected_frames(&self) -> Option<u64> {
        self.expected
    }

    /// The rate of the samples handed out so far, or the one the container
    /// declares before the first packet.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Channels in the source, before the downmix.
    pub fn source_channels(&self) -> u16 {
        self.source_channels
    }

    /// The next packet's samples, downmixed, beside the same frames as
    /// 16-bit stereo: what rekordbox's waveform analysis reads, one channel
    /// twice for a mono file and the first two of any more.
    pub fn next_frames(&mut self) -> Option<(&[f32], &[[i16; 2]])> {
        self.next_chunk()?;
        Some((&self.mono, &self.stereo))
    }

    /// [`MonoStream::next_frames`] for a caller that must not mistake a
    /// broken file for a short one: `Ok(None)` only at the true end of the
    /// audio.
    ///
    /// An error where [`MonoStream::next_frames`] quietly ends:
    ///
    /// - reading the file failed ([`AudioError::Io`]);
    /// - the decoder gave up on the stream ([`AudioError::Unsupported`]);
    /// - the file ran out before the frames its container declares, for a
    ///   container whose count is exact: PCM (WAV, AIFF) and FLAC
    ///   ([`AudioError::Truncated`]); and, to within one frame, for an MP3
    ///   whose Xing, Info or VBRI frame counts its frames. An MP3 with no
    ///   such frame cut short cannot be told from a shorter one (its length
    ///   is an estimate from its size) and ends as it would have; and, to
    ///   within one packet, for AAC or ALAC in MP4, whose sample table
    ///   counts its packets (a cut file usually does not open; one whose
    ///   `mdat` runs to the end of the file does, and stops short here).
    ///
    /// A damaged packet inside the stream is still its own length of
    /// silence, as in rekordbox and on a deck: the audio keeps its place.
    pub fn try_next_frames(&mut self) -> Result<Option<Frames<'_>>> {
        if self.advance()? {
            Ok(Some((&self.mono, &self.stereo)))
        } else {
            Ok(None)
        }
    }

    /// The next packet's samples, downmixed; `None` at the end of the file.
    ///
    /// `next_packet` reports the end of the stream as an error, so any error
    /// there ends it. A damaged packet mid-file is skipped rather than
    /// discarding what came before it.
    pub fn next_chunk(&mut self) -> Option<&[f32]> {
        // Any failure ends the stream here; `try_next_frames` reports it.
        if self.advance().ok()? {
            Some(&self.mono)
        } else {
            None
        }
    }

    /// Fills `mono` and `stereo` with the next packet's samples. `Ok(false)`
    /// at the end of the audio; an error when it stopped short of it.
    fn advance(&mut self) -> Result<bool> {
        use symphonia::core::errors::Error;
        if self.limit.is_some_and(|limit| self.handed >= limit) {
            return Ok(false);
        }
        if self.lead > 0 {
            let lead = usize::try_from(self.lead).unwrap_or(0);
            self.mono.clear();
            self.mono.resize(lead, 0.0);
            self.stereo.clear();
            self.stereo.resize(lead, [0, 0]);
            self.lead = 0;
            self.handed += lead as u64;
            return Ok(true);
        }
        loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                // How symphonia says the stream is over: the bytes ran out.
                // Whether that is the end of the audio is for the count.
                Err(Error::IoError(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return match self.expected {
                        Some(expected)
                            if self.slack.is_some_and(|slack| self.handed.saturating_add(slack) < expected) =>
                        {
                            Err(AudioError::Truncated { expected, decoded: self.handed })
                        }
                        _ => Ok(false),
                    };
                }
                Err(Error::IoError(error)) => return Err(AudioError::Io(error)),
                Err(error) => return Err(AudioError::Unsupported(error.to_string())),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let frames = match self.decoder.decode(&packet) {
                Ok(d) => d,
                // A damaged packet lasts as long as it would have, in silence,
                // as it does in rekordbox: dropping it would move everything
                // after it earlier.
                Err(symphonia::core::errors::Error::DecodeError(_)) => {
                    let silence = usize::try_from(packet.dur).unwrap_or(0);
                    self.mono.clear();
                    self.mono.resize(silence, 0.0);
                    self.stereo.clear();
                    self.stereo.resize(silence, [0, 0]);
                    if self.drop_lead() {
                        self.hold_to_limit();
                        self.handed += self.mono.len() as u64;
                        return Ok(true);
                    }
                    continue;
                }
                Err(Error::IoError(error)) => return Err(AudioError::Io(error)),
                Err(error) => return Err(AudioError::Unsupported(error.to_string())),
            };

            let spec = *frames.spec();
            self.sample_rate = spec.rate;
            self.source_channels = u16::try_from(spec.channels.count()).unwrap_or(2);

            let interleaved =
                self.buffer.get_or_insert_with(|| SampleBuffer::new(frames.capacity() as u64, spec));
            interleaved.copy_interleaved_ref(frames);

            let channels = spec.channels.count().max(1);
            self.mono.clear();
            self.mono.extend(interleaved.samples().chunks(channels).map(|frame| frame.iter().sum::<f32>() / channels as f32));
            self.stereo.clear();
            self.stereo.extend(
                interleaved.samples().chunks(channels).map(|frame| [to_i16(frame[0]), to_i16(*frame.get(1).unwrap_or(&frame[0]))]),
            );
            if self.drop_lead() {
                self.hold_to_limit();
                self.handed += self.mono.len() as u64;
                return Ok(true);
            }
        }
    }

    /// Cuts what was just decoded at the end of the file's sound, so bytes
    /// after it (an AIFF's trailing `ID3 ` chunk: symphonia 0.5.5 reads the
    /// `SSND` chunk 8 bytes too far) never become audio.
    fn hold_to_limit(&mut self) {
        if let Some(limit) = self.limit {
            let left = usize::try_from(limit.saturating_sub(self.handed)).unwrap_or(usize::MAX);
            self.mono.truncate(left);
            self.stereo.truncate(left);
        }
    }

    /// Drops what a negative lead still owes from the front of `mono`, and
    /// says whether anything is left to hand out.
    fn drop_lead(&mut self) -> bool {
        if self.lead < 0 {
            let owed = usize::try_from(self.lead.unsigned_abs()).unwrap_or(usize::MAX);
            let dropped = owed.min(self.mono.len());
            self.mono.drain(..dropped);
            self.stereo.drain(..dropped);
            self.lead += dropped as i64;
        }
        !self.mono.is_empty()
    }
}

/// How far short of the container's frame count a whole file may end, or
/// `None` when the count is only an estimate and a shorter file cannot be
/// told from a cut one.
///
/// Uncompressed PCM's count is the size of its data and FLAC's header
/// carries it: exact. An MP3's is exact when its Xing, Info or VBRI frame
/// counts the frames (symphonia then gives that count times the frame's
/// samples) and an estimate from the file's size otherwise; one frame is
/// allowed for an encoder that counts its tag frame among them. AAC and
/// ALAC in MP4 (the only AAC the Library holds) are held to the sample
/// table's count within a packet: symphonia refuses to open most cut MP4s
/// (it checks each box against its size), but not one whose `mdat` box is
/// written as running to the end of the file.
fn slack(params: &symphonia::core::codecs::CodecParameters, path: Option<&Path>) -> Option<u64> {
    use symphonia::core::codecs::{CODEC_TYPE_AAC, CODEC_TYPE_ALAC, CODEC_TYPE_MP3};
    if declares_exact_length(params.codec) {
        return Some(0);
    }
    if [CODEC_TYPE_AAC, CODEC_TYPE_ALAC].contains(&params.codec) && path.is_some_and(is_mp4) {
        // An MP4's sample table counts every packet, so its length is exact
        // even when its `mdat` legally runs "to the end of the file" (size
        // 0) and a cut leaves the table whole: a packet's worth of slack.
        return params.n_frames.map(|_| 1_024);
    }
    if params.codec == CODEC_TYPE_MP3 {
        let file = std::fs::File::open(path?).ok();
        return file
            .is_some_and(|mut file| rbl_core::mpeg::tag_counts_frames(&mut file))
            .then_some(1_152);
    }
    None
}

/// Whether a file is an ISO media (MP4, M4A) file: its first box is `ftyp`.
fn is_mp4(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0_u8; 8];
    std::fs::File::open(path).and_then(|mut file| file.read_exact(&mut head)).is_ok() && &head[4..8] == b"ftyp"
}

/// Whether a container's frame count for this codec is what the file must
/// hold: uncompressed PCM, whose count is the size of its data, and FLAC,
/// whose header carries it.
fn declares_exact_length(codec: symphonia::core::codecs::CodecType) -> bool {
    use symphonia::core::codecs::{
        CODEC_TYPE_FLAC, CODEC_TYPE_PCM_ALAW, CODEC_TYPE_PCM_F32BE, CODEC_TYPE_PCM_F32LE, CODEC_TYPE_PCM_F64BE,
        CODEC_TYPE_PCM_F64LE, CODEC_TYPE_PCM_MULAW, CODEC_TYPE_PCM_S16BE, CODEC_TYPE_PCM_S16LE, CODEC_TYPE_PCM_S24BE,
        CODEC_TYPE_PCM_S24LE, CODEC_TYPE_PCM_S32BE, CODEC_TYPE_PCM_S32LE, CODEC_TYPE_PCM_S8, CODEC_TYPE_PCM_U8,
    };
    [
        CODEC_TYPE_FLAC,
        CODEC_TYPE_PCM_S8,
        CODEC_TYPE_PCM_U8,
        CODEC_TYPE_PCM_S16LE,
        CODEC_TYPE_PCM_S16BE,
        CODEC_TYPE_PCM_S24LE,
        CODEC_TYPE_PCM_S24BE,
        CODEC_TYPE_PCM_S32LE,
        CODEC_TYPE_PCM_S32BE,
        CODEC_TYPE_PCM_F32LE,
        CODEC_TYPE_PCM_F32BE,
        CODEC_TYPE_PCM_F64LE,
        CODEC_TYPE_PCM_F64BE,
        CODEC_TYPE_PCM_ALAW,
        CODEC_TYPE_PCM_MULAW,
    ]
    .contains(&codec)
}

/// A decoded sample as a 16-bit decode holds it: symphonia's own conversion,
/// so the frames match what decoding straight to `i16` gives.
fn to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * 32_768.0) as i16
}

/// Writes the stretch of a file between `from_secs` and `to_secs` as a
/// 16-bit PCM WAV at the file's own rate and channel count: rekordbox's
/// Export Loop As WAV. Returns the frames written. Nothing is written when
/// the range holds no audio.
pub fn write_range_wav(source: &Path, from_secs: f64, to_secs: f64, out: &Path) -> Result<u64> {
    if to_secs <= from_secs || from_secs < 0.0 || !to_secs.is_finite() {
        return Err(AudioError::Unsupported("the range is empty".into()));
    }
    let file = std::fs::File::open(source)?;
    let stream = MediaSourceStream::new(Box::new(file), symphonia::core::io::MediaSourceStreamOptions::default());
    let mut hint = Hint::new();
    if let Some(ext) = source.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, stream, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| AudioError::Unsupported(e.to_string()))?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or(AudioError::NoTrack)?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| AudioError::Unsupported(e.to_string()))?;

    let mut sample_rate = track.codec_params.sample_rate.unwrap_or(44_100);
    let mut channels = track.codec_params.channels.map_or(2, |c| c.count().max(1));
    // Interleaved frames of the range, as `i16`.
    let mut pcm: Vec<i16> = Vec::new();
    let mut buffer: Option<SampleBuffer<f32>> = None;
    // The range is on rekordbox's timeline, where the first decoded frame is
    // at the file's lead; before it is silence. See [`rekordbox_lead`].
    let lead = rekordbox_lead(source, track.codec_params.codec);
    let mut frame_at = lead;
    let mut first = (from_secs * f64::from(sample_rate)) as u64;
    let mut last = (to_secs * f64::from(sample_rate)) as u64;
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }
        if frame_at > first as i64 && pcm.is_empty() {
            // The range opens inside a positive lead: its silence first.
            let quiet = (frame_at.min(last as i64) - first as i64) as usize;
            pcm.resize(quiet * channels, 0);
        }
        let silent: Vec<f32>;
        let samples = match decoder.decode(&packet) {
            Ok(frames) => {
                let spec = *frames.spec();
                if spec.rate != sample_rate {
                    sample_rate = spec.rate;
                    first = (from_secs * f64::from(sample_rate)) as u64;
                    last = (to_secs * f64::from(sample_rate)) as u64;
                }
                channels = spec.channels.count().max(1);
                let interleaved = buffer.get_or_insert_with(|| SampleBuffer::new(frames.capacity() as u64, spec));
                interleaved.copy_interleaved_ref(frames);
                interleaved.samples()
            }
            // A damaged packet keeps its place, in silence, as in rekordbox.
            Err(symphonia::core::errors::Error::DecodeError(_)) => {
                silent = vec![0.0; usize::try_from(packet.dur).unwrap_or(0) * channels];
                &silent
            }
            Err(_) => break,
        };
        let count = (samples.len() / channels) as i64;
        let packet_start = frame_at;
        frame_at += count;
        if frame_at <= first as i64 {
            continue;
        }
        let take_from = (first as i64 - packet_start).max(0) as usize;
        let take_to = ((last as i64 - packet_start).max(0) as usize).min(count as usize);
        for frame in samples.chunks(channels).take(take_to).skip(take_from) {
            for &sample in frame {
                pcm.push((sample.clamp(-1.0, 1.0) * 32767.0) as i16);
            }
        }
        if frame_at >= last as i64 {
            break;
        }
    }
    if pcm.is_empty() {
        return Err(AudioError::Unsupported("no audio in the range".into()));
    }
    let channels_u16 = u16::try_from(channels).unwrap_or(2);
    let block_align = channels_u16 * 2;
    let data_len = u32::try_from(pcm.len() * 2).unwrap_or(u32::MAX);
    let mut bytes = Vec::with_capacity(44 + pcm.len() * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&channels_u16.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * u32::from(block_align)).to_le_bytes());
    bytes.extend_from_slice(&block_align.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in &pcm {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(out, bytes)?;
    Ok((pcm.len() / channels) as u64)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
mod tests {
    use super::*;

    /// Writes a 16-bit PCM WAV, so the test needs no fixture file.
    fn write_wav(path: &Path, sample_rate: u32, channels: u16, samples: &[f32]) {
        let bits = 16_u16;
        let block_align = channels * bits / 8;
        let byte_rate = sample_rate * u32::from(block_align);
        let data_len = u32::try_from(samples.len() * 2).unwrap();
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16_u32.to_le_bytes());
        out.extend_from_slice(&1_u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&sample_rate.to_le_bytes());
        out.extend_from_slice(&byte_rate.to_le_bytes());
        out.extend_from_slice(&block_align.to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for s in samples {
            out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
        std::fs::write(path, out).unwrap();
    }

    /// Bytes that can be read up to `fails_at` and not past it.
    struct Flaky {
        bytes: std::io::Cursor<Vec<u8>>,
        fails_at: u64,
    }

    impl std::io::Read for Flaky {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let left = self.fails_at.saturating_sub(self.bytes.position());
            if left == 0 {
                return Err(std::io::Error::other("the disk went away"));
            }
            let take = buf.len().min(usize::try_from(left).unwrap());
            self.bytes.read(&mut buf[..take])
        }
    }

    impl std::io::Seek for Flaky {
        fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
            self.bytes.seek(to)
        }
    }

    impl symphonia::core::io::MediaSource for Flaky {
        fn is_seekable(&self) -> bool {
            true
        }
        fn byte_len(&self) -> Option<u64> {
            Some(self.bytes.get_ref().len() as u64)
        }
    }

    /// Everything a stream hands out through the fallible reader, or the
    /// error that stopped it and how many frames came before.
    fn drain(stream: &mut MonoStream) -> std::result::Result<usize, (AudioError, usize)> {
        let mut frames = 0;
        loop {
            match stream.try_next_frames() {
                Ok(Some((_, stereo))) => frames += stereo.len(),
                Ok(None) => return Ok(frames),
                Err(error) => return Err((error, frames)),
            }
        }
    }

    #[test]
    fn the_fallible_reader_reads_a_whole_file_to_its_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tone.wav");
        write_wav(&path, 8_000, 2, &vec![0.25; 2 * 12_000]);

        let mut stream = MonoStream::open(&path).unwrap();
        assert_eq!(drain(&mut stream).unwrap(), 12_000);
    }

    #[test]
    fn a_file_cut_short_is_an_error_to_the_fallible_reader_and_an_end_to_the_other() {
        // The header declares 12,000 frames; the file was cut after 5,000.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cut.wav");
        write_wav(&path, 8_000, 2, &vec![0.25; 2 * 12_000]);
        let whole = std::fs::read(&path).unwrap();
        std::fs::write(&path, &whole[..44 + 5_000 * 4]).unwrap();

        let mut stream = MonoStream::open(&path).unwrap();
        let (error, before) = drain(&mut stream).unwrap_err();
        assert!(
            matches!(error, AudioError::Truncated { expected: 12_000, decoded } if decoded as usize == before && decoded <= 5_000),
            "{error:?} after {before} frames"
        );

        // The reader analysis uses is unchanged: it ends where the audio does.
        let mut stream = MonoStream::open(&path).unwrap();
        let mut handed_out = 0;
        while let Some((_, stereo)) = stream.next_frames() {
            handed_out += stereo.len();
        }
        assert_eq!(handed_out, before);
    }

    /// A 16-bit stereo AIFF as the format defines it: the `SSND` chunk's size
    /// counts its offset and block-size fields as well as the samples.
    fn aiff_bytes(frames: u32) -> Vec<u8> {
        let data = frames * 4;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"FORM");
        bytes.extend_from_slice(&(46 + data).to_be_bytes());
        bytes.extend_from_slice(b"AIFFCOMM");
        bytes.extend_from_slice(&18_u32.to_be_bytes());
        bytes.extend_from_slice(&2_u16.to_be_bytes());
        bytes.extend_from_slice(&frames.to_be_bytes());
        bytes.extend_from_slice(&16_u16.to_be_bytes());
        bytes.extend_from_slice(&[0x40, 0x0e, 0xac, 0x44, 0, 0, 0, 0, 0, 0]);
        bytes.extend_from_slice(b"SSND");
        bytes.extend_from_slice(&(8 + data).to_be_bytes());
        bytes.extend_from_slice(&[0; 8]);
        for i in 0..frames {
            let sample = ((i % 100) as i16 * 300).to_be_bytes();
            bytes.extend_from_slice(&sample);
            bytes.extend_from_slice(&sample);
        }
        bytes
    }

    #[test]
    fn the_fallible_reader_reads_a_whole_aiff_to_its_end_and_refuses_one_cut_short() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tone.aiff");
        let whole = aiff_bytes(12_000);
        std::fs::write(&path, &whole).unwrap();
        assert_eq!(MonoStream::open(&path).unwrap().expected_frames(), Some(12_000));
        assert_eq!(drain(&mut MonoStream::open(&path).unwrap()).unwrap(), 12_000);

        std::fs::write(&path, &whole[..54 + 5_000 * 4]).unwrap();
        let (error, _) = drain(&mut MonoStream::open(&path).unwrap()).unwrap_err();
        assert!(matches!(error, AudioError::Truncated { expected: 12_000, .. }), "{error:?}");
    }

    #[test]
    fn a_chunk_after_an_aiff_s_sound_is_never_read_as_audio() {
        // An `ID3 ` chunk after `SSND`, whose header bytes are not zero: a
        // reader that runs past the sound would hand them out as samples.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tagged.aiff");
        let plain = aiff_bytes(12_000);
        let mut tagged = plain.clone();
        let id3 = [b"ID3 ".as_slice(), &16_u32.to_be_bytes(), &[0x49, 0x44, 0x33, 4, 0, 0, 0x7f, 0x7f, 0x7f, 0x7f, 1, 2, 3, 4, 5, 6]]
            .concat();
        tagged.extend_from_slice(&id3);
        let form = u32::try_from(tagged.len() - 8).unwrap();
        tagged[4..8].copy_from_slice(&form.to_be_bytes());
        std::fs::write(&path, &tagged).unwrap();

        let reference = dir.path().join("plain.aiff");
        std::fs::write(&reference, &plain).unwrap();
        let (_, want) = frames_of(&reference);
        let (_, got) = frames_of(&path);
        assert_eq!(want.len(), 12_000);
        assert_eq!(got.len(), want.len(), "the sound's frames and no more");
        assert_eq!(got, want, "the samples, and nothing of the chunk after them");
        assert_eq!(drain(&mut MonoStream::open(&path).unwrap()).unwrap(), 12_000);
    }

    /// The fixture's fast-start M4A (sample table ahead of the audio) with its
    /// `mdat` box's size written as 0, which means "to the end of the file".
    fn faststart_m4a_to_eof() -> Vec<u8> {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tone-faststart.m4a");
        let mut bytes = std::fs::read(fixture).unwrap();
        let mdat = bytes.windows(4).position(|w| w == b"mdat").unwrap() - 4;
        let moov = bytes.windows(4).position(|w| w == b"moov").unwrap();
        assert!(moov < mdat, "the fixture is fast-start");
        bytes[mdat..mdat + 4].copy_from_slice(&0_u32.to_be_bytes());
        bytes
    }

    #[test]
    fn a_fast_start_m4a_whose_audio_runs_to_the_end_of_the_file_is_read_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("whole.m4a");
        std::fs::write(&path, faststart_m4a_to_eof()).unwrap();
        let expected = MonoStream::open(&path).unwrap().expected_frames().unwrap();
        let frames = drain(&mut MonoStream::open(&path).unwrap()).unwrap() as u64;
        assert!(frames >= expected, "{frames} of {expected}");
    }

    #[test]
    fn a_fast_start_m4a_cut_inside_audio_that_runs_to_the_end_of_the_file_is_an_error() {
        // The sample table is whole and the `mdat` is legal as cut (size 0),
        // so the file opens; its packets then run out early.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cut.m4a");
        let whole = faststart_m4a_to_eof();
        std::fs::write(&path, &whole[..whole.len() / 2]).unwrap();
        let Ok(mut stream) = MonoStream::open(&path) else {
            return; // refused at the door is as good
        };
        let result = drain(&mut stream);
        assert!(matches!(result, Err((AudioError::Truncated { .. } | AudioError::Io(_), _))), "{result:?}");
        // The reader analysis uses still plays what is there.
        assert!(MonoStream::open(&path).unwrap().next_frames().is_some());
    }

    #[test]
    fn a_read_that_fails_midway_is_an_error_to_the_fallible_reader() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tone.wav");
        write_wav(&path, 8_000, 2, &vec![0.25; 2 * 12_000]);
        let bytes = std::fs::read(&path).unwrap();
        let flaky = || {
            let source = Flaky { bytes: std::io::Cursor::new(bytes.clone()), fails_at: 44 + 5_000 * 4 };
            MonoStream::over(Box::new(source), aifc::Aifc::default(), Some("wav"), None).unwrap()
        };

        let (error, before) = drain(&mut flaky()).unwrap_err();
        assert!(matches!(error, AudioError::Io(_)), "{error:?}");
        assert!(before <= 5_000);

        let mut stream = flaky();
        let mut handed_out = 0;
        while let Some((_, stereo)) = stream.next_frames() {
            handed_out += stereo.len();
        }
        assert_eq!(handed_out, before, "the other reader ends quietly, as it always has");
    }

    #[test]
    fn a_range_is_written_as_a_wav_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("two.wav");
        // Stereo, 1000 Hz, 2 seconds: the left channel a ramp, the right zero.
        let mut samples = Vec::new();
        for i in 0..2000 {
            samples.push(i as f32 / 2000.0);
            samples.push(0.0);
        }
        write_wav(&source, 1000, 2, &samples);
        let out = dir.path().join("loop.wav");
        let frames = write_range_wav(&source, 0.5, 1.0, &out).unwrap();
        assert_eq!(frames, 500);
        let back = decode_mono(&out, None).unwrap();
        assert_eq!((back.sample_rate, back.source_channels, back.samples.len()), (1000, 2, 500));
        // The mono mix of the ramp from 0.5 s: half of 0.25.
        assert!((back.samples[0] - 0.125).abs() < 0.01, "{}", back.samples[0]);
        assert!(write_range_wav(&source, 1.0, 0.5, &out).is_err());
    }

    /// rekordbox's waveforms read both channels, so the stream hands out each
    /// chunk's frames as the file holds them beside the downmix; a mono file's
    /// one channel stands for both.
    #[test]
    fn the_frames_beside_the_downmix_are_the_files_own_stereo() {
        let dir = tempfile::tempdir().unwrap();
        let stereo = dir.path().join("stereo.wav");
        let samples: Vec<f32> = (0..2_000).flat_map(|i| [i as f32 / 4_000.0, -0.25]).collect();
        write_wav(&stereo, 8_000, 2, &samples);
        let mut stream = MonoStream::open(&stereo).unwrap();
        let mut frames = Vec::new();
        while let Some((mono, chunk)) = stream.next_frames() {
            assert_eq!(mono.len(), chunk.len());
            frames.extend_from_slice(chunk);
        }
        let written = |x: f32| (x * 32767.0) as i16;
        assert_eq!(frames.len(), 2_000);
        assert_eq!(frames[1_000], [written(0.25), written(-0.25)]);

        let mono = dir.path().join("mono.wav");
        write_wav(&mono, 8_000, 1, &[0.5; 100]);
        let mut stream = MonoStream::open(&mono).unwrap();
        let (_, chunk) = stream.next_frames().unwrap();
        assert_eq!(chunk[0], [written(0.5), written(0.5)]);
    }

    #[test]
    fn decodes_a_mono_wav() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tone.wav");
        let samples: Vec<f32> = (0..44_100)
            .map(|i| (i as f32 * 2.0 * std::f32::consts::PI * 440.0 / 44_100.0).sin() * 0.5)
            .collect();
        write_wav(&path, 44_100, 1, &samples);

        let audio = decode_mono(&path, None).unwrap();
        assert_eq!(audio.sample_rate, 44_100);
        assert_eq!(audio.source_channels, 1);
        assert!((audio.duration_secs() - 1.0).abs() < 0.01);
        // The tone should survive round-tripping through 16-bit.
        let peak = audio.samples.iter().fold(0.0_f32, |a, s| a.max(s.abs()));
        assert!((peak - 0.5).abs() < 0.01, "peak was {peak}");
    }

    #[test]
    fn a_stream_knows_its_length_before_it_is_decoded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tone.wav");
        write_wav(&path, 8_000, 2, &vec![0.25; 2 * 12_000]);

        let mut stream = MonoStream::open(&path).unwrap();
        assert_eq!(stream.expected_frames(), Some(12_000));
        let mut handed_out = 0;
        while let Some(chunk) = stream.next_chunk() {
            handed_out += chunk.len();
        }
        assert_eq!(handed_out, 12_000, "the estimate is what the stream then hands out");
    }

    #[test]
    fn downmixes_stereo_to_mono() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stereo.wav");
        // Left +0.5, right -0.5 must cancel to silence.
        let mut samples = Vec::new();
        for _ in 0..1000 {
            samples.push(0.5);
            samples.push(-0.5);
        }
        write_wav(&path, 44_100, 2, &samples);

        let audio = decode_mono(&path, None).unwrap();
        assert_eq!(audio.source_channels, 2);
        assert_eq!(audio.samples.len(), 1000);
        let peak = audio.samples.iter().fold(0.0_f32, |a, s| a.max(s.abs()));
        assert!(peak < 0.01, "opposite channels should cancel, peak was {peak}");
    }

    #[test]
    fn respects_the_duration_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long.wav");
        let samples: Vec<f32> = (0..44_100 * 10).map(|i| (i as f32 / 1000.0).sin()).collect();
        write_wav(&path, 44_100, 1, &samples);

        let audio = decode_mono(&path, Some(2.0)).unwrap();
        assert!(audio.duration_secs() <= 2.05, "got {}", audio.duration_secs());
        assert!(audio.duration_secs() >= 1.95);
    }

    #[test]
    fn a_missing_file_is_an_error() {
        assert!(decode_mono(Path::new("/definitely/not/here.wav"), None).is_err());
    }

    #[test]
    fn a_non_audio_file_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-audio.wav");
        std::fs::write(&path, b"this is not a wav file at all").unwrap();
        assert!(decode_mono(&path, None).is_err());
    }

    /// A LAME MP3 of a second of stereo tone, joint stereo with a LAME Info
    /// frame in the same mode: the kind of file rekordbox's timeline starts a
    /// frame earlier on (#277).
    pub(crate) fn lame_mp3(dir: &Path) -> std::path::PathBuf {
        let wav = dir.join("tone.wav");
        let samples: Vec<f32> = (0..44_100)
            .flat_map(|i| {
                let v = (i as f32 * std::f32::consts::TAU * 440.0 / 44_100.0).sin() * 0.5;
                [v, v * 0.5]
            })
            .collect();
        write_wav(&wav, 44_100, 2, &samples);
        let mp3 = dir.join("tone.mp3");
        crate::compatibility::convert(&wav, &mp3, crate::compatibility::Format::Mp3).unwrap();
        mp3
    }

    /// Where an MP3's frame `index` starts, counting its Info frame as 0.
    fn frame_at(bytes: &[u8], index: usize) -> usize {
        let mut at = bytes.windows(2).position(|w| w == [0xFF, 0xFB]).unwrap();
        for _ in 0..index {
            at += 1044 + usize::from((bytes[at + 2] >> 1) & 1);
        }
        assert_eq!(&bytes[at..at + 2], &[0xFF, 0xFB]);
        at
    }

    /// The MP3 with its fourth frame claiming 511 big values in its first
    /// granule, more than a granule holds: symphonia refuses to decode it, as
    /// it does the broken frames some files open with.
    fn damaged_mp3(mp3: &Path) -> std::path::PathBuf {
        let mut bytes = std::fs::read(mp3).unwrap();
        let at = frame_at(&bytes, 3);
        // Side information: main_data_begin (9 bits), private bits (3),
        // scfsi (8), part2_3_length (12), then big_values (9).
        bytes[at + 8] = 0xFF;
        bytes[at + 9] |= 0x80;
        let damaged = mp3.with_file_name("damaged.mp3");
        std::fs::write(&damaged, bytes).unwrap();
        damaged
    }

    /// The MP3 with its first audio frame in plain stereo, where the Info
    /// frame and the rest are joint: rekordbox starts its map a frame after
    /// that one, so its timeline begins a frame into symphonia's.
    fn restarted_mp3(mp3: &Path) -> std::path::PathBuf {
        let mut bytes = std::fs::read(mp3).unwrap();
        let at = frame_at(&bytes, 1);
        bytes[at + 3] &= 0x3F;
        let restarted = mp3.with_file_name("restarted.mp3");
        std::fs::write(&restarted, bytes).unwrap();
        restarted
    }

    /// Everything `next_frames` hands out, the downmix and the stereo frames
    /// as two runs, each chunk's two views checked to be as long.
    fn frames_of(path: &Path) -> (Vec<f32>, Vec<[i16; 2]>) {
        let mut stream = MonoStream::open(path).unwrap();
        let (mut mono, mut stereo) = (Vec::new(), Vec::new());
        while let Some((chunk, frames)) = stream.next_frames() {
            assert_eq!(chunk.len(), frames.len(), "a chunk's downmix and stereo are the same frames");
            mono.extend_from_slice(chunk);
            stereo.extend_from_slice(frames);
        }
        (mono, stereo)
    }

    /// Each stereo frame is the one its downmix sample was made from: a frame
    /// out of step on a 440 Hz tone is off by thousands of times more.
    fn assert_in_step(mono: &[f32], stereo: &[[i16; 2]]) {
        assert_eq!(mono.len(), stereo.len());
        let worst = mono
            .iter()
            .zip(stereo)
            .map(|(m, [l, r])| (m - (f32::from(*l) + f32::from(*r)) / 65_536.0).abs())
            .fold(0.0_f32, f32::max);
        assert!(worst < 2.0 / 32_768.0, "{worst}");
    }

    /// The file as symphonia decodes it, mono, with nothing moved.
    fn symphonia_mono(path: &Path) -> Vec<f32> {
        let file = std::fs::File::open(path).unwrap();
        let stream = MediaSourceStream::new(Box::new(file), Default::default());
        let mut format = symphonia::default::get_probe()
            .format(&Hint::new(), stream, &FormatOptions::default(), &MetadataOptions::default())
            .unwrap()
            .format;
        let track = format.default_track().unwrap().clone();
        let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).unwrap();
        let mut mono = Vec::new();
        while let Ok(packet) = format.next_packet() {
            let Ok(decoded) = decoder.decode(&packet) else { continue };
            let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
            buffer.copy_interleaved_ref(decoded);
            mono.extend(buffer.samples().chunks(2).map(|f| f.iter().sum::<f32>() / 2.0));
        }
        mono
    }

    #[test]
    fn an_mp3_whose_tag_counts_its_frames_is_held_to_that_count() {
        let dir = tempfile::tempdir().unwrap();
        let mp3 = lame_mp3(dir.path());
        let whole = std::fs::read(&mp3).unwrap();
        let expected = MonoStream::open(&mp3).unwrap().expected_frames().unwrap();
        assert_eq!(drain(&mut MonoStream::open(&mp3).unwrap()).unwrap() as u64, expected);

        // Cut after its twentieth frame: the Info frame still says how many
        // frames there were.
        std::fs::write(&mp3, &whole[..frame_at(&whole, 20)]).unwrap();
        let (error, before) = drain(&mut MonoStream::open(&mp3).unwrap()).unwrap_err();
        assert!(
            matches!(error, AudioError::Truncated { expected: e, decoded } if e == expected && decoded as usize == before),
            "{error:?}"
        );
        // Analysis's reader still plays what is there.
        assert!(frames_of(&mp3).1.len() >= before);
    }

    #[test]
    fn an_mp3_without_a_counting_tag_cut_short_cannot_be_told_from_a_shorter_one() {
        let dir = tempfile::tempdir().unwrap();
        let mp3 = lame_mp3(dir.path());
        let whole = std::fs::read(&mp3).unwrap();
        // The audio frames alone: no Info frame, so the length is only an
        // estimate from the file's size.
        let untagged = &whole[frame_at(&whole, 1)..];
        std::fs::write(&mp3, &untagged[..frame_at(untagged, 20)]).unwrap();
        let mut stream = MonoStream::open(&mp3).unwrap();
        assert_eq!(drain(&mut stream).unwrap(), 20 * 1152);
    }

    #[test]
    fn an_aac_file_in_mp4_cut_short_is_an_error_not_a_shorter_file() {
        let m4a = Path::new(env!("CARGO_MANIFEST_DIR")).join("../rbl-db/tests/fixtures/silent.m4a");
        let expected = MonoStream::open(&m4a).unwrap().expected_frames().unwrap();
        assert_eq!(drain(&mut MonoStream::open(&m4a).unwrap()).unwrap() as u64, expected);

        // symphonia reads every box of an MP4 to its stated end before it
        // hands out audio, so a file cut anywhere, before its sample table
        // or inside its audio, is refused when it is opened.
        let dir = tempfile::tempdir().unwrap();
        let cut = dir.path().join("cut.m4a");
        let whole = std::fs::read(&m4a).unwrap();
        let audio = whole.windows(4).position(|w| w == b"mdat").unwrap() + 4;
        std::fs::write(&cut, &whole[..audio + 12]).unwrap();
        assert!(MonoStream::open(&cut).is_err(), "a cut MP4 does not open");
    }

    #[test]
    fn an_mp3_is_decoded_on_rekordbox_s_timeline() {
        let dir = tempfile::tempdir().unwrap();
        let mp3 = lame_mp3(dir.path());
        assert_eq!(rbl_core::mpeg::rekordbox_lead_frames(&mut std::fs::File::open(&mp3).unwrap()), 1152);
        let raw = symphonia_mono(&mp3);
        let audio = decode_mono(&mp3, None).unwrap();
        // rekordbox plays the Info frame as a frame of silence; then the same
        // samples, a frame later.
        assert_eq!(audio.samples.len(), raw.len() + 1152);
        assert!(audio.samples[..1152].iter().all(|&s| s == 0.0));
        assert_eq!(&audio.samples[1152..], &raw[..]);
    }

    #[test]
    fn a_damaged_mp3_packet_keeps_its_place_in_silence() {
        let dir = tempfile::tempdir().unwrap();
        let mp3 = lame_mp3(dir.path());
        let whole = decode_mono(&mp3, None).unwrap().samples;
        let damaged = damaged_mp3(&mp3);
        let audio = decode_mono(&damaged, None).unwrap().samples;
        assert_eq!(audio.len(), whole.len(), "a dropped packet moves everything after it");
        // What follows the damage is where it was.
        let tail = whole.len() - 22_050;
        let drift = audio[tail..].iter().zip(&whole[tail..]).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(drift < 1e-6, "{drift}");
    }

    #[test]
    fn an_mp3_s_stereo_frames_open_with_the_silence_rekordbox_plays() {
        let dir = tempfile::tempdir().unwrap();
        let mp3 = lame_mp3(dir.path());
        let (mono, stereo) = frames_of(&mp3);
        assert_eq!(mono, decode_mono(&mp3, None).unwrap().samples);
        assert!(stereo[..1152].iter().all(|&frame| frame == [0, 0]));
        assert_in_step(&mono, &stereo);
    }

    #[test]
    fn an_mp3_s_stereo_frames_drop_what_rekordbox_does_not_play() {
        let dir = tempfile::tempdir().unwrap();
        let restarted = restarted_mp3(&lame_mp3(dir.path()));
        assert_eq!(rbl_core::mpeg::rekordbox_lead_frames(&mut std::fs::File::open(&restarted).unwrap()), -1152);
        let (mono, stereo) = frames_of(&restarted);
        assert_eq!(mono.len(), symphonia_mono(&restarted).len() - 1152);
        assert_in_step(&mono, &stereo);
    }

    #[test]
    fn a_damaged_mp3_packet_is_silence_in_the_stereo_frames_too() {
        let dir = tempfile::tempdir().unwrap();
        let mp3 = lame_mp3(dir.path());
        let (mono, stereo) = frames_of(&damaged_mp3(&mp3));
        assert_eq!(mono, decode_mono(&damaged_mp3(&mp3), None).unwrap().samples);
        assert_eq!(stereo.len(), frames_of(&mp3).1.len(), "the damaged packet keeps its place");
        // The Info frame's silence and two good frames, then the damaged one.
        assert!(stereo[3 * 1152..4 * 1152].iter().all(|&frame| frame == [0, 0]));
        assert_in_step(&mono, &stereo);
    }

    #[test]
    fn a_range_of_an_mp3_is_cut_on_rekordbox_s_timeline() {
        let dir = tempfile::tempdir().unwrap();
        let mp3 = lame_mp3(dir.path());
        let raw = symphonia_mono(&mp3);
        let out = dir.path().join("range.wav");
        // From 10 ms, inside the frame rekordbox plays as silence, to 50 ms.
        let frames = write_range_wav(&mp3, 0.01, 0.05, &out).unwrap();
        assert_eq!(frames, 2205 - 441);
        let cut = decode_mono(&out, None).unwrap().samples;
        let quiet = 1152 - 441;
        assert!(cut[..quiet].iter().all(|&s| s == 0.0));
        let drift = cut[quiet..].iter().zip(&raw).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(drift < 1e-3, "{drift}");
    }
}
