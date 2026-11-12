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
pub(crate) struct Exported {
    /// The path the library holds, relative to the Stick's root, with
    /// forward slashes and no leading one. Spelled as the library spells
    /// it, which a directory listing may not.
    pub(crate) path: String,
    /// The track's file on this machine.
    pub(crate) file: PathBuf,
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
pub(crate) fn exported(
    stick: &Path,
    library_id: u64,
    track_id: u64,
) -> Result<Exported, NativeStemsError> {
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
        let library = match device_library::read(stick, format) {
            Ok(library) => library,
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
            let path = track.path.replace('\\', "/");
            let path = path.trim_start_matches('/').to_owned();
            let file = StickFiles::new(stick).locate(&path);
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
