//! Making and arranging playlists from a client (wbx-uia): a playlist
//! (optionally in a folder, optionally with its first tracks in order), a
//! folder, tracks added at the end or at a position, a playlist or folder
//! renamed or moved.
//!
//! Each is the write the window makes for the same edit, recorded in the
//! core's edit history under the window's own label (so it is the window's
//! next Undo), through [`OpenLibrary`]'s single edit path, which refuses
//! while Library Protection is on. These commands are answered through
//! [`crate::playlist_edits`], so the Ledger records them under the caller's
//! source and Run and the hosting window is told, as for its older edits.
//!
//! Two of them are two of the window's steps, because the window has no
//! single step for them: a playlist made with tracks is Create Playlist
//! then Add Tracks to Playlist, and tracks added at a position are Add
//! Tracks to Playlist then Reorder Playlist. Everything they name is
//! checked before the first write, so neither stops half-way over a bad
//! argument.

use rbl_db::write::{ATTRIBUTE_FOLDER, ATTRIBUTE_PLAYLIST, ATTRIBUTE_SMART, ROOT};
use rbl_library::history::LibraryEdit;
use rbl_library::Touched;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::library::OpenLibrary;
use crate::playlist_edits::{codes as edit_codes, Done};
use crate::protocol::{codes as shared_codes, Request, ServiceError};
use crate::reading::codes as reading_codes;

/// The names of the commands this module answers.
pub mod commands {
    /// A new playlist: takes a [`CreatePlaylist`](super::CreatePlaylist),
    /// answers a [`PlaylistMade`](super::PlaylistMade).
    pub const PLAYLISTS_CREATE: &str = "playlists.create";
    /// A new folder: takes a [`CreateFolder`](super::CreateFolder), answers
    /// a [`PlaylistMade`](super::PlaylistMade).
    pub const PLAYLISTS_CREATE_FOLDER: &str = "playlists.create_folder";
    /// Tracks added to a playlist: takes an [`AddTracks`](super::AddTracks),
    /// answers a [`TracksAdded`](super::TracksAdded).
    pub const PLAYLISTS_ADD: &str = "playlists.add";
    /// A playlist or folder renamed: takes a
    /// [`RenamePlaylist`](super::RenamePlaylist), answers a
    /// [`PlaylistPlaced`](super::PlaylistPlaced).
    pub const PLAYLISTS_RENAME: &str = "playlists.rename";
    /// A playlist or folder moved: takes a
    /// [`MovePlaylist`](super::MovePlaylist), answers a
    /// [`PlaylistPlaced`](super::PlaylistPlaced).
    pub const PLAYLISTS_MOVE: &str = "playlists.move";
}

/// The error codes these commands add.
pub mod codes {
    /// A playlist or folder can be put only at the top level or in a
    /// folder: not in a playlist or an intelligent playlist.
    pub const NOT_A_FOLDER: &str = "not_a_folder";
}

/// The arguments of `playlists.create`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatePlaylist {
    pub name: String,
    /// The folder to make it in; left out, the top level (`"root"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Its place among the folder's items, from 1; left out, the last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<usize>,
    /// Its first tracks, in this order (a track named twice is put in
    /// once).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracks: Vec<String>,
}

/// The arguments of `playlists.create_folder`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateFolder {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<usize>,
}

/// The arguments of `playlists.add`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddTracks {
    pub playlist: String,
    /// In the order they go in.
    pub tracks: Vec<String>,
    /// The position the first of them takes, from 1; left out, the end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<usize>,
    /// Add a track the playlist already holds again. Left out, such a
    /// track is skipped (and listed in the answer).
    #[serde(default)]
    pub allow_duplicates: bool,
}

/// The arguments of `playlists.rename`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenamePlaylist {
    pub playlist: String,
    pub name: String,
}

/// The arguments of `playlists.move`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MovePlaylist {
    pub playlist: String,
    /// The folder to move it into; left out, the top level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Its place among the folder's items, from 1; left out, the last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<usize>,
}

/// What `playlists.create` or `playlists.create_folder` made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistMade {
    /// The new playlist's or folder's identifier.
    pub playlist: String,
    pub name: String,
    /// `"playlist"` or `"folder"`.
    pub kind: String,
    /// The folder it is in, or `"root"`.
    pub parent: String,
    /// The entries its first tracks became, in order (`entry_id` of
    /// `playlists.tracks`).
    pub added: Vec<String>,
    pub generation: u32,
}

/// What `playlists.add` did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TracksAdded {
    pub playlist: String,
    /// The new entries, in the order they went in.
    pub added: Vec<String>,
    /// Tracks not added because the playlist already held them.
    pub skipped: Vec<String>,
    pub generation: u32,
}

/// Where a playlist or folder is, and what it is called, after
/// `playlists.rename` or `playlists.move`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistPlaced {
    pub playlist: String,
    pub name: String,
    /// The folder it is in, or `"root"`.
    pub parent: String,
    /// Its place among the folder's items, from 1.
    pub at: usize,
    pub generation: u32,
}

pub(crate) fn answers(command: &str) -> bool {
    matches!(
        command,
        commands::PLAYLISTS_CREATE
            | commands::PLAYLISTS_CREATE_FOLDER
            | commands::PLAYLISTS_ADD
            | commands::PLAYLISTS_RENAME
            | commands::PLAYLISTS_MOVE
    )
}

/// Makes the edit `request` names. An `Err` means nothing was changed.
pub(crate) fn make(library: &OpenLibrary, request: &Request) -> Result<Done, ServiceError> {
    let command = request.command.as_str();
    let args = &request.args;
    match command {
        commands::PLAYLISTS_CREATE => create(library, &arguments(command, args)?),
        commands::PLAYLISTS_CREATE_FOLDER => {
            let folder: CreateFolder = arguments(command, args)?;
            make_node(
                library,
                &folder.name,
                folder.parent.as_deref(),
                folder.at,
                ATTRIBUTE_FOLDER,
            )
            .map(|(made, change)| (Some(change), to_value(&made)))
        }
        commands::PLAYLISTS_ADD => add(library, &arguments(command, args)?),
        commands::PLAYLISTS_RENAME => rename(library, &arguments(command, args)?),
        _ => move_to(library, &arguments(command, args)?),
    }
}

/// Why `playlist` (of `attribute`) has no entries to edit, for
/// `playlists.add`, `remove` and `reorder`.
pub(crate) fn no_entries(playlist: &str, attribute: Option<i64>) -> ServiceError {
    let why = match attribute {
        Some(ATTRIBUTE_SMART) => format!(
            "{playlist} is an intelligent playlist: its tracks are whatever its rule admits, so none can be added, taken out or put in order by hand"
        ),
        Some(ATTRIBUTE_FOLDER) => {
            format!("{playlist} is a folder, which has no tracks of its own")
        }
        _ => format!("{playlist} is not a playlist with tracks of its own"),
    };
    ServiceError::new(edit_codes::NOT_A_PLAYLIST, why)
}

/// A live playlist's `Attribute`, or `None` when there is none.
pub(crate) fn attribute_of(
    library: &OpenLibrary,
    playlist: &str,
) -> Result<Option<i64>, ServiceError> {
    library.with_db(|db| {
        Ok(db
            .connection()
            .query_row(
                "SELECT Attribute FROM djmdPlaylist WHERE ID = ?1 AND rb_local_deleted = 0",
                rusqlite::params![playlist],
                |row| row.get::<_, i64>(0),
            )
            .ok())
    })
}

fn create(library: &OpenLibrary, create: &CreatePlaylist) -> Result<Done, ServiceError> {
    // Checked before the playlist is made, so a bad track makes nothing.
    known_tracks(library, &create.tracks)?;
    let (mut made, change) = make_node(
        library,
        &create.name,
        create.parent.as_deref(),
        create.at,
        ATTRIBUTE_PLAYLIST,
    )?;
    if create.tracks.is_empty() {
        return Ok((Some(change), to_value(&made)));
    }
    match add_entries(library, &made.playlist, &create.tracks, false) {
        Ok(((added, _), change)) => {
            made.added = added;
            made.generation = change.generation;
            Ok((Some(change), to_value(&made)))
        }
        // The playlist was made; say so with the failure.
        Err(error) => Ok((
            Some(change),
            Err(ServiceError::new(
                &error.code,
                format!(
                    "playlist {} \"{}\" was made, but its tracks were not added: {}",
                    made.playlist, made.name, error.message
                ),
            )),
        )),
    }
}

fn make_node(
    library: &OpenLibrary,
    name: &str,
    parent: Option<&str>,
    at: Option<usize>,
    attribute: i64,
) -> Result<(PlaylistMade, crate::protocol::LibraryChanged), ServiceError> {
    let name = a_name(name)?;
    let parent = folder(library, parent)?;
    let index = place(at)?;
    let (label, kind) = if attribute == ATTRIBUTE_FOLDER {
        ("Create Folder", "folder")
    } else {
        ("Create Playlist", "playlist")
    };
    let (wanted, under) = (name.clone(), parent.clone());
    let (id, change) = library.recorded_edit(Touched::Playlists, label, move |writer| {
        writer
            .create_with_undo(&wanted, &under, attribute, index)
            .map(|(id, made)| (id, LibraryEdit::CreatePlaylist(made)))
    })?;
    Ok((
        PlaylistMade {
            playlist: id,
            name,
            kind: kind.to_owned(),
            parent,
            added: Vec::new(),
            generation: change.generation,
        },
        change,
    ))
}

fn add(library: &OpenLibrary, add: &AddTracks) -> Result<Done, ServiceError> {
    match attribute_of(library, &add.playlist)? {
        None => return Err(not_found(&add.playlist)),
        Some(ATTRIBUTE_PLAYLIST) => {}
        other => return Err(no_entries(&add.playlist, other)),
    }
    if add.tracks.is_empty() {
        return Err(ServiceError::new(
            shared_codes::BAD_REQUEST,
            "name the tracks to add",
        ));
    }
    let position = place(add.at)?;
    known_tracks(library, &add.tracks)?;
    let ((added, skipped), change) =
        add_entries(library, &add.playlist, &add.tracks, add.allow_duplicates)?;
    let mut answer = TracksAdded {
        playlist: add.playlist.clone(),
        added,
        skipped,
        generation: change.generation,
    };
    let Some(position) = position.filter(|_| !answer.added.is_empty()) else {
        return Ok((Some(change), to_value(&answer)));
    };
    // The window's Reorder Playlist, of the entries just added.
    let (playlist, moving) = (add.playlist.clone(), answer.added.clone());
    let placed = library.recorded_edit(Touched::Playlists, "Reorder Playlist", move |writer| {
        let mut order: Vec<String> =
            rbl_library::entry_order::entries(writer.library().connection(), &playlist)?
                .into_iter()
                .map(|entry| entry.id)
                .filter(|entry| !moving.contains(entry))
                .collect();
        let at = position.min(order.len());
        order.splice(at..at, moving);
        rbl_library::entry_order::reorder(writer, &playlist, &order)
            .map(|edit| ((), LibraryEdit::ReorderEntries(edit)))
    });
    match placed {
        Ok(((), placed)) => {
            answer.generation = placed.generation;
            Ok((Some(placed), to_value(&answer)))
        }
        Err(error) => Ok((
            Some(change),
            Err(ServiceError::new(
                &error.code,
                format!(
                    "the tracks were added at the end of playlist {}, but not moved to position {}: {}",
                    add.playlist,
                    position + 1,
                    error.message
                ),
            )),
        )),
    }
}

/// The new entries, in order, and the tracks skipped.
type Added = (Vec<String>, Vec<String>);

/// The window's Add Tracks to Playlist: the new entries, in order, and the
/// tracks skipped because the playlist held them already.
fn add_entries(
    library: &OpenLibrary,
    playlist: &str,
    tracks: &[String],
    allow_duplicates: bool,
) -> Result<(Added, crate::protocol::LibraryChanged), ServiceError> {
    let (playlist, tracks) = (playlist.to_owned(), tracks.to_vec());
    library.recorded_edit(
        Touched::Playlists,
        "Add Tracks to Playlist",
        move |writer| {
            let (_, addition) =
                writer.add_tracks_with_undo(&playlist, &tracks, allow_duplicates)?;
            let entries =
                rbl_library::entry_order::entries(writer.library().connection(), &playlist)?;
            let added: Vec<String> = addition.added().to_vec();
            let mut left: Vec<String> = tracks.clone();
            for entry in entries.iter().filter(|entry| added.contains(&entry.id)) {
                if let Some(at) = left.iter().position(|track| *track == entry.track_id) {
                    left.remove(at);
                }
            }
            // In the order the entries are in the playlist.
            let added_in_order: Vec<String> = entries
                .into_iter()
                .map(|entry| entry.id)
                .filter(|entry| added.contains(entry))
                .collect();
            Ok((
                (added_in_order, left),
                LibraryEdit::AddPlaylistTracks {
                    addition,
                    transaction: None,
                },
            ))
        },
    )
}

fn rename(library: &OpenLibrary, rename: &RenamePlaylist) -> Result<Done, ServiceError> {
    let name = a_name(&rename.name)?;
    if attribute_of(library, &rename.playlist)?.is_none() {
        return Err(not_found(&rename.playlist));
    }
    let (playlist, wanted) = (rename.playlist.clone(), name);
    let ((), change) =
        library.recorded_edit(Touched::Playlists, "Rename Playlist", move |writer| {
            writer
                .rename_with_undo(&playlist, &wanted)
                .map(|(_, edit)| ((), LibraryEdit::RenamePlaylist(edit)))
        })?;
    Ok((Some(change), placed(library, &rename.playlist, change)))
}

fn move_to(library: &OpenLibrary, moving: &MovePlaylist) -> Result<Done, ServiceError> {
    if attribute_of(library, &moving.playlist)?.is_none() {
        return Err(not_found(&moving.playlist));
    }
    let parent = folder(library, moving.parent.as_deref())?;
    let index = place(moving.at)?;
    let playlist = moving.playlist.clone();
    let ((), change) =
        library.recorded_edit(Touched::Playlists, "Move Playlist", move |writer| {
            writer
                .move_with_undo(&playlist, &parent, index)
                .map(|(_, edit)| ((), LibraryEdit::MovePlaylist(edit)))
        })?;
    Ok((Some(change), placed(library, &moving.playlist, change)))
}

/// Where `playlist` is now, read back.
fn placed(
    library: &OpenLibrary,
    playlist: &str,
    change: crate::protocol::LibraryChanged,
) -> Result<Value, ServiceError> {
    let (name, parent) = library.with_db(|db| {
        db.connection()
            .query_row(
                "SELECT COALESCE(Name, ''), COALESCE(ParentID, 'root') FROM djmdPlaylist
                 WHERE ID = ?1 AND rb_local_deleted = 0",
                rusqlite::params![playlist],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .map_err(|e| ServiceError::new(shared_codes::SERVICE_FAILED, e.to_string()))
    })?;
    let siblings: Vec<String> = library.with_db(|db| {
        let connection = db.connection();
        let mut statement = connection
            .prepare(
                "SELECT ID FROM djmdPlaylist WHERE ParentID = ?1 AND rb_local_deleted = 0
                 ORDER BY Seq, ID",
            )
            .map_err(|e| ServiceError::new(shared_codes::SERVICE_FAILED, e.to_string()))?;
        let ids = statement
            .query_map(rusqlite::params![parent], |row| row.get::<_, String>(0))
            .and_then(Iterator::collect)
            .map_err(|e| ServiceError::new(shared_codes::SERVICE_FAILED, e.to_string()));
        ids
    })?;
    let at = siblings
        .iter()
        .position(|id| id == playlist)
        .map_or(0, |at| at + 1);
    to_value(&PlaylistPlaced {
        playlist: playlist.to_owned(),
        name,
        parent,
        at,
        generation: change.generation,
    })
}

/// The folder named, or the top level; refused for anything else.
fn folder(library: &OpenLibrary, parent: Option<&str>) -> Result<String, ServiceError> {
    let Some(parent) = parent.filter(|parent| *parent != ROOT) else {
        return Ok(ROOT.to_owned());
    };
    match attribute_of(library, parent)? {
        None => Err(not_found(parent)),
        Some(ATTRIBUTE_FOLDER) => Ok(parent.to_owned()),
        Some(_) => Err(ServiceError::new(
            codes::NOT_A_FOLDER,
            format!(
                "{parent} is a playlist, not a folder: only a folder holds playlists and folders"
            ),
        )),
    }
}

/// A place from 1, as an index from 0.
fn place(at: Option<usize>) -> Result<Option<usize>, ServiceError> {
    match at {
        Some(0) => Err(ServiceError::new(
            shared_codes::BAD_REQUEST,
            "a position counts from 1",
        )),
        at => Ok(at.map(|at| at - 1)),
    }
}

fn a_name(name: &str) -> Result<String, ServiceError> {
    if name.trim().is_empty() {
        return Err(ServiceError::new(
            shared_codes::BAD_REQUEST,
            "a playlist or folder needs a name",
        ));
    }
    Ok(name.to_owned())
}

/// Refuses a track the Library does not have.
fn known_tracks(library: &OpenLibrary, tracks: &[String]) -> Result<(), ServiceError> {
    let index = library.index()?;
    match tracks.iter().find(|track| index.row_of(track).is_none()) {
        Some(missing) => Err(ServiceError::new(
            shared_codes::TRACK_NOT_FOUND,
            format!("there is no track {missing} in the Library"),
        )),
        None => Ok(()),
    }
}

fn not_found(playlist: &str) -> ServiceError {
    ServiceError::new(
        reading_codes::PLAYLIST_NOT_FOUND,
        format!("there is no playlist or folder {playlist} in the Library"),
    )
}

fn to_value<T: Serialize>(value: &T) -> Result<Value, ServiceError> {
    serde_json::to_value(value)
        .map_err(|e| ServiceError::new(shared_codes::SERVICE_FAILED, e.to_string()))
}

fn arguments<T: serde::de::DeserializeOwned>(
    command: &str,
    args: &Value,
) -> Result<T, ServiceError> {
    serde_json::from_value(args.clone()).map_err(|e| {
        ServiceError::new(
            shared_codes::BAD_REQUEST,
            format!("bad arguments to {command}: {e}"),
        )
    })
}
