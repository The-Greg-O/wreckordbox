//! The Stick: whether it may be written to, and what its exported library
//! holds for a track.

use std::path::{Path, PathBuf};

use rbl_export::device_library::{self, Format, StickFiles};

use crate::NativeStemsError;

/// The file at a Stick's root that makes it a Spare stick: one the DJ has
/// declared safe to overwrite. An empty file; only its name matters.
///
/// It is the marker upstream's real-stick tests already ask for before they
/// write to a stick, so one declaration covers both and there is one name
/// to keep off a Gig stick.
pub const SPARE_STICK_MARKER: &str = ".rbxport-disposable";

/// Whether the Stick at `stick` carries the Spare stick marker.
#[must_use]
pub fn is_spare_stick(stick: &Path) -> bool {
    std::fs::symlink_metadata(stick.join(SPARE_STICK_MARKER)).is_ok_and(|marker| marker.is_file())
}

/// A track as the Stick's exported library holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exported {
    /// The path the library holds, relative to the Stick's root, with
    /// forward slashes and no leading one. Spelled as the library spells
    /// it, which a directory listing may not.
    pub path: String,
    /// The track's file on this machine: on the Stick, if it is there at
    /// all (see [`exported`]).
    pub file: PathBuf,
}

/// The export of the Library's track `track_id` on the Stick.
///
/// A Stick's library names, for each track, the Library it was exported
/// from and the track's identifier there. A row that names another Library
/// is another track, whatever its identifier; a row that names none is
/// taken by the identifier alone. The Device Library Plus database is read
/// first (it is the one an XDJ-AZ reads), then the older one, whose rows
/// often name no Library: such a row is not taken when the other database
/// says the same file came from another Library.
///
/// The path that library holds is not trusted to stay on the Stick: one
/// that climbs out with `..`, or that reaches its file through a symbolic
/// link (or names one), is refused as [`NativeStemsError::OffStick`]. A
/// file that is there is a regular file under the Stick's root; one that
/// is not there is for the caller to find missing.
///
/// Like [`write`](crate::write), this reads nothing of a Stick that does
/// not carry the Spare stick marker.
pub fn exported(
    stick: &Path,
    library_id: u64,
    track_id: u64,
) -> Result<Exported, NativeStemsError> {
    if !is_spare_stick(stick) {
        return Err(NativeStemsError::NotSpareStick {
            stick: stick.to_path_buf(),
        });
    }
    let formats = device_library::formats(stick);
    if formats.is_empty() {
        return Err(NativeStemsError::NoLibrary {
            stick: stick.to_path_buf(),
        });
    }
    let mut unreadable = None;
    let mut foreign: Vec<String> = Vec::new();
    for format in [Format::OneLibrary, Format::DeviceLibrary] {
        if !formats.contains(&format) {
            continue;
        }
        // Read without settling anything (wbx-45.5's reader): a Stick whose
        // last export did not finish is refused, never recovered here.
        let library = match wreckordbox_stick_outline::library(stick, format) {
            Ok(library) => library,
            Err(wreckordbox_stick_outline::ReadError::UnfinishedWrite) => {
                return Err(NativeStemsError::UnfinishedWrite { stick: stick.to_path_buf() });
            }
            Err(error) => {
                unreadable = Some(error.to_string());
                continue;
            }
        };
        let mut found = None;
        for track in &library.tracks {
            if track.master_content_id != track_id || track_id == 0 {
                continue;
            }
            let named = track.master_db_id;
            if library_id != 0 && named != 0 && named != library_id {
                foreign.push(track.path.clone());
            } else if !foreign.contains(&track.path) {
                found = Some(track);
                break;
            }
        }
        if let Some(track) = found {
            // As the library spells it: a backslash is a character of a
            // name here, never a separator (the layout's `file` and CRC
            // are over forward slashes, which is what the library holds).
            let path = track.path.trim_start_matches('/').to_owned();
            let file = on_stick(stick, &path)?;
            return Ok(Exported { path, file });
        }
    }
    match unreadable {
        Some(message) => Err(NativeStemsError::LibraryUnreadable {
            stick: stick.to_path_buf(),
            message,
        }),
        None => Err(NativeStemsError::TrackNotOnStick { track_id }),
    }
}

/// The file the library's `path` names, which must be on the Stick.
fn on_stick(stick: &Path, path: &str) -> Result<PathBuf, NativeStemsError> {
    let off = |at: &Path, why: &str| NativeStemsError::OffStick {
        path: at.to_path_buf(),
        why: why.to_owned(),
    };
    // Either slash: the export crates' resolver takes both as separators.
    if path.split(['/', '\\']).any(|part| part == "..") {
        return Err(off(
            &stick.join(path),
            "is where the Stick's library says the track is, which climbs out of the Stick",
        ));
    }
    let file = StickFiles::new(stick).locate(path);
    let Ok(metadata) = std::fs::symlink_metadata(&file) else {
        // Not there: the caller says so.
        return Ok(file);
    };
    if !metadata.is_file() {
        return Err(off(
            &file,
            "is where the Stick's library says the track is, and is not a regular file",
        ));
    }
    // With every link on the way followed, still under the Stick's root.
    let inside = match (stick.canonicalize(), file.canonicalize()) {
        (Ok(root), Ok(real)) => real.starts_with(root),
        _ => false,
    };
    if !inside {
        return Err(off(
            &file,
            "is reached through a symbolic link that leads off the Stick",
        ));
    }
    Ok(file)
}
