//! What writing an [`ExportSelection`] to a Stick would do, worked out
//! before anything is written (wbx-45.4, all three reviews; full lists for
//! wbx-70).
//!
//! The plan is the export's own: `rbl_export::plan::preview` for the audio
//! and the room it takes, and a comparison with what the Stick holds (its
//! manifest and databases): each track already there whose data differs,
//! field by field, and each playlist written again, with the entries it
//! gains and loses and whether the entries it keeps change order. Cues and
//! analysis are rewritten from the Library and not itemised.
//!
//! Nothing here writes or settles anything: the Stick's databases are read
//! through the non-recovering reader (wbx-45.5), and the audio preview is
//! not run on a Stick where opening its database could create files.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::selection::{held_on_stick_of, library_db_id, ExportSelection, LibrarySource};
use crate::{Result, StickWriteError};

/// The cache of converted audio every export shares, the window's and the
/// service's: one cache, one size limit.
pub static EXPORT_CACHE: LazyLock<rbl_export::cache::PreparationCache> = LazyLock::new(|| {
    let cache =
        rbl_export::cache::PreparationCache::new(rbl_backup::state_dir().join("cache/usb-export"));
    if let Err(error) = cache.prune(8 * 1024 * 1024 * 1024) {
        tracing::warn!(%error, "could not prune export cache");
    }
    cache
});

/// How a playlist on the Stick is changed by the write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlaylistChange {
    /// Not on the Stick from this Library yet.
    Created,
    /// On the Stick already, written again with what the Library holds now.
    Resynchronised,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackChange {
    pub title: String,
    /// The fields that differ, in the window's words (`Energy`, `Title`, `MyTags`).
    pub fields: Vec<String>,
    /// The track's Library id (`0` for none).
    #[serde(default)]
    pub id: u64,
}

/// One track named in a plan's lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedTrack {
    /// Its Library id; `0` for a track that did not come from the Library.
    pub id: u64,
    pub title: String,
}

/// What a write of an export selection would do to the Stick.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPlan {
    /// Whether the audio below was worked out: not on a Stick whose library
    /// the export's preview could not open without writing to it.
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
    /// The most room the write takes on the Stick at once: the new files,
    /// the staging folders and a second copy of the databases until the
    /// new library is published (the export's own reckoning).
    pub peak_bytes: u64,
    /// The sizes are estimates (a conversion, or a source whose revision
    /// could not be told).
    pub size_uncertain: bool,
    /// The tracks new to the Stick, in the order they are written.
    pub added: Vec<PlannedTrack>,
    /// The tracks on the Stick whose audio is copied again: changed at the source.
    pub audio_changed: Vec<PlannedTrack>,
    /// The tracks the write takes off the Stick.
    pub removed: Vec<PlannedTrack>,
    /// The tracks chosen whose audio file is missing: not written.
    pub missing: Vec<PlannedTrack>,
    /// This Library's playlists on the Stick that the write takes off it.
    pub playlists_removed: Vec<String>,
}

/// The audio from the export's own preview, and the rest from comparing the
/// selection with what the Stick holds.
#[allow(
    clippy::too_many_lines,
    reason = "the plan's parts, each read from the Stick once"
)]
pub fn export_plan(
    state: &dyn LibrarySource,
    library: &rbl_index::Library,
    destination: &Path,
    selection: &ExportSelection,
    compatibility: Option<rbl_export::CompatibilityFormat>,
) -> Result<ExportPlan> {
    export_plan_of(
        library_db_id(state),
        library,
        destination,
        selection,
        compatibility,
    )
}

/// [`export_plan`] for the Library whose identifier is `db_id` and whose
/// index snapshot is `library`: reads only the Stick and the selection's
/// files, never the Library, so it needs none of its locks (a load reading
/// a Stick back while it holds the Stick's lock, wbx-70).
#[allow(
    clippy::too_many_lines,
    reason = "the plan's parts, each read from the Stick once"
)]
pub fn export_plan_of(
    db_id: u64,
    library: &rbl_index::Library,
    destination: &Path,
    selection: &ExportSelection,
    compatibility: Option<rbl_export::CompatibilityFormat>,
) -> Result<ExportPlan> {
    let options = rbl_export::ExportOptions {
        sync: Some(&selection.sync),
        compatibility,
        keep_unlisted: selection.keep_unlisted,
        cache: Some(&EXPORT_CACHE),
        ..Default::default()
    };
    let read_error = |error| StickWriteError::Read {
        mount: destination.to_path_buf(),
        error,
    };
    // The Stick's databases are read through the non-recovering reader
    // (wbx-45.5): its off-Stick copies, validated when the read finishes.
    let read = wreckordbox_stick_outline::StickRead::begin(destination).map_err(read_error)?;
    let mirror = read.mirror().map_err(read_error)?;
    let snapshot = rbl_export::snapshot::Snapshot::read(mirror.path()).ok();
    read.finish().map_err(read_error)?;
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
            .map_err(|e| StickWriteError::internal(e.to_string()))?,
        )
    } else {
        None
    };
    let (held, _) = held_on_stick_of(db_id, library, destination);
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
                id: track.id,
            });
        }
    }

    // The full lists, from the preview's own reckoning of each track.
    let mut plan = ExportPlan::default();
    if let Some(preview) = &preview {
        let recorded: HashSet<String> = manifest
            .iter()
            .flat_map(|m| &m.tracks)
            .map(rbl_export::ManifestTrack::key)
            .collect();
        let mut written: HashSet<String> = HashSet::new();
        for track in &preview.tracks {
            let key = rbl_export::track_key(track.id, &track.source_path.to_string_lossy());
            if !written.insert(key.clone()) {
                continue;
            }
            let line = PlannedTrack {
                id: track.id,
                title: track.title.clone(),
            };
            match preview.estimate.actions.get(&key) {
                Some(
                    rbl_export::estimate::AudioAction::Copy
                    | rbl_export::estimate::AudioAction::Convert,
                ) => {
                    if recorded.contains(&key) {
                        plan.audio_changed.push(line);
                    } else {
                        plan.added.push(line);
                    }
                }
                Some(rbl_export::estimate::AudioAction::Missing) => plan.missing.push(line),
                _ => {}
            }
        }
        if let Some(manifest) = &manifest {
            // As the estimate counts them: what the Stick's record holds
            // that the write does not, but the Library's own files and the
            // tracks put on the Stick on their own.
            let loose: BTreeSet<u64> = manifest.loose.iter().copied().collect();
            let titles: HashMap<u32, &str> = stick_tracks
                .iter()
                .map(|(id, t)| (*id, t.title.as_str()))
                .collect();
            for entry in manifest
                .tracks
                .iter()
                .filter(|t| !written.contains(&t.key()) && !t.in_place)
                .filter(|t| !loose.contains(&t.library_id))
            {
                plan.removed.push(PlannedTrack {
                    id: entry.library_id,
                    title: titles
                        .get(&entry.export_id)
                        .map_or_else(|| entry.audio.clone(), |t| (*t).to_owned()),
                });
            }
            let kept: HashSet<u64> = preview.playlists.iter().map(|p| p.id).collect();
            if !selection.keep_unlisted {
                plan.playlists_removed = manifest
                    .playlists
                    .iter()
                    .filter(|p| !p.folder && !p.device_only && p.library_id != 0)
                    .filter(|p| !kept.contains(&p.library_id))
                    .map(|p| p.name.clone())
                    .collect();
            }
        }
        plan.peak_bytes = preview.peak_bytes;
        plan.size_uncertain = preview.unknown_size || preview.estimate.approximate;
    }
    let found = preview
        .as_ref()
        .map(|p| p.estimate.clone())
        .unwrap_or_default();
    Ok(ExportPlan {
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
        ..plan
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
