//! A playlist's entries for the window, and the reorder by entry (wbx-59).
//!
//! The second list reorders its target by entry, so a track that is in the
//! playlist twice is two rows that move on their own, each keeping its
//! identity (a Mutant is keyed to it). The read and the write are
//! `rbl_library::entry_order`'s, beside the protected writer; the write
//! goes through the same recorded edit path as every playlist edit: gated,
//! refused under Library Protection, told to the Ledger, one Undo step
//! labelled "Reorder Playlist", and a `library:changed` afterwards.
use std::sync::Arc;

use tauri::State;

use crate::commands::{blocking, recorded_edit, write_error, Touched};
use crate::state::{AppState, LibraryEdit};
use crate::AppResult;

/// One entry of a playlist, in the playlist's order.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistEntryDto {
    /// The entry's own identifier.
    pub entry_id: String,
    /// The track it is.
    pub track_id: String,
}

/// The entries of a plain playlist, in its order, as the list lists its
/// tracks: an entry whose track is not in the Library's index is left
/// out, as the list leaves it out. Read under the edit gate, so no edit
/// lands between the index and the rows. Any other id answers none.
#[tauri::command]
pub async fn playlist_entries(state: State<'_, Arc<AppState>>, playlist: String) -> AppResult<Vec<PlaylistEntryDto>> {
    let state = Arc::clone(&state);
    blocking("playlist_entries", move || {
        let _gate = state.edit_gate().lock();
        let library = state.library()?;
        let entries = state
            .read_db(|db| rbl_library::entry_order::entries(db.connection(), &playlist))
            .map_err(write_error)?;
        Ok(entries
            .into_iter()
            .filter(|entry| library.row_of(&entry.track_id).is_some())
            .map(|entry| PlaylistEntryDto { entry_id: entry.id, track_id: entry.track_id })
            .collect())
    })
    .await
}

/// Puts a playlist's entries in exactly this order: every live entry once.
/// A stale order (an entry added, removed or unknown) is refused as a
/// conflict with nothing written. Answers the Library's new generation.
#[tauri::command]
pub async fn reorder_playlist_entries<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, Arc<AppState>>,
    playlist: String,
    entries: Vec<String>,
) -> AppResult<u32> {
    recorded_edit(app, state, "reorder_playlist_entries", Touched::Playlists, "Reorder Playlist", move |w| {
        rbl_library::entry_order::reorder(w, &playlist, &entries).map(LibraryEdit::ReorderEntries)
    })
    .await
    .map(|dto| dto.generation)
}
