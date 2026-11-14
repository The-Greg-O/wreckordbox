//! Import operation boundaries. Native writers and kept replacement history
//! stay on the edit path; result batches and warnings follow released guards.
use crate::commands::{
    announce_history, blocking, history_dto, reload, write_error, wrote_outside_history,
    IMPORT_MAX_DEPTH, IMPORT_MAX_ENTRIES, REPLACE_PLAYLIST_CONTENTS,
};
use crate::dto::{ExportProgressDto, FolderPlaylistDto, XmlImportReportDto};
use crate::error::{AppError, AppResult, ErrorKind};
use crate::state::{AppState, LibraryEdit};
use rbl_library::bulk::{Kind, Operation};
use serde_json::json;
use std::sync::Arc;
use tauri::State;

pub(crate) fn finish_import_recording<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    operation: &Operation,
) -> Option<String> {
    let warning = operation.finish();
    if let Some(warning) = &warning {
        let _ = tauri::Emitter::emit(app, "ledger:recording-warning", warning);
    }
    warning
}

pub(crate) async fn import_collection_recorded<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, Arc<AppState>>,
    path: String,
    name: &'static str,
    replace: bool,
    parse: impl FnOnce(&str) -> AppResult<rbl_db::xml::XmlLibrary> + Send + 'static,
) -> AppResult<XmlImportReportDto> {
    let operation = Operation::new(state.core(), Kind::Import);
    let state_for_reload = Arc::clone(&state);
    let writing = Arc::clone(&state);
    let progress_app = app.clone();
    let (report, changed_playlists) = blocking(name, move || {
        let text = std::fs::read_to_string(&path).map_err(|e| {
            AppError::new(ErrorKind::NotFound, "That file could not be read.").with_detail(e.to_string())
        })?;
        let document = parse(&text)?;
        let mut on_progress = |done: usize, total: usize| {
            if done != 0 && done != total && done % 128 != 0 { return; }
            let _ = tauri::Emitter::emit(
                &progress_app,
                "import:progress",
                ExportProgressDto {
                    path: path.clone(),
                    state: "writing",
                    done: u32::try_from(done).unwrap_or(u32::MAX),
                    total: u32::try_from(total).unwrap_or(u32::MAX),
                    title: String::new(),
                    ..Default::default()
                },
            );
        };
        let mut result = operation.run_generated(json!({"operation":name,"path":path}), document.tracks.len() + 1, || {
        // Looked for under the same edit gate as the import, so nothing can
        // make a same-named list between the question and the write.
        let _gate = writing.edit_gate().lock();
        // A Replace existing is planned for the Mutants of the playlists it
        // may replace (wbx-80).
        let outcome = crate::mutant_window::write_replacing(&writing, replace, |writer| {
                if !replace {
                    let same_named = rbl_db::xml::same_named_lists(writer.library(), &document)?;
                    if !same_named.is_empty() {
                        return Ok(Err(same_named));
                    }
                }
                on_progress(0, document.tracks.len());
                rbl_db::xml::import(writer, &document, &mut on_progress).map(Ok)
            })
            .map_err(write_error)?;
        let mut report = match outcome {
            Ok(report) => report,
            Err(same_named) => {
                return Ok((XmlImportReportDto {
                    recording_warning: None,
                    imported: 0,
                    existing: 0,
                    skipped: Vec::new(),
                    playlists: 0,
                    cues: 0,
                    tracks: Vec::new(),
                    same_named,
                }, false));
            }
        };
        // A replaced playlist may have changed without a list or track being
        // made: that, too, needs a reload.
        let changed_playlists = report.playlist_tracks > 0 || report.playlists_replaced > 0;
        let replacements = std::mem::take(&mut report.replacements);
        if !replacements.is_empty() {
            // Replace existing is one step of history (#501): Undo puts back
            // every replaced playlist's members and order, Redo the file's.
            // The tracks and lists the import added stay. Recording it also
            // ends the redo branch; older entries stay, each checking its
            // own rows when it runs (#506).
            writing.edit_history().lock().record(
                LibraryEdit::ReplacePlaylistContents(replacements),
                REPLACE_PLAYLIST_CONTENTS,
            );
        } else if replace {
            // A write of the history's own that it keeps no entry for.
            writing.edit_history().lock().unrecorded(None);
        } else if report.imported > 0 || report.playlists > 0 || changed_playlists {
            wrote_outside_history(&writing, false);
        }
        Ok((XmlImportReportDto {
            recording_warning: None,
            imported: u32::try_from(report.imported).unwrap_or(u32::MAX),
            existing: u32::try_from(report.existing).unwrap_or(u32::MAX),
            skipped: report.skipped,
            playlists: u32::try_from(report.playlists).unwrap_or(u32::MAX),
            cues: u32::try_from(report.cues).unwrap_or(u32::MAX),
            tracks: report.tracks.into_iter().map(|(id, title)| crate::dto::ImportedTrackDto { id, title }).collect(),
            same_named: Vec::new(),
        }, changed_playlists))
        }, |(report, _), emit| {
            emit(json!({"operation":name,"path":path,"imported":report.imported,"existing":report.existing,"playlists":report.playlists,"cues":report.cues,"skipped":report.skipped.len()}));
            for track in &report.tracks { emit(json!({"operation":name,"track":track.id,"title":track.title})); }
        });
        let warning = finish_import_recording(&progress_app, &operation);
        if let Ok((report, _)) = &mut result { report.recording_warning = warning; }
        result
    })
    .await?;
    if report.imported > 0 || report.playlists > 0 || changed_playlists {
        reload(app.clone(), Arc::clone(&state_for_reload)).await?;
        announce_history(&app, &state_for_reload);
    }
    Ok(report)
}

#[allow(
    clippy::too_many_arguments,
    reason = "the existing window import interface"
)]
pub(crate) async fn import_folder_recorded<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, Arc<AppState>>,
    path: String,
    parent: String,
    replace: Option<String>,
    at: Option<u32>,
    copy: Option<bool>,
) -> AppResult<FolderPlaylistDto> {
    let operation = Operation::new(state.core(), Kind::Import);
    let reporter = app.clone();
    let writing = Arc::clone(&state);
    let report = blocking("import_folder_playlist", move || {
        let dir = std::path::PathBuf::from(&path);
        let name = dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut report = FolderPlaylistDto {
            recording_warning: None,
            name: name.clone(),
            renamed: None,
            replaced: None,
            playlist: None,
            conflict: None,
            folder: dir.is_dir(),
            imported: 0,
            skipped: Vec::new(),
            tracks: Vec::new(),
            existing: 0,
            at,
        };
        if !report.folder || name.is_empty() {
            return Ok(report);
        }
        let mut budget = rbl_db::import::WalkBudget::new(IMPORT_MAX_DEPTH, IMPORT_MAX_ENTRIES);
        let files = rbl_db::import::audio_files_in(&dir, &mut budget);
        let mut result = operation.run_generated(json!({"operation":"folder_import","path":path}), files.len()+1, || {
        // Held from the write to the history entry, so no Undo runs between.
        let _gate = writing.edit_gate().lock();
        // A Replace existing is planned for the replaced list's Mutants (wbx-80).
        let replacing = replace.is_some();
        let mut outcome = crate::mutant_window::write_replacing(&writing, replacing, |writer| {
                let clash = match (replace.as_deref(), copy.unwrap_or(false)) {
                    (Some(id), _) => rbl_db::write::FolderClash::Replace(id),
                    (None, true) => rbl_db::write::FolderClash::Copy,
                    (None, false) => rbl_db::write::FolderClash::Ask,
                };
                writer.import_folder_as_playlist(
                    &name,
                    &parent,
                    &files,
                    clash,
                    at.and_then(|at| usize::try_from(at).ok()),
                )
            })
            .map_err(write_error)?;
        report.playlist = outcome.playlist;
        report.renamed = outcome.renamed;
        report.replaced = outcome.replaced.map(|r| crate::dto::ReplacedListDto {
            lists: u32::try_from(r.lists).unwrap_or(u32::MAX),
            tracks: u32::try_from(r.tracks).unwrap_or(u32::MAX),
        });
        report.conflict = outcome.conflict;
        let replacement = outcome.replacement.take();
        report.imported = u32::try_from(outcome.imported.len()).unwrap_or(u32::MAX);
        report.existing = u32::try_from(outcome.existing.len()).unwrap_or(u32::MAX);
        report.skipped = outcome.skipped;
        report.at = outcome.at.map(|at| u32::try_from(at).unwrap_or(u32::MAX)).or(at);
        report.tracks = outcome
            .imported
            .into_iter()
            .map(|(id, file)| crate::dto::ImportedTrackDto {
                id,
                title: file
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            })
            .collect();
        if let Some(replacement) = replacement {
            // Replace existing is one step of history (#426): Undo takes the
            // new playlist away and puts the replaced list back. The tracks
            // it imported stay in the collection.
            writing.edit_history().lock().record(LibraryEdit::ReplacePlaylist(replacement), "Replace Playlist");
        } else if replacing {
            // A write of the history's own that it keeps no entry for.
            writing.edit_history().lock().unrecorded(None);
        } else if report.playlist.is_some() {
            // An edit outside the undo history starts a new branch.
            writing.edit_history().lock().clear_redo();
        }
        Ok::<_, AppError>(report)
        }, |report, emit| {
            emit(json!({"operation":"folder_import","playlist":report.playlist,"name":report.name,"imported":report.imported,"existing":report.existing,"replaced":report.replaced}));
            for track in &report.tracks { emit(json!({"operation":"folder_import","track":track.id,"title":track.title})); }
        });
        let warning = finish_import_recording(&reporter, &operation);
        if let Ok(report) = &mut result { report.recording_warning = warning; }
        result
    })
    .await?;

    // Both the tree and, usually, the collection changed.
    if report.playlist.is_some() {
        let generation = reload(app.clone(), Arc::clone(&state)).await?;
        let history = history_dto(generation, &state.edit_history().lock());
        let _ = tauri::Emitter::emit(&app, "edit-history:changed", history);
    }
    Ok(report)
}

pub(crate) fn import_device_rows_recorded(
    state: &AppState,
    operation: &Operation,
    name: &str,
    entries: &[rbl_db::write::DeviceEntry],
) -> AppResult<rbl_db::write::ImportedPlaylist> {
    operation.run_generated(
        json!({"operation":"device_import","name":name}),
        entries.len() + 1,
        || state.write(|w| w.import_device_playlist_entries(name, entries)).map_err(write_error),
        |made, emit| {
            emit(json!({"operation":"device_import","playlist":made.id,"name":made.name,"tracks":made.tracks,"imported":made.added.len()}));
            for (track, path) in &made.added {
                emit(json!({"operation":"device_import","track":track,"path":path}));
            }
        },
    )
}

pub(crate) async fn end_import_recording<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    operation: Operation,
) -> AppResult<Option<String>> {
    blocking("record_import", move || {
        Ok(finish_import_recording(&app, &operation))
    })
    .await
}
