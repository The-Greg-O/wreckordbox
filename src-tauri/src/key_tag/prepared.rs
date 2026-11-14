//! Audio staging is outside the Library gate. Only checked publication and
//! rollback run inside the existing edit path; the history retains values.
mod forward;
use super::{KeyTagWrite, KeyTagWrites};
use crate::{
    commands::write_error,
    error::{AppError, AppResult},
    state::AppState,
};
use rbl_db::{
    key_tag::{
        prepared::{prepare_checked, Outcome, Prepared},
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
    tracks: Vec<String>,
    requested: PathBuf,
    file: Box<Prepared>,
}
struct Selected {
    id: String,
    path: PathBuf,
    title: String,
}
pub(crate) struct PreparedFiles {
    state_dir: PathBuf,
    files: parking_lot::Mutex<Vec<FilePlan>>,
    selected: Vec<Selected>,
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
    let selected: Vec<_> = tracks
        .iter()
        .filter_map(|id| {
            library.row_of(id).map(|row| Selected {
                id: id.clone(),
                path: PathBuf::from(library.folder_path.get(row as usize)),
                title: library.title.get(row as usize).to_owned(),
            })
        })
        .collect();
    let all_paths: Vec<_> = tracks
        .iter()
        .filter_map(|id| library.row_of(id))
        .map(|row| Path::new(library.folder_path.get(row as usize)))
        .filter(|p| !p.as_os_str().is_empty())
        .collect();
    let access = crate::stick_guard::enter_files(state.backup_dir(), all_paths)?;
    // Own staged copies immediately, so every early return authorizes cleanup
    // too (including a later file refusing after this Stick was replaced).
    let prepared = Arc::new(PreparedFiles {
        state_dir: state.backup_dir().to_path_buf(),
        files: parking_lot::Mutex::new(Vec::new()),
        selected,
    });
    let mut files = prepared.files.lock();
    let mut warnings = Vec::new();

    for (track, requested) in candidates {
        let canonical = match std::fs::canonicalize(&requested) {
            Ok(path) => path,
            Err(why) => {
                warnings.push(format!("{}: {why}", requested.display()));
                continue;
            }
        };
        access.recheck_for(&requested)?;
        if let Some(existing) = files.iter_mut().find(|p| p.file.path() == canonical) {
            existing.tracks.push(track);
            continue;
        }
        match prepare_checked(&requested, None, Some(key), || {
            access
                .recheck_for(&requested)
                .map_err(|e| std::io::Error::other(e.message))
        }) {
            Ok(Outcome::Ready(file)) => files.push(FilePlan {
                tracks: vec![track],
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
    if !warnings.is_empty() {
        return Err(AppError::new(
            crate::error::ErrorKind::ReadOnly,
            format!("No keys changed: {}", warnings.join("; ")),
        ));
    }
    drop(files);
    Ok(prepared)
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
            for selected in self.selected.iter().filter(|s| plan.tracks.contains(&s.id)) {
                let path: String = writer.library().connection().query_row(
                    "SELECT FolderPath FROM djmdContent WHERE ID=?1 AND rb_local_deleted=0",
                    [&selected.id],
                    |r| r.get(0),
                )?;
                if Path::new(&path) != selected.path
                    || std::fs::canonicalize(&selected.path)? != plan.file.path()
                {
                    return Err(DbError::Conflict(
                        "The track's audio path changed while its key tag was prepared".into(),
                    ));
                }
            }
            plan.file.check().map_err(error)?;
        }
        Ok(())
    }
}

pub(crate) fn prepare_key_history(
    files: &KeyTagWrites,
    undo: bool,
    label: &str,
) -> Result<Arc<dyn OutsideSteps>, DbError> {
    let access = crate::stick_guard::enter_files(
        &files.state_dir,
        files.files.iter().map(|f| f.path.as_path()),
    )
    .map_err(|e| super::protected(undo, label, &e))?;
    // Preserve the established conflict/refusal wording, outside the gate.
    super::check_restore(&files.files, undo, label)?;
    let prepared = Arc::new(PreparedFiles {
        state_dir: files.state_dir.clone(),
        files: parking_lot::Mutex::new(Vec::new()),
        selected: Vec::new(),
    });
    let mut plans = prepared.files.lock();
    for file in &files.files {
        let (expected, key) = file.step(undo);
        match prepare_checked(&file.path, Some(expected), key, || {
            access
                .recheck_for(&file.path)
                .map_err(|e| std::io::Error::other(e.message))
        })
        .map_err(error)?
        {
            Outcome::Ready(plan) => plans.push(FilePlan {
                tracks: Vec::new(),
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
        crate::test_holds::point("key_tag.file_prepared", &files.state_dir);
    }
    drop(plans);
    Ok(prepared)
}
impl OutsideSteps for PreparedFiles {
    fn unrestored_values(&self) -> Option<serde_json::Value> {
        let files = self.files.lock();
        let values: Vec<_> = files.iter().filter(|p|p.file.is_published()).map(|p| {
            serde_json::json!({"path":p.file.path(),"before":p.file.before(),"after":p.file.after()})
        }).collect();
        (!values.is_empty()).then_some(serde_json::Value::Array(values))
    }

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
    fn step(&self, undo: bool, label: &str) -> Result<rbl_library::history::OutsideHold, DbError> {
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
                    if access.recheck_for(files[done].file.path()).is_err()
                        || files[done].file.rollback().is_err()
                    {
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
        crate::test_holds::point(super::TAGS_STEPPED, &self.state_dir);
        Ok(Box::new(access))
    }
    fn revert(&self, held: rbl_library::history::OutsideHold, undo: bool) -> (usize, usize) {
        let mut files = self.files.lock();
        let count = files.len();
        let Ok(access) = held.downcast::<crate::stick_guard::StickAccess>() else {
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
            if access.recheck_for(files[index].file.path()).is_err()
                || files[index].file.rollback().is_err()
            {
                files[index].file.retain_temporary();
                stranded += 1;
            }
        }
        (stranded, count)
    }
}

/// One auxiliary publication of an analysis Operation. `begin_file` joins that
/// operation's existing intent/Run; its bounded completion records these values.
pub fn write_analysis_key_tag(
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
            let access = crate::stick_guard::enter_files(state.backup_dir(), [Path::new(path)])?;
            match prepare_checked(Path::new(path), None, Some(key), || {
                access
                    .recheck_for(Path::new(path))
                    .map_err(|e| std::io::Error::other(e.message))
            })
            .map_err(|e| write_error(error(e)))?
            {
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
                tracks: vec![track.into()],
                requested: path.into(),
                file: plan,
            }]),
            selected: Vec::new(),
        };
        // plans was declared first: its cleanup runs after every guard drops.
        let _gate = state.core().edit_gate().lock();
        let access = crate::stick_guard::enter_files(state.backup_dir(), [Path::new(path)])?;
        crate::test_holds::point("key_tag.audio_authorized", state.backup_dir());
        let mut edit = state.file_edit()?;
        let mut files = plans.files.lock();
        let plan = &mut files[0].file;
        access.recheck_for(plan.path())?;
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
                why.full_message()
            )),
        ),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    #[test]
    fn failed_preparation_retains_earlier_copies_on_a_replaced_stick() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().join("state");
        let stick = dir.path().join("SPARE");
        std::fs::create_dir(&stick).unwrap();
        std::fs::write(
            stick.join(wreckordbox_stick_protection::STAND_IN_MARKER),
            b"",
        )
        .unwrap();
        let mut files = Vec::new();
        for name in ["first.mp3", "second.mp3"] {
            let path = stick.join(name);
            let mut mp3 = Vec::new();
            for _ in 0..40 {
                mp3.extend([0xFF, 0xFB, 0x90, 0x64]);
                mp3.extend([0; 413]);
            }
            std::fs::write(&path, mp3).unwrap();
            rbl_db::key_tag::write_key(&path, "Fm").unwrap();
            files.push(KeyTagWrite {
                path,
                before: Some("Cm".into()),
                after: "Fm".into(),
            });
        }
        wreckordbox_stick_protection::unlock(&state_dir, &stick, Some("SPARE")).unwrap();
        let held = crate::test_holds::arm("key_tag.file_prepared", &state_dir);
        let history = KeyTagWrites {
            files,
            state_dir: state_dir.clone(),
        };
        let worker =
            std::thread::spawn(move || prepare_key_history(&history, true, "Undo").is_err());
        held.reached();
        let names: Vec<_> = std::fs::read_dir(&stick)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n.to_string_lossy().contains(".rbx-key-"))
            .collect();
        assert_eq!(names.len(), 2);
        let removed = dir.path().join("removed");
        std::fs::rename(&stick, &removed).unwrap();
        std::fs::create_dir(&stick).unwrap();
        std::fs::write(
            stick.join(wreckordbox_stick_protection::STAND_IN_MARKER),
            b"",
        )
        .unwrap();
        // A different Stick can contain the same names; they are not ours to delete.
        for name in &names {
            std::fs::write(stick.join(name), b"replacement's bytes").unwrap();
        }
        held.release();
        assert!(worker.join().unwrap());
        for name in &names {
            assert_eq!(
                std::fs::read(stick.join(name)).unwrap(),
                b"replacement's bytes"
            );
            assert!(removed.join(name).exists());
        }
    }

    #[test]
    fn analysis_tag_takes_stick_authorization_before_the_analysis_lock() {
        let dir = tempfile::tempdir().unwrap();
        let location =
            rbl_db::fixture::build(dir.path(), rbl_db::fixture::Shape::default()).unwrap();
        let state = Arc::new(AppState::with_backups(dir.path().join("state")));
        state.core().set_protection(false).unwrap();
        let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
        let (library, _) = rbl_index::load(&db).unwrap();
        state.set_library(library, false, db.schema().db_version, 0, location);
        let stick = dir.path().join("SPARE");
        std::fs::create_dir(&stick).unwrap();
        std::fs::write(
            stick.join(wreckordbox_stick_protection::STAND_IN_MARKER),
            b"",
        )
        .unwrap();
        let audio = stick.join("tag.mp3");
        let mut mp3 = Vec::new();
        for _ in 0..40 {
            mp3.extend([0xFF, 0xFB, 0x90, 0x64]);
            mp3.extend([0; 413]);
        }
        std::fs::write(&audio, mp3).unwrap();
        rbl_db::key_tag::write_key(&audio, "Cm").unwrap();
        wreckordbox_stick_protection::unlock(state.backup_dir(), &stick, Some("SPARE")).unwrap();
        let held = crate::test_holds::arm("key_tag.audio_authorized", state.backup_dir());
        let (writing, path) = (state.clone(), audio.clone());
        let worker = std::thread::spawn(move || {
            write_analysis_key_tag(&writing, "fixture", path.to_str().unwrap(), "Fm")
        });
        held.reached();
        let analysis = state.core().analysis_write().lock();
        let locking = if let Some(free) =
            wreckordbox_stick_protection::try_exec_lock(state.backup_dir(), &stick).unwrap()
        {
            wreckordbox_stick_protection::lock(state.backup_dir(), &stick).unwrap();
            drop(free);
            None
        } else {
            let (state, path) = (state.clone(), stick.clone());
            Some(std::thread::spawn(move || {
                wreckordbox_stick_protection::lock(state.backup_dir(), &path).unwrap()
            }))
        };
        held.release();
        drop(analysis);
        let (_, warning) = worker.join().unwrap();
        if let Some(locking) = locking {
            locking.join().unwrap();
        }
        assert!(warning.is_none(), "{warning:?}");
        assert_eq!(
            rbl_db::key_tag::read_key(&audio).unwrap().as_deref(),
            Some("Fm")
        );
    }
}
