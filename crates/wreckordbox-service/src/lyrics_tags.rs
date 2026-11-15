//! Lyrics already in an audio file's own tags, read and never written.
//!
//! Where each format keeps them:
//!
//! - ID3v2 (MP3, AIFF's `ID3 ` chunk, WAV's `id3 ` chunk, and the rare
//!   ID3v2 in FLAC): `SYLT` (synced, with millisecond times) is preferred
//!   to `USLT` (unsynced). A `SYLT` timed in MPEG frames is taken as plain
//!   lines, since its times cannot be read without decoding.
//! - Vorbis comments (FLAC, Ogg Vorbis, Opus): `LYRICS`, else
//!   `UNSYNCEDLYRICS`.
//! - MP4 / M4A: the `©lyr` atom.
//!
//! Only reading: the file is opened read-only and nothing is written back.

use std::fs::File;
use std::path::Path;

use lofty::config::ParseOptions;
use lofty::file::{AudioFile, FileType};
use lofty::id3::v2::{Frame, Id3v2Tag, SynchronizedTextFrame, TimestampFormat};
use lofty::mp4::{AtomData, AtomIdent, Mp4File};
use lofty::ogg::{OpusFile, VorbisComments, VorbisFile};
use wreckordbox_sidecar::lyrics::SyncedLine;

/// Lyrics found in a file's tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Embedded {
    pub(crate) text: String,
    pub(crate) lines: Vec<SyncedLine>,
    pub(crate) language: Option<String>,
    /// The tag they came from, such as `ID3v2 USLT`.
    pub(crate) tag: &'static str,
}

/// The lyrics in the file at `path`; `None` when its tags hold none.
///
/// # Errors
///
/// The file cannot be opened or its format or tags cannot be read, in
/// words for the DJ.
pub(crate) fn read(path: &Path) -> Result<Option<Embedded>, String> {
    let open = || File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()));
    let failed = |e: lofty::error::LoftyError| format!("cannot read the tags of {}: {e}", path.display());
    let options = ParseOptions::new().read_properties(false);
    let kind = lofty::probe::Probe::new(std::io::BufReader::new(open()?))
        .guess_file_type()
        .map_err(|e| failed(e.into()))?
        .file_type();
    let found = match kind {
        Some(FileType::Mpeg) => {
            let file = lofty::mpeg::MpegFile::read_from(&mut open()?, options).map_err(failed)?;
            file.id3v2().and_then(from_id3)
        }
        Some(FileType::Aiff) => {
            let file = lofty::iff::aiff::AiffFile::read_from(&mut open()?, options).map_err(failed)?;
            file.id3v2().and_then(from_id3)
        }
        Some(FileType::Wav) => {
            let file = lofty::iff::wav::WavFile::read_from(&mut open()?, options).map_err(failed)?;
            file.id3v2().and_then(from_id3)
        }
        Some(FileType::Flac) => {
            let file = lofty::flac::FlacFile::read_from(&mut open()?, options).map_err(failed)?;
            file.vorbis_comments()
                .and_then(from_vorbis)
                .or_else(|| file.id3v2().and_then(from_id3))
        }
        Some(FileType::Vorbis) => {
            let file = VorbisFile::read_from(&mut open()?, options).map_err(failed)?;
            from_vorbis(file.vorbis_comments())
        }
        Some(FileType::Opus) => {
            let file = OpusFile::read_from(&mut open()?, options).map_err(failed)?;
            from_vorbis(file.vorbis_comments())
        }
        Some(FileType::Mp4) => {
            let file = Mp4File::read_from(&mut open()?, options).map_err(failed)?;
            file.ilst().and_then(|ilst| {
                let atom = ilst.get(&AtomIdent::Fourcc(*b"\xa9lyr"))?;
                let text = atom.data().find_map(|data| match data {
                    AtomData::UTF8(text) | AtomData::UTF16(text) => Some(text.clone()),
                    _ => None,
                })?;
                plain(text, None, "MP4 ©lyr")
            })
        }
        _ => None,
    };
    Ok(found)
}

/// Words with something in them, or nothing.
fn plain(text: String, language: Option<String>, tag: &'static str) -> Option<Embedded> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    (!text.trim().is_empty()).then(|| Embedded {
        text: text.trim().to_owned(),
        lines: Vec::new(),
        language,
        tag,
    })
}

/// An ID3 language code, unless it says "unknown".
fn language(code: [u8; 3]) -> Option<String> {
    let text = String::from_utf8_lossy(&code).trim().to_ascii_lowercase();
    let known = text.len() == 3
        && text.chars().all(|c| c.is_ascii_alphabetic())
        && !matches!(text.as_str(), "xxx" | "und" | "zxx");
    known.then_some(text)
}

fn from_id3(tag: &Id3v2Tag) -> Option<Embedded> {
    for frame in tag {
        let Frame::Binary(binary) = frame else {
            continue;
        };
        if binary.id().as_str() != "SYLT" {
            continue;
        }
        let Ok(synced) = SynchronizedTextFrame::parse(&binary.data, binary.flags()) else {
            continue;
        };
        let lines: Vec<SyncedLine> = synced
            .content
            .iter()
            .map(|(at, text)| SyncedLine {
                at_ms: *at,
                text: text.trim_matches(['\n', '\r']).to_owned(),
            })
            .filter(|line| !line.text.trim().is_empty())
            .collect();
        if lines.is_empty() {
            continue;
        }
        let text = lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let code = language(synced.language);
        if synced.timestamp_format != TimestampFormat::MS {
            return plain(text, code, "ID3v2 SYLT");
        }
        return Some(Embedded {
            text,
            lines,
            language: code,
            tag: "ID3v2 SYLT",
        });
    }
    tag.unsync_text()
        .find(|frame| !frame.content.trim().is_empty())
        .and_then(|frame| plain(frame.content.clone(), language(frame.language), "ID3v2 USLT"))
}

fn from_vorbis(comments: &VorbisComments) -> Option<Embedded> {
    [("LYRICS", "Vorbis LYRICS"), ("UNSYNCEDLYRICS", "Vorbis UNSYNCEDLYRICS")]
        .into_iter()
        .find_map(|(key, tag)| {
            comments
                .get_all(key)
                .find(|text| !text.trim().is_empty())
                .and_then(|text| plain(text.to_owned(), None, tag))
        })
}
