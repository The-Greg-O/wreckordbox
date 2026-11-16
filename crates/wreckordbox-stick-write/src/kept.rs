//! What a write leaves on a Stick that another writer put there (wbx-dc0).
//!
//! The export keeps another writer's tracks and playlists (rekordbox's
//! export from another computer, a player's edits), as rekordbox's own export
//! does: `rbl_export::reconcile` carries each such track into the write with
//! `device.preserve`, its source the Stick's own file, and the export leaves
//! that file where it is (`retained_audio`). The export's estimate does not
//! know that: it reckons such a track as audio to copy whenever no record
//! of ours names it, so a plan built on it reported every one of them as
//! "added" and counted their audio again in the room needed (an 11-track
//! playlist onto a rekordbox Stick of 280 tracks: 291 added, 10.7 GB).
//!
//! This module tells those tracks and playlists apart from the preview, so
//! the plan says they stay, and works out the room the estimate counted for
//! them, so it can be taken off again. `rbl_export` is left as it is
//! (ADR 0003).

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
    /// Of those tracks, how many the estimate counted as new audio.
    pub counted_new: usize,
    /// The room the estimate counted to copy their audio again, in whole
    /// clusters, as it counted it.
    pub counted_bytes: u64,
}

/// Whether `track` is the Stick's own copy, carried as it is.
pub(crate) fn carried(track: &rbl_export::SourceTrack, destination: &Path) -> bool {
    track.device.as_ref().is_some_and(|d| d.preserve) && track.source_path.starts_with(destination)
}

/// The tracks and playlists of `preview` that the write keeps rather than
/// writes: those the selection did not name.
pub(crate) fn of(
    destination: &Path,
    selection: &crate::selection::ExportSelection,
    preview: &rbl_export::plan::SyncPlan,
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
        if matches!(
            preview.estimate.actions.get(&key),
            Some(rbl_export::estimate::AudioAction::Copy)
        ) {
            kept.counted_new += 1;
            let size = std::fs::metadata(&track.source_path).map_or(0, |m| m.len());
            kept.counted_bytes += size.div_ceil(unit).saturating_mul(unit);
            // Its analysis files, which the export does not write again
            // either, counted as the preview counts them for a track it
            // copies (each 256 bytes longer).
            if let Some(base) = &track.analysis_source {
                for ext in ["DAT", "EXT", "2EX"] {
                    if let Ok(m) = std::fs::metadata(base.with_extension(ext)) {
                        kept.counted_bytes += (m.len() + 256).div_ceil(unit).saturating_mul(unit);
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
