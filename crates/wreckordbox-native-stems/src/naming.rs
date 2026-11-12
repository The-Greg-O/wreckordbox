//! The names of the layout: a track's key, and the folder that carries its
//! Native stems.
//!
//! The deck matches a folder to a track by the track's **key**: the last
//! component of the path its library holds for the track, with everything
//! from the last `.` removed. It compares bytes, ignoring ASCII case only,
//! so the key must come from the library's spelling of the path and not
//! from a directory listing, which may spell the same name with another
//! Unicode normalization.

/// Why a key cannot have Native stems: no folder can carry its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Unnameable {
    /// Empty, or only spaces: nothing for the deck to match.
    #[error("its file name before the extension is empty")]
    Empty,
    /// The deck skips every name that starts with a dot.
    #[error("its file name starts with a dot, and the deck skips names that do")]
    StartsWithDot,
    /// A character FAT and exFAT cannot store.
    #[error("its file name holds {}, which a FAT or exFAT Stick cannot store in a folder name", shown(*.0))]
    Character(char),
    /// Longer than the deck's buffer for a file name, so the deck would
    /// work out another key.
    #[error("its file name is longer than the 255 bytes the deck keeps of one")]
    TooLong,
    /// The folder its Native stems would go in (the key, with the `_` or
    /// the `-XXXXXXXX` the layout adds) has a name longer than a Stick
    /// can store.
    #[error(
        "the stems folder it needs would have a name longer than the 255 bytes a Stick can store"
    )]
    FolderTooLong,
}

fn shown(character: char) -> String {
    if character.is_control() {
        format!("the control character U+{:04X}", u32::from(character))
    } else {
        format!("'{character}'")
    }
}

/// The most bytes of a file name the deck keeps (a 256-byte buffer).
const NAME_BYTES: usize = 255;

/// The key of the track at `path`, as the Stick's library holds it.
#[must_use]
pub fn track_key(path: &str) -> &str {
    let name = file_name(path);
    name.rfind('.').map_or(name, |dot| &name[..dot])
}

/// The last component of `path`.
#[must_use]
pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The folder that carries the Native stems of a track with this key: the
/// key itself, or the key and `_` when FAT and exFAT cannot store the key
/// as written (it ends in `.` or a space, or the part before its first `.`
/// is a Windows device name). Refused when no folder can carry it.
pub fn folder_name(key: &str) -> Result<String, Unnameable> {
    if key.chars().all(|c| c == ' ') {
        return Err(Unnameable::Empty);
    }
    if key.starts_with('.') {
        return Err(Unnameable::StartsWithDot);
    }
    if let Some(character) = key
        .chars()
        .find(|c| matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || *c < ' ')
    {
        return Err(Unnameable::Character(character));
    }
    if key.ends_with('.') || key.ends_with(' ') || is_device_name(key) {
        return Ok(format!("{key}_"));
    }
    Ok(key.to_owned())
}

/// Whether a track's file name is one the deck reads whole. A longer one
/// is cut short on the deck, which then looks for another key.
pub fn check_file_name(name: &str) -> Result<(), Unnameable> {
    if name.len() > NAME_BYTES {
        return Err(Unnameable::TooLong);
    }
    Ok(())
}

/// Whether a Stick can store a stems folder of this name: FAT, exFAT and
/// the deck's own buffer all stop at 255.
pub fn check_folder_name(folder: &str) -> Result<(), Unnameable> {
    if folder.len() > NAME_BYTES {
        return Err(Unnameable::FolderTooLong);
    }
    Ok(())
}

fn is_device_name(key: &str) -> bool {
    let stem = key.split('.').next().unwrap_or(key).trim_end_matches(' ');
    let upper = stem.to_ascii_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let numbered = upper
        .strip_prefix("COM")
        .or_else(|| upper.strip_prefix("LPT"));
    numbered.is_some_and(|digit| matches!(digit.as_bytes(), [b'1'..=b'9']))
}

/// The folder for a second song with the same file name: the key, `-`, and
/// the CRC-32 of the track's path relative to the Stick's root (UTF-8,
/// forward slashes) as 8 lowercase hex digits.
#[must_use]
pub fn suffixed_folder_name(key: &str, stick_relative_path: &str) -> String {
    format!(
        "{key}-{:08x}",
        crc32fast::hash(stick_relative_path.as_bytes())
    )
}
