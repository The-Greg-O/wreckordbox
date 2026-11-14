//! Audio preparation does not monopolize the Library. Recheck the selected
//! history entry after staging; a concurrent command must not lose its entry.
use super::{HistoryEntry, LibraryEdit, StepError};
use rbl_db::DbError;
use std::sync::Arc;

type Prepared<'a> = (
    parking_lot::ReentrantMutexGuard<'a, ()>,
    HistoryEntry,
    LibraryEdit,
);

impl crate::LibraryCore {
    pub(super) fn prepare_history_step(&self, undo: bool) -> Result<Prepared<'_>, StepError> {
        let gate = self.edit_gate().lock();
        let entry = self
            .history()
            .lock()
            .next(undo)
            .cloned()
            .ok_or(StepError::Nothing)?;
        let LibraryEdit::KeyEdit { files, tracks } = &entry.edit else {
            return Ok((gate, entry.clone(), entry.edit));
        };
        if self.protection().protect_library {
            return Err(StepError::Write(crate::protection::refusal()));
        }
        if let Some(reason) = self.write_refusal() {
            return Err(StepError::Write(DbError::WriteRefused(reason)));
        }
        drop(gate);
        let prepared = files.prepare(undo, entry.label);
        let gate = self.edit_gate().lock();
        let same = self.history().lock().next(undo).is_some_and(|now| {
            now.ledger == entry.ledger
                && matches!(&now.edit,
                LibraryEdit::KeyEdit { files: current, .. } if Arc::ptr_eq(files, current))
        });
        if !same {
            return Err(StepError::Write(DbError::WriteRefused(
                "The edit history changed while its audio was being prepared. Nothing was changed."
                    .into(),
            )));
        }
        let executable = match prepared {
            Ok(Some(files)) => LibraryEdit::KeyEdit {
                tracks: tracks.clone(),
                files,
            },
            Ok(None) => entry.edit.clone(),
            Err(DbError::Conflict(reason)) => {
                self.history().lock().drop_next(undo);
                return Err(StepError::Conflict {
                    label: entry.label,
                    reason,
                });
            }
            Err(error) => return Err(StepError::Write(error)),
        };
        Ok((gate, entry, executable))
    }
}
