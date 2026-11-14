//! Recorded native analysis locks; selection reads and writes share one transaction.
use super::{next_usn, set_counter, time, TrackEdit, Writer};
use crate::{DbError, Result};
use rusqlite::{params, types::Value, TransactionBehavior};

impl Writer {
    /// Set one native lock through the same atomic adapter as a selection.
    pub fn set_analysis_lock_with_undo(
        &mut self,
        content: &str,
        on: bool,
    ) -> Result<Option<TrackEdit>> {
        Ok(self
            .set_analysis_locks_with_undo(&[content.to_owned()], on)?
            .pop())
    }

    /// Retain exact before/after values, including NULL and unrelated bits.
    /// A refused row rolls back every row and revision in this selection.
    /// Repeated values write nothing and produce no assertion.
    pub fn set_analysis_locks_with_undo(
        &mut self,
        contents: &[String],
        on: bool,
    ) -> Result<Vec<TrackEdit>> {
        self.prepare()?;
        let tx = self
            .library
            .connection_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut edits = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for content in contents {
            if !seen.insert(content) {
                continue;
            }
            let (before, locked, after): (Value, bool, i64) = tx.query_row(
                "SELECT Analysed, (COALESCE(Analysed,0)&128)!=0, (COALESCE(Analysed,0)&~128)|?2 FROM djmdContent WHERE ID=?1 AND rb_local_deleted=0",
                params![content, if on { 128 } else { 0 }],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            if locked != on {
                edits.push(TrackEdit {
                    content: content.clone(),
                    column: "Analysed",
                    before,
                    after: Value::Integer(after),
                });
            }
        }
        if !edits.is_empty() {
            let usn = next_usn(&tx)?;
            let stamp = time::now();
            for edit in &mut edits {
                let rows = tx.execute(
                    "UPDATE djmdContent SET Analysed=?2, rb_local_usn=?3, updated_at=?4 WHERE ID=?1 AND rb_local_deleted=0",
                    params![edit.content, edit.after, usn, stamp],
                )?;
                if rows != 1 {
                    return Err(DbError::WriteRefused("Track no longer exists".into()));
                }
                edit.after = tx.query_row(
                    "SELECT Analysed FROM djmdContent WHERE ID=?1",
                    [&edit.content],
                    |r| r.get(0),
                )?;
            }
            set_counter(&tx, usn)?;
        }
        tx.commit()?;
        Ok(edits)
    }
}
