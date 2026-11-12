//! Ledger completion for an individual file-only edit. Bulk analysis and
//! imports must use bounded recording batches, not this per-edit interface.

use std::{marker::PhantomData, sync::Arc};

use crate::{file_edit::FileEdit, ledger::Ledger};

/// An intent held from immediately before publication to its known outcome.
/// Its borrow keeps the file-edit guard alive. Never nest a row write inside
/// this interval: that write has an intent of its own.
#[must_use = "confirm publication with written(), or successful rollback with cancelled()"]
pub struct FileRecord<'a> {
    ledger: Option<Arc<dyn Ledger>>,
    kind: &'static str,
    track: String,
    finished: bool,
    _guard: PhantomData<&'a std::cell::Cell<()>>,
}

impl FileEdit<'_> {
    /// Record intent after validation/no-op checks and before the first file
    /// changes. All protection checks have already passed under this guard.
    /// Uses the existing Ledger admission policy, including refusal when an
    /// intent cannot be saved; completion never rejects a published write.
    pub fn recording(
        &mut self,
        kind: &'static str,
        track: &str,
    ) -> Result<FileRecord<'_>, rbl_db::DbError> {
        let ledger = self.core.history().lock().ledger();
        if let Some(ledger) = &ledger {
            ledger.begin_file(kind, track).map_err(|why| {
                rbl_db::DbError::WriteRefused(format!(
                    "The Ledger cannot record a change, so nothing was changed: {why}"
                ))
            })?;
        }
        Ok(FileRecord {
            ledger,
            kind,
            track: track.to_owned(),
            finished: false,
            _guard: PhantomData,
        })
    }
}

impl FileRecord<'_> {
    /// The file was published. An append failure keeps the entry for replay;
    /// it cannot turn the successful file write into an error.
    pub fn written(mut self) {
        self.finished = true;
        if let Some(ledger) = &self.ledger {
            ledger.written(true);
            ledger.file_written(self.kind, &self.track);
        }
    }

    /// Nothing changed, or rollback is known to have restored every file.
    pub fn cancelled(mut self) {
        self.finished = true;
        if let Some(ledger) = &self.ledger {
            ledger.written(false);
        }
    }
}

impl Drop for FileRecord<'_> {
    fn drop(&mut self) {
        if !self.finished {
            // Publication/rollback failed with an uncertain outcome. Preserve
            // the intent as unconfirmed instead of silently forgetting it.
            if let Some(ledger) = &self.ledger {
                ledger.written(true);
            }
        }
    }
}
