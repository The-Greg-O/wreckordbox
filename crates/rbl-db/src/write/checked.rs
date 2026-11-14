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

/// One `djmdSongMyTag` row of a track and a My Tag, live or soft-deleted,
/// as stored (wbx-29): what a My Tag assignment's revision is made of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagRow {
    pub id: String,
    pub usn: i64,
    pub updated_at: String,
    pub live: bool,
}

const TAG_ROWS: &str = "SELECT COALESCE(ContentID, ''), COALESCE(MyTagID, ''), COALESCE(ID, ''),
        COALESCE(rb_local_usn, 0), COALESCE(updated_at, ''), COALESCE(rb_local_deleted, 0) = 0
     FROM djmdSongMyTag";

fn tag_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<(String, String, TagRow)> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        TagRow {
            id: r.get(2)?,
            usn: r.get(3)?,
            updated_at: r.get(4)?,
            live: r.get(5)?,
        },
    ))
}

/// Every `djmdSongMyTag` row, live or soft-deleted, as `(track, My Tag,
/// row)`, ordered by track, My Tag and row id: one read, whatever the size
/// of the Library. The rows of one track and tag are compared by
/// [`Writer::set_my_tags_if_unchanged`] exactly as read here.
pub fn my_tag_rows(conn: &rusqlite::Connection) -> Result<Vec<(String, String, TagRow)>> {
    let mut statement = conn.prepare(&format!("{TAG_ROWS} ORDER BY ContentID, MyTagID, ID"))?;
    let rows = statement
        .query_map([], tag_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn rows_of(tx: &rusqlite::Connection, content: &str, tag: &str) -> Result<Vec<TagRow>> {
    let mut statement = tx.prepare_cached(&format!(
        "{TAG_ROWS} WHERE ContentID = ?1 AND MyTagID = ?2 ORDER BY ID"
    ))?;
    let rows = statement
        .query_map(params![content, tag], tag_row)?
        .map(|row| row.map(|(_, _, row)| row))
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// One track's My Tags to change with [`Writer::set_my_tags_if_unchanged`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagChanges {
    pub content: String,
    /// Each My Tag to put on (`true`) or take off (`false`).
    pub changes: Vec<(String, bool)>,
    /// Each of those tags' rows for the track, as the caller read them.
    pub expected: std::collections::BTreeMap<String, Vec<TagRow>>,
}

/// What [`Writer::set_my_tags_if_unchanged`] did to one track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagsWritten {
    /// The track's whole My Tag set before and after, to undo.
    pub edit: super::TrackTagEdit,
    /// Each changed tag's rows for the track as the transaction left them.
    pub rows: std::collections::BTreeMap<String, Vec<TagRow>>,
}

impl Writer {
    /// For each of `tracks`, puts each of its My Tags on (`true`) or takes
    /// it off (`false`) only while that tag's rows for the track are exactly
    /// the `expected` ones (every row, live or not, by id, USN and
    /// `updated_at`, as [`my_tag_rows`] reads them): every comparison and
    /// every write in ONE immediate transaction, so a write another
    /// connection commits after the caller's last read is never overwritten
    /// and the batch is written whole or not at all (wbx-29). The tracks'
    /// other My Tags are left as they are. Answers, per track, the edit to
    /// undo (as [`Self::set_my_tags_with_undo`] does) and the changed tags'
    /// rows as the transaction left them. A tag already in the wanted state
    /// is left as it is. Any difference, or a track that is gone, is a
    /// [`DbError::Conflict`]; a tag that is not a live My Tag is
    /// `WriteRefused`; either way nothing is written.
    pub fn set_my_tags_if_unchanged(
        &mut self,
        tracks: &[TagChanges],
    ) -> Result<(Changed, Vec<TagsWritten>)> {
        self.prepare()?;
        let stamp = time::now();
        let ids: Vec<Vec<(String, String)>> = tracks
            .iter()
            .map(|t| {
                t.changes
                    .iter()
                    .map(|_| (self.rng.uuid4(), self.rng.uuid4()))
                    .collect()
            })
            .collect();
        let tx = self
            .library
            .connection_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for track in tracks {
            let content = track.content.as_str();
            if !super::content_exists(&tx, content)? {
                return Err(DbError::Conflict(format!(
                    "track {content} is no longer in the collection"
                )));
            }
            for (tag, _) in &track.changes {
                let known: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM djmdMyTag WHERE ID = ?1 AND Attribute = 0 AND rb_local_deleted = 0",
                    params![tag],
                    |r| r.get(0),
                )?;
                if known == 0 {
                    return Err(DbError::WriteRefused(format!("no My Tag {tag}")));
                }
                if rows_of(&tx, content, tag)?
                    != track.expected.get(tag).cloned().unwrap_or_default()
                {
                    return Err(DbError::Conflict(format!(
                        "track {content}'s My Tag {tag} was written since it was read"
                    )));
                }
            }
        }
        let live = |tx: &rusqlite::Connection, content: &str| -> Result<Vec<String>> {
            let mut statement = tx.prepare_cached(
                "SELECT DISTINCT MyTagID FROM djmdSongMyTag
                 WHERE ContentID = ?1 AND rb_local_deleted = 0 ORDER BY MyTagID",
            )?;
            let tags = statement
                .query_map(params![content], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(tags)
        };
        let mut rows = 0;
        // Numbered on from the counter once, not by a scan per row.
        let mut usn = next_usn(&tx)? - 1;
        let mut written = Vec::with_capacity(tracks.len());
        for (track, ids) in tracks.iter().zip(&ids) {
            let content = track.content.as_str();
            let before = live(&tx, content)?;
            for ((tag, on), (id, uuid)) in track.changes.iter().zip(ids) {
                if *on == before.contains(tag) {
                    continue;
                }
                usn += 1;
                if *on {
                    let track_no: i64 = tx.query_row(
                        "SELECT COALESCE(MAX(TrackNo), 0) + 1 FROM djmdSongMyTag WHERE MyTagID = ?1 AND rb_local_deleted = 0",
                        params![tag],
                        |r| r.get(0),
                    )?;
                    rows += tx.execute(
                        "INSERT INTO djmdSongMyTag
                            (ID, MyTagID, ContentID, TrackNo, UUID,
                             rb_data_status, rb_local_data_status, rb_local_deleted, rb_local_synced,
                             usn, rb_local_usn, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, 0, 0, 0, 0, NULL, ?6, ?7, ?7)",
                        params![id, tag, content, track_no, uuid, usn, stamp],
                    )?;
                } else {
                    rows += tx.execute(
                        "UPDATE djmdSongMyTag SET rb_local_deleted = 1, rb_local_usn = ?1, updated_at = ?2
                         WHERE ContentID = ?3 AND MyTagID = ?4 AND rb_local_deleted = 0",
                        params![usn, stamp, content, tag],
                    )?;
                }
            }
            let after = live(&tx, content)?;
            let mut left = std::collections::BTreeMap::new();
            for (tag, _) in &track.changes {
                left.insert(tag.clone(), rows_of(&tx, content, tag)?);
            }
            written.push(TagsWritten {
                edit: super::TrackTagEdit {
                    content: content.to_owned(),
                    before,
                    after,
                },
                rows: left,
            });
        }
        if rows > 0 {
            set_counter(&tx, usn)?;
        }
        tx.commit()?;
        Ok((Changed { rows, usn }, written))
    }
}
