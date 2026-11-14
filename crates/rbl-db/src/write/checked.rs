//! A track's rating or colour set only while its row is still at the
//! revision the caller read (wreckordbox, wbx-27).
//!
//! Beside [`Writer::set_rating_with_undo`] and [`Writer::set_color_with_undo`],
//! not instead of them (ADR 0003: nothing in this crate changes shape): the
//! comparison and the update are made in ONE immediate write transaction,
//! so a write another connection commits after the caller's last read (a
//! player, rekordbox, an import) is never overwritten. The row is compared
//! by its `rb_local_usn` and `updated_at`; anything else refuses with
//! [`DbError::Conflict`] and writes nothing.

use rusqlite::types::Value;
use rusqlite::{params, OptionalExtension, TransactionBehavior};

use super::{next_usn, set_counter, Changed, TrackEdit, Writer};
use crate::{DbError, Result};
use rbl_core::time;

/// A track row's revision, as the caller read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    pub usn: i64,
    pub updated_at: String,
}

/// What [`Writer::set_track_if_unchanged`] sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckedField {
    /// The rating, 0 to 5 stars.
    Rating(u8),
    /// The colour label's id, 1 to 8, or none with 0.
    Colour(u8),
}

impl CheckedField {
    fn column(self) -> &'static str {
        match self {
            Self::Rating(_) => "Rating",
            Self::Colour(_) => "ColorID",
        }
    }

    /// The value written, as the plain setters write it.
    fn stored(self) -> Value {
        match self {
            Self::Rating(stars) => Value::Integer(i64::from(stars)),
            Self::Colour(0) => Value::Null,
            Self::Colour(id) => Value::Text(id.to_string()),
        }
    }

    /// The expression a row's value is compared by, as the plain setters
    /// compare it to tell an unchanged edit.
    fn shown(self) -> &'static str {
        match self {
            Self::Rating(_) => "COALESCE(Rating, 0)",
            Self::Colour(_) => "COALESCE(CAST(NULLIF(ColorID, '') AS INTEGER), 0)",
        }
    }

    fn wanted(self) -> i64 {
        match self {
            Self::Rating(n) | Self::Colour(n) => i64::from(n),
        }
    }
}

impl Writer {
    /// Sets `field` on track `content` only while its row is at `expected`
    /// (USN and `updated_at`), checked and written in one immediate
    /// transaction; answers the edit to undo, as the `_with_undo` setters
    /// do, and the revision the row is at after it: the one this write
    /// committed (or `expected`, when nothing was written). A caller making
    /// a further checked write to the row expects exactly that revision,
    /// never one read back afterwards, which another connection may have
    /// moved on. A track already holding the value is left as it is (nothing
    /// written, an unchanged edit). A row at another revision, or gone, is
    /// a [`DbError::Conflict`], with nothing written.
    pub fn set_track_if_unchanged(
        &mut self,
        content: &str,
        field: CheckedField,
        expected: &Revision,
    ) -> Result<(Changed, TrackEdit, Revision)> {
        if let CheckedField::Rating(stars) = field {
            if stars > 5 {
                return Err(DbError::WriteRefused(format!(
                    "{stars} is not a rating between 0 and 5"
                )));
            }
        }
        let column = field.column();
        self.prepare()?;
        let stamp = time::now();
        let tx = self
            .library
            .connection_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row: Option<(i64, String, Value, i64)> = tx
            .query_row(
                &format!(
                    "SELECT COALESCE(rb_local_usn, 0), COALESCE(updated_at, ''), `{column}`, {}
                     FROM djmdContent WHERE ID = ?1 AND rb_local_deleted = 0",
                    field.shown()
                ),
                params![content],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((usn, updated_at, before, shown)) = row else {
            return Err(DbError::Conflict(
                "the track is no longer in the collection".to_owned(),
            ));
        };
        if usn != expected.usn || updated_at != expected.updated_at {
            return Err(DbError::Conflict(format!(
                "track {content} was written since it was read"
            )));
        }
        if shown == field.wanted() {
            return Ok((
                Changed::default(),
                TrackEdit {
                    content: content.to_owned(),
                    column,
                    before: before.clone(),
                    after: before,
                },
                expected.clone(),
            ));
        }
        let next = next_usn(&tx)?;
        let rows = tx.execute(
            &format!(
                "UPDATE djmdContent SET `{column}` = ?1, rb_local_usn = ?2, updated_at = ?3
                 WHERE ID = ?4 AND rb_local_deleted = 0
                   AND COALESCE(rb_local_usn, 0) = ?5 AND COALESCE(updated_at, '') = ?6"
            ),
            params![
                field.stored(),
                next,
                stamp,
                content,
                expected.usn,
                expected.updated_at
            ],
        )?;
        if rows != 1 {
            return Err(DbError::Conflict(format!(
                "track {content} was written since it was read"
            )));
        }
        set_counter(&tx, next)?;
        let after: Value = tx.query_row(
            &format!("SELECT `{column}` FROM djmdContent WHERE ID = ?1"),
            params![content],
            |r| r.get(0),
        )?;
        tx.commit()?;
        Ok((
            Changed { rows, usn: next },
            TrackEdit {
                content: content.to_owned(),
                column,
                before,
                after,
            },
            Revision {
                usn: next,
                updated_at: stamp,
            },
        ))
    }
}
