//! Audio staging is outside the Library gate. Only checked publication and
//! rollback run inside the existing edit path; the history retains values.
use super::{KeyTagWrite, KeyTagWrites};
use crate::{
    commands::write_error,
    error::{AppError, AppResult},
    state::AppState,
};
use rbl_db::{
    key_tag::{
        prepared::{prepare, Outcome, Prepared},
        KeySwap,
    },
    DbError,
};
use rbl_library::history::OutsideSteps;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

struct FilePlan {
    track: String,
    requested: PathBuf,
    file: Box<Prepared>,
}
pub(crate) struct PreparedFiles {
    state_dir: PathBuf,
    files: parking_lot::Mutex<Vec<FilePlan>>,
    warnings: Vec<String>,
}
fn error(error: impl std::fmt::Display) -> DbError {
    DbError::Io(std::io::Error::other(format!("Audio key tag: {error}")))
}
fn snapshot(plan: &Prepared) -> KeyTagWrite {
    KeyTagWrite {
        path: plan.path().to_path_buf(),
        before: plan.before().map(str::to_owned),
        after: plan.after().unwrap_or_default().to_owned(),
    }
}
impl Drop for PreparedFiles {
    fn drop(&mut self) {
        let files = self.files.get_mut();
        // Cleanup is a write too. If the Stick is no longer authorized, leave
        // the temporary instead of silently deleting from a protected Stick.
        let access =
            crate::stick_guard::enter_files(&self.state_dir, files.iter().map(|p| p.file.path()));
        if let Err(refused) = &access {
            for plan in files.iter_mut() {
                plan.file.retain_temporary();
            }
            tracing::warn!(%refused, "audio key temporaries retained: cleanup was refused");
        }
        files.clear();
    }
}

pub(crate) fn prepare_key_edit(
    state: &AppState,
    library: &rbl_index::Library,
    tracks: &[String],
    key: &str,
) -> AppResult<Arc<PreparedFiles>> {
    // Initial refusal before staging. Publication rechecks under the real edit
    // path, including a protection turn while audio is being prepared.
    drop(state.file_edit()?);
    let candidates: Vec<_> = tracks
        .iter()
        .filter_map(|track| {
            let row = library.row_of(track)? as usize;
            if library
                .keys
                .name(library.key.get(row).copied().unwrap_or(0))
                == key
            {
                return None;
            }
            let path = PathBuf::from(library.folder_path.get(row));
            (!path.as_os_str().is_empty()).then(|| (track.clone(), path))
        })
        .collect();
    let all_paths: Vec<_> = tracks
        .iter()
        .filter_map(|id| library.row_of(id))
        .map(|row| Path::new(library.folder_path.get(row as usize)))
        .filter(|p| !p.as_os_str().is_empty())
        .collect();
    let _access = crate::stick_guard::enter_files(state.backup_dir(), all_paths)?;
    let mut files = Vec::new();
    let mut warnings = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (track, requested) in candidates {
        let canonical = match std::fs::canonicalize(&requested) {
            Ok(path) => path,
            Err(why) => {
                warnings.push(format!("{}: {why}", requested.display()));
                continue;
            }
        };
        if !seen.insert(canonical) {
            continue;
        }
        match prepare(&requested, None, Some(key)) {
            Ok(Outcome::Ready(file)) => files.push(FilePlan {
                track,
                requested,
                file,
            }),
            Ok(Outcome::Skipped(KeySwap::Unsupported)) => {
                tracing::info!(track, "this audio format has no writable key tag");
            }
            Ok(Outcome::Skipped(why)) => warnings.push(format!("{}: {why:?}", requested.display())),
            Err(why) => warnings.push(format!("{}: {why}", requested.display())),
        }
    }
    Ok(Arc::new(PreparedFiles {
        state_dir: state.backup_dir().to_path_buf(),
        files: parking_lot::Mutex::new(files),
        warnings,
    }))
}
impl PreparedFiles {
    /// Before the row edit: validate the files and the selected tracks' paths.
    /// Point reads only, no before-value query or audio read for recording.
    pub(crate) fn check_key_edit(&self, writer: &rbl_db::write::Writer) -> Result<(), DbError> {
        let files = self.files.lock();
        let _access =
            crate::stick_guard::enter_files(&self.state_dir, files.iter().map(|p| p.file.path()))
                .map_err(error)?;
        for plan in files.iter() {
            let path: String = writer.library().connection().query_row(
                "SELECT FolderPath FROM djmdContent WHERE ID=?1 AND rb_local_deleted=0",
                [&plan.track],
                |r| r.get(0),
            )?;
            if Path::new(&path) != plan.requested {
                return Err(DbError::Conflict(
                    "The track's audio path changed while its key tag was prepared".into(),
                ));
            }
            plan.file.check().map_err(error)?;
        }
        Ok(())
    }
    /// Rows already accepted: retain every successfully published file in the
    /// history even if another publication fails. The caller shows the warning.
    pub(crate) fn publish_changed(
        &self,
        tracks: &[rbl_db::write::TrackEdit],
    ) -> (Vec<KeyTagWrite>, Option<String>) {
        let mut files = self.files.lock();
        let access =
            crate::stick_guard::enter_files(&self.state_dir, files.iter().map(|p| p.file.path()));
        let access = match access {
            Ok(access) => access,
            Err(error) => return (Vec::new(), Some(error.to_string())),
        };
        let mut written = Vec::new();
        let mut warnings = self.warnings.clone();
        for plan in files
            .iter_mut()
            .filter(|p| tracks.iter().any(|t| t.content() == p.track))
        {
            let result = access
                .recheck()
                .map_err(error)
                .and_then(|()| plan.file.publish().map_err(error));
            match result {
                Ok(()) => written.push(snapshot(&plan.file)),
                Err(why) => warnings.push(why.to_string()),
            }
        }
        (written, (!warnings.is_empty()).then(|| warnings.join("; ")))
    }
}

pub(crate) fn prepare_key_history(
    files: &KeyTagWrites,
    undo: bool,
    label: &str,
) -> Result<Arc<dyn OutsideSteps>, DbError> {
    let _access = crate::stick_guard::enter_files(
        &files.state_dir,
        files.files.iter().map(|f| f.path.as_path()),
    )
    .map_err(|e| super::protected(undo, label, &e))?;
    // Preserve the established conflict/refusal wording, outside the gate.
    super::check_restore(&files.files, undo, label)?;
    let mut plans = Vec::new();
    for file in &files.files {
        let (expected, key) = file.step(undo);
        match prepare(&file.path, Some(expected), key).map_err(error)? {
            Outcome::Ready(plan) => plans.push(FilePlan {
                track: String::new(),
                requested: file.path.clone(),
                file: plan,
            }),
            Outcome::Skipped(KeySwap::Changed { .. }) => return Err(super::changed(file)),
            Outcome::Skipped(KeySwap::ReadOnly) => {
                return Err(super::failure(
                    undo,
                    label,
                    &file.name(),
                    "the file is read-only",
                    0,
                ))
            }
            Outcome::Skipped(_) => {
                return Err(super::failure(
                    undo,
                    label,
                    &file.name(),
                    "the file is no longer one a key is written into",
                    0,
                ))
            }
        }
    }
    Ok(Arc::new(PreparedFiles {
        state_dir: files.state_dir.clone(),
        files: parking_lot::Mutex::new(plans),
        warnings: Vec::new(),
    }))
}
impl OutsideSteps for PreparedFiles {
    fn is_empty(&self) -> bool {
        self.files.lock().is_empty()
    }
    fn check(&self, _undo: bool, _label: &str) -> Result<(), DbError> {
        let files = self.files.lock();
        let _access =
            crate::stick_guard::enter_files(&self.state_dir, files.iter().map(|p| p.file.path()))
                .map_err(error)?;
        files.iter().try_for_each(|p| p.file.check().map_err(error))
    }
    fn step(&self, undo: bool, label: &str) -> Result<(), DbError> {
        let mut files = self.files.lock();
        let access =
            crate::stick_guard::enter_files(&self.state_dir, files.iter().map(|p| p.file.path()))
                .map_err(error)?;
        let order: Vec<_> = if undo {
            (0..files.len()).rev().collect()
        } else {
            (0..files.len()).collect()
        };
        for (at, &index) in order.iter().enumerate() {
            let result = access
                .recheck()
                .map_err(error)
                .and_then(|()| files[index].file.publish().map_err(error));
            if let Err(why) = result {
                let mut stranded = 0;
                for &done in order[..at].iter().rev() {
                    if files[done].file.rollback().is_err() {
                        files[done].file.retain_temporary();
                        stranded += 1;
                    }
                }
                return Err(super::failure(
                    undo,
                    label,
                    &files[index].requested.display().to_string(),
                    &why.to_string(),
                    stranded,
                ));
            }
        }
        Ok(())
    }
    fn revert(&self, undo: bool) -> (usize, usize) {
        let mut files = self.files.lock();
        let count = files.len();
        let Ok(_access) =
            crate::stick_guard::enter_files(&self.state_dir, files.iter().map(|p| p.file.path()))
        else {
            for plan in files.iter_mut() {
                plan.file.retain_temporary();
            }
            return (count, count);
        };
        let order: Vec<_> = if undo {
            (0..count).collect()
        } else {
            (0..count).rev().collect()
        };
        let mut stranded = 0;
        for index in order {
            if files[index].file.rollback().is_err() {
                files[index].file.retain_temporary();
                stranded += 1;
            }
        }
        (stranded, count)
    }
}

/// One auxiliary publication of an analysis Operation. `begin_file` joins that
/// operation's existing intent/Run; its bounded completion records these values.
pub(crate) fn write_analysis_key_tag(
    state: &AppState,
    track: &str,
    path: &str,
    key: &str,
) -> (Option<KeyTagWrite>, Option<String>) {
    let write = || -> AppResult<Option<KeyTagWrite>> {
        if path.is_empty() {
            return Ok(None);
        }
        drop(state.file_edit()?);
        let plan = {
            let _access = crate::stick_guard::enter_files(state.backup_dir(), [Path::new(path)])?;
            match prepare(Path::new(path), None, Some(key)).map_err(|e| write_error(error(e)))? {
                Outcome::Ready(file) => file,
                Outcome::Skipped(why) => {
                    return Err(AppError::internal(format!(
                        "Key tag was not written: {why:?}"
                    )))
                }
            }
        };
        let plans = PreparedFiles {
            state_dir: state.backup_dir().to_path_buf(),
            files: parking_lot::Mutex::new(vec![FilePlan {
                track: track.into(),
                requested: path.into(),
                file: plan,
            }]),
            warnings: Vec::new(),
        };
        // plans was declared first: its cleanup runs after every guard drops.
        let mut edit = state.file_edit()?;
        let mut files = plans.files.lock();
        let plan = &mut files[0].file;
        let _access = crate::stick_guard::enter_files(state.backup_dir(), [plan.path()])?;
        plan.check().map_err(|e| write_error(error(e)))?;
        let values =
            serde_json::json!({"path":plan.path(),"before":plan.before(),"after":plan.after()});
        let record = edit
            .recording_values("audio_key_tag", track, &values)
            .map_err(write_error)?;
        match plan.publish() {
            Ok(()) => {
                let written = snapshot(plan);
                record.written();
                Ok(Some(written))
            }
            Err(why) => {
                record.cancelled();
                Err(write_error(error(why)))
            }
        }
    };
    match write() {
        Ok(file) => (file, None),
        Err(why) => (
            None,
            Some(format!(
                "Analysis was saved; the audio key tag was not written: {}",
                why.message
            )),
        ),
    }
}
