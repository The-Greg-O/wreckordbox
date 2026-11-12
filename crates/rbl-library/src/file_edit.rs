//! The one guarded start of an edit that writes the Library's own files:
//! analysis files (`.DAT`, `.EXT`, `.2EX`) under the share folder, with or
//! without a database row beside them.
//!
//! [`LibraryCore::write_then`] guards a database write. An edit that
//! publishes files must be refused *before* the first file is published,
//! not when its row is written after them, so it starts here instead:
//! [`LibraryCore::file_edit`] takes the edit gate, Stick authorization and
//! the analysis-file lock, and makes every check `write_then` makes before a
//! write. Holding the answer for the whole edit keeps Library Protection
//! from being turned on half-way through (a turn takes the same gate). A
//! row written inside it through `write_then` is checked again, which
//! changes nothing: the gate is reentrant and nothing can turn protection
//! while it is held.

use crate::{protection, LibraryCore};

/// Held for the whole of an edit of the Library's files: the edit gate and
/// the analysis-file lock. Dropping it ends the edit.
#[must_use = "the edit is guarded only while this is held"]
pub struct FileEdit<'a> {
    pub(crate) core: &'a LibraryCore,
    _files: parking_lot::MutexGuard<'a, ()>,
    _stick: wreckordbox_stick_protection::Authorization,
    _gate: parking_lot::ReentrantMutexGuard<'a, ()>,
}

impl std::fmt::Debug for FileEdit<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FileEdit")
    }
}

impl LibraryCore {
    /// Starts an edit of the Library's files: takes the edit gate, Stick
    /// authorization, then the analysis-file lock. Refuses, before any write, what
    /// [`Self::write_then`] refuses: a holder that is not the Library's
    /// writer, Library Protection ([`protection::refusal`]), an unfinished
    /// restore, rekordbox running on the installed Library. Hold the answer
    /// until every file is published and every row written.
    ///
    /// The analysis-file lock is not reentrant: never call this while
    /// holding it. Blocking.
    pub fn file_edit(&self) -> Result<FileEdit<'_>, rbl_db::DbError> {
        let location = self
            .location()
            .map_err(|e| rbl_db::DbError::NotInstalled(e.to_string()))?;
        let gate = self.edit_gate().lock();
        if let Some(reason) = self.write_refusal() {
            return Err(rbl_db::DbError::WriteRefused(reason));
        }
        if self.protection().protect_library {
            return Err(protection::refusal());
        }
        // Same order and lifetime as row writes. File-only edits must not
        // rely on a later row write (or the Ledger) to enforce this guard.
        let stick = crate::stick::leave(self.recovery_dir(), &location)?;
        let files = self.analysis_write().lock();
        if rbl_backup::journal::pending(self.recovery_dir()) {
            return Err(rbl_db::DbError::WriteRefused(
                "A library restore is unfinished. Restart the app to recover it before editing."
                    .into(),
            ));
        }
        if let Some(reason) = rbl_db::write_refusal_reason(
            location.is_real_install,
            rbl_db::test_mode(),
            rbl_db::is_rekordbox_running(),
        ) {
            return Err(rbl_db::DbError::WriteRefused(reason.to_owned()));
        }
        Ok(FileEdit {
            core: self,
            _files: files,
            _stick: stick,
            _gate: gate,
        })
    }
}
