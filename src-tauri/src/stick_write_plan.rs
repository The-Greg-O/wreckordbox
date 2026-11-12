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

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::commands::{
    blocking, held_on_stick, track_export_selection, ExportSelection, EXPORT_CACHE,
};
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
    /// The entries it keeps are in another order than on the Stick; `None`
    /// when that cannot be compared (as `added`).
    pub reordered: Option<bool>,
}

/// A track already on the Stick whose exported data the write changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackChange {
    pub title: String,
    /// The fields that differ, in the window's words (`Energy`, `Title`, `MyTags`).
    pub fields: Vec<String>,
}

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
    blocking("stick_write_approve", move || stick_approval::approve(&state_dir, &plan)).await
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

/// The audio from the export's own preview, and the rest from comparing the
/// selection with what the Stick holds.
#[allow(
    clippy::too_many_lines,
    reason = "the plan's parts, each read from the Stick once"
)]
fn export_plan(
    state: &AppState,
    library: &rbl_index::Library,
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
    // The Stick's databases are read through the non-recovering reader
    // (wbx-45.5): its off-Stick copies, validated when the read finishes.
    let read = wreckordbox_stick_outline::StickRead::begin(destination)
        .map_err(|e| crate::stick_recovery::read_error(destination, e))?;
    let mirror = read
        .mirror()
        .map_err(|e| crate::stick_recovery::read_error(destination, e))?;
    let snapshot = rbl_export::snapshot::Snapshot::read(mirror.path()).ok();
    read.finish()
        .map_err(|e| crate::stick_recovery::read_error(destination, e))?;
    // The audio preview is the export's own code, which opens the Stick's
    // `exportLibrary.db` in place: on a Stick where that could create files,
    // it is not run, and the plan says the audio was not estimated, as the
    // Sync Manager's estimate does (wbx-45.5).
    let audio_estimated = !wreckordbox_stick_outline::one_library::sqlite_untouchable(destination);
    let preview = if audio_estimated {
        Some(
            rbl_export::plan::preview(
                destination,
                &selection.tracks,
                &selection.playlists,
                &selection.my_tags,
                &options,
            )
            .map_err(|e| AppError::internal(e.to_string()))?,
        )
    } else {
        None
    };
    let (held, _) = held_on_stick(state, library, destination);
    let manifest = rbl_export::Manifest::load(destination);

    // Each playlist: created, or written again with what it gains and loses.
    let entries_on_stick: HashMap<u64, Option<Vec<String>>> = manifest
        .as_ref()
        .map(|m| {
            m.playlists
                .iter()
                .filter(|p| !p.device_only)
                .map(|p| (p.library_id, p.library_entries.clone()))
                .collect()
        })
        .unwrap_or_default();
    let playlists = selection
        .playlists
        .iter()
        .map(|p| {
            let change = if held.iter().any(|id| *id == p.id.to_string()) {
                PlaylistChange::Resynchronised
            } else {
                PlaylistChange::Created
            };
            // Keyed as the export records them (`rbl_export::track_key`).
            let next: Vec<String> = p
                .track_indices
                .iter()
                .filter_map(|i| selection.tracks.get(*i))
                .map(|t| rbl_export::track_key(t.id, &t.source_path.to_string_lossy()))
                .collect();
            let (added, removed, reordered) = if p.folder {
                (None, None, None)
            } else if change == PlaylistChange::Created {
                (Some(next.len()), Some(0), Some(false))
            } else if let Some(Some(before)) = entries_on_stick.get(&p.id) {
                let (gained, lost) = multiset_difference(before, &next);
                (Some(gained), Some(lost), Some(reordered(before, &next)))
            } else {
                (None, None, None)
            };
            PlannedPlaylist {
                name: p.name.clone(),
                folder: p.folder,
                change,
                added,
                removed,
                reordered,
            }
        })
        .collect();

    // Tracks already on the Stick whose data differs.
    let on_stick: HashMap<u64, u32> = manifest
        .as_ref()
        .map(|m| {
            m.tracks
                .iter()
                .map(|t| (t.library_id, t.export_id))
                .collect()
        })
        .unwrap_or_default();
    let stick_tracks: HashMap<u32, &rbl_export::snapshot::Track> = snapshot
        .as_ref()
        .and_then(|s| s.legacy.as_ref())
        .map(|l| l.tracks.iter().map(|t| (t.id, t)).collect())
        .unwrap_or_default();
    let stick_tag_names: HashMap<u64, &str> = snapshot
        .as_ref()
        .map(|s| s.my_tags.iter().map(|t| (t.id, t.name.as_str())).collect())
        .unwrap_or_default();
    let library_tag_names: HashMap<u64, &str> = selection
        .my_tags
        .iter()
        .map(|t| (t.id, t.name.as_str()))
        .collect();
    let mut track_changes = Vec::new();
    for track in &selection.tracks {
        let Some(export_id) = on_stick.get(&track.id) else {
            continue;
        };
        let Some(was) = stick_tracks.get(export_id) else {
            continue;
        };
        let mut fields: Vec<String> = Vec::new();
        for (name, same) in [
            ("Title", was.title == track.title),
            ("Artist", was.artist == track.artist),
            ("Album", was.album == track.album),
            ("Genre", was.genre == track.genre),
            ("Label", was.label == track.label),
            ("Comment", was.comment == track.comment),
            ("BPM", was.bpm == track.bpm_x100),
            ("Key", was.key == track.key),
            ("Energy", was.rating == u32::from(track.rating)),
            ("Colour", was.color == u32::from(track.color_id)),
        ] {
            if !same {
                fields.push(name.to_owned());
            }
        }
        if let Some(memberships) = snapshot.as_ref().map(|s| &s.tag_memberships) {
            let before: BTreeSet<&str> = memberships
                .get(export_id)
                .map(|ids| {
                    ids.iter()
                        .filter_map(|id| stick_tag_names.get(id).copied())
                        .collect()
                })
                .unwrap_or_default();
            let after: BTreeSet<&str> = track
                .my_tags
                .iter()
                .filter_map(|id| library_tag_names.get(id).copied())
                .collect();
            if before != after {
                fields.push("MyTags".to_owned());
            }
        }
        if !fields.is_empty() {
            track_changes.push(TrackChange {
                title: track.title.clone(),
                fields,
            });
        }
    }
    let found = preview
        .as_ref()
        .map(|p| p.estimate.clone())
        .unwrap_or_default();
    Ok(StickWritePlan {
        audio_estimated,
        tracks_new: found.tracks_new,
        tracks_changed: found.tracks_changed,
        audio_reused: found.tracks_kept,
        tracks_removed: found.tracks_removed,
        tracks_missing: found.tracks_missing,
        track_changes,
        analysis_written: selection.tracks.len().saturating_sub(found.tracks_missing),
        playlists,
        net_additional_bytes: preview.as_ref().map_or(0, |p| p.net_additional_bytes),
        ..StickWritePlan::default()
    })
}

/// Whether the entries `before` and `next` share are in another order in
/// `next`: each list reduced to the entries the other also has (counting
/// repeats), then compared in order.
fn reordered(before: &[String], next: &[String]) -> bool {
    fn kept<'a>(from: &'a [String], other: &[String]) -> Vec<&'a str> {
        let mut left: BTreeMap<&str, usize> = BTreeMap::new();
        for id in other {
            *left.entry(id).or_default() += 1;
        }
        from.iter()
            .filter(|id| {
                left.get_mut(id.as_str()).is_some_and(|n| {
                    let had = *n > 0;
                    *n = n.saturating_sub(1);
                    had
                })
            })
            .map(String::as_str)
            .collect()
    }
    kept(before, next) != kept(next, before)
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
    let gained: u64 = counts
        .values()
        .filter(|n| **n > 0)
        .map(|n| n.unsigned_abs())
        .sum();
    let lost: u64 = counts
        .values()
        .filter(|n| **n < 0)
        .map(|n| n.unsigned_abs())
        .sum();
    (
        usize::try_from(gained).unwrap_or(usize::MAX),
        usize::try_from(lost).unwrap_or(usize::MAX),
    )
}

#[cfg(test)]
mod tests {
    use super::reordered;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_playlist_whose_kept_entries_change_order_is_reordered() {
        assert!(!reordered(
            &ids(&["a", "b", "c"]),
            &ids(&["a", "b", "c", "d"])
        ));
        assert!(
            !reordered(&ids(&["a", "b", "c"]), &ids(&["a", "c"])),
            "a removal alone"
        );
        assert!(reordered(&ids(&["a", "b", "c"]), &ids(&["c", "a", "b"])));
        assert!(reordered(&ids(&["a", "b"]), &ids(&["x", "b", "a"])));
    }
}
