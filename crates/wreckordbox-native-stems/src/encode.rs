//! One Stem, from the Sidecar's audio file to the Ogg Vorbis the deck reads.
//!
//! The layout asks for Ogg Vorbis at 44100 Hz, stereo, any libvorbis
//! quality, with frame 0 of the Stem being frame 0 of the track and no
//! trimming, padding or fades. Vorbis keeps a stream's exact length in its
//! granule positions, so the encoded Stem decodes to exactly as many sample
//! frames as went in, starting at the same frame.

use std::fs::File;
use std::io::BufWriter;
use std::num::{NonZeroU32, NonZeroU8};
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use vorbis_rs::{VorbisBitrateManagementStrategy, VorbisEncoderBuilder};

use crate::NativeStemsError;

/// The sample rate the deck plays without resampling.
pub const SAMPLE_RATE: u32 = 44_100;
/// The libvorbis quality written, on libvorbis's -1 to 10 scale: what the
/// AZ-MODS app writes at its default Stem Quality.
pub const VORBIS_QUALITY: u8 = 5;

fn audio(path: &Path, what: impl std::fmt::Display) -> NativeStemsError {
    NativeStemsError::Audio {
        file: path.to_path_buf(),
        message: what.to_string(),
    }
}

fn encoding(path: &Path, what: impl std::fmt::Display) -> NativeStemsError {
    NativeStemsError::Encode {
        file: path.to_path_buf(),
        message: what.to_string(),
    }
}

/// Encodes the audio file `source` as stereo Ogg Vorbis at `to`, flushed to
/// the disk, and answers how many sample frames it holds. Mono is written
/// to both sides; of more than two channels, the first two are kept.
pub(crate) fn encode(source: &Path, to: &Path) -> Result<u64, NativeStemsError> {
    let file = File::open(source).map_err(|e| audio(source, e))?;
    let stream = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
    let mut hint = Hint::new();
    if let Some(extension) = source.extension().and_then(|e| e.to_str()) {
        hint.with_extension(extension);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| audio(source, e))?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| audio(source, "it holds no audio"))?;
    let track_id = track.id;
    let rate = track.codec_params.sample_rate.unwrap_or(0);
    if rate != SAMPLE_RATE {
        return Err(audio(
            source,
            format!("its sample rate is {rate} Hz; Native stems are written at {SAMPLE_RATE} Hz and nothing here resamples"),
        ));
    }
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| audio(source, e))?;

    let out = File::create(to).map_err(|e| NativeStemsError::io(to, e))?;
    let (rate, two) = (
        NonZeroU32::new(SAMPLE_RATE).ok_or_else(|| encoding(to, "no sample rate"))?,
        NonZeroU8::new(2).ok_or_else(|| encoding(to, "no channels"))?,
    );
    let mut encoder = VorbisEncoderBuilder::new(rate, two, BufWriter::new(out))
        .map_err(|e| encoding(to, e))?
        .bitrate_management_strategy(VorbisBitrateManagementStrategy::QualityVbr {
            target_quality: f32::from(VORBIS_QUALITY) / 10.0,
        })
        .build()
        .map_err(|e| encoding(to, e))?;

    let mut frames = 0_u64;
    let mut samples: Option<SampleBuffer<f32>> = None;
    let (mut left, mut right) = (Vec::new(), Vec::new());
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break
            }
            Err(e) => return Err(audio(source, e)),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let audio_buffer = decoder.decode(&packet).map_err(|e| audio(source, e))?;
        let channels = audio_buffer.spec().channels.count();
        if channels == 0 {
            return Err(audio(source, "it has no channels"));
        }
        let buffer = samples.get_or_insert_with(|| {
            SampleBuffer::new(audio_buffer.capacity() as u64, *audio_buffer.spec())
        });
        buffer.copy_interleaved_ref(audio_buffer);
        left.clear();
        right.clear();
        for frame in buffer.samples().chunks_exact(channels) {
            let first = frame.first().copied().unwrap_or(0.0);
            left.push(first);
            right.push(frame.get(1).copied().unwrap_or(first));
        }
        if left.is_empty() {
            continue;
        }
        frames += left.len() as u64;
        encoder
            .encode_audio_block([&left, &right])
            .map_err(|e| encoding(to, e))?;
    }
    let out = encoder.finish().map_err(|e| encoding(to, e))?;
    let out = out
        .into_inner()
        .map_err(|e| NativeStemsError::io(to, e.into_error()))?;
    out.sync_all().map_err(|e| NativeStemsError::io(to, e))?;
    Ok(frames)
}

/// How long the audio file at `path` is by its container: the sample
/// frames of every packet of its audio stream, added up without decoding
/// any, and its sample rate. For a lossy file that counts the codec's own
/// padding, which a decoder may or may not hand back. `None` when the
/// file cannot be read as audio or does not say.
#[must_use]
pub fn container_frames(path: &Path) -> Option<(u64, u32)> {
    let file = File::open(path).ok()?;
    let stream = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
    let mut format = symphonia::default::get_probe()
        .format(
            &Hint::new(),
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .ok()?
        .format;
    let track = format.default_track()?;
    let (track_id, rate) = (track.id, track.codec_params.sample_rate?);
    let mut frames = 0_u64;
    loop {
        match format.next_packet() {
            Ok(packet) if packet.track_id() == track_id => frames += packet.dur(),
            Ok(_) => {}
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break
            }
            Err(_) => return None,
        }
    }
    (frames > 0 && rate > 0).then_some((frames, rate))
}
