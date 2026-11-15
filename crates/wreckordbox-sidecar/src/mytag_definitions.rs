//! What MyTag Projection (wbx-14.14) keeps about MyTag **definitions**:
//! beside [`crate::mytag_projection`] (assignments), one MyTag at a time.
//!
//! A [`DefinitionMark`] records, for one `djmdMyTag` row, the Tag schema
//! term it stands for (a durable link the DJ's file does not hold, so a
//! label changed in the schema is told from a new term) and, when a
//! projection made or renamed the row, the row exactly as that write left
//! it, read back in the write's own transaction: name, `Seq`,
//! `rb_local_usn` and `updated_at`. A later write of any kind to the row
//! (the DJ in the window, the CLI or rekordbox; an Undo or a Redo of the
//! projection) gives it another update number, so no clock is ever
//! compared. Kept in the `meta` table, one key per MyTag
//! (`projection_def:<tag>`); [`Sidecar::set_definition_marks`] is the only
//! write in this module, made only by an apply that wrote.
//!
//! A mark is evidence of the projection's ownership only while the Ledger
//! entry of the write that left it was recorded (`entry`: the write's
//! intent id, which its entry takes): a write cut off before its entry left
//! a mark that proves nothing.
#![allow(
    clippy::doc_markdown,
    reason = "MyTag and the like are the glossary's words, not code items"
)]
use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::{FactsReader, Result, Sidecar};

/// The `meta` key of a MyTag's [`DefinitionMark`], before the tag's id.
const MARK_KEY: &str = "projection_def:";
/// The first key after every mark key (`:` + 1 is `;`).
const MARK_END: &str = "projection_def;";

/// The row a projection's write left for one MyTag, and the term it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionMark {
    /// The MyTag category the row is in.
    pub category: String,
    /// The Tag schema's dimension and term the MyTag stands for.
    pub dimension: String,
    pub term: String,
    /// Whether a projection made or renamed this row. `false`: the link
    /// alone, for a MyTag the DJ made, which a projection never changes.
    pub owned: bool,
    /// The row as the write left it (meaningful when `owned`).
    pub name: String,
    pub seq: Option<i64>,
    pub usn: i64,
    pub updated_at: String,
    /// The Ledger entry of the write that left it (0 for none).
    pub entry: i64,
    /// The Ledger's order position when it was left.
    pub ledger_seq: i64,
}

/// The definition marks, and which of their entries the Ledger holds.
#[derive(Debug, Clone, Default)]
pub struct DefinitionMarks {
    /// By MyTag id.
    pub marks: HashMap<String, DefinitionMark>,
    /// The marks' entries recorded in the Ledger as MyTag edits.
    pub recorded: HashSet<i64>,
}

fn read(db: &Connection) -> Result<DefinitionMarks> {
    let mut statement =
        db.prepare_cached("SELECT key, value FROM meta WHERE key >= ?1 AND key < ?2")?;
    let rows = statement
        .query_map(params![MARK_KEY, MARK_END], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // A mark that does not read is no mark: the MyTag is then the DJ's.
    let marks: HashMap<String, DefinitionMark> = rows
        .into_iter()
        .filter_map(|(key, value)| {
            let tag = key.strip_prefix(MARK_KEY)?;
            Some((tag.to_owned(), serde_json::from_str(&value).ok()?))
        })
        .collect();
    let mut recorded = HashSet::new();
    let mut found = db.prepare_cached(
        "SELECT EXISTS(SELECT 1 FROM ledger WHERE id = ?1 AND store = 'library' AND kind = 'track_tags')",
    )?;
    for entry in marks.values().map(|m| m.entry).filter(|e| *e > 0) {
        if !recorded.contains(&entry) && found.query_row(params![entry], |r| r.get::<_, bool>(0))? {
            recorded.insert(entry);
        }
    }
    Ok(DefinitionMarks { marks, recorded })
}

impl Sidecar {
    /// Every MyTag's [`DefinitionMark`].
    pub fn definition_marks(&self) -> Result<DefinitionMarks> {
        read(&self.db())
    }

    /// Keeps `marks`, by MyTag id, each replacing that MyTag's earlier one,
    /// together.
    pub fn set_definition_marks(&self, marks: &[(String, DefinitionMark)]) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        {
            let mut put = tx.prepare_cached(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            )?;
            for (tag, mark) in marks {
                let value = serde_json::to_string(mark).unwrap_or_default();
                put.execute(params![format!("{MARK_KEY}{tag}"), value])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

impl FactsReader {
    /// [`Sidecar::definition_marks`] on a read-only snapshot.
    pub fn definition_marks(&self) -> Result<DefinitionMarks> {
        read(self.connection())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn marks_are_kept_by_tag_and_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = Sidecar::open(dir.path()).unwrap();
        let mark = DefinitionMark {
            category: "1".into(),
            dimension: "mood".into(),
            term: "dark".into(),
            owned: true,
            name: "Dark".into(),
            seq: Some(3),
            usn: 10,
            updated_at: "t".into(),
            entry: 99,
            ledger_seq: 4,
        };
        sidecar
            .set_definition_marks(&[("77".into(), mark.clone())])
            .unwrap();
        let renamed = DefinitionMark {
            name: "Darker".into(),
            ..mark
        };
        sidecar
            .set_definition_marks(&[("77".into(), renamed.clone())])
            .unwrap();
        let read = sidecar.definition_marks().unwrap();
        assert_eq!(read.marks.get("77"), Some(&renamed));
        assert_eq!(read.marks.len(), 1);
        // Entry 99 was never recorded: no evidence.
        assert!(read.recorded.is_empty());
    }
}
