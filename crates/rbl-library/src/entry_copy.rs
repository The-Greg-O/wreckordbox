//! A playlist copied, with its entries, in one write (wbx-80).
//!
//! The copy is a new plain playlist beside the source (under the same
//! parent, after its last sibling) holding the source's entries in order,
//! the same track twice kept twice. The playlist row and every entry row
//! are written in one immediate transaction on the guarded writer's own
//! connection, beside the protected writer (ADR 0003), as
//! [`crate::entry_order`] writes: it is gated, refused under Library
//! Protection and told to the Ledger by the core's write path, and keeps
//! the writer's bookkeeping (a fresh local update number per row, the
//! registry's counter advanced in the same transaction).
//!
//! The identifiers of the new playlist and its entries are chosen before
//! the write ([`CopyIds`]), so whoever records the copy (the Ledger, for
//! the Mutants of the source's entries) can say which entry becomes which
//! before anything is changed.
//!
//! Undo tombstones exactly the rows the copy made, refused with nothing
//! written when the copy has changed since (its entries moved, removed or
//! added to); Redo brings the same rows back.

use rusqlite::{params, Connection, OptionalExtension};

use rbl_db::DbError;

use crate::entry_order::{
    is_plain_playlist, next_usn, refuse_while_rekordbox_runs, Entry, Immediate,
};

/// The identifiers a copy will have: the playlist's, and one per source
/// entry, in the source's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyIds {
    pub playlist: String,
    pub entries: Vec<String>,
}

impl CopyIds {
    /// Fresh identifiers for a copy of `count` entries: a playlist id not
    /// in use in `conn`, and a new UUID per entry.
    ///
    /// # Errors
    /// A failed read.
    pub fn fresh(conn: &Connection, count: usize) -> Result<Self, DbError> {
        let mut rng = rbl_core::ids::Rng::from_entropy();
        let playlist = loop {
            let candidate = rng.numeric_id(rbl_core::ids::MAX_PLAYLIST_ID);
            let taken: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM djmdPlaylist WHERE ID = ?1)",
                params![candidate],
                |r| r.get(0),
            )?;
            if !taken {
                break candidate;
            }
        };
        Ok(Self {
            playlist,
            entries: (0..count).map(|_| rng.uuid4()).collect(),
        })
    }
}

/// A copy, as the edit history keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistCopy {
    source: String,
    playlist: String,
    name: String,
    /// `(source entry, new entry, track)`, in order.
    entries: Vec<(String, String, String)>,
}

impl PlaylistCopy {
    /// The source playlist, the copy, and each `(source entry, new entry,
    /// track)` in order.
    #[must_use]
    pub fn parts(&self) -> (&str, &str, &[(String, String, String)]) {
        (&self.source, &self.playlist, &self.entries)
    }

    /// The copy's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Copies `source` as `name` with the identifiers `ids` chosen for it.
///
/// `ids` must name one entry per live entry of the source; anything else is
/// [`DbError::Conflict`] with nothing written (the source changed since the
/// identifiers were chosen).
///
/// # Errors
/// Not a plain playlist, a stale `ids`, a taken playlist id, rekordbox
/// holding a real install, or a failed write.
pub fn copy(
    writer: &mut rbl_db::write::Writer,
    source: &str,
    name: &str,
    ids: &CopyIds,
) -> Result<PlaylistCopy, DbError> {
    refuse_while_rekordbox_runs(writer)?;
    let conn = writer.library().connection();
    let tx = Immediate::begin(conn)?;
    if !is_plain_playlist(conn, source)? {
        return Err(DbError::WriteRefused(
            "only a playlist's own entries can be copied".into(),
        ));
    }
    let entries = crate::entry_order::entries(conn, source)?;
    if entries.len() != ids.entries.len() {
        return Err(DbError::Conflict(
            "the playlist's tracks have changed since".into(),
        ));
    }
    let taken: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM djmdPlaylist WHERE ID = ?1)",
        params![ids.playlist],
        |r| r.get(0),
    )?;
    if taken {
        return Err(DbError::Conflict(format!(
            "a playlist {} is already in the Library",
            ids.playlist
        )));
    }
    let parent: String = conn.query_row(
        "SELECT ParentID FROM djmdPlaylist WHERE ID = ?1",
        params![source],
        |r| r.get(0),
    )?;
    let seq: i64 = conn.query_row(
        "SELECT COALESCE(MAX(Seq), -1) + 1 FROM djmdPlaylist
         WHERE ParentID = ?1 AND rb_local_deleted = 0",
        params![parent],
        |r| r.get(0),
    )?;
    let stamp = rbl_core::time::now();
    let mut rng = rbl_core::ids::Rng::from_entropy();
    let mut usn = next_usn(conn)?;
    conn.execute(
        "INSERT INTO djmdPlaylist
            (ID, Seq, Name, ImagePath, Attribute, ParentID, SmartList, UUID,
             rb_data_status, rb_local_data_status, rb_local_deleted, rb_local_synced,
             usn, rb_local_usn, created_at, updated_at)
         VALUES (?1, ?2, ?3, NULL, ?4, ?5, NULL, ?6, 0, 0, 0, 0, NULL, ?7, ?8, ?8)",
        params![
            ids.playlist,
            seq,
            name,
            rbl_db::write::ATTRIBUTE_PLAYLIST,
            parent,
            rng.uuid4(),
            usn,
            stamp
        ],
    )?;
    let mut copied = Vec::with_capacity(entries.len());
    for ((track_no, Entry { id, track_id }), new) in (1_i64..).zip(entries).zip(&ids.entries) {
        usn += 1;
        conn.execute(
            "INSERT INTO djmdSongPlaylist
                (ID, PlaylistID, ContentID, TrackNo, UUID,
                 rb_data_status, rb_local_data_status, rb_local_deleted, rb_local_synced,
                 usn, rb_local_usn, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, 0, 0, 0, NULL, ?6, ?7, ?7)",
            params![
                new,
                ids.playlist,
                track_id,
                track_no,
                rng.uuid4(),
                usn,
                stamp
            ],
        )?;
        copied.push((id, new.clone(), track_id));
    }
    count(conn, usn, &stamp)?;
    tx.commit()?;
    Ok(PlaylistCopy {
        source: source.to_owned(),
        playlist: ids.playlist.clone(),
        name: name.to_owned(),
        entries: copied,
    })
}

/// Undoes (`undo`) or redoes a copy: refused with nothing written unless
/// the copy's rows are exactly as the step expects.
///
/// # Errors
/// [`DbError::Conflict`] when the copy changed since; a failed write.
pub fn step(
    writer: &mut rbl_db::write::Writer,
    edit: &PlaylistCopy,
    undo: bool,
) -> Result<(), DbError> {
    refuse_while_rekordbox_runs(writer)?;
    let conn = writer.library().connection();
    let tx = Immediate::begin(conn)?;
    let changed = || DbError::Conflict("the copied playlist has changed since".into());
    let (from, to) = (i64::from(!undo), i64::from(undo));
    let deleted: Option<i64> = conn
        .query_row(
            "SELECT rb_local_deleted FROM djmdPlaylist WHERE ID = ?1",
            params![edit.playlist],
            |r| r.get(0),
        )
        .optional()?;
    if deleted != Some(from) {
        return Err(changed());
    }
    let mut live = conn.prepare(
        "SELECT ID FROM djmdSongPlaylist
         WHERE PlaylistID = ?1 AND rb_local_deleted = 0 ORDER BY TrackNo, rowid",
    )?;
    let rows = live
        .query_map(params![edit.playlist], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(live);
    let ours: Vec<&String> = edit.entries.iter().map(|(_, new, _)| new).collect();
    let expected: Vec<&String> = if undo { ours.clone() } else { Vec::new() };
    if rows.iter().collect::<Vec<_>>() != expected {
        return Err(changed());
    }
    let stamp = rbl_core::time::now();
    let mut usn = next_usn(conn)? - 1;
    let mut flip = |table: &str, id: &str| -> Result<(), DbError> {
        usn += 1;
        let written = conn.execute(
            &format!(
                "UPDATE {table} SET rb_local_deleted = ?1, rb_local_usn = ?2, updated_at = ?3
                 WHERE ID = ?4 AND rb_local_deleted = ?5"
            ),
            params![to, usn, stamp, id, from],
        )?;
        if written == 1 {
            Ok(())
        } else {
            Err(changed())
        }
    };
    // Foreign-key order: entries go before their playlist and come back
    // after it.
    if undo {
        for id in &ours {
            flip("djmdSongPlaylist", id)?;
        }
        flip("djmdPlaylist", &edit.playlist)?;
    } else {
        flip("djmdPlaylist", &edit.playlist)?;
        for id in &ours {
            flip("djmdSongPlaylist", id)?;
        }
    }
    count(conn, usn, &stamp)?;
    tx.commit()?;
    Ok(())
}

fn count(conn: &Connection, usn: i64, stamp: &str) -> Result<(), DbError> {
    let counted = conn.execute(
        "UPDATE agentRegistry SET int_1 = ?1, updated_at = ?2 WHERE registry_id = 'localUpdateCount'",
        params![usn, stamp],
    )?;
    if counted != 1 {
        return Err(DbError::WriteRefused(
            "missing or duplicate local update counter".into(),
        ));
    }
    Ok(())
}
