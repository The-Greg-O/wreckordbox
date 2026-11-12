//! What a Stick write would do, worked out before anything is written
//! (wbx-45.4, all three reviews).
//!
//! The window shows the plan and asks for the deliberate hold; only then
//! does it call `stick_write_approve` with the plan's id, which answers the
//! token the write must carry (`stick_approval`). The plan id alone writes
//! nothing; a cancelled plan is revoked with `stick_write_revoke`.
//!
//! For an export or sync the plan is the export's own: the same selection
//! the write would build (without `recover()`), `rbl_export::plan::preview`
//! for the audio, and a comparison with what the Stick holds (its manifest
//! and databases): each track already there whose data differs, field by
//! field, and each playlist written again, with the entries it gains and
//! loses and whether the entries it keeps change order. Cues and analysis
//! are rewritten from the Library and not itemised; the plan says so.
//!
//! The plan also says whether the destination is a Stick at all (an
//! ordinary folder needs no plan, token or unlock) and whether Stick
//! Protection would refuse the write now (wbx-45.11).
//!
//! Nothing here writes or settles anything. A Stick with an unfinished
//! write is reported `pending`, with nothing planned.

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::commands::{blocking, held_on_stick, track_export_selection, ExportSelection};
use crate::device_library::DeviceEditDto;
use crate::device_settings::DeviceSettingsDto;
use crate::device_settings::StickDefaultsDto;
use crate::error::{AppError, AppResult, ErrorKind};
use crate::state::AppState;
use crate::stick_approval::{self, bound, StickOperation};

/// A write to plan, on the Stick at `destination`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StickWriteRequest {
    pub destination: String,
    pub operation: PlannedOperation,
}

/// The writes the window can ask to plan, each with every argument its
/// command takes that changes what is written.
#[derive(Debug, Clone, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PlannedOperation {
    /// Export Playlist (or Export Folder).
    ExportPlaylist {
        playlist: String,
        delete_unlisted_music: Option<bool>,
        defaults: Option<StickDefaultsDto>,
        compatibility_format: Option<rbl_export::CompatibilityFormat>,
    },
    /// Export Track.
    ExportTracks {
        tracks: Vec<String>,
        defaults: Option<StickDefaultsDto>,
        compatibility_format: Option<rbl_export::CompatibilityFormat>,
    },
    /// The Sync Manager's SYNC, for this Stick.
    Sync {
        playlists: Vec<String>,
        delete_unlisted_music: Option<bool>,
        automatic: Option<bool>,
        defaults: Option<StickDefaultsDto>,
        compatibility_format: Option<rbl_export::CompatibilityFormat>,
    },
    /// Library tracks, in this order, added to one of the Stick's
    /// playlists; the ones it lacks are exported to it first.
    AddLibraryTracks {
        format: String,
        playlist: String,
        tracks: Vec<String>,
        duplicates: bool,
        defaults: Option<StickDefaultsDto>,
        compatibility_format: Option<rbl_export::CompatibilityFormat>,
    },
    /// One edit to one library's playlists on the Stick.
    PlaylistEdit { format: String, edit: DeviceEditDto },
    /// Loose files on the Stick added to one of its playlists.
    AddFiles {
        format: String,
        playlist: String,
        files: Vec<String>,
    },
    /// The Stick's own settings.
    SaveSettings { settings: DeviceSettingsDto },
    /// An empty library put on a blank Stick, from these defaults.
    EnsureLibrary { defaults: Option<StickDefaultsDto> },
    /// The Stick erased.
    Format { layout: String },
    /// The unfinished write finished or rolled back (`stick_settle`): the one
    /// write planned on a Stick that holds one.
    Settle,
}

pub use wreckordbox_stick_write::plan::{PlannedPlaylist, PlaylistChange, TrackChange};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent facts the window shows, as sent"
)]
pub struct StickWritePlan {
    /// The destination is on a mounted Stick. When not (an ordinary
    /// folder), nothing else here applies: no plan, token or unlock.
    pub on_stick: bool,
    /// The Stick's name, as the DJ types it to unlock a Gig stick.
    pub stick_name: String,
    /// Stick Protection refuses any write to it now (wbx-45.11): the DJ
    /// must unlock it first.
    pub protected: bool,
    /// The Stick carries the Spare stick marker.
    pub spare: bool,
    /// An earlier write did not finish: nothing may be written until it is
    /// settled, nothing is planned and there is no token.
    pub pending: bool,
    /// Whether the audio counts below were worked out: not on a Stick whose
    /// library the export's preview could not open without writing to it.
    pub audio_estimated: bool,
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
    /// For adding Library tracks to a Stick playlist: how many entries go in.
    pub adding: Option<usize>,
    /// The plan's id, which `stick_write_approve` turns into the write's
    /// token once the DJ confirms; `None` when nothing may be written.
    pub plan: Option<String>,
}

#[tauri::command]
pub async fn stick_write_plan(
    state: tauri::State<'_, Arc<AppState>>,
    request: StickWriteRequest,
) -> AppResult<StickWritePlan> {
    let state = Arc::clone(&state);
    blocking("stick_write_plan", move || plan(&state, &request)).await
}

/// The DJ held to confirm the plan `plan`: answers the token its write
/// carries. Refused when the Stick changed or was replaced since the plan.
#[tauri::command]
pub async fn stick_write_approve(
    state: tauri::State<'_, Arc<AppState>>,
    plan: String,
) -> AppResult<String> {
    let state_dir = state.backup_dir().to_path_buf();
    blocking("stick_write_approve", move || {
        stick_approval::approve(&state_dir, &plan)
    })
    .await
}

/// The DJ cancelled the plan `plan`: it can no longer be confirmed.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri passes command arguments by value"
)]
pub fn stick_write_revoke(plan: String) {
    stick_approval::revoke(&plan);
}

/// The plan for `request`, and its id; writes nothing.
#[allow(
    clippy::too_many_lines,
    reason = "one arm per Stick-writing operation, each a few lines"
)]
pub fn plan(state: &AppState, request: &StickWriteRequest) -> AppResult<StickWritePlan> {
    let asked = Path::new(&request.destination);
    if !asked.is_dir() {
        return Err(AppError::new(
            ErrorKind::NotFound,
            "That device is no longer connected.",
        ));
    }
    let exporting = matches!(
        request.operation,
        PlannedOperation::ExportPlaylist { .. }
            | PlannedOperation::ExportTracks { .. }
            | PlannedOperation::Sync { .. }
    );
    // Exports can go to an ordinary folder: nothing to plan or confirm.
    let Some(root) =
        crate::stick_guard::stick_of(asked).or_else(|| (!exporting).then(|| asked.to_path_buf()))
    else {
        return Ok(StickWritePlan::default());
    };
    let destination = asked;
    let protection = wreckordbox_stick_protection::status(state.backup_dir(), &root);
    let spare = protection.spare;
    let stick = |found: StickWritePlan| StickWritePlan {
        on_stick: true,
        stick_name: protection.name.clone(),
        protected: protection.protected,
        spare,
        ..found
    };
    // As the writes do, in the one lock order (`stick_guard`): the Library
    // edit gate, then the Stick's execution lock; nothing writes to the
    // Stick, or edits the Library, while it is planned.
    let _gate = state.edit_gate().lock();
    let _lock = stick_approval::lock(state.backup_dir(), &root)?;
    let _lease = rbl_core::durable::read_lease(destination)
        .map_err(|e| AppError::internal(e.to_string()))?;
    let pending = rbl_export::recovery_pending(destination);
    if matches!(request.operation, PlannedOperation::Settle) {
        // Settling is for a Stick with an unfinished write, and only then:
        // bound to the Stick as it is, journal and all.
        if !pending {
            return Err(AppError::new(
                ErrorKind::NotFound,
                "There is no unfinished write on this Stick to settle.",
            ));
        }
        return Ok(stick(StickWritePlan {
            pending: true,
            plan: Some(stick_approval::propose(
                destination,
                StickOperation::Settle,
                None,
            )?),
            ..StickWritePlan::default()
        }));
    }
    if pending {
        return Ok(stick(StickWritePlan {
            pending: true,
            ..StickWritePlan::default()
        }));
    }
    // One snapshot of the Library and its generation: the plan reads it, the
    // token binds it, and the write checks it against its own (wbx-45.4).
    let (library, generation) = state.library_at_generation()?;
    let share_root = state.share_root();
    let (found, operation) = match &request.operation {
        PlannedOperation::ExportPlaylist {
            playlist,
            delete_unlisted_music,
            defaults,
            compatibility_format,
        } => {
            let delete = delete_unlisted_music.unwrap_or(false);
            let selection = playlist_selection(state, &library, destination, playlist, delete)?;
            (
                export_plan(
                    state,
                    &library,
                    destination,
                    &selection,
                    *compatibility_format,
                )?,
                StickOperation::ExportPlaylist {
                    playlist: playlist.clone(),
                    delete_unlisted_music: delete,
                    defaults: bound(defaults.as_ref()),
                    compatibility: bound(compatibility_format.as_ref()),
                },
            )
        }
        PlannedOperation::ExportTracks {
            tracks,
            defaults,
            compatibility_format,
        } => {
            let selection =
                track_export_selection(state, &library, &share_root, destination, tracks)?;
            (
                export_plan(
                    state,
                    &library,
                    destination,
                    &selection,
                    *compatibility_format,
                )?,
                StickOperation::ExportTracks {
                    tracks: tracks.clone(),
                    defaults: bound(defaults.as_ref()),
                    compatibility: bound(compatibility_format.as_ref()),
                },
            )
        }
        PlannedOperation::Sync {
            playlists,
            delete_unlisted_music,
            automatic,
            defaults,
            compatibility_format,
        } => {
            let delete = delete_unlisted_music.unwrap_or(false);
            let automatic = automatic.unwrap_or(false);
            let selection = ExportSelection::from_playlists(
                state,
                &library,
                &share_root,
                playlists,
                automatic,
            )?;
            let selection = match selection.for_stick_inspected(
                state,
                &library,
                &share_root,
                destination,
                delete,
                false,
            )? {
                std::borrow::Cow::Owned(kept) => kept,
                std::borrow::Cow::Borrowed(_) => selection,
            };
            (
                export_plan(
                    state,
                    &library,
                    destination,
                    &selection,
                    *compatibility_format,
                )?,
                StickOperation::Sync {
                    playlists: playlists.clone(),
                    delete_unlisted_music: delete,
                    automatic,
                    defaults: bound(defaults.as_ref()),
                    compatibility: bound(compatibility_format.as_ref()),
                },
            )
        }
        PlannedOperation::AddLibraryTracks {
            format,
            playlist,
            tracks,
            duplicates,
            defaults,
            compatibility_format,
        } => {
            let resolved = crate::device_library::resolve_library_tracks(
                state,
                destination,
                format,
                playlist,
                tracks,
                *duplicates,
            )?;
            let mut found = if resolved.missing.is_empty() {
                StickWritePlan::default()
            } else {
                let selection = track_export_selection(
                    state,
                    &library,
                    &share_root,
                    destination,
                    &resolved.missing,
                )?;
                export_plan(
                    state,
                    &library,
                    destination,
                    &selection,
                    *compatibility_format,
                )?
            };
            found.adding = Some(resolved.adding);
            (
                found,
                StickOperation::AddLibraryTracks {
                    format: format.clone(),
                    playlist: playlist.clone(),
                    tracks: tracks.clone(),
                    duplicates: *duplicates,
                    defaults: bound(defaults.as_ref()),
                    compatibility: bound(compatibility_format.as_ref()),
                },
            )
        }
        PlannedOperation::PlaylistEdit { format, edit } => (
            StickWritePlan::default(),
            StickOperation::PlaylistEdit {
                format: format.clone(),
                edit: json(edit)?,
            },
        ),
        PlannedOperation::AddFiles {
            format,
            playlist,
            files,
        } => (
            StickWritePlan::default(),
            StickOperation::PlaylistAddFiles {
                format: format.clone(),
                playlist: playlist.clone(),
                files: files.clone(),
            },
        ),
        PlannedOperation::SaveSettings { settings } => (
            StickWritePlan::default(),
            StickOperation::SaveSettings {
                settings: json(settings)?,
            },
        ),
        PlannedOperation::EnsureLibrary { defaults } => (
            StickWritePlan::default(),
            StickOperation::EnsureLibrary {
                defaults: bound(defaults.as_ref()),
            },
        ),
        PlannedOperation::Format { layout } => (
            StickWritePlan::default(),
            StickOperation::Format {
                layout: layout.clone(),
            },
        ),
        PlannedOperation::Settle => unreachable!("settling is planned above"),
    };
    let id = stick_approval::propose(destination, operation, Some(generation))?;
    Ok(stick(StickWritePlan {
        plan: Some(id),
        ..found
    }))
}

/// The same JSON the write command makes of its argument.
fn json<T: Serialize>(value: &T) -> AppResult<serde_json::Value> {
    serde_json::to_value(value).map_err(|e| AppError::internal(e.to_string()))
}

/// Export Playlist's selection, as `export_playlist` builds it, without
/// settling anything first.
fn playlist_selection(
    state: &AppState,
    library: &rbl_index::Library,
    destination: &Path,
    playlist: &str,
    delete: bool,
) -> AppResult<ExportSelection> {
    let share_root = state.share_root();
    let (mut playlists, automatic) = held_on_stick(state, library, destination);
    if !playlists.iter().any(|id| id == playlist) {
        playlists.push(playlist.to_owned());
    }
    let mut selection =
        ExportSelection::from_playlists(state, library, &share_root, &playlists, automatic)?;
    selection.keep_unlisted = true;
    Ok(
        match selection.for_stick_inspected(
            state,
            library,
            &share_root,
            destination,
            delete,
            false,
        )? {
            std::borrow::Cow::Owned(kept) => kept,
            std::borrow::Cow::Borrowed(_) => selection,
        },
    )
}

/// The shared planner's reckoning (`wreckordbox_stick_write::plan`, the
/// one the service's Stick load uses too), in the window's plan.
fn export_plan(
    state: &AppState,
    library: &rbl_index::Library,
    destination: &Path,
    selection: &ExportSelection,
    compatibility: Option<rbl_export::CompatibilityFormat>,
) -> AppResult<StickWritePlan> {
    let found = wreckordbox_stick_write::plan::export_plan(
        state,
        library,
        destination,
        selection,
        compatibility,
    )?;
    Ok(StickWritePlan {
        audio_estimated: found.audio_estimated,
        tracks_new: found.tracks_new,
        tracks_changed: found.tracks_changed,
        audio_reused: found.audio_reused,
        tracks_removed: found.tracks_removed,
        tracks_missing: found.tracks_missing,
        track_changes: found.track_changes,
        analysis_written: found.analysis_written,
        playlists: found.playlists,
        net_additional_bytes: found.net_additional_bytes,
        ..StickWritePlan::default()
    })
}
