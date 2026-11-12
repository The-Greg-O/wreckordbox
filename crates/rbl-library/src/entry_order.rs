//! A playlist's entries, read and put in a new order by entry (wbx-59).
//!
//! A playlist entry is a row of `djmdSongPlaylist`; its `ID` is its
//! identity, which a Mutant is keyed to (ADR 0008). A track that is in a
//! playlist twice is two entries. Upstream's writer reorders by track
//! (`Writer::reorder` updates every row of a `ContentID` together), so it
//! cannot move one of two entries of one track: both get one number. This
//! module reorders by entry instead, beside the protected writer (ADR
//! 0003), without changing it.
//!
//! The write runs inside [`crate::LibraryCore::write_then`] (through the
//! shell's `recorded_edit`), so it is gated, refused under Library
//! Protection, told to the Ledger, and opened through the guarded
//! [`rbl_db::write::Writer`]. It uses that writer's own connection, in one
//! immediate transaction, and keeps the writer's bookkeeping: each row whose
//! number moves gets a fresh local update number (USN) and `updated_at`,
//! and the registry's update counter is advanced in the same transaction,
//! as `Writer::reorder` does. The writer's own rekordbox-running check is
//! repeated for a real install, since the automatic backup it would take
//! first is the core's to take (the core turns it off on its writers).
//!
//! The numbering after a reorder is 1..N in the order given; Undo and Redo
//! put back the exact numbers each row had, refused with nothing written
//! when the playlist's live rows are not exactly what the step expects.

use rusqlite::{params, Connection, OptionalExtension};

use rbl_db::DbError;

/// One live entry of a playlist, in the playlist's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The entry's own identifier (`djmdSongPlaylist.ID`).
    pub id: String,
    /// The track it is (`ContentID`).
    pub track_id: String,
}

/// Entries with their numbers (`TrackNo`), as a reorder keeps them.
pub type Numbers = [(String, i64)];

/// A reorder by entry, as the edit history keeps it: every live row of the
/// playlist with its number, before and after.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EntryReorder {
    playlist: String,
    before: Vec<(String, i64)>,
    after: Vec<(String, i64)>,
}

impl EntryReorder {
    /// Rebuilds one from its parts (a stored payload, a test).
    #[must_use]
    pub fn from_parts(
        playlist: String,
        before: Vec<(String, i64)>,
        after: Vec<(String, i64)>,
    ) -> Self {
        Self {
            playlist,
            before,
            after,
        }
    }

    /// The reorder left every row where it was.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.before == self.after
    }

    /// The playlist, and every row's number before and after.
    #[must_use]
    pub fn parts(&self) -> (&str, &Numbers, &Numbers) {
        (&self.playlist, &self.before, &self.after)
    }
}

/// The live entries of a plain playlist, in its order (`TrackNo`, then the
/// row's age, as the index lists them). Any other id answers none.
///
/// # Errors
/// A failed read.
pub fn entries(conn: &Connection, playlist: &str) -> Result<Vec<Entry>, DbError> {
    if !is_plain_playlist(conn, playlist)? {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT ID, ContentID FROM djmdSongPlaylist
         WHERE PlaylistID = ?1 AND rb_local_deleted = 0 ORDER BY TrackNo, rowid",
    )?;
    let rows = stmt
        .query_map(params![playlist], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
            ))
        })?
        .filter_map(|row| match row {
            Ok((Some(id), Some(track_id))) => Some(Ok(Entry { id, track_id })),
            Ok(_) => None,
            Err(e) => Some(Err(e)),
        })
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Puts the playlist's entries in exactly this order, numbered 1..N.
///
/// `order` must name every live entry of the playlist once and nothing
/// else; anything else is [`DbError::Conflict`] with nothing written (the
/// window read the playlist before something changed it).
///
/// # Errors
/// Not a plain playlist, a stale order, rekordbox holding a real install,
/// or a failed write.
pub fn reorder(
    writer: &mut rbl_db::write::Writer,
    playlist: &str,
    order: &[String],
) -> Result<EntryReorder, DbError> {
    refuse_while_rekordbox_runs(writer)?;
    let conn = writer.library().connection();
    let tx = Immediate::begin(conn)?;
    if !is_plain_playlist(conn, playlist)? {
        return Err(DbError::WriteRefused(
            "only a playlist's own entries can be put in order".into(),
        ));
    }
    let before = numbered(conn, playlist)?;
    let mut named: Vec<&str> = order.iter().map(String::as_str).collect();
    let mut live: Vec<&str> = before.iter().map(|(id, _)| id.as_str()).collect();
    named.sort_unstable();
    live.sort_unstable();
    if named != live {
        return Err(DbError::Conflict(
            "the playlist's tracks have changed since".into(),
        ));
    }
    let after: Vec<(String, i64)> = order
        .iter()
        .enumerate()
        .map(|(at, id)| (id.clone(), i64::try_from(at + 1).unwrap_or(i64::MAX)))
        .collect();
    write_numbers(conn, playlist, &before, &after)?;
    tx.commit()?;
    let mut sorted_after = after;
    sorted_after.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    Ok(EntryReorder {
        playlist: playlist.to_owned(),
        before,
        after: sorted_after,
    })
}

/// Undoes (`undo`) or redoes a reorder: refused with nothing written unless
/// the playlist's live rows are exactly what the step expects.
///
/// # Errors
/// [`DbError::Conflict`] when the playlist changed since; a failed write.
pub fn step(
    writer: &mut rbl_db::write::Writer,
    edit: &EntryReorder,
    undo: bool,
) -> Result<(), DbError> {
    refuse_while_rekordbox_runs(writer)?;
    let (expected, target) = if undo {
        (&edit.after, &edit.before)
    } else {
        (&edit.before, &edit.after)
    };
    let conn = writer.library().connection();
    let tx = Immediate::begin(conn)?;
    if &numbered(conn, &edit.playlist)? != expected {
        return Err(DbError::Conflict(
            "the playlist's tracks have changed since".into(),
        ));
    }
    write_numbers(conn, &edit.playlist, expected, target)?;
    tx.commit()?;
    Ok(())
}

/// Every live row of the playlist with its number, in the writer's order
/// for numbered rows (`TrackNo`, then `ID`), so two reads compare equal.
fn numbered(conn: &Connection, playlist: &str) -> Result<Vec<(String, i64)>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT ID, TrackNo FROM djmdSongPlaylist
         WHERE PlaylistID = ?1 AND rb_local_deleted = 0 ORDER BY TrackNo, ID",
    )?;
    let rows = stmt
        .query_map(params![playlist], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Writes `target`'s numbers over `current`'s; only rows whose number moves
/// are written, each with its own fresh USN.
fn write_numbers(
    conn: &Connection,
    playlist: &str,
    current: &[(String, i64)],
    target: &[(String, i64)],
) -> Result<(), DbError> {
    let now: std::collections::HashMap<&str, i64> =
        current.iter().map(|(id, no)| (id.as_str(), *no)).collect();
    let stamp = rbl_core::time::now();
    let mut usn = next_usn(conn)? - 1;
    let mut rows = 0;
    for (id, number) in target {
        if now.get(id.as_str()) == Some(number) {
            continue;
        }
        usn += 1;
        rows += conn.execute(
            "UPDATE djmdSongPlaylist SET TrackNo = ?1, rb_local_usn = ?2, updated_at = ?3
             WHERE ID = ?4 AND PlaylistID = ?5 AND rb_local_deleted = 0",
            params![number, usn, stamp, id, playlist],
        )?;
    }
    if rows > 0 {
        let counted = conn.execute(
            "UPDATE agentRegistry SET int_1 = ?1, updated_at = ?2 WHERE registry_id = 'localUpdateCount'",
            params![usn, stamp],
        )?;
        if counted != 1 {
            return Err(DbError::WriteRefused(
                "missing or duplicate local update counter".into(),
            ));
        }
    }
    Ok(())
}

/// The next local update number: one past the registry's counter and every
/// `rb_local_usn` in the Library. The protected writer's own function reads
/// a fixed list of tables; this reads every table that has the column, a
/// superset, so the number is never lower than the writer would choose.
fn next_usn(conn: &Connection) -> Result<i64, DbError> {
    let counter: i64 = conn
        .query_row(
            "SELECT COALESCE(int_1, 0) FROM agentRegistry WHERE registry_id = 'localUpdateCount'",
            [],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0);
    let mut tables = conn.prepare(
        "SELECT m.name FROM sqlite_master m JOIN pragma_table_info(m.name) c
         WHERE m.type = 'table' AND c.name = 'rb_local_usn'",
    )?;
    let names = tables
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut highest = counter;
    for table in names {
        let max: i64 = conn.query_row(
            &format!("SELECT COALESCE(MAX(rb_local_usn), 0) FROM \"{table}\""),
            [],
            |r| r.get(0),
        )?;
        highest = highest.max(max);
    }
    highest
        .checked_add(1)
        .ok_or_else(|| DbError::WriteRefused("local update counter overflow".into()))
}

fn is_plain_playlist(conn: &Connection, playlist: &str) -> Result<bool, DbError> {
    let attribute: Option<i64> = conn
        .query_row(
            "SELECT Attribute FROM djmdPlaylist WHERE ID = ?1 AND rb_local_deleted = 0",
            params![playlist],
            |r| r.get(0),
        )
        .optional()?;
    // Not a folder, an intelligent playlist or one of the Library's own lists.
    Ok(attribute == Some(rbl_db::write::ATTRIBUTE_PLAYLIST))
}

fn refuse_while_rekordbox_runs(writer: &rbl_db::write::Writer) -> Result<(), DbError> {
    if writer.library().location().is_real_install && rbl_db::is_rekordbox_running() {
        return Err(DbError::WriteRefused(
            "rekordbox is running. Quit it before making changes.".to_owned(),
        ));
    }
    Ok(())
}

/// An immediate transaction on a shared connection, rolled back unless
/// committed.
struct Immediate<'a> {
    conn: &'a Connection,
    done: bool,
}

impl<'a> Immediate<'a> {
    fn begin(conn: &'a Connection) -> Result<Self, DbError> {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        Ok(Self { conn, done: false })
    }
    fn commit(mut self) -> Result<(), DbError> {
        self.conn.execute_batch("COMMIT")?;
        self.done = true;
        Ok(())
    }
}

impl Drop for Immediate<'_> {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
    }
}
