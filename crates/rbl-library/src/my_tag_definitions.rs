//! MyTag definitions made, renamed and put in order as one reversible step
//! of the edit history (wbx-14.14): what MyTag Projection writes to the
//! Library's vocabulary itself, beside the tracks' assignments.
//!
//! The rows are written on the guarded writer's own connection, beside the
//! protected writer (ADR 0003), as [`crate::entry_copy`] writes: a new tag
//! is rekordbox's own shape (`Writer::create_my_tag`: a random 32-bit id
//! that is not a category's, a fresh UUID, `Seq` one past its category's
//! highest, `Attribute` 0); a rename writes the name and nothing else; an
//! order writes `Seq` 1..N over the rows whose number changes. Each row gets
//! a fresh local update number and the registry's counter is advanced in
//! the same transaction.
//!
//! [`write`] runs inside a transaction the caller holds (MyTag Projection
//! writes the definitions and the assignments that use them in one), and
//! refuses, with nothing written, a category that is not one, a name that
//! is empty or already a sibling's, a category already holding
//! [`rbl_db::write::MY_TAG_LIMIT`] tags, an id already in the table, or an
//! order that does not name exactly one run of siblings.
//!
//! [`step`] undoes or redoes a [`Definitions`] in one immediate
//! transaction, refused as [`DbError::Conflict`] with nothing written unless
//! every row is exactly as the step expects: Undo takes the orders back
//! (each run must be exactly as left), the names back (each must still be
//! the one written), then marks each made tag deleted (refused while any
//! track still carries it); Redo brings the same rows back in the other
//! order. Every step takes a new update number, so a projection's record
//! of the row it left no longer matches after an Undo or a Redo.
#![allow(
    clippy::doc_markdown,
    reason = "MyTag and the like are the glossary's words, not code items"
)]

use rusqlite::{params, Connection, OptionalExtension};

use rbl_db::write::{MY_TAG_LIMIT, ROOT};
use rbl_db::DbError;

use crate::entry_order::{next_usn, refuse_while_rekordbox_runs, Immediate};

/// A MyTag to make, with the identifiers chosen before the write
/// ([`fresh_ids`]), so the assignments written with it can name it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTag {
    pub id: String,
    pub uuid: String,
    pub category: String,
    pub name: String,
}

/// What to write: tags to make (at the end of their category, in this
/// order), tags or categories to rename, and runs of siblings to put in a
/// new order (each the whole run, made tags included).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DefinitionChanges {
    pub create: Vec<NewTag>,
    /// `(id, new name)`.
    pub rename: Vec<(String, String)>,
    /// `(parent, every live sibling's id in the wanted order)`: `parent` is
    /// [`ROOT`] for the categories.
    pub order: Vec<(String, Vec<String>)>,
}

impl DefinitionChanges {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.create.is_empty() && self.rename.is_empty() && self.order.is_empty()
    }
}

/// A tag made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    pub id: String,
    pub uuid: String,
    pub category: String,
    pub name: String,
    pub seq: i64,
}

/// A tag or category renamed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Renamed {
    pub id: String,
    pub before: String,
    pub after: String,
}

/// A run of siblings put in a new order: every live sibling's id and exact
/// `Seq`, in the order read, before and after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ordered {
    pub parent: String,
    pub before: Vec<(String, Option<i64>)>,
    pub after: Vec<(String, Option<i64>)>,
}

/// What [`write`] did, as the edit history keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Definitions {
    pub created: Vec<Created>,
    pub renamed: Vec<Renamed>,
    pub ordered: Vec<Ordered>,
}

impl Definitions {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.created.is_empty()
            && self.renamed.iter().all(|r| r.before == r.after)
            && self.ordered.iter().all(|o| o.before == o.after)
    }
}

/// `count` fresh `(id, uuid)` pairs for new MyTags: ids rekordbox could
/// have chosen (not 0 to 4, the categories', nor `0xFFFFFFFF`) that no row
/// of `djmdMyTag` holds, deleted rows included, and distinct.
///
/// # Errors
/// A failed read, or no free id found.
pub fn fresh_ids(conn: &Connection, count: usize) -> Result<Vec<(String, String)>, DbError> {
    let mut rng = rbl_core::ids::Rng::from_entropy();
    let mut chosen: Vec<(String, String)> = Vec::with_capacity(count);
    for _ in 0..count {
        let mut found = None;
        for _ in 0..1000 {
            let value = (rng.next_u64() >> 32) as u32;
            if value < 5 || value == u32::MAX {
                continue;
            }
            let candidate = value.to_string();
            if chosen.iter().any(|(id, _)| *id == candidate) {
                continue;
            }
            let taken: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM djmdMyTag WHERE ID = ?1)",
                params![candidate],
                |r| r.get(0),
            )?;
            if !taken {
                found = Some(candidate);
                break;
            }
        }
        let id = found.ok_or_else(|| {
            DbError::WriteRefused("could not find an unused id for djmdMyTag".into())
        })?;
        chosen.push((id, rng.uuid4()));
    }
    Ok(chosen)
}

/// The live siblings under `parent`, by `Seq` then id: the categories under
/// [`ROOT`], a category's tags otherwise.
///
/// # Errors
/// A failed read.
pub fn run(conn: &Connection, parent: &str) -> Result<Vec<(String, Option<i64>)>, DbError> {
    let attribute = i64::from(parent == ROOT);
    let mut statement = conn.prepare(
        "SELECT ID, Seq FROM djmdMyTag
         WHERE ParentID = ?1 AND COALESCE(Attribute, 0) = ?2 AND rb_local_deleted = 0
         ORDER BY Seq, ID",
    )?;
    let run = statement
        .query_map(params![parent, attribute], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(run)
}

/// `(parent, name, deleted)` of a row.
fn row(conn: &Connection, id: &str) -> Result<Option<(String, String, bool)>, DbError> {
    Ok(conn
        .query_row(
            "SELECT COALESCE(ParentID, ''), COALESCE(Name, ''), rb_local_deleted <> 0
             FROM djmdMyTag WHERE ID = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?)
}

/// Whether a live sibling under `parent` other than `id` is named `name`.
fn name_taken(conn: &Connection, parent: &str, id: &str, name: &str) -> Result<bool, DbError> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM djmdMyTag
         WHERE ParentID = ?1 AND ID <> ?2 AND Name = ?3 AND rb_local_deleted = 0)",
        params![parent, id, name],
        |r| r.get(0),
    )?)
}

fn refused(why: impl Into<String>) -> DbError {
    DbError::WriteRefused(why.into())
}

/// Writes `changes` inside the transaction the caller holds on `conn`:
/// the tags made, then the renames, then the orders. Anything refused
/// returns an error and the caller's transaction must be rolled back.
///
/// # Errors
/// See the module's documentation; a failed write.
pub fn write(conn: &Connection, changes: &DefinitionChanges) -> Result<Definitions, DbError> {
    let stamp = rbl_core::time::now();
    let mut usn = next_usn(conn)? - 1;
    let mut done = Definitions::default();
    for new in &changes.create {
        if new.name.is_empty() {
            return Err(refused("a My Tag needs a name"));
        }
        let category: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM djmdMyTag
             WHERE ID = ?1 AND Attribute = 1 AND ParentID = 'root' AND rb_local_deleted = 0)",
            params![new.category],
            |r| r.get(0),
        )?;
        if !category {
            return Err(refused(format!("no My Tag category {}", new.category)));
        }
        if row(conn, &new.id)?.is_some() {
            return Err(DbError::Conflict(format!(
                "a My Tag {} is already in the Library",
                new.id
            )));
        }
        if name_taken(conn, &new.category, &new.id, &new.name)? {
            return Err(refused(rbl_db::write::MY_TAG_NAME_TAKEN));
        }
        let (held, seq): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(MAX(Seq), 0) + 1 FROM djmdMyTag
             WHERE ParentID = ?1 AND rb_local_deleted = 0",
            params![new.category],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if usize::try_from(held).unwrap_or(usize::MAX) >= MY_TAG_LIMIT {
            return Err(refused(format!(
                "a My Tag category holds at most {MY_TAG_LIMIT} tags"
            )));
        }
        usn += 1;
        conn.execute(
            "INSERT INTO djmdMyTag
                (ID, Seq, Name, Attribute, ParentID, UUID,
                 rb_data_status, rb_local_data_status, rb_local_deleted, rb_local_synced,
                 usn, rb_local_usn, created_at, updated_at)
             VALUES (?1, ?2, ?3, 0, ?4, ?5, 0, 0, 0, 0, NULL, ?6, ?7, ?7)",
            params![new.id, seq, new.name, new.category, new.uuid, usn, stamp],
        )?;
        done.created.push(Created {
            id: new.id.clone(),
            uuid: new.uuid.clone(),
            category: new.category.clone(),
            name: new.name.clone(),
            seq,
        });
    }
    // Renames are checked against the names the batch leaves, so two tags
    // may swap names.
    let mut renames: Vec<(String, String, String, String)> = Vec::new();
    for (id, name) in &changes.rename {
        if name.is_empty() {
            return Err(refused("a My Tag needs a name"));
        }
        let Some((parent, current, false)) = row(conn, id)? else {
            return Err(refused(format!("no My Tag {id}")));
        };
        if current != *name {
            renames.push((id.clone(), parent, current, name.clone()));
        }
    }
    for (id, parent, _, name) in &renames {
        let renamed_away = |other: &str| renames.iter().any(|(o, _, _, n)| o == other && n != name);
        let mut statement = conn.prepare(
            "SELECT ID FROM djmdMyTag
             WHERE ParentID = ?1 AND ID <> ?2 AND Name = ?3 AND rb_local_deleted = 0",
        )?;
        let holders = statement
            .query_map(params![parent, id, name], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let twice = renames
            .iter()
            .filter(|(_, p, _, n)| p == parent && n == name)
            .count()
            > 1;
        if twice || holders.iter().any(|holder| !renamed_away(holder)) {
            return Err(refused(rbl_db::write::MY_TAG_NAME_TAKEN));
        }
    }
    for (id, _, before, after) in renames {
        usn += 1;
        conn.execute(
            "UPDATE djmdMyTag SET Name = ?1, rb_local_usn = ?2, updated_at = ?3
             WHERE ID = ?4 AND rb_local_deleted = 0",
            params![after, usn, stamp, id],
        )?;
        done.renamed.push(Renamed { id, before, after });
    }
    for (parent, order) in &changes.order {
        let before = run(conn, parent)?;
        let mut seen: Vec<&str> = Vec::with_capacity(order.len());
        for id in order {
            if seen.contains(&id.as_str()) || !before.iter().any(|(sibling, _)| sibling == id) {
                return Err(refused(format!(
                    "My Tag {id} is not once in the run under {parent}"
                )));
            }
            seen.push(id);
        }
        if seen.len() != before.len() {
            return Err(refused(format!(
                "the order under {parent} leaves out a My Tag"
            )));
        }
        let after: Vec<(String, Option<i64>)> =
            order.iter().cloned().zip((1_i64..).map(Some)).collect();
        write_seqs(conn, &before, &after, &stamp, &mut usn)?;
        done.ordered.push(Ordered {
            parent: parent.clone(),
            before,
            after,
        });
    }
    if !done.is_empty() {
        count(conn, usn, &stamp)?;
    }
    Ok(done)
}

/// Writes `target`'s `Seq` over the rows whose number differs.
fn write_seqs(
    conn: &Connection,
    current: &[(String, Option<i64>)],
    target: &[(String, Option<i64>)],
    stamp: &str,
    usn: &mut i64,
) -> Result<(), DbError> {
    for (id, seq) in target {
        let was = current
            .iter()
            .find(|(other, _)| other == id)
            .map(|(_, seq)| *seq);
        if was == Some(*seq) {
            continue;
        }
        *usn += 1;
        conn.execute(
            "UPDATE djmdMyTag SET Seq = ?1, rb_local_usn = ?2, updated_at = ?3
             WHERE ID = ?4 AND rb_local_deleted = 0",
            params![seq, *usn, stamp, id],
        )?;
    }
    Ok(())
}

fn count(conn: &Connection, usn: i64, stamp: &str) -> Result<(), DbError> {
    let counted = conn.execute(
        "UPDATE agentRegistry SET int_1 = ?1, updated_at = ?2 WHERE registry_id = 'localUpdateCount'",
        params![usn, stamp],
    )?;
    if counted != 1 {
        return Err(refused("missing or duplicate local update counter"));
    }
    Ok(())
}

/// Why `edit` cannot be undone (`undo`) or redone now, read without
/// writing: `None` when it can.
///
/// # Errors
/// A failed read.
pub fn conflict(
    conn: &Connection,
    edit: &Definitions,
    undo: bool,
) -> Result<Option<String>, DbError> {
    let changed = |what: &str| Ok(Some(format!("{what} has changed since")));
    if undo {
        for order in &edit.ordered {
            if run(conn, &order.parent)? != order.after {
                return changed("the order of those My Tags");
            }
        }
    } else {
        for new in &edit.created {
            match row(conn, &new.id)? {
                Some((parent, name, true)) if parent == new.category && name == new.name => {}
                _ => return changed(&format!("My Tag {}", new.id)),
            }
            if name_taken(conn, &new.category, &new.id, &new.name)? {
                return changed(&format!("the category of My Tag {}", new.id));
            }
        }
    }
    for renamed in &edit.renamed {
        let (expected, target) = if undo {
            (&renamed.after, &renamed.before)
        } else {
            (&renamed.before, &renamed.after)
        };
        match row(conn, &renamed.id)? {
            Some((parent, name, false)) if name == *expected => {
                let swapped = |other: &str| edit.renamed.iter().any(|r| r.id == other);
                let mut statement = conn.prepare(
                    "SELECT ID FROM djmdMyTag
                     WHERE ParentID = ?1 AND ID <> ?2 AND Name = ?3 AND rb_local_deleted = 0",
                )?;
                let holders = statement
                    .query_map(params![parent, renamed.id, target], |r| {
                        r.get::<_, String>(0)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                if holders.iter().any(|holder| !swapped(holder)) {
                    return changed(&format!("the names beside My Tag {}", renamed.id));
                }
            }
            _ => return changed(&format!("the name of My Tag {}", renamed.id)),
        }
    }
    if undo {
        for new in &edit.created {
            match row(conn, &new.id)? {
                Some((parent, name, false)) if parent == new.category && name == new.name => {}
                // A made tag renamed by this same edit is checked above.
                Some((parent, _, false))
                    if parent == new.category && edit.renamed.iter().any(|r| r.id == new.id) => {}
                _ => return changed(&format!("My Tag {}", new.id)),
            }
            let carried: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM djmdSongMyTag WHERE MyTagID = ?1 AND rb_local_deleted = 0)",
                params![new.id],
                |r| r.get(0),
            )?;
            if carried {
                return Ok(Some(format!("a track carries My Tag {} since", new.id)));
            }
        }
    }
    // An order redone is checked once the tags it holds are back.
    Ok(None)
}

/// Undoes (`undo`) or redoes `edit` in one immediate transaction: refused
/// with nothing written unless every row is as the step expects.
///
/// # Errors
/// [`DbError::Conflict`] when something changed since; a failed write.
pub fn step(
    writer: &mut rbl_db::write::Writer,
    edit: &Definitions,
    undo: bool,
) -> Result<(), DbError> {
    refuse_while_rekordbox_runs(writer)?;
    let conn = writer.library().connection();
    let tx = Immediate::begin(conn)?;
    if let Some(why) = conflict(conn, edit, undo)? {
        return Err(DbError::Conflict(why));
    }
    let stamp = rbl_core::time::now();
    let mut usn = next_usn(conn)? - 1;
    let flip = |id: &str, to: bool, usn: &mut i64| -> Result<(), DbError> {
        *usn += 1;
        let written = conn.execute(
            "UPDATE djmdMyTag SET rb_local_deleted = ?1, rb_local_usn = ?2, updated_at = ?3
             WHERE ID = ?4 AND rb_local_deleted = ?5",
            params![i64::from(to), *usn, stamp, id, i64::from(!to)],
        )?;
        if written == 1 {
            Ok(())
        } else {
            Err(DbError::Conflict(format!("My Tag {id} has changed since")))
        }
    };
    let name = |id: &str, to: &str, usn: &mut i64| -> Result<(), DbError> {
        *usn += 1;
        conn.execute(
            "UPDATE djmdMyTag SET Name = ?1, rb_local_usn = ?2, updated_at = ?3
             WHERE ID = ?4 AND rb_local_deleted = 0",
            params![to, *usn, stamp, id],
        )?;
        Ok(())
    };
    if undo {
        for order in edit.ordered.iter().rev() {
            write_seqs(conn, &order.after, &order.before, &stamp, &mut usn)?;
        }
        for renamed in edit.renamed.iter().rev() {
            name(&renamed.id, &renamed.before, &mut usn)?;
        }
        for new in edit.created.iter().rev() {
            flip(&new.id, true, &mut usn)?;
        }
    } else {
        for new in &edit.created {
            flip(&new.id, false, &mut usn)?;
        }
        for renamed in &edit.renamed {
            name(&renamed.id, &renamed.after, &mut usn)?;
        }
        for order in &edit.ordered {
            if run(conn, &order.parent)? != order.before {
                return Err(DbError::Conflict(
                    "the order of those My Tags has changed since".into(),
                ));
            }
            write_seqs(conn, &order.before, &order.after, &stamp, &mut usn)?;
        }
    }
    count(conn, usn, &stamp)?;
    tx.commit()?;
    Ok(())
}
