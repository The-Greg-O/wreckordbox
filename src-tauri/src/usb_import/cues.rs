//! Guard cue publication in bounded slices. Recording is completed only after
//! the slice releases its edit/Stick/file guards, with one operation intent.
use super::{
    database_cues_match, err, write_error, Analysis, AppError, AppResult, AppState, ImportReport,
};
use std::collections::HashMap;

#[allow(
    clippy::too_many_arguments,
    reason = "validated source and bounded recording operation"
)]
pub(super) fn import(
    state: &AppState,
    editor: &crate::grid::GridEditor,
    location: &rbl_db::LibraryLocation,
    tracks: &HashMap<u32, (String, String)>,
    read: &HashMap<String, AppResult<Analysis>>,
    report: &mut ImportReport,
    operation: &rbl_library::bulk::Operation,
) -> AppResult<()> {
    let tracks: Vec<_> = tracks.values().collect();
    for chunk in tracks.chunks(128) {
        // One order slot per publication, not chunk.len() slots per track.
        operation.run(
            serde_json::json!({"operation":"cue_grid","tracks":chunk.iter().map(|(id, _)| id).collect::<Vec<_>>()}),
            || {
                let mut edit = state.file_edit()?;
                let mut records = Vec::with_capacity(chunk.len());
                for (id, analysis) in chunk {
                    if one(state, editor, location, id, analysis, read, report, &mut edit)? {
                        records.push(serde_json::json!({"operation":"cue_grid","track":id,"analysis":analysis}));
                    }
                }
                Ok::<_, AppError>(records)
            },
            Clone::clone,
        )?;
    }
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "one already-guarded cue publication"
)]
fn one(
    state: &AppState,
    editor: &crate::grid::GridEditor,
    location: &rbl_db::LibraryLocation,
    id: &str,
    analysis: &str,
    read: &HashMap<String, AppResult<Analysis>>,
    report: &mut ImportReport,
    edit: &mut rbl_library::file_edit::FileEdit<'_>,
) -> AppResult<bool> {
    // Read on the open handle. Opening the library, for the lock
    // bit here and the analysis path below, derives its key again:
    // about 155 ms each, two per track, so a stick of 20,000 tracks
    // held the library, and every analysis and sync, for well over
    // an hour (#284).
    if editor.is_locked(id)
        || state
            .read_db(|db| crate::grid::locked_in(db, id))
            .map_err(write_error)?
    {
        report.skipped += 1;
        return Ok(false);
    }
    if analysis.is_empty() {
        report.skipped += 1;
        return Ok(false);
    }
    let (source_dat, source_cues) = match read.get(analysis) {
        Some(Ok(Analysis::Read { dat, cues })) => (dat, cues),
        Some(Ok(Analysis::Missing)) => {
            report.skipped += 1;
            report.warnings.push(format!("Track {id}: USB analysis file is missing ({analysis}); cues and grid were left unchanged."));
            return Ok(false);
        }
        Some(Err(e)) => return Err(e.clone()),
        None => return Err(err(format!("Track {id}: its analysis file was not read"))),
    };
    if source_cues.section(b"PCO2").is_none() {
        report.skipped += 1;
        return Ok(false);
    }
    let entries = source_cues.cue_entries();
    let beats = source_dat.beat_grid().unwrap_or_default();
    let previous_bpm = state
        .read_db(|db| {
            Ok(db.connection().query_row(
                "SELECT COALESCE(BPM,0) FROM djmdContent WHERE ID=?1 AND rb_local_deleted=0",
                [id],
                |r| r.get::<_, u32>(0),
            )?)
        })
        .map_err(write_error)?;
    let bpm = beats
        .first()
        .map_or(previous_bpm, |b| u32::from(b.tempo_x100));
    let bpm_changed = bpm != previous_bpm;
    let relative = state
        .read_db(|db| rbl_db::write::analysis_data_path(db.connection(), id))
        .map_err(write_error)?;
    let target = rbl_anlz::resolve(&location.share_root, &relative);
    let mut files = Vec::new();
    // A stick that carries the cues and grid the library already has
    // is not a change: rewriting it would bump the track's analysis,
    // drop its grid undo history and reload the whole library for
    // nothing, and the next sync would copy it all back (#134).
    let mut differs = false;
    for extension in ["DAT", "EXT"] {
        let target = target.with_extension(extension);
        if !target.exists() {
            continue;
        }
        let mut dest = rbl_anlz::Anlz::read(&target).map_err(err)?;
        let src = if extension == "DAT" {
            source_dat
        } else {
            source_cues
        };
        differs |= !dest
            .sections
            .iter()
            .filter(|s| s.is_cue_list())
            .eq(src.sections.iter().filter(|s| s.is_cue_list()));
        if extension == "DAT" && !beats.is_empty() {
            differs |= !dest
                .sections
                .iter()
                .filter(|s| s.as_beat_grid().is_some())
                .eq(source_dat.section(b"PQTZ"));
        }
        dest.sections.retain(|s| !s.is_cue_list());
        dest.sections
            .extend(src.sections.iter().filter(|s| s.is_cue_list()).cloned());
        if extension == "DAT" && !beats.is_empty() {
            dest.sections.retain(|s| s.as_beat_grid().is_none());
            if let Some(grid) = source_dat.section(b"PQTZ") {
                dest.sections.push(grid.clone());
            }
        }
        let bytes = if extension == "EXT" {
            dest.with_extended_grid_cleared()
                .unwrap_or_else(|| dest.to_bytes())
        } else {
            dest.to_bytes()
        };
        files.push((target, bytes));
    }
    if files.is_empty() {
        report.skipped += 1;
        return Ok(false);
    }
    if !differs && !bpm_changed && database_cues_match(state, id, &entries)? {
        report.unchanged += 1;
        return Ok(false);
    }
    let record = edit
        .recording_values("round_trip_cues", id, &serde_json::Value::Null)
        .map_err(write_error)?;
    let journal = match crate::file_journal::FileJournal::prepare(
        state.backup_dir(),
        location,
        id,
        bpm,
        None,
        true,
        &files,
    ) {
        Ok(journal) => journal,
        Err(error) => {
            record.cancelled();
            return Err(error);
        }
    };
    if let Err(e) = journal.publish() {
        journal.rollback()?;
        record.cancelled();
        return Err(e);
    }
    if let Err(e) = record.write_row(state, |w| w.import_usb_cues(id, &entries, bpm)) {
        journal.reconcile(location)?;
        return Err(write_error(e));
    }
    journal.commit()?;
    record.written();
    editor.forget_history(id);
    report.changed.push(id.to_owned());
    report.tracks += 1;
    Ok(true)
}
