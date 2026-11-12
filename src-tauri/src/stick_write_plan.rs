//! What a Stick write from the shell's tree would do, worked out before
//! anything is written (wbx-45.4 review, finding 5).
//!
//! The tree stages every write to a Stick: it shows which Stick (and whether
//! it is a Gig stick or a Spare stick), exactly what would change, and asks
//! for the deliberate hold before writing. For an export that means the same
//! selection the export itself would write (Export Playlist adds the chosen
//! playlist to what the Stick holds; Export Track adds the tracks and
//! re-synchronises every playlist the Stick holds from this Library) and the
//! export's own reconciliation (`rbl_export::plan::preview`).
//!
//! Nothing here writes or settles anything: the Stick's lock is only leased
//! for reading, the selection is built without `recover()`, and a Stick with
//! an unfinished write is reported as `pending` with nothing planned.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::commands::{blocking, held_on_stick, track_export_selection, ExportSelection, EXPORT_CACHE};
use crate::error::{AppError, AppResult, ErrorKind};
use crate::state::AppState;

/// The write being staged.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum StickWriteRequest {
    /// Export Playlist (or Export Folder) to the Stick.
    Playlist {
        playlist: String,
        destination: String,
        delete_unlisted_music: Option<bool>,
        compatibility_format: Option<rbl_export::CompatibilityFormat>,
    },
    /// Export Track: the tracks, in no playlist.
    Tracks {
        tracks: Vec<String>,
        destination: String,
        compatibility_format: Option<rbl_export::CompatibilityFormat>,
    },
}

/// How a playlist on the Stick is changed by the write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PlaylistChange {
    /// Not on the Stick from this Library yet.
    Created,
    /// On the Stick already, written again with what the Library holds now.
    Resynchronised,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedPlaylist {
    pub name: String,
    pub folder: bool,
    pub change: PlaylistChange,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StickWritePlan {
    /// The Stick carries the Spare stick marker.
    pub spare: bool,
    /// An earlier write did not finish: nothing may be written until it is
    /// settled, and nothing is planned.
    pub pending: bool,
    pub tracks_new: usize,
    pub tracks_changed: usize,
    pub tracks_kept: usize,
    pub tracks_removed: usize,
    pub tracks_missing: usize,
    /// Every playlist and folder the write puts on the Stick, in order.
    pub playlists: Vec<PlannedPlaylist>,
    pub net_additional_bytes: i64,
}

#[tauri::command]
pub async fn stick_write_plan(
    state: tauri::State<'_, std::sync::Arc<AppState>>,
    request: StickWriteRequest,
) -> AppResult<StickWritePlan> {
    let state = std::sync::Arc::clone(&state);
    blocking("stick_write_plan", move || plan(&state, &request)).await
}

/// The plan for `request`, writing nothing.
pub(crate) fn plan(state: &AppState, request: &StickWriteRequest) -> AppResult<StickWritePlan> {
    let (destination, compatibility) = match request {
        StickWriteRequest::Playlist { destination, compatibility_format, .. }
        | StickWriteRequest::Tracks { destination, compatibility_format, .. } => (Path::new(destination), *compatibility_format),
    };
    if !destination.is_dir() {
        return Err(AppError::new(ErrorKind::NotFound, "That device is no longer connected."));
    }
    let spare = destination.join(wreckordbox_stick_outline::SPARE_MARKER).is_file();
    let _lease = rbl_core::durable::read_lease(destination).map_err(|e| AppError::internal(e.to_string()))?;
    if rbl_export::recovery_pending(destination) {
        return Ok(StickWritePlan { spare, pending: true, ..StickWritePlan::default() });
    }
    let library = state.library()?;
    let share = state.share_root();
    let (held, _) = held_on_stick(state, &library, destination);
    let selection = match request {
        StickWriteRequest::Playlist { playlist, delete_unlisted_music, .. } => {
            // As Export Playlist builds it, without settling anything first.
            let (mut playlists, automatic) = held_on_stick(state, &library, destination);
            if !playlists.iter().any(|id| id == playlist) {
                playlists.push(playlist.clone());
            }
            let mut selection = ExportSelection::from_playlists(state, &library, &share, &playlists, automatic)?;
            selection.keep_unlisted = true;
            match selection.for_stick_inspected(state, &library, &share, destination, delete_unlisted_music.unwrap_or(false), false)? {
                std::borrow::Cow::Owned(kept) => kept,
                std::borrow::Cow::Borrowed(_) => selection,
            }
        }
        StickWriteRequest::Tracks { tracks, .. } => track_export_selection(state, &library, &share, destination, tracks)?,
    };
    let options = rbl_export::ExportOptions {
        sync: Some(&selection.sync),
        compatibility,
        keep_unlisted: selection.keep_unlisted,
        cache: Some(&EXPORT_CACHE),
        ..Default::default()
    };
    let found = rbl_export::plan::preview(destination, &selection.tracks, &selection.playlists, &selection.my_tags, &options)
        .map_err(|e| AppError::internal(e.to_string()))?;
    let playlists = selection
        .playlists
        .iter()
        .map(|p| PlannedPlaylist {
            name: p.name.clone(),
            folder: p.folder,
            change: if held.iter().any(|id| *id == p.id.to_string()) { PlaylistChange::Resynchronised } else { PlaylistChange::Created },
        })
        .collect();
    Ok(StickWritePlan {
        spare,
        pending: false,
        tracks_new: found.estimate.tracks_new,
        tracks_changed: found.estimate.tracks_changed,
        tracks_kept: found.estimate.tracks_kept,
        tracks_removed: found.estimate.tracks_removed,
        tracks_missing: found.estimate.tracks_missing,
        playlists,
        net_additional_bytes: found.net_additional_bytes,
    })
}
