//! What a write leaves on a Stick that another writer put there (wbx-dc0).
//!
//! The export keeps another writer's tracks and playlists (rekordbox's
//! export from another computer, a player's edits), as rekordbox's own export
//! does: `rbl_export::reconcile` carries each such track into the write with
//! `device.preserve`, its source the Stick's own file, and the export leaves
//! that file where it is (`retained_audio`). The export's estimate did not
//! know that: it reckoned such a track as audio to copy whenever no record
//! of ours named it, so a plan built on it reported every one of them as
//! "added" and counted their audio again in the room needed (an 11-track
//! playlist onto a rekordbox Stick of 280 tracks: 291 added, 10.7 GB).
//!
//! This module tells those tracks and playlists apart from the preview, so
//! the plan says they stay. The room is the export's own to reckon: its
//! estimate now knows such a track's audio is not copied (it has to, since
//! the export checks the Stick's room with it before it writes); what is
//! taken off here is only the analysis files the export writes over those
//! already there, which take room while it works but none afterwards.
//!
//! Once a load of ours has been on the Stick, its record (the manifest)
//! names those tracks too, marked as carried, and an unchanged selection is
//! answered from that record alone, without the Stick's tracks in the
//! preview: they are then read from the record ([`of_record`]).

use std::collections::HashSet;
use std::path::Path;

use crate::plan::PlannedTrack;

/// Another writer's part of a write: kept on the Stick as it is.
#[derive(Debug, Default)]
pub(crate) struct Kept {
    /// The tracks, in the order the write lists them.
    pub tracks: Vec<PlannedTrack>,
    /// The playlists (not folders) the write keeps without being asked to.
    pub playlists: Vec<String>,
    /// The room the preview counted for their analysis files, which the
    /// export writes over those already there: needed while it works, not
    /// taken afterwards. In whole clusters, as the preview counted it.
    pub rewritten_bytes: u64,
}

/// Whether `track` is the Stick's own copy, carried as it is.
pub(crate) fn carried(track: &rbl_export::SourceTrack, destination: &Path) -> bool {
    // Not a chosen track whose file here is gone, which the export carries
    // from the Stick's copy the same way: that one is this Library's.
    track.device.as_ref().is_some_and(|d| d.preserve)
        && !track.source_unavailable
        && track.source_path.starts_with(destination)
}

/// Whether `entry` of the Stick's record is a track another writer put
/// there, which the export carried (as `rbl_export` tells them: the record
/// says so, and an older record names the Stick's own file as its source).
pub(crate) fn carried_in_record(entry: &rbl_export::ManifestTrack, destination: &Path) -> bool {
    entry.preserved.unwrap_or_else(|| {
        entry.library_id == 0
            && !entry.audio.is_empty()
            && Path::new(&entry.source) == destination.join(entry.audio.trim_start_matches('/'))
    })
}

/// Whether `preview` was answered from the Stick's record alone: nothing to
/// do, so the export did not read the Stick's own tracks and playlists into
/// it. (Every other preview is an estimate, and says so.)
pub(crate) fn from_record_alone(preview: &rbl_export::plan::SyncPlan) -> bool {
    !preview.approximate
}

/// [`of`] when the preview was answered from the Stick's record alone
/// ([`from_record_alone`]): what that record says another writer put there.
/// `titles` are the Stick's tracks' titles by the id a player sees.
pub(crate) fn of_record(
    destination: &Path,
    manifest: &rbl_export::Manifest,
    titles: &std::collections::HashMap<u32, &str>,
) -> Kept {
    Kept {
        tracks: manifest
            .tracks
            .iter()
            .filter(|t| carried_in_record(t, destination))
            .map(|t| PlannedTrack {
                id: t.library_id,
                title: titles
                    .get(&t.export_id)
                    .map_or_else(|| t.audio.clone(), |title| (*title).to_owned()),
            })
            .collect(),
        playlists: manifest
            .playlists
            .iter()
            .filter(|p| p.device_only && !p.folder)
            .map(|p| p.name.clone())
            .collect(),
        rewritten_bytes: 0,
    }
}

/// The tracks and playlists of `preview` that the write keeps rather than
/// writes: those the selection did not name.
pub(crate) fn of(
    destination: &Path,
    selection: &crate::selection::ExportSelection,
    preview: &rbl_export::plan::SyncPlan,
    recorded: &HashSet<String>,
) -> Kept {
    let unit = rbl_export::plan::allocation_unit(destination);
    let mut kept = Kept::default();
    let mut seen = HashSet::new();
    for track in preview.tracks.iter().filter(|t| carried(t, destination)) {
        let key = rbl_export::track_key(track.id, &track.source_path.to_string_lossy());
        if !seen.insert(key.clone()) {
            continue;
        }
        kept.tracks.push(PlannedTrack {
            id: track.id,
            title: track.title.clone(),
        });
        // A track no record of ours names has its analysis files written
        // again, over those the Stick has: counted as the preview counts
        // them (each 256 bytes longer).
        if !recorded.contains(&key) {
            if let Some(base) = &track.analysis_source {
                for ext in ["DAT", "EXT", "2EX"] {
                    if let Ok(m) = std::fs::metadata(base.with_extension(ext)) {
                        kept.rewritten_bytes += (m.len() + 256).div_ceil(unit).saturating_mul(unit);
                    }
                }
            }
        }
    }
    let named: HashSet<u64> = selection.playlists.iter().map(|p| p.id).collect();
    kept.playlists = preview
        .playlists
        .iter()
        .filter(|p| !p.folder && !named.contains(&p.id))
        .map(|p| p.name.clone())
        .collect();
    kept
}
