//! A durable Unwind's native changes, kept as one step in the window.
//! Children describe the actual before/after values, including forced edits.
use crate::history::LibraryEdit;
use rbl_db::{write::Writer, DbError};

#[derive(Clone)]
pub struct Unwind {
    pub edits: Vec<LibraryEdit>,
    /// Ledger-owned metadata (original entries and Fact visibility changes).
    pub metadata: serde_json::Value,
}

impl Unwind {
    pub fn check(&self, writer: &Writer, undo: bool) -> Result<(), DbError> {
        for edit in &self.edits {
            edit.check(writer, undo)?;
        }
        Ok(())
    }

    pub fn apply(&self, writer: &mut Writer, undo: bool) -> Result<(), DbError> {
        self.check(writer, undo)?;
        let ordered: Vec<_> = if undo { self.edits.iter().rev().collect() } else { self.edits.iter().collect() };
        for (at, edit) in ordered.iter().enumerate() {
            let result = match edit {
                LibraryEdit::Track(values) => writer.step_track_edits_at_once(values, undo).map(|_| ()),
                _ => edit.apply(writer, undo),
            };
            if let Err(error) = result {
                for previous in ordered[..at].iter().rev() {
                    if let Err(back) = previous.apply(writer, !undo) {
                        return Err(DbError::Io(std::io::Error::other(format!(
                            "Unwind failed: {error}; could not put an accepted change back: {back}. Inspect the Ledger before retrying."
                        ))));
                    }
                }
                return Err(error);
            }
        }
        Ok(())
    }
}
