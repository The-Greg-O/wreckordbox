//! `source.json`: which file a set of Native stems was made from.
//!
//! Optional in the layout, and written here: it is what lets the deck tell
//! two songs with the same file name apart, and what lets a set be proven
//! to belong to its track instead of being taken by name alone.

use std::fmt::Write as _;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// The name of the record inside a track's folder.
pub(crate) const FILE_NAME: &str = "source.json";

/// The most bytes of the record the deck reads.
const RECORD_BYTES: usize = 4095;
/// A `file` that decodes to this many bytes or more makes the deck treat
/// the whole record as unknown.
const FILE_BYTES: usize = 1024;

/// The layout's fingerprint of a track file: 64-bit FNV-1a over the file's
/// size as decimal text and a newline, then up to 65536 bytes from its
/// start, from its middle and from its end. No audio is decoded.
pub(crate) fn fingerprint(path: &Path) -> std::io::Result<String> {
    const REGION: u64 = 65_536;
    let mut file = std::fs::File::open(path)?;
    let size = file.metadata()?.len();
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    feed(format!("{size}\n").as_bytes());
    let mut region = Vec::new();
    for offset in [
        0,
        (size / 2).saturating_sub(REGION / 2),
        size.saturating_sub(REGION),
    ] {
        file.seek(SeekFrom::Start(offset))?;
        region.clear();
        (&mut file).take(REGION).read_to_end(&mut region)?;
        feed(&region);
    }
    Ok(format!("fnv1a64:{hash:016x}"))
}

/// What a set was made from: the track file on the Stick.
pub(crate) struct Origin {
    /// Relative to the Stick's root, forward slashes, as the library holds it.
    pub(crate) file: String,
    pub(crate) size: u64,
    /// Seconds since the Unix epoch.
    pub(crate) mtime: i64,
    pub(crate) fingerprint: String,
}

impl Origin {
    /// Reads the track file at `on_disk`, which the Stick's library holds
    /// as `file`.
    pub(crate) fn of(file: &str, on_disk: &Path) -> std::io::Result<Self> {
        let metadata = std::fs::metadata(on_disk)?;
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(0));
        Ok(Self {
            file: file.to_owned(),
            size: metadata.len(),
            mtime,
            fingerprint: fingerprint(on_disk)?,
        })
    }

    /// The record as the deck's parser reads it: one flat JSON object on
    /// one line, ASCII only (anything else as `\uXXXX`, in surrogate pairs
    /// above U+FFFF), and in `file` no escape but `\"`, `\\` and `\uXXXX`.
    /// `file` is left out when it would be too long for the deck, which
    /// would otherwise ignore the size and fingerprint with it.
    pub(crate) fn record(&self) -> String {
        const OPENING: &str = "{\"file\": \"";
        const CLOSING: &str = "\", ";
        let mut file = String::new();
        for character in self.file.chars() {
            match character {
                '"' => file.push_str("\\\""),
                '\\' => file.push_str("\\\\"),
                ' '..='~' => file.push(character),
                other => {
                    for unit in other.encode_utf16(&mut [0; 2]) {
                        let _ = write!(file, "\\u{unit:04x}");
                    }
                }
            }
        }
        let rest = format!(
            "\"size\": {}, \"mtime\": {}, \"fp\": \"{}\"}}\n",
            self.size, self.mtime, self.fingerprint
        );
        let fits = self.file.len() < FILE_BYTES
            && !self.file.contains('\0')
            && OPENING.len() + file.len() + CLOSING.len() + rest.len() <= RECORD_BYTES;
        if fits {
            format!("{OPENING}{file}{CLOSING}{rest}")
        } else {
            format!("{{{rest}")
        }
    }
}

/// Whether the set in `folder` was made from this same track file, as far
/// as its record says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Held {
    /// The same file: the deck would take the folder as proven for this
    /// track.
    ThisTrack,
    /// Nothing says: the deck would load the folder for whatever track
    /// its name matches.
    Unknown,
    /// Another file: another song with the same file name.
    AnotherTrack,
}

/// Compares the record in `folder` with `origin` in the deck's order: the
/// path and size together, then the fingerprint, then either alone.
pub(crate) fn held(folder: &Path, origin: &Origin) -> Held {
    let Some(record) = read(folder) else {
        return Held::Unknown;
    };
    let file = record.get("file").and_then(serde_json::Value::as_str);
    let size = record.get("size").and_then(serde_json::Value::as_u64);
    let fp = record.get("fp").and_then(serde_json::Value::as_str);
    let same_file = file.map(|file| file.eq_ignore_ascii_case(&origin.file));
    let same_size = size.map(|size| size == origin.size);
    if same_file == Some(true) && same_size == Some(true) {
        return Held::ThisTrack;
    }
    if let Some(fp) = fp {
        return if fp == origin.fingerprint {
            Held::ThisTrack
        } else {
            Held::AnotherTrack
        };
    }
    if same_file == Some(true) || same_size == Some(true) {
        return Held::ThisTrack;
    }
    if file.is_some() || size.is_some() {
        return Held::AnotherTrack;
    }
    Held::Unknown
}

fn read(folder: &Path) -> Option<serde_json::Map<String, serde_json::Value>> {
    let record = folder.join(FILE_NAME);
    // The record itself, not something a link of its name leads to.
    if !std::fs::symlink_metadata(&record).is_ok_and(|metadata| metadata.is_file()) {
        return None;
    }
    let mut bytes = std::fs::read(record).ok()?;
    bytes.truncate(RECORD_BYTES);
    match serde_json::from_slice(&bytes).ok()? {
        serde_json::Value::Object(record) => Some(record),
        _ => None,
    }
}
