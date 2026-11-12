//! Ledger completion for an individual file-only edit. Bulk analysis and
//! imports must use bounded recording batches, not this per-edit interface.

use std::{marker::PhantomData, sync::Arc};

use crate::{file_edit::FileEdit, ledger::Ledger};

/// An intent held from immediately before publication to its known outcome.
/// Its borrow keeps the file-edit guard alive. Join a companion row update
/// through `write_row`, so it shares this intent and all edit-path checks.
#[must_use = "confirm publication with written(), or successful rollback with cancelled()"]
pub struct FileRecord<'a> {
    core: &'a crate::LibraryCore,
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
        self.recording_values(kind, track, &serde_json::Value::Null)
    }

    /// Values already in hand, not snapshots obtained by doing more I/O.
    pub fn recording_values(
        &mut self,
        kind: &'static str,
        track: &str,
        values: &serde_json::Value,
    ) -> Result<FileRecord<'_>, rbl_db::DbError> {
        let ledger = self.core.history().lock().ledger();
        if let Some(ledger) = &ledger {
            ledger.begin_file(kind, track, values).map_err(|why| {
                rbl_db::DbError::WriteRefused(format!(
                    "The Ledger cannot record a change, so nothing was changed: {why}"
                ))
            })?;
        }
        Ok(FileRecord {
            core: self.core,
            ledger,
            kind,
            track: track.to_owned(),
            finished: false,
            _guard: PhantomData,
        })
    }
}

impl FileRecord<'_> {
    /// Write this file operation's companion row through the single edit path.
    /// Protection, writer checks, Redo invalidation and host notifications still
    /// apply. The outer file operation owns the intent and its completion.
    pub fn write_row<T>(
        &self,
        host: &(impl crate::Host + ?Sized),
        edit: impl FnOnce(&mut rbl_db::write::Writer) -> Result<T, rbl_db::DbError>,
    ) -> Result<T, rbl_db::DbError> {
        self.core.write_then_recording(
            host,
            crate::History::Outside,
            &crate::ledger::Preparation::default(),
            edit,
            |_, value| Ok(value),
            false,
        )
    }

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
