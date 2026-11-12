//! Editing playlists from a client (wbx-80): taking tracks out of a
//! playlist, deleting one, putting its entries in a new order, copying it,
//! and removing tracks from the collection.
//!
//! Each edit is the write the window makes for the same edit, recorded in
//! the core's edit history as the window records it (so Undo in the window
//! takes it back), through [`OpenLibrary`]'s single edit path, which
//! refuses while Library Protection is on. What an edit means for the
//! Mutants of the entries it moves is not done here: the Ledger records it
//! with the edit ([`crate::mutant_follow`]), the window's edits and their
//! Undo and Redo alike. Here the Mutants are only reported: each answer
//! lists the Mutants the edit touched, as they are after it, and
//! `tracks.remove_from_collection` reports the Mutants it would remove
//! before it removes anything (it removes only with `confirm`).

use rbl_library::history::LibraryEdit;
use rbl_library::Touched;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::entries;
use crate::library::OpenLibrary;
use crate::mutants::commands as mutant_commands;
use crate::protocol::{codes as shared_codes, LibraryChanged, Request, ServiceError};
use crate::reading::codes as reading_codes;
use crate::stems::Stems;
use wreckordbox_sidecar::mutant_follow::Copy;

/// The names of the commands this module answers.
pub mod commands {
    /// Tracks taken out of a playlist (every entry of each): takes a
    /// [`RemoveTracks`](super::RemoveTracks), answers a
    /// [`PlaylistEdited`](super::PlaylistEdited).
    pub const PLAYLISTS_REMOVE: &str = "playlists.remove";
    /// A playlist or folder deleted with everything in it: takes a
    /// [`DeletePlaylist`](super::DeletePlaylist), answers a
    /// [`PlaylistEdited`](super::PlaylistEdited).
    pub const PLAYLISTS_DELETE: &str = "playlists.delete";
    /// A playlist's entries in a new order: takes a
    /// [`ReorderEntries`](super::ReorderEntries), answers a
    /// [`PlaylistEdited`](super::PlaylistEdited).
    pub const PLAYLISTS_REORDER: &str = "playlists.reorder";
    /// A new playlist beside one, with its entries and its Mutants: takes a
    /// [`CopyPlaylist`](super::CopyPlaylist), answers a
    /// [`PlaylistCopied`](super::PlaylistCopied).
    pub const PLAYLISTS_COPY: &str = "playlists.copy";
    /// Tracks out of the collection and every playlist, reported first:
    /// takes a [`RemoveFromCollection`](super::RemoveFromCollection),
    /// answers a [`CollectionRemoval`](super::CollectionRemoval).
    pub const TRACKS_REMOVE_FROM_COLLECTION: &str = "tracks.remove_from_collection";
}

/// The error codes these commands add.
pub mod codes {
    /// A track named is not in the playlist.
    pub const TRACK_NOT_IN_PLAYLIST: &str = "track_not_in_playlist";
    /// A new order must name every entry of the playlist once and nothing
    /// else (read them with `playlists.tracks`).
    pub const INVALID_ENTRY_ORDER: &str = "invalid_entry_order";
    /// Only a plain playlist's entries can be removed, put in order or
    /// copied: not a folder's or an intelligent playlist's.
    pub const NOT_A_PLAYLIST: &str = "not_a_playlist";
}

/// The arguments of `playlists.remove`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveTracks {
    pub playlist: String,
    /// Track identifiers; every entry of each leaves the playlist.
    pub tracks: Vec<String>,
}

/// The arguments of `playlists.delete`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletePlaylist {
    pub playlist: String,
}

/// The arguments of `playlists.reorder`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReorderEntries {
    pub playlist: String,
    /// Every entry of the playlist (`entry_id` of `playlists.tracks`) once,
    /// in the new order.
    pub entries: Vec<String>,
}

/// The arguments of `playlists.copy`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CopyPlaylist {
    pub playlist: String,
    /// The copy's name; left out, `<name> copy`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// The arguments of `tracks.remove_from_collection`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveFromCollection {
    pub tracks: Vec<String>,
    /// Remove them. Left out, nothing is removed and the answer says what
    /// would be: the tracks and the Mutants that would go with them.
    #[serde(default)]
    pub confirm: bool,
}

/// What a playlist edit did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaylistEdited {
    pub playlist: String,
    /// Whether the Library changed.
    pub changed: bool,
    pub generation: u32,
    /// The Mutants of the entries it moved, as they are now (a removed
    /// entry's Mutant is `removed: true`), as `mutants.get` answers them.
    pub mutants: Vec<Value>,
}

/// What `playlists.copy` made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaylistCopied {
    /// The copy.
    pub playlist: String,
    pub name: String,
    /// The playlist copied.
    pub from: String,
    /// Entries copied.
    pub tracks: usize,
    pub generation: u32,
    /// The copy's Mutants: one for each Mutant of the source.
    pub mutants: Vec<Value>,
}

/// What `tracks.remove_from_collection` would remove, or removed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectionRemoval {
    pub tracks: Vec<String>,
    /// Whether they were removed (only with `confirm`).
    pub removed: bool,
    /// The generation after the removal; `None` when nothing was removed.
    pub generation: Option<u32>,
    /// The Mutants of these tracks: those that would go with them, or
    /// those that went, as they are now.
    pub mutants: Vec<Value>,
}

/// Whether `command` is one of this module's.
pub(crate) fn answers(command: &str) -> bool {
    matches!(
        command,
        commands::PLAYLISTS_REMOVE
            | commands::PLAYLISTS_DELETE
            | commands::PLAYLISTS_REORDER
            | commands::PLAYLISTS_COPY
            | commands::TRACKS_REMOVE_FROM_COLLECTION
    )
}

/// Makes the edit `request` names. Answers the result and, when the
/// Library changed, what to tell the hosting process.
pub(crate) fn answer(
    library: &OpenLibrary,
    stems: &Arc<Stems>,
    request: &Request,
) -> Result<(Value, Option<LibraryChanged>), ServiceError> {
    let edits = Edits { library, stems };
    let command = request.command.as_str();
    match command {
        commands::PLAYLISTS_REMOVE => edits.remove(&arguments(command, &request.args)?),
        commands::PLAYLISTS_DELETE => edits.delete(&arguments(command, &request.args)?),
        commands::PLAYLISTS_REORDER => edits.reorder(&arguments(command, &request.args)?),
        commands::PLAYLISTS_COPY => edits.copy(&arguments(command, &request.args)?),
        _ => edits.remove_from_collection(&arguments(command, &request.args)?),
    }
}

struct Edits<'a> {
    library: &'a OpenLibrary,
    stems: &'a Arc<Stems>,
}

impl Edits<'_> {
    fn remove(
        &self,
        remove: &RemoveTracks,
    ) -> Result<(Value, Option<LibraryChanged>), ServiceError> {
        let entries = self.plain(&remove.playlist)?;
        if let Some(missing) = remove
            .tracks
            .iter()
            .find(|track| !entries.iter().any(|entry| &entry.track_id == *track))
        {
            return Err(ServiceError::new(
                codes::TRACK_NOT_IN_PLAYLIST,
                format!("track {missing} is not in playlist {}", remove.playlist),
            ));
        }
        let touched: Vec<String> = entries
            .iter()
            .filter(|entry| remove.tracks.contains(&entry.track_id))
            .map(|entry| entry.id.clone())
            .collect();
        let before = self.mutants_of(&touched)?;
        let (playlist, tracks) = (remove.playlist.clone(), remove.tracks.clone());
        let (rows, changed) = self.library.recorded_edit(
            Touched::Playlists,
            "Remove Tracks from Playlist",
            move |writer| {
                writer
                    .remove_tracks_with_undo(&playlist, &tracks)
                    .map(|(changed, edit)| (changed.rows, LibraryEdit::RemovePlaylistTracks(edit)))
            },
        )?;
        self.edited(&remove.playlist, rows > 0, changed, &before)
    }

    fn delete(
        &self,
        delete: &DeletePlaylist,
    ) -> Result<(Value, Option<LibraryChanged>), ServiceError> {
        let index = self.library.index()?;
        if entries::playlist_name(&index, &delete.playlist).is_none() {
            return Err(not_found(&delete.playlist));
        }
        let before: Vec<i64> = self
            .stored(|sidecar| sidecar.mutants(None))?
            .into_iter()
            .map(|mutant| mutant.id)
            .collect();
        let playlist = delete.playlist.clone();
        let (rows, changed) =
            self.library
                .recorded_edit(Touched::Playlists, "Delete Playlist", move |writer| {
                    writer
                        .delete_playlist_with_undo(&playlist)
                        .map(|(changed, edit)| (changed.rows, LibraryEdit::DeletePlaylist(edit)))
                })?;
        // The Mutants that went with it: those live before and not now.
        let gone: Vec<i64> = before
            .into_iter()
            .filter(|id| matches!(self.mutant(*id), Ok(Some(m)) if m["removed"] == true))
            .collect();
        self.edited(&delete.playlist, rows > 0, changed, &gone)
    }

    fn reorder(
        &self,
        reorder: &ReorderEntries,
    ) -> Result<(Value, Option<LibraryChanged>), ServiceError> {
        let entries = self.plain(&reorder.playlist)?;
        let mut have: Vec<&str> = entries.iter().map(|entry| entry.id.as_str()).collect();
        let mut wanted: Vec<&str> = reorder.entries.iter().map(String::as_str).collect();
        have.sort_unstable();
        wanted.sort_unstable();
        if have != wanted {
            return Err(ServiceError::new(
                codes::INVALID_ENTRY_ORDER,
                format!(
                    "a new order names every entry of playlist {} once ({} entries) and nothing else",
                    reorder.playlist,
                    entries.len()
                ),
            ));
        }
        let touched: Vec<String> = entries.iter().map(|entry| entry.id.clone()).collect();
        let before = self.mutants_of(&touched)?;
        let (playlist, order) = (reorder.playlist.clone(), reorder.entries.clone());
        let (moved, changed) =
            self.library
                .recorded_edit(Touched::Playlists, "Reorder Playlist", move |writer| {
                    rbl_library::entry_order::reorder(writer, &playlist, &order)
                        .map(|edit| (!edit.is_empty(), LibraryEdit::ReorderEntries(edit)))
                })?;
        self.edited(&reorder.playlist, moved, changed, &before)
    }

    fn copy(&self, copy: &CopyPlaylist) -> Result<(Value, Option<LibraryChanged>), ServiceError> {
        let index = self.library.index()?;
        let Some(source_name) = entries::playlist_name(&index, &copy.playlist) else {
            return Err(not_found(&copy.playlist));
        };
        self.plain(&copy.playlist)?;
        let name = copy
            .name
            .clone()
            .unwrap_or_else(|| format!("{source_name} copy"));
        let (source, wanted) = (copy.playlist.clone(), name.clone());
        let (made, mut changed) = self.library.recorded_edit(
            Touched::Playlists,
            "Create Playlist",
            move |writer| {
                let parent: String = writer
                    .library()
                    .connection()
                    .query_row(
                        "SELECT ParentID FROM djmdPlaylist WHERE ID = ?1 AND rb_local_deleted = 0",
                        [&source],
                        |row| row.get(0),
                    )
                    .map_err(rbl_db::DbError::from)?;
                writer
                    .create_with_undo(&wanted, &parent, rbl_db::write::ATTRIBUTE_PLAYLIST, None)
                    .map(|(id, edit)| (id, LibraryEdit::CreatePlaylist(edit)))
            },
        )?;
        // The entries, read again under the gate of the write that copies
        // them; their Mutants are copied with the addition's Ledger entry.
        let (source, target) = (copy.playlist.clone(), made.clone());
        let added = self.library.recorded_edit(
            Touched::Playlists,
            "Add Tracks to Playlist",
            move |writer| {
                let entries =
                    rbl_library::entry_order::entries(writer.library().connection(), &source)?;
                if entries.is_empty() {
                    return Err(rbl_db::DbError::WriteRefused(NOTHING_TO_COPY.to_owned()));
                }
                let tracks: Vec<String> = entries.iter().map(|e| e.track_id.clone()).collect();
                let (_, addition) = writer.add_tracks_with_undo(&target, &tracks, true)?;
                let copies = entries
                    .iter()
                    .zip(addition.added())
                    .map(|(from, to)| Copy {
                        from: from.id.clone(),
                        playlist: target.clone(),
                        to: to.clone(),
                    })
                    .collect();
                crate::mutant_follow::expect_copies(copies);
                Ok((
                    addition.added().len(),
                    LibraryEdit::AddPlaylistTracks {
                        addition,
                        transaction: None,
                    },
                ))
            },
        );
        crate::mutant_follow::forget_copies();
        let tracks = match added {
            Ok((tracks, more)) => {
                changed = more;
                tracks
            }
            Err(error) if error.message == NOTHING_TO_COPY => 0,
            Err(error) => return Err(error),
        };
        let mutants = self.listed(json!({ "playlist": made }))?;
        let copied = PlaylistCopied {
            playlist: made,
            name,
            from: copy.playlist.clone(),
            tracks,
            generation: changed.generation,
            mutants,
        };
        Ok((to_value(&copied)?, Some(changed)))
    }

    fn remove_from_collection(
        &self,
        remove: &RemoveFromCollection,
    ) -> Result<(Value, Option<LibraryChanged>), ServiceError> {
        if remove.tracks.is_empty() {
            return Err(ServiceError::new(
                shared_codes::BAD_REQUEST,
                "name the tracks to remove from the collection",
            ));
        }
        let index = self.library.index()?;
        let mut tracks = Vec::new();
        for track in &remove.tracks {
            let Some(row) = index.row_of(track) else {
                return Err(ServiceError::new(
                    shared_codes::TRACK_NOT_FOUND,
                    format!("there is no track {track} in the Library"),
                ));
            };
            tracks.push(
                index
                    .ids
                    .get(row as usize)
                    .map_or_else(|| track.clone(), u64::to_string),
            );
        }
        // Reported first: the Mutants that go with the tracks.
        let mut affected = Vec::new();
        for track in &tracks {
            affected.extend(self.listed(json!({ "track": track }))?);
        }
        if !remove.confirm {
            let report = CollectionRemoval {
                tracks,
                removed: false,
                generation: None,
                mutants: affected,
            };
            return Ok((to_value(&report)?, None));
        }
        let ids: Vec<i64> = affected.iter().filter_map(|m| m["id"].as_i64()).collect();
        let doomed = tracks.clone();
        let ((), changed) = self
            .library
            .permanent_edit(Touched::Tracks, move |writer| {
                writer.delete_tracks(&doomed).map(drop)
            })?;
        let mut mutants = Vec::new();
        for id in ids {
            if let Some(mutant) = self.mutant(id)? {
                mutants.push(mutant);
            }
        }
        let report = CollectionRemoval {
            tracks,
            removed: true,
            generation: Some(changed.generation),
            mutants,
        };
        Ok((to_value(&report)?, Some(changed)))
    }

    /// The entries of a plain playlist, refused for anything else.
    fn plain(&self, playlist: &str) -> Result<Vec<entries::Entry>, ServiceError> {
        let index = self.library.index()?;
        if entries::playlist_name(&index, playlist).is_none() {
            return Err(not_found(playlist));
        }
        let plain = self.library.with_db(|db| {
            Ok(db
                .connection()
                .query_row(
                    "SELECT Attribute = ?2 FROM djmdPlaylist WHERE ID = ?1 AND rb_local_deleted = 0",
                    rusqlite::params![playlist, rbl_db::write::ATTRIBUTE_PLAYLIST],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap_or(false))
        })?;
        if !plain {
            return Err(ServiceError::new(
                codes::NOT_A_PLAYLIST,
                format!("{playlist} is a folder or an intelligent playlist, which has no entries of its own"),
            ));
        }
        entries::of_playlist(self.library, &index, playlist)
    }

    /// The live Mutants of these entries, by identifier.
    fn mutants_of(&self, entries: &[String]) -> Result<Vec<i64>, ServiceError> {
        let mut found = Vec::new();
        for entry in entries {
            let live = self.stored(|sidecar| {
                sidecar
                    .mutant_of_entry(entry)
                    .map(|mutant| mutant.map(|mutant| mutant.id))
            })?;
            if let Some(mutant) = live {
                found.push(mutant);
            }
        }
        Ok(found)
    }

    /// Reads the Sidecar, when there is one, without making one.
    fn stored<T: Default>(
        &self,
        read: impl FnOnce(&wreckordbox_sidecar::Sidecar) -> Result<T, wreckordbox_sidecar::SidecarError>,
    ) -> Result<T, ServiceError> {
        let Some(sidecar) = self.stems.sidecar_if_made()? else {
            return Ok(T::default());
        };
        read(&sidecar)
            .map_err(|e| ServiceError::new(crate::stems::codes::SIDECAR_FAILED, e.to_string()))
    }

    /// A Mutant as `mutants.get` answers it.
    fn mutant(&self, id: i64) -> Result<Option<Value>, ServiceError> {
        match self.mutants_call(mutant_commands::MUTANTS_GET, json!({ "id": id })) {
            Ok(answer) => Ok(Some(answer["mutant"].clone())),
            Err(error) if error.code == crate::mutants::codes::MUTANT_NOT_FOUND => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// The live Mutants `mutants.list` answers for `args`.
    fn listed(&self, args: Value) -> Result<Vec<Value>, ServiceError> {
        let answer = self.mutants_call(mutant_commands::MUTANTS_LIST, args)?;
        Ok(answer["mutants"].as_array().cloned().unwrap_or_default())
    }

    fn mutants_call(&self, command: &str, args: Value) -> Result<Value, ServiceError> {
        let request = Request {
            id: 0,
            command: command.to_owned(),
            args,
            run: None,
        };
        crate::mutants::answer(self.stems, &request, &|read| read(self.library))
    }

    fn edited(
        &self,
        playlist: &str,
        changed: bool,
        change: LibraryChanged,
        mutants: &[i64],
    ) -> Result<(Value, Option<LibraryChanged>), ServiceError> {
        let mut shown = Vec::new();
        for id in mutants {
            if let Some(mutant) = self.mutant(*id)? {
                shown.push(mutant);
            }
        }
        let edited = PlaylistEdited {
            playlist: playlist.to_owned(),
            changed,
            generation: change.generation,
            mutants: shown,
        };
        Ok((to_value(&edited)?, Some(change)))
    }
}

/// An empty playlist's copy has nothing to add: the write is not made.
const NOTHING_TO_COPY: &str = "the playlist has no entries to copy";

fn not_found(playlist: &str) -> ServiceError {
    ServiceError::new(
        reading_codes::PLAYLIST_NOT_FOUND,
        format!("there is no playlist {playlist} in the Library"),
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
