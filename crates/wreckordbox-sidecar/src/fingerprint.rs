//! The fingerprint of a track's audio.
//!
//! A fingerprint names the audio, not the file: it is a hash of the encoded
//! audio inside the file (every packet of the file's audio stream, in
//! order, with the stream's sample rate and channel count), found by
//! demultiplexing the container without decoding anything. So:
//!
//! - moving or renaming the file changes nothing (no path, name or
//!   extension goes in);
//! - rewriting the file's tags or artwork changes nothing (they are not in
//!   the audio stream);
//! - any change to the audio changes it: a re-encode, a trim, a different
//!   master, a replacement file.
//!
//! Hashing the encoded packets rather than decoded samples keeps it fast
//! (one read of the file) and independent of decoder versions, which do not
//! always agree to the last sample.
//!
//! A file whose container is not recognised is hashed whole instead, which
//! still survives a move or a rename. The text form says which was done and
//! under which version of the scheme: `a1:<hex>` for the audio stream,
//! `f1:<hex>` for the whole file.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::{io, Result};

/// The fingerprint of a track's audio, as text: `a1:<64 hex>` or
/// `f1:<64 hex>`. Compare for equality; nothing else about it is promised.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Fingerprint(String);

impl Fingerprint {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A fingerprint read back from where one was stored.
    #[must_use]
    pub fn from_stored(text: String) -> Self {
        Self(text)
    }
}

impl std::fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What goes into every audio-stream hash first, so a change of scheme can
/// never collide with this one.
const AUDIO_DOMAIN: &[u8] = b"wreckordbox audio fingerprint 1\0";

/// Fingerprints the audio in the file at `path`. Reads the whole file once.
pub fn fingerprint_file(path: &Path) -> Result<Fingerprint> {
    if let Some(audio) = audio_stream_hash(path)? {
        return Ok(Fingerprint(format!("a1:{}", audio.to_hex())));
    }
    let mut file = File::open(path).map_err(io("could not open", path))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(io("could not read", path))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(Fingerprint(format!("f1:{}", hasher.finalize().to_hex())))
}

/// The hash of the file's audio stream, or `None` when the file cannot be
/// read as a container this build knows, start to finish.
fn audio_stream_hash(path: &Path) -> Result<Option<blake3::Hash>> {
    let file = File::open(path).map_err(io("could not open", path))?;
    let stream = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
    // No hint from the file's name: a rename must not change the answer.
    let Ok(probed) = symphonia::default::get_probe().format(
        &Hint::new(),
        stream,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    ) else {
        return Ok(None);
    };
    let mut format = probed.format;
    let Some(track) = format.default_track() else {
        return Ok(None);
    };
    let track_id = track.id;
    let mut hasher = blake3::Hasher::new();
    hasher.update(AUDIO_DOMAIN);
    hasher.update(&track.codec_params.sample_rate.unwrap_or(0).to_le_bytes());
    let channels = track
        .codec_params
        .channels
        .map_or(0, symphonia::core::audio::Channels::count);
    hasher.update(&u32::try_from(channels).unwrap_or(u32::MAX).to_le_bytes());
    let mut packets = 0_u64;
    loop {
        match format.next_packet() {
            Ok(packet) if packet.track_id() == track_id => {
                hasher.update(&packet.data);
                packets += 1;
            }
            Ok(_) => {}
            // How this reader says the stream has ended.
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            // Damaged part-way: not something to half-hash.
            Err(_) => return Ok(None),
        }
    }
    Ok((packets > 0).then(|| hasher.finalize()))
}
