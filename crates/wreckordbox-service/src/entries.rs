//! Playlist entries: one track's place in one playlist, with an identity
//! of its own.
//!
//! The Library keeps a row per entry (`djmdSongPlaylist`), and the row's
//! own identifier (`ID`, a UUID rekordbox gives it when the track is put
//! in the playlist) is the entry's identity. It is not the position:
//!
//! - reordering a playlist renumbers the rows and keeps their identifiers;
//! - removing an entry marks its row deleted, and undoing the removal
//!   brings the same row, and so the same identifier, back;
//! - putting the track in the playlist afresh makes a new row with a new
//!   identifier, as does replacing a playlist's whole contents
//!   (`set_tracks`, an import's "replace existing"), which retires every
//!   row and writes new ones;
//! - a track that is in a playlist twice has two rows, so two entries.
//!
//! The index holds a playlist's tracks in order but not these identifiers,
//! so they are read from the Library's database, read-only, and matched to
//! the index's rows. Nothing here writes.

use rusqlite::params;

use crate::library::OpenLibrary;
use crate::protocol::{codes, ServiceError};
use crate::reading::ListedTrack;

/// One entry of a playlist, as the Library holds it now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    /// The entry's own identifier.
    pub(crate) id: String,
    pub(crate) playlist_id: String,
    pub(crate) track_id: String,
    /// Its place in the playlist, from 1, as every listing numbers it.
    pub(crate) position: usize,
}

/// The entries of a playlist, in its order. An entry whose track is no
/// longer in the Library is left out, as every listing leaves it out, so
/// the positions here are the positions `playlists.tracks` gives. A folder,
/// an intelligent playlist and an unknown identifier all have none: the
/// kind is checked, so a membership row left under a folder or an
/// intelligent playlist (an import can leave one) is not an entry.
pub(crate) fn of_playlist(
    library: &OpenLibrary,
    index: &rbl_index::Library,
    playlist_id: &str,
) -> Result<Vec<Entry>, ServiceError> {
    if !is_plain(index, playlist_id) {
        return Ok(Vec::new());
    }
    let rows: Vec<(String, String)> = library.with_db(|db| {
        let mut statement = db
            .connection()
            .prepare(
                "SELECT ID, ContentID FROM djmdSongPlaylist
                 WHERE PlaylistID = ?1 AND rb_local_deleted = 0 ORDER BY TrackNo, rowid",
            )
            .map_err(|e| failed(&e))?;
        let rows = statement
            .query_map(params![playlist_id], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            })
            .map_err(|e| failed(&e))?
            .filter_map(|row| match row {
                Ok((Some(id), Some(track))) => Some(Ok((id, track))),
                Ok(_) => None,
                Err(e) => Some(Err(failed(&e))),
            })
            .collect::<Result<Vec<_>, ServiceError>>()?;
        Ok(rows)
    })?;
    Ok(rows
        .into_iter()
        .filter(|(_, track)| index.row_of(track).is_some())
        .enumerate()
        .map(|(at, (id, track_id))| Entry {
            id,
            playlist_id: playlist_id.to_owned(),
            track_id,
            position: at + 1,
        })
        .collect())
}

/// The entry with this identifier, if the Library has it now: not removed,
/// in a playlist that is still there, of a track that is still there.
pub(crate) fn find(
    library: &OpenLibrary,
    index: &rbl_index::Library,
    entry_id: &str,
) -> Result<Option<Entry>, ServiceError> {
    let playlist: Option<String> = library.with_db(|db| {
        let mut statement = db
            .connection()
            .prepare(
                "SELECT PlaylistID FROM djmdSongPlaylist
                 WHERE ID = ?1 AND rb_local_deleted = 0",
            )
            .map_err(|e| failed(&e))?;
        let mut rows = statement
            .query_map(params![entry_id], |row| row.get::<_, Option<String>>(0))
            .map_err(|e| failed(&e))?;
        match rows.next() {
            Some(Ok(playlist)) => Ok(playlist),
            Some(Err(e)) => Err(failed(&e)),
            None => Ok(None),
        }
    })?;
    let Some(playlist) = playlist else {
        return Ok(None);
    };
    if playlist_name(index, &playlist).is_none() {
        return Ok(None);
    }
    Ok(of_playlist(library, index, &playlist)?
        .into_iter()
        .find(|entry| entry.id == entry_id))
}

/// Whether the Library has a plain playlist (not a folder, not an
/// intelligent playlist) with this identifier now.
fn is_plain(index: &rbl_index::Library, playlist_id: &str) -> bool {
    let playlists = index.playlists();
    let Some(at) = playlist_id
        .parse()
        .ok()
        .and_then(|id| playlists.index_of(id))
    else {
        return false;
    };
    !playlists.is_folder(at) && !playlists.is_smart(at)
}

/// The name of a playlist that is in the Library now.
pub(crate) fn playlist_name(index: &rbl_index::Library, playlist_id: &str) -> Option<String> {
    let playlists = index.playlists();
    let at = playlists.index_of(playlist_id.parse().ok()?)?;
    Some(playlists.name(at).to_owned())
}

/// Gives each listed track of a playlist its entry's identifier. `tracks`
/// are the playlist's first tracks in its order, as the index has them;
/// each takes the entry at its position when that entry is of the same
/// track, and otherwise the first entry of its track not yet taken (two
/// entries numbered alike may be read in either order).
pub(crate) fn attach(
    library: &OpenLibrary,
    index: &rbl_index::Library,
    playlist_id: &str,
    tracks: &mut [ListedTrack],
) -> Result<(), ServiceError> {
    let entries = of_playlist(library, index, playlist_id)?;
    let mut taken = vec![false; entries.len()];
    for (at, listed) in tracks.iter_mut().enumerate() {
        let found = if entries
            .get(at)
            .is_some_and(|entry| entry.track_id == listed.track.id && !taken[at])
        {
            Some(at)
        } else {
            entries
                .iter()
                .enumerate()
                .position(|(other, entry)| !taken[other] && entry.track_id == listed.track.id)
        };
        if let Some(found) = found {
            taken[found] = true;
            listed.entry_id = Some(entries[found].id.clone());
        }
    }
    Ok(())
}

fn failed(error: &rusqlite::Error) -> ServiceError {
    ServiceError::new(
        codes::SERVICE_FAILED,
        format!("could not read the playlist's entries from the Library: {error}"),
    )
}
