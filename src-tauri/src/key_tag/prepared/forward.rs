//! Forward key edits keep the files' authorization through every row rollback.
use super::{snapshot, AppError, Arc, DbError, KeyTagWrite, KeyTagWrites, PreparedFiles};
use rbl_library::history::LibraryEdit;

impl PreparedFiles {
    pub(crate) fn enter_key_edit(&self) -> Result<crate::stick_guard::StickAccess, DbError> {
        crate::stick_guard::enter_files(
            &self.state_dir,
            self.selected
                .iter()
                .map(|s| s.path.as_path())
                .filter(|p| !p.as_os_str().is_empty()),
        )
        .map_err(|e| DbError::WriteRefused(e.message))
    }

    pub(crate) fn finish_key_edit(
        &self,
        writer: &mut rbl_db::write::Writer,
        tracks: Vec<rbl_db::write::TrackEdit>,
        key: &str,
        access: &crate::stick_guard::StickAccess,
    ) -> Result<(LibraryEdit, Option<AppError>), DbError> {
        let mut files = self.files.lock();
        let mut stopped = None;
        for plan in files.iter_mut().filter(|p| {
            tracks
                .iter()
                .any(|t| p.tracks.iter().any(|id| id == t.content()))
        }) {
            let result = access.recheck_for(plan.file.path()).and_then(|()| {
                plan.file
                    .publish()
                    .map_err(|e| AppError::new(crate::error::ErrorKind::ReadOnly, e.to_string()))
            });
            match result {
                Ok(()) => crate::test_holds::point(super::super::TAG_WRITTEN, &self.state_dir),
                Err(why) => {
                    stopped = Some(why);
                    break;
                }
            }
        }
        let Some(why) = stopped else {
            let written = files
                .iter()
                .filter(|p| p.file.is_published())
                .map(|p| snapshot(&p.file))
                .collect();
            let entry = self.entry(tracks, written, &files);
            drop(files);
            let warning = self.record_evidence().err().map(|why| {
                AppError::internal(format!(
                    "Key written; evidence was not fully carried: {why}"
                ))
            });
            return Ok((entry, warning));
        };
        for plan in files.iter_mut().rev().filter(|p| p.file.is_published()) {
            if access.recheck_for(plan.file.path()).is_err() || plan.file.rollback().is_err() {
                plan.file.retain_temporary();
            }
        }
        let title = |track: &str| {
            self.selected
                .iter()
                .find(|s| s.id == track)
                .map_or_else(|| track.to_owned(), |s| s.title.clone())
        };
        let (mut kept, mut undone, mut disagreement) = (Vec::new(), Vec::new(), Vec::new());
        for step in tracks.iter().rev() {
            let retained_file = files
                .iter()
                .any(|p| p.file.is_published() && p.tracks.iter().any(|id| id == step.content()));
            if retained_file {
                kept.push(step.clone());
            } else if let Err(error) = writer.undo_track_edit(step) {
                // SQLite can refuse the rollback too. Preserve the exact row
                // edit in history/Ledger and explicitly name the disagreement.
                disagreement.push(format!("{}: {error}", step.content()));
                kept.push(step.clone());
            } else {
                undone.push(format!("“{}”", title(step.content())));
            }
        }
        kept.reverse();
        undone.reverse();
        let changed: Vec<_> = kept
            .iter()
            .map(|s| format!("“{}”", title(s.content())))
            .collect();
        let not_changed = if undone.is_empty() {
            "none".into()
        } else {
            undone.join(", ")
        };
        if kept.is_empty() {
            return Err(DbError::WriteRefused(format!("The key was not changed: {} Nothing was changed. Changed: none. Not changed: {not_changed}.", why.message)));
        }
        let outcome = if disagreement.is_empty() {
            "in the Library and the file".to_owned()
        } else {
            format!(
                "in the Library; audio may disagree where SQLite refused rollback: {}",
                disagreement.join("; ")
            )
        };
        let message = format!("The key was not written to every file: {} Changed to {key}: {}, {outcome}. Not changed: {not_changed}. The kept edits were recorded; Undo puts the changed ones back once their Stick is back and unlocked.",why.message,changed.join(", "));
        let written = files
            .iter()
            .filter(|p| p.file.is_published())
            .map(|p| snapshot(&p.file))
            .collect();
        let entry = self.entry(kept, written, &files);
        drop(files);
        let message = match self.record_evidence() {
            Ok(()) => message,
            Err(error) => format!("{message} Evidence was not fully carried: {error}"),
        };
        Ok((entry, Some(AppError::new(why.kind, message))))
    }

    fn entry(
        &self,
        tracks: Vec<rbl_db::write::TrackEdit>,
        files: Vec<KeyTagWrite>,
        plans: &[super::FilePlan],
    ) -> LibraryEdit {
        if files.is_empty() {
            LibraryEdit::Track(tracks)
        } else {
            LibraryEdit::KeyEdit {
                tracks,
                files: Arc::new(KeyTagWrites {
                    evidence_tracks: plans
                        .iter()
                        .filter(|p| p.file.is_published())
                        .map(|p| (p.file.path().to_path_buf(), p.tracks.clone()))
                        .collect(),
                    ledger: self.ledger.clone(),
                    files,
                    state_dir: self.state_dir.clone(),
                }),
            }
        }
    }
}
