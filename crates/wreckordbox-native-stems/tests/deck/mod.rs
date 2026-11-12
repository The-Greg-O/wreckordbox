//! An independent reader of Native stems, standing in for the deck.
//!
//! Written from the published AZ-MODS stems layout alone (the text kept in
//! the notes as `az-mods/STEMS-FORMAT.md`), section by section, and sharing
//! no code with the writer under test: not its naming rules, not its
//! fingerprint, not its Vorbis library (this decodes with `lewton`, a
//! decoder written in Rust; the writer encodes with libvorbis). A set this
//! reader loads is a set written to the layout, not merely one the writer
//! can read back.
//!
//! It does what the layout says the deck does on a track load, and what it
//! says the AZ-MODS app looks for; it does not model the deck's memory
//! floor, its offset measurement or its envelope cache.
#![allow(
    dead_code,
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// "Identity" of a folder against the loaded track file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Identity {
    Proven,
    Unknown,
    DifferentFile,
}

/// One stem file as decoded.
#[derive(Debug, Clone)]
pub struct Decoded {
    pub file_name: String,
    pub sample_rate: u32,
    pub channels: u8,
    /// Sample frames.
    pub frames: u64,
    /// Interleaved samples.
    pub samples: Vec<i16>,
}

impl Decoded {
    /// The largest absolute sample in the frames `from..to`.
    pub fn peak(&self, from: u64, to: u64) -> i16 {
        let channels = self.channels as usize;
        self.samples[from as usize * channels..to as usize * channels]
            .iter()
            .map(|s| s.saturating_abs())
            .max()
            .unwrap_or(0)
    }
}

/// The folder the deck chose for a track, and what it found in it.
#[derive(Debug, Clone)]
pub struct Loaded {
    /// The chosen folder.
    pub folder: PathBuf,
    /// 0 `<key>`, 1 `<key>_`, 2 `<key>-XXXXXXXX`.
    pub rank: u8,
    pub identity: Identity,
    /// The stems that decoded, by pad: 1 vocals, 2 other, 3 bass, 4 drums.
    pub pads: BTreeMap<u8, Decoded>,
}

impl Loaded {
    /// "The deck uses the length of the shortest stem it decoded."
    pub fn frames(&self) -> u64 {
        self.pads.values().map(|d| d.frames).min().unwrap_or(0)
    }

    /// "At least one file decodes: the deck plays stems."
    pub fn plays_stems(&self) -> bool {
        !self.pads.is_empty()
    }
}

/// "The track key": the last component of the track's path, with
/// everything from the last `.` removed.
pub fn track_key(track_path: &str) -> String {
    let name = track_path.rsplit('/').next().unwrap();
    match name.rfind('.') {
        Some(dot) => name[..dot].to_owned(),
        None => name.to_owned(),
    }
}

/// Comparison is "bytes, ignoring ASCII case only".
fn same(a: &str, b: &str) -> bool {
    a.as_bytes().eq_ignore_ascii_case(b.as_bytes())
}

/// "Folder names": the rank at which a folder name belongs to a key, if it
/// does. "Nothing else matches."
pub fn folder_rank(folder: &str, key: &str) -> Option<u8> {
    if same(folder, key) {
        return Some(0);
    }
    let (f, k) = (folder.as_bytes(), key.as_bytes());
    if f.len() == k.len() + 1 && f[..k.len()].eq_ignore_ascii_case(k) && f[k.len()] == b'_' {
        return Some(1);
    }
    if f.len() == k.len() + 9
        && f[..k.len()].eq_ignore_ascii_case(k)
        && f[k.len()] == b'-'
        && f[k.len() + 1..].iter().all(u8::is_ascii_hexdigit)
    {
        return Some(2);
    }
    None
}

/// The `STEMS` directories of one mounted USB: "a directory at the root of
/// the USB, name matched ignoring ASCII case"; dot names are skipped.
fn stems_dirs(usb: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(usb) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            !name.starts_with('.') && same(&name, "STEMS")
        })
        .map(|e| e.path())
        .collect()
}

/// "When stems are active": a non-empty `STEMS/` (at least one entry not
/// starting with `.`) on any mounted USB.
pub fn stems_active(usbs: &[&Path]) -> bool {
    usbs.iter().flat_map(|usb| stems_dirs(usb)).any(|dir| {
        std::fs::read_dir(dir).is_ok_and(|entries| {
            entries
                .filter_map(Result::ok)
                .any(|e| !e.file_name().to_string_lossy().starts_with('.'))
        })
    })
}

/// "Fingerprint": 64-bit FNV-1a over the size line and three regions.
pub fn fingerprint(path: &Path) -> String {
    let data = std::fs::read(path).unwrap();
    let size = data.len();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    feed(format!("{size}\n").as_bytes());
    for offset in [
        0,
        (size / 2).saturating_sub(32_768),
        size.saturating_sub(65_536),
    ] {
        let end = (offset + 65_536).min(size);
        feed(&data[offset..end]);
    }
    format!("fnv1a64:{hash:016x}")
}

/// A `source.json` as the deck's parser reads it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SourceRecord {
    pub file: Option<String>,
    pub size: Option<u64>,
    pub mtime: Option<i64>,
    pub fp: Option<String>,
}

/// Reads `source.json` under the deck's constraints: a single flat JSON
/// object, the first 4095 bytes, non-ASCII escaped as `\uXXXX`, and in
/// `file` only the escapes `\"`, `\\`, `\/` and `\uXXXX` (never `\u0000`),
/// decoding to fewer than 1024 bytes. Anything else "makes the record
/// unknown": `None`.
pub fn read_source_record(folder: &Path) -> Option<SourceRecord> {
    let mut bytes = std::fs::read(folder.join("source.json")).ok()?;
    bytes.truncate(4095);
    if !bytes.is_ascii() {
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let object = value.as_object()?;
    if object.values().any(|v| v.is_object() || v.is_array()) {
        return None;
    }
    let mut record = SourceRecord::default();
    if let Some(file) = object.get("file") {
        let file = file.as_str()?;
        if file.len() >= 1024 || file.contains('\0') {
            return None;
        }
        // The raw text of the value may hold only the four escapes.
        let start = text.find("\"file\"")? + 6;
        let raw = &text[start..];
        let open = raw.find('"')? + 1;
        let mut chars = raw[open..].chars();
        loop {
            match chars.next()? {
                '"' => break,
                '\\' => match chars.next()? {
                    '"' | '\\' | '/' => {}
                    'u' => {
                        let hex: String = chars.by_ref().take(4).collect();
                        if hex.len() != 4 || u16::from_str_radix(&hex, 16).ok()? == 0 {
                            return None;
                        }
                    }
                    _ => return None,
                },
                _ => {}
            }
        }
        record.file = Some(file.to_owned());
    }
    record.size = object.get("size").and_then(serde_json::Value::as_u64);
    record.mtime = object.get("mtime").and_then(serde_json::Value::as_i64);
    record.fp = object
        .get("fp")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    Some(record)
}

/// "How identity is decided": the first step that applies gives the answer.
pub fn identity(folder: &Path, track_file: &Path) -> Identity {
    let Ok(metadata) = std::fs::metadata(track_file) else {
        // "When the deck cannot read the track file's size, every folder
        // is unknown."
        return Identity::Unknown;
    };
    let Some(record) = read_source_record(folder) else {
        return Identity::Unknown;
    };
    let size_equal = record.size.map(|size| size == metadata.len());
    // "`file` matches" on the deck: the track's absolute path ends with
    // `/` followed by `file`, ignoring ASCII case.
    let file_matches = record.file.as_ref().map(|file| {
        let absolute = track_file.to_string_lossy().into_owned();
        let wanted = format!("/{file}");
        absolute.len() >= wanted.len()
            && absolute.as_bytes()[absolute.len() - wanted.len()..]
                .eq_ignore_ascii_case(wanted.as_bytes())
    });
    // Step 1.
    if file_matches == Some(true) && size_equal == Some(true) {
        return Identity::Proven;
    }
    // Step 2.
    if let Some(fp) = &record.fp {
        return if *fp == fingerprint(track_file) {
            Identity::Proven
        } else {
            Identity::DifferentFile
        };
    }
    // Step 3.
    if size_equal == Some(true) || file_matches == Some(true) {
        return Identity::Proven;
    }
    // Step 4.
    if record.size.is_some() || record.file.is_some() {
        return Identity::DifferentFile;
    }
    // Step 5.
    Identity::Unknown
}

/// "Stem files": the role of a file by the substrings in its name, tested
/// in order; `None` for a file the deck ignores.
pub fn pad_of(file_name: &str) -> Option<u8> {
    let lower = file_name.to_ascii_lowercase();
    if file_name.starts_with('.') || !lower.ends_with(".ogg") {
        return None;
    }
    if lower.contains("vocal") {
        Some(1)
    } else if lower.contains("instrument") || lower.contains("inst") || lower.contains("other") {
        Some(2)
    } else if lower.contains("bass") {
        Some(3)
    } else if lower.contains("drum") {
        Some(4)
    } else {
        None
    }
}

/// Decodes an Ogg Vorbis file whole, as the deck does with its own decoder.
pub fn decode(path: &Path) -> Option<Decoded> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = lewton::inside_ogg::OggStreamReader::new(file).ok()?;
    let channels = reader.ident_hdr.audio_channels;
    let sample_rate = reader.ident_hdr.audio_sample_rate;
    let mut samples = Vec::new();
    while let Some(packet) = reader.read_dec_packet_itl().ok()? {
        samples.extend_from_slice(&packet);
    }
    Some(Decoded {
        file_name: path.file_name()?.to_string_lossy().into_owned(),
        sample_rate,
        channels,
        frames: (samples.len() / channels as usize) as u64,
        samples,
    })
}

/// The stems in a folder: directory order, the first file of each role,
/// roles whose file fails to decode left out.
fn pads(folder: &Path) -> BTreeMap<u8, Decoded> {
    let mut found = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(folder) else {
        return found;
    };
    let mut first: BTreeMap<u8, PathBuf> = BTreeMap::new();
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(pad) = pad_of(&name) {
            first.entry(pad).or_insert_with(|| entry.path());
        }
    }
    for (pad, path) in first {
        if let Some(decoded) = decode(&path) {
            found.insert(pad, decoded);
        }
    }
    found
}

/// A track load. `usbs` are the mounted USBs, `track_file` is the track's
/// absolute path on one of them, spelled as the deck's library holds it.
/// Answers the folder the deck chooses ("How the deck chooses a folder")
/// and the stems it decodes from it, or `None` when no folder is loaded.
pub fn load(usbs: &[&Path], track_file: &Path) -> Option<Loaded> {
    if !stems_active(usbs) {
        return None;
    }
    let key = track_key(&track_file.to_string_lossy());
    // (identity, other USB, rank), smaller first; ties keep the first found.
    let mut best: Option<((Identity, bool, u8), PathBuf)> = None;
    for usb in usbs {
        let same_usb = track_file.starts_with(usb);
        for stems in stems_dirs(usb) {
            let Ok(entries) = std::fs::read_dir(&stems) else {
                continue;
            };
            for entry in entries.filter_map(Result::ok) {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || !entry.path().is_dir() {
                    continue;
                }
                let Some(rank) = folder_rank(&name, &key) else {
                    continue;
                };
                let identity = identity(&entry.path(), track_file);
                if identity == Identity::DifferentFile {
                    continue;
                }
                let order = (identity, !same_usb, rank);
                if best.as_ref().is_none_or(|(held, _)| order < *held) {
                    best = Some((order, entry.path()));
                }
            }
        }
    }
    let ((identity, _, rank), folder) = best?;
    // "The deck does not check that a folder is complete before choosing."
    let pads = pads(&folder);
    Some(Loaded {
        folder,
        rank,
        identity,
        pads,
    })
}

/// What the AZ-MODS app counts as a finished set: a file named `vocals`,
/// `other`, `bass` and `drums` (any extension).
pub fn counts_as_made(folder: &Path) -> bool {
    let stems: Vec<String> = std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter_map(|e| {
                    Path::new(&e.file_name())
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                })
                .collect()
        })
        .unwrap_or_default();
    ["vocals", "other", "bass", "drums"]
        .iter()
        .all(|role| stems.iter().any(|stem| stem == role))
}

/// Every file under `dir`, relative, with its bytes: for showing that a
/// folder was not touched.
pub fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(&path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}
