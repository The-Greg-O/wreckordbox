//! Only device edits since our export baseline can replace desktop ratings.
use std::collections::{BTreeMap, HashMap};
use rbl_export::{Manifest, snapshot::Snapshot};
use super::{AppResult, AppState, ImportReport, err, write_error};

/// `current` is the Stick's libraries as Round-trip read them before
/// applying anything (wbx-45.5), or why they could not be read.
#[allow(clippy::too_many_arguments, reason = "validated source and one operation recorder")]
pub(super) fn import(state: &AppState, current: Result<&Snapshot, &str>, manifest: Option<&Manifest>, matched: &HashMap<u32, String>, db_id: u64, report: &mut ImportReport, operation: &rbl_library::bulk::Operation) -> AppResult<()> {
    // apply already checked the native file-edit guards, including Protection
    // and rekordbox-running, even when this snapshot has no rating changes.
    let Some((manifest, baseline)) = manifest.and_then(|m| m.baseline.as_ref().map(|b| (m, b))) else {
        report.warnings.push("Track ratings were not imported: no export baseline for this library was found. Sync this USB to create one.".into());
        return Ok(());
    };
    let current = current.map_err(err)?;
    let records: HashMap<_, _> = manifest.tracks.iter().map(|t| (t.export_id, t)).collect();
    let mut changes = BTreeMap::new();
    for (name, before, after) in [
        ("Device Library", &baseline.legacy, &current.legacy),
        ("OneLibrary", &baseline.one, &current.one),
    ] {
        let Some(after) = after else { continue; };
        let Some(before) = before else {
            report.warnings.push(format!("{name} ratings were not imported: this format has no export baseline."));
            continue;
        };
        let before: HashMap<_, _> = before.tracks.iter().map(|t| (t.id, t)).collect();
        for track in &after.tracks {
            let Some(old) = before.get(&track.id) else { continue; };
            if track.rating == old.rating { continue; }
            let identity_matches = name != "OneLibrary"
                || current.identity.get(&track.id) == baseline.identity.get(&track.id)
                    && current.identity.get(&track.id).is_some_and(|(owner, id)| *owner == db_id && matched.get(&track.id).is_some_and(|local| local == &id.to_string()));
            let path_matches = records.get(&track.id).is_some_and(|record|
                record.library_id != 0 && track.path == old.path
                    && track.path.trim_start_matches('/') == record.audio.trim_start_matches('/'));
            if !identity_matches || !path_matches || !matched.contains_key(&track.id) {
                report.skipped += 1;
                report.warnings.push(format!("{name} track {} rating was skipped: its library identity or source path could not be verified.", track.id));
                continue;
            }
            if track.rating > 5 || old.rating > 5 {
                return Err(err(format!("{name} track {} has an invalid rating. Sync this USB to regenerate 0–5 star ratings.", track.id)));
            }
            if let Some(previous) = changes.insert(track.id, track.rating) {
                if previous != track.rating {
                    return Err(err(format!("The device libraries disagree on the rating for track {}. Reconcile its ratings in rekordbox before importing.", track.id)));
                }
            }
        }
    }
    let baseline_ratings: HashMap<_, _> = baseline.one.as_ref().or(baseline.legacy.as_ref())
        .into_iter().flat_map(|library| &library.tracks).map(|track| (track.id, track.rating)).collect();
    let mut conflicts = 0;
    let changes: Vec<_> = changes.into_iter().collect();
    // Exact before values and conflict decisions are read by the writer, under
    // its gate. Recording flushes only after each bounded transaction returns.
    for chunk in changes.chunks(128) {
        let (_, replaced, written) = operation.run_many(
            serde_json::json!({"operation":"ratings","tracks":chunk.iter().filter_map(|(id, _)| matched.get(id)).collect::<Vec<_>>()}), chunk.len(),
            || state.write(|writer| {
                let mut records = Vec::new();
                let mut replaced = 0;
                let mut written = 0;
                for (export_id, rating) in chunk {
                    let Some(id) = matched.get(export_id) else { continue; };
                    let (_, edit) = writer.set_rating_with_undo(id, u8::try_from(*rating).map_err(|e| rbl_db::DbError::WriteRefused(e.to_string()))?)?;
                    let parts = edit.parts();
                    if parts.before == parts.after {
                        operation.assert_player_fields();
                        records.push(crate::player_ledger::rating(edit));
                        continue;
                    }
                    written += 1;
                    let desktop = match &parts.before {
                        rusqlite::types::Value::Null => 0,
                        rusqlite::types::Value::Integer(value) => *value,
                        _ => -1,
                    };
                    if baseline_ratings.get(export_id).is_some_and(|old| desktop != i64::from(*old)) { replaced += 1; }
                    records.push(crate::player_ledger::rating(edit));
                }
                Ok((records, replaced, written))
            }).map_err(write_error),
            |(records, _, _)| records.clone(),
        )?;
        report.ratings += written;
        conflicts += replaced;
    }
    if conflicts > 0 {
        let noun = if conflicts == 1 { "rating" } else { "ratings" };
        report.warnings.push(format!("Device ratings replaced {conflicts} {noun} also edited in your library."));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
