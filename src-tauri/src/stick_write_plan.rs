//! What a Stick write would do, worked out before anything is written, and
//! the approval token that alone lets the write run (wbx-45.4, both reviews).
//!
//! Every Stick-writing command refuses without a token from here
//! (`stick_approval`). The window shows the plan, asks for the deliberate
//! hold, and passes the token with the write. The token is bound to this
//! exact operation, the Stick's volume and revision as read here, and (for
//! a write whose contents come from the Library) the Library generation.
//!
//! For an export or sync the plan is the export's own: the same selection
//! the write would build (without `recover()`), `rbl_export::plan::preview`
//! for the audio, and a comparison with what the Stick holds (its manifest
//! and databases) for what else changes: each track already there whose
//! data differs, field by field, and each playlist written again, with the
//! entries it gains and loses. Cues and analysis are rewritten from the
//! Library and not itemised; the plan says so.
//!
//! Nothing here writes or settles anything. A Stick with an unfinished
//! write is reported `pending`, with nothing planned and no token.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::commands::{blocking, held_on_stick, track_export_selection, ExportSelection, EXPORT_CACHE};
use crate::device_library::DeviceEditDto;
use crate::device_settings::DeviceSettingsDto;
use crate::error::{AppError, AppResult, ErrorKind};
use crate::state::AppState;
use crate::stick_approval::{self, Grant, StickOperation};

/// A write to plan, on the Stick at `destination`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StickWriteRequest {
    pub destination: String,
    pub operation: PlannedOperation,
}

/// The writes the window can ask to plan.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PlannedOperation {
    /// Export Playlist (or Export Folder).
    ExportPlaylist { playlist: String, delete_unlisted_music: Option<bool>, compatibility_format: Option<rbl_export::CompatibilityFormat> },
    /// Export Track.
    ExportTracks { tracks: Vec<String>, compatibility_format: Option<rbl_export::CompatibilityFormat> },
    /// The Sync Manager's SYNC, for this Stick.
    Sync {
        playlists: Vec<String>,
        delete_unlisted_music: Option<bool>,
        automatic: Option<bool>,
        compatibility_format: Option<rbl_export::CompatibilityFormat>,
    },
    /// Tracks added to one of the Stick's playlists: the ones it lacks are
    /// exported to it first (`export_first`, Library ids), then at most
    /// `count` entries go into the playlist.
    AddToPlaylist {
        format: String,
        playlist: String,
        export_first: Vec<String>,
        count: usize,
        duplicates: bool,
        compatibility_format: Option<rbl_export::CompatibilityFormat>,
    },
    /// One edit to one library's playlists on the Stick.
    PlaylistEdit { format: String, edit: DeviceEditDto },
    /// Loose files on the Stick added to one of its playlists.
    AddFiles { format: String, playlist: String, files: Vec<String> },
    /// The Stick's own settings.
    SaveSettings { settings: DeviceSettingsDto },
    /// Defaults written to a Stick that has none.
    WriteDefaults,
    /// An empty library put on a blank Stick.
    EnsureLibrary,
    /// The Stick erased.
    Format { layout: String },
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
    /// Entries it gains and loses against what the Stick holds; `None` when
    /// the Stick does not record its entries (an older export) or for a folder.
    pub added: Option<usize>,
    pub removed: Option<usize>,
}

/// A track already on the Stick whose exported data the write changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackChange {
    pub title: String,
    /// The fields that differ, in the window's words ("Energy", "Title", "MyTags").
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StickWritePlan {
    /// The Stick carries the Spare stick marker.
    pub spare: bool,
    /// An earlier write did not finish: nothing may be written until it is
    /// settled, nothing is planned and there is no token.
    pub pending: bool,
    /// Tracks whose audio is copied to the Stick (new, or changed at the source).
    pub tracks_new: usize,
    pub tracks_changed: usize,
    /// Tracks whose audio is on the Stick already and is reused. Their data
    /// (tags, Energy, cues, analysis) is still written; see `track_changes`.
    pub audio_reused: usize,
    pub tracks_removed: usize,
    pub tracks_missing: usize,
    /// Tracks already on the Stick whose exported data differs from the Library's.
    pub track_changes: Vec<TrackChange>,
    /// How many tracks' cues and analysis are written from the Library (not itemised).
    pub analysis_written: usize,
    /// Every playlist and folder the write puts on the Stick, in order.
    pub playlists: Vec<PlannedPlaylist>,
    pub net_additional_bytes: i64,
    /// The token the write must carry; `None` when nothing may be written.
    pub approval: Option<String>,
}

#[tauri::command]
pub async fn stick_write_plan(
    state: tauri::State<'_, std::sync::Arc<AppState>>,
    request: StickWriteRequest,
) -> AppResult<StickWritePlan> {
    let state = std::sync::Arc::clone(&state);
    blocking("stick_write_plan", move || plan(&state, &request)).await
}

/// The plan for `request`, and its token; writes nothing.
pub fn plan(state: &AppState, request: &StickWriteRequest) -> AppResult<StickWritePlan> {
    let destination = Path::new(&request.destination);
    if !destination.is_dir() {
        return Err(AppError::new(ErrorKind::NotFound, "That device is no longer connected."));
    }
    let spare = destination.join(wreckordbox_stick_outline::SPARE_MARKER).is_file();
    // As the writes do: nothing of ours writes to the Stick while it is planned.
    let _lock = stick_approval::lock(destination);
    let _lease = rbl_core::durable::read_lease(destination).map_err(|e| AppError::internal(e.to_string()))?;
    if rbl_export::recovery_pending(destination) {
        return Ok(StickWritePlan { spare, pending: true, ..StickWritePlan::default() });
    }
    let generation = u64::from(state.summary().3);
    let (mut found, steps, bound) = match &request.operation {
        PlannedOperation::ExportPlaylist { playlist, delete_unlisted_music, compatibility_format } => {
            let delete = delete_unlisted_music.unwrap_or(false);
            let selection = playlist_selection(state, destination, playlist, delete)?;
            let found = export_plan(state, destination, &selection, *compatibility_format)?;
            (found, vec![Grant::Exact(StickOperation::ExportPlaylist { playlist: playlist.clone(), delete_unlisted_music: delete })], true)
        }
        PlannedOperation::ExportTracks { tracks, compatibility_format } => {
            let library = state.library()?;
            let selection = track_export_selection(state, &library, &state.share_root(), destination, tracks)?;
            let found = export_plan(state, destination, &selection, *compatibility_format)?;
            (found, vec![Grant::Exact(StickOperation::ExportTracks { tracks: tracks.clone() })], true)
        }
        PlannedOperation::Sync { playlists, delete_unlisted_music, automatic, compatibility_format } => {
            let delete = delete_unlisted_music.unwrap_or(false);
            let automatic = automatic.unwrap_or(false);
            let library = state.library()?;
            let share_root = state.share_root();
            let selection = ExportSelection::from_playlists(state, &library, &share_root, playlists, automatic)?;
            let selection = match selection.for_stick_inspected(state, &library, &share_root, destination, delete, false)? {
                std::borrow::Cow::Owned(kept) => kept,
                std::borrow::Cow::Borrowed(_) => selection,
            };
            let found = export_plan(state, destination, &selection, *compatibility_format)?;
            (found, vec![Grant::Exact(StickOperation::Sync { playlists: playlists.clone(), delete_unlisted_music: delete, automatic })], true)
        }
        PlannedOperation::AddToPlaylist { format, playlist, export_first, count, duplicates, compatibility_format } => {
            let found = if export_first.is_empty() {
                StickWritePlan::default()
            } else {
                let library = state.library()?;
                let selection = track_export_selection(state, &library, &state.share_root(), destination, export_first)?;
                export_plan(state, destination, &selection, *compatibility_format)?
            };
            let mut steps = Vec::new();
            if !export_first.is_empty() {
                steps.push(Grant::ExportSome(export_first.iter().cloned().collect::<BTreeSet<_>>()));
            }
            steps.push(Grant::AddTo { format: format.clone(), playlist: playlist.clone(), max: *count, duplicates: *duplicates });
            (found, steps, !export_first.is_empty())
        }
        PlannedOperation::PlaylistEdit { format, edit } => (
            StickWritePlan::default(),
            vec![Grant::Exact(StickOperation::PlaylistEdit { format: format.clone(), edit: json(edit)? })],
            false,
        ),
        PlannedOperation::AddFiles { format, playlist, files } => (
            StickWritePlan::default(),
            vec![Grant::Exact(StickOperation::PlaylistAddFiles { format: format.clone(), playlist: playlist.clone(), files: files.clone() })],
            false,
        ),
        PlannedOperation::SaveSettings { settings } => (
            StickWritePlan::default(),
            vec![Grant::Exact(StickOperation::SaveSettings { settings: json(settings)? })],
            false,
        ),
        PlannedOperation::WriteDefaults => (StickWritePlan::default(), vec![Grant::Exact(StickOperation::WriteDefaults)], false),
        PlannedOperation::EnsureLibrary => (StickWritePlan::default(), vec![Grant::Exact(StickOperation::EnsureLibrary)], false),
        PlannedOperation::Format { layout } => {
            (StickWritePlan::default(), vec![Grant::Exact(StickOperation::Format { layout: layout.clone() })], false)
        }
    };
    found.spare = spare;
    found.approval = Some(stick_approval::issue(destination, steps, bound.then_some(generation))?);
    Ok(found)
}

/// The same JSON the write command makes of its argument.
fn json<T: Serialize>(value: &T) -> AppResult<serde_json::Value> {
    serde_json::to_value(value).map_err(|e| AppError::internal(e.to_string()))
}

/// Export Playlist's selection, as `export_playlist` builds it, without
/// settling anything first.
fn playlist_selection(state: &AppState, destination: &Path, playlist: &str, delete: bool) -> AppResult<ExportSelection> {
    let library = state.library()?;
    let share_root = state.share_root();
    let (mut playlists, automatic) = held_on_stick(state, &library, destination);
    if !playlists.iter().any(|id| id == playlist) {
        playlists.push(playlist.to_owned());
    }
    let mut selection = ExportSelection::from_playlists(state, &library, &share_root, &playlists, automatic)?;
    selection.keep_unlisted = true;
    Ok(match selection.for_stick_inspected(state, &library, &share_root, destination, delete, false)? {
        std::borrow::Cow::Owned(kept) => kept,
        std::borrow::Cow::Borrowed(_) => selection,
    })
}

/// The audio from the export's own preview, and the rest from comparing the
/// selection with what the Stick holds.
fn export_plan(
    state: &AppState,
    destination: &Path,
    selection: &ExportSelection,
    compatibility: Option<rbl_export::CompatibilityFormat>,
) -> AppResult<StickWritePlan> {
    let options = rbl_export::ExportOptions {
        sync: Some(&selection.sync),
        compatibility,
        keep_unlisted: selection.keep_unlisted,
        cache: Some(&EXPORT_CACHE),
        ..Default::default()
    };
    let preview = rbl_export::plan::preview(destination, &selection.tracks, &selection.playlists, &selection.my_tags, &options)
        .map_err(|e| AppError::internal(e.to_string()))?;
    let library = state.library()?;
    let (held, _) = held_on_stick(state, &library, destination);
    let manifest = rbl_export::Manifest::load(destination);
    let snapshot = rbl_export::snapshot::Snapshot::read(destination).ok();

    // Each playlist: created, or written again with what it gains and loses.
    let entries_on_stick: HashMap<u64, Option<Vec<String>>> = manifest
        .as_ref()
        .map(|m| m.playlists.iter().filter(|p| !p.device_only).map(|p| (p.library_id, p.library_entries.clone())).collect())
        .unwrap_or_default();
    let playlists = selection
        .playlists
        .iter()
        .map(|p| {
            let change = if held.iter().any(|id| *id == p.id.to_string()) { PlaylistChange::Resynchronised } else { PlaylistChange::Created };
            // Keyed as the export records them (`rbl_export::track_key`).
            let next: Vec<String> = p
                .track_indices
                .iter()
                .filter_map(|i| selection.tracks.get(*i))
                .map(|t| rbl_export::track_key(t.id, &t.source_path.to_string_lossy()))
                .collect();
            let (added, removed) = if p.folder {
                (None, None)
            } else if change == PlaylistChange::Created {
                (Some(next.len()), Some(0))
            } else if let Some(Some(before)) = entries_on_stick.get(&p.id) {
                let (gained, lost) = multiset_difference(before, &next);
                (Some(gained), Some(lost))
            } else {
                (None, None)
            };
            PlannedPlaylist { name: p.name.clone(), folder: p.folder, change, added, removed }
        })
        .collect();

    // Tracks already on the Stick whose data differs.
    let on_stick: HashMap<u64, u32> = manifest.as_ref().map(|m| m.tracks.iter().map(|t| (t.library_id, t.export_id)).collect()).unwrap_or_default();
    let stick_tracks: HashMap<u32, &rbl_export::snapshot::Track> =
        snapshot.as_ref().and_then(|s| s.legacy.as_ref()).map(|l| l.tracks.iter().map(|t| (t.id, t)).collect()).unwrap_or_default();
    let stick_tag_names: HashMap<u64, &str> = snapshot.as_ref().map(|s| s.my_tags.iter().map(|t| (t.id, t.name.as_str())).collect()).unwrap_or_default();
    let library_tag_names: HashMap<u64, &str> = selection.my_tags.iter().map(|t| (t.id, t.name.as_str())).collect();
    let mut track_changes = Vec::new();
    for track in &selection.tracks {
        let Some(export_id) = on_stick.get(&track.id) else { continue };
        let Some(was) = stick_tracks.get(export_id) else { continue };
        let mut fields: Vec<String> = Vec::new();
        for (name, same) in [
            ("Title", was.title == track.title),
            ("Artist", was.artist == track.artist),
            ("Album", was.album == track.album),
            ("Genre", was.genre == track.genre),
            ("Label", was.label == track.label),
            ("Comment", was.comment == track.comment),
            ("BPM", was.bpm == track.bpm_x100),
            ("Energy", was.rating == u32::from(track.rating)),
            ("Colour", was.color == u32::from(track.color_id)),
        ] {
            if !same {
                fields.push(name.to_owned());
            }
        }
        if let Some(memberships) = snapshot.as_ref().map(|s| &s.tag_memberships) {
            let before: BTreeSet<&str> =
                memberships.get(export_id).map(|ids| ids.iter().filter_map(|id| stick_tag_names.get(id).copied()).collect()).unwrap_or_default();
            let after: BTreeSet<&str> = track.my_tags.iter().filter_map(|id| library_tag_names.get(id).copied()).collect();
            if before != after {
                fields.push("MyTags".to_owned());
            }
        }
        if !fields.is_empty() {
            track_changes.push(TrackChange { title: track.title.clone(), fields });
        }
    }
    let found = preview.estimate;
    Ok(StickWritePlan {
        spare: false,
        pending: false,
        tracks_new: found.tracks_new,
        tracks_changed: found.tracks_changed,
        audio_reused: found.tracks_kept,
        tracks_removed: found.tracks_removed,
        tracks_missing: found.tracks_missing,
        track_changes,
        analysis_written: selection.tracks.len().saturating_sub(found.tracks_missing),
        playlists,
        net_additional_bytes: preview.net_additional_bytes,
        approval: None,
    })
}

/// How many entries `next` has that `before` does not, and the reverse, counting repeats.
fn multiset_difference(before: &[String], next: &[String]) -> (usize, usize) {
    let mut counts: BTreeMap<&str, i64> = BTreeMap::new();
    for id in before {
        *counts.entry(id).or_default() -= 1;
    }
    for id in next {
        *counts.entry(id).or_default() += 1;
    }
    let gained: u64 = counts.values().filter(|n| **n > 0).map(|n| n.unsigned_abs()).sum();
    let lost: u64 = counts.values().filter(|n| **n < 0).map(|n| n.unsigned_abs()).sum();
    (usize::try_from(gained).unwrap_or(usize::MAX), usize::try_from(lost).unwrap_or(usize::MAX))
}
