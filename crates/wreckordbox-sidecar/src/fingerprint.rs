//! The fingerprint of a track's audio.
//!
//! A fingerprint names the audio, not the file: it is a hash of the encoded
//! audio inside the file (every packet of the file's audio stream, in
//! order, with the stream's codec, sample format and decoding parameters), found by
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
//! (normally one audio pass) and independent of decoder versions, which do not
//! always agree to the last sample.
//!
//! A file whose container is not recognised is hashed whole instead, which
//! still survives a move or a rename. The text form says which was done and
//! under which version of the scheme: `a2:<decoding hash>` for the audio stream,
//! `f1:<hex>` for the whole file.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde::{Deserialize, Serialize};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::{io, Result};

/// The fingerprint of a track's audio. New stream fingerprints are
/// `a2:<64 hex>`. A freshly read value also carries ephemeral a1 evidence,
/// never serialized: that old digest can include AIFF/FLAC trailing tags.
/// Stored a1 and f1 fingerprints remain supported; use `matches_stored`
/// to verify older stored evidence. Equality and hashing use only identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Fingerprint {
    identity: String,
    #[serde(skip)]
    legacy: Option<String>,
}
impl PartialEq for Fingerprint {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}
impl Eq for Fingerprint {}
impl std::hash::Hash for Fingerprint {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.identity.hash(state);
    }
}

impl Fingerprint {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.identity
    }

    /// A fingerprint read back from where one was stored.
    #[must_use]
    pub fn from_stored(text: String) -> Self {
        Self {
            identity: text,
            legacy: None,
        }
    }

    /// Compare freshly observed audio with the version of stored evidence.
    /// This deliberately is NOT Eq: two a2 identities can have the same a1
    /// digest while decoding differently. Never use compatibility as a key.
    #[must_use]
    pub fn matches_stored(&self, stored: &Self) -> bool {
        self == stored
            || ((stored.identity.starts_with("a1:") || stored.identity.starts_with("f1:"))
                && self.legacy() == *stored)
    }

    /// The original algorithm's digest carried by a freshly observed value.
    /// Deserialized/stored fingerprints carry only their own identity.
    #[must_use]
    pub fn legacy(&self) -> Self {
        self.legacy
            .as_ref()
            .map_or_else(|| self.clone(), |old| Self::from_stored(old.clone()))
    }
}

impl std::fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.identity)
    }
}

/// What goes into every audio-stream hash first, so a change of scheme can
/// never collide with this one.
const AUDIO_DOMAIN: &[u8] = b"wreckordbox audio fingerprint 1\0";

/// Fingerprints the audio in the file at `path`. Ordinarily one audio pass;
/// raw FLAC/M4A trailers require an additional original-view legacy pass.
pub fn fingerprint_file(path: &Path) -> Result<Fingerprint> {
    let before = crate::stems::file_stamp(path);
    let result = read_fingerprint(path)?;
    let mut cache = fingerprints();
    match (before, crate::stems::file_stamp(path)) {
        (Some(before), Some(after)) if before == after => {
            if cache.len() >= 4096 {
                cache.clear();
            }
            cache.insert(path.to_path_buf(), (after, result.clone()));
        }
        _ => {
            cache.remove(path);
        }
    }
    Ok(result)
}

type Fingerprints =
    std::collections::HashMap<std::path::PathBuf, (crate::stems::FileStamp, Fingerprint)>;
fn fingerprints() -> std::sync::MutexGuard<'static, Fingerprints> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Fingerprints>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Last verified fingerprint, only while size, modification time and inode match.
/// Does not open the audio, or write any observation to the Sidecar.
#[must_use]
pub fn known_fingerprint(path: &Path) -> Option<Fingerprint> {
    let now = crate::stems::file_stamp(path)?;
    fingerprints()
        .get(path)
        .filter(|(then, _)| *then == now)
        .map(|(_, fp)| fp.clone())
}

/// Verify on cache miss. Renders/Stick loads still use `fingerprint_file` freshly.
pub fn fingerprint_cached(path: &Path) -> Result<Fingerprint> {
    known_fingerprint(path).map_or_else(|| fingerprint_file(path), Ok)
}

fn read_fingerprint(path: &Path) -> Result<Fingerprint> {
    let mut file = File::open(path).map_err(io("could not open", path))?;
    let end =
        crate::fingerprint_bounds::stream_end(&mut file).map_err(io("could not inspect", path))?;
    if let Some((audio, old)) = audio_stream_hash(path, end)? {
        let legacy = if end.is_some() {
            // The original demuxer can omit the last FLAC frame or reject an
            // M4A with trailers. Preserve precisely that old evidence rule.
            match audio_stream_hash(path, None)? {
                Some((_, old)) => format!("a1:{}", old.to_hex()),
                None => whole_file(path)?,
            }
        } else {
            format!("a1:{}", old.to_hex())
        };
        return Ok(Fingerprint {
            identity: format!("a2:{}", audio.to_hex()),
            legacy: Some(legacy),
        });
    }
    Ok(Fingerprint::from_stored(whole_file(path)?))
}
fn whole_file(path: &Path) -> Result<String> {
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
    Ok(format!("f1:{}", hasher.finalize().to_hex()))
}

/// The hash of the file's audio stream, or `None` when the file cannot be
/// read as a container this build knows, start to finish.
fn audio_stream_hash(
    path: &Path,
    end: Option<u64>,
) -> Result<Option<(blake3::Hash, blake3::Hash)>> {
    let mut file = File::open(path).map_err(io("could not open", path))?;
    // Read only chunk/footer headers before demuxing. Legacy still sees the
    // original packets; only a2 excludes bytes outside the audio payload.
    let mut remaining =
        crate::fingerprint_bounds::audio_bytes(&mut file).map_err(io("could not inspect", path))?;
    file.seek(SeekFrom::Start(0))
        .map_err(io("could not seek", path))?;
    let source: Box<dyn symphonia::core::io::MediaSource> = match end {
        Some(end) => Box::new(crate::fingerprint_bounds::Bounded { file, end }),
        None => Box::new(file),
    };
    let stream = MediaSourceStream::new(source, MediaSourceStreamOptions::default());
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
    let mut current = crate::fingerprint_format::hash(&track.codec_params);
    let mut packets = 0_u64;
    loop {
        match format.next_packet() {
            Ok(packet) if packet.track_id() == track_id => {
                hasher.update(&packet.data);
                let take = remaining.map_or(packet.data.len(), |left| {
                    usize::try_from(left.min(packet.data.len() as u64)).unwrap_or(packet.data.len())
                });
                if let Some(left) = &mut remaining {
                    *left -= take as u64;
                }
                if take > 0 {
                    current.update(&packet.trim_start().to_le_bytes());
                    current.update(&packet.trim_end().to_le_bytes());
                    current.update(&(take as u64).to_le_bytes());
                    current.update(&packet.data[..take]);
                }
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
    Ok((packets > 0).then(|| (current.finalize(), hasher.finalize())))
}
