//! The Ledger (ADR 0004): the durable, append-only record of every edit to
//! the Library, kept in the Sidecar's database beside the Facts, and the
//! reading of a Run's changes to both.
//!
//! ```text
//! ledger(id, run_id, at, source_kind, source_name, action, kind, label,
//!        command, reverses, reversible, format, payload)
//! ledger_pending(id, at, source_kind, source_name, command)
//! ```
//!
//! A `ledger` row is one edit to the Library: which Run, which source,
//! what it was (`action`, `kind`, `label`, and the service `command` that
//! made it when one did), and in `payload` the exact before and after
//! values, as JSON whose shape `format` versions. The Sidecar does not look
//! inside a payload; whoever writes Library edits owns its shape. Rows are
//! never changed or removed: two triggers refuse it. An Undo, a Redo or an
//! Unwind is a new row whose `reverses` names the row it steps.
//!
//! Changes to the Sidecar itself are not copied here. A Fact already
//! carries its Run, its source and its time, and the Fact its source had
//! before it is kept, so [`Sidecar::run_entries`] reads a Run's Facts
//! beside its Library edits as entries of one list.
//!
//! `ledger_pending` is the write-ahead half. A row is written before a
//! write to the Library begins and leaves, in the transaction that appends
//! the entry, once the write is on record. One left behind by a service
//! that died is turned into an entry saying so when the Ledger is next
//! used ([`Sidecar::settle_pending`]), so the Library is never changed with
//! nothing on record.

use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::store::{now, read_fact, select, SourceKind};
use crate::{Fact, Result, Run, RunId, Sidecar, Source};

pub(crate) const SCHEMA: &str = "
CREATE TABLE ledger (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id      INTEGER NOT NULL REFERENCES run (id),
    at          INTEGER NOT NULL,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('dj', 'agent', 'tool', 'web')),
    source_name TEXT NOT NULL,
    action      TEXT NOT NULL CHECK (action IN ('edit', 'undo', 'redo', 'write')),
    kind        TEXT NOT NULL,
    label       TEXT NOT NULL,
    command     TEXT,
    reverses    INTEGER REFERENCES ledger (id),
    reversible  INTEGER NOT NULL CHECK (reversible IN (0, 1)),
    format      INTEGER NOT NULL,
    payload     TEXT
) STRICT;

CREATE INDEX ledger_by_run ON ledger (run_id);
CREATE INDEX ledger_by_reversed ON ledger (reverses);

CREATE TRIGGER ledger_is_never_changed BEFORE UPDATE ON ledger
BEGIN SELECT RAISE(ABORT, 'the Ledger is append-only: an entry is never changed'); END;

CREATE TRIGGER ledger_is_never_removed BEFORE DELETE ON ledger
BEGIN SELECT RAISE(ABORT, 'the Ledger is append-only: an entry is never removed'); END;

CREATE TABLE ledger_pending (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    at          INTEGER NOT NULL,
    source_kind TEXT,
    source_name TEXT,
    command     TEXT
) STRICT;
";

/// What an entry was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// An edit to the Library.
    Edit,
    /// The entry `reverses` names, taken back.
    Undo,
    /// The entry `reverses` names, made again after an Undo.
    Redo,
    /// A write to the Library known only to have happened: nothing says
    /// what it changed.
    Write,
    /// A Fact recorded in the Sidecar.
    Record,
}

impl Action {
    fn as_str(self) -> &'static str {
        match self {
            Self::Edit => "edit",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::Write => "write",
            Self::Record => "record",
        }
    }

    fn from_stored(text: &str) -> Self {
        match text {
            "undo" => Self::Undo,
            "redo" => Self::Redo,
            "write" => Self::Write,
            "record" => Self::Record,
            _ => Self::Edit,
        }
    }
}

/// Which store an entry changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Store {
    Library,
    Sidecar,
}

/// An edit to the Library, to append.
#[derive(Debug, Clone)]
pub struct NewEntry<'a> {
    pub run: RunId,
    pub source: &'a Source,
    pub action: Action,
    /// What kind of edit, in a stable word: `track`, `track_tags`, ...
    pub kind: &'a str,
    /// The edit's name for a person, as Undo shows it.
    pub label: &'a str,
    /// The service command that made the edit, `{"name", "args"}`.
    pub command: Option<&'a Value>,
    /// The entry this one takes back or makes again.
    pub reverses: Option<i64>,
    /// Whether `payload` is everything needed to take the edit back.
    pub reversible: bool,
    /// The version of `payload`'s shape.
    pub format: u32,
    pub payload: Option<&'a Value>,
}

/// One change in a Run: an edit to the Library, or a Fact recorded in the
/// Sidecar.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub store: Store,
    /// The `ledger` row for the Library, the Fact's identifier for the
    /// Sidecar: unique within its store.
    pub id: i64,
    pub run: RunId,
    /// Milliseconds since the Unix epoch.
    pub at: i64,
    pub source: Source,
    pub action: Action,
    pub kind: String,
    pub label: String,
    pub command: Option<Value>,
    pub reverses: Option<i64>,
    pub reversible: bool,
    pub format: u32,
    pub payload: Option<Value>,
    /// For the Sidecar: the Fact recorded, and the Fact of the same track,
    /// kind and source it took the place of.
    pub fact: Option<Fact>,
    pub replaced: Option<Fact>,
}

/// How many changes of one kind a Run made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Count {
    pub store: Store,
    pub kind: String,
    pub count: u64,
}

/// A Run and how much it changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSummary {
    pub run: Run,
    /// By store and kind, the Library's first, each by kind.
    pub counts: Vec<Count>,
}

/// A write to the Library that began and was never put on record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub id: i64,
    pub at: i64,
    pub source: Option<Source>,
    pub command: Option<String>,
}

impl Sidecar {
    /// Says a write to the Library is about to begin, durably, and answers
    /// the note's identifier. `clear` is an earlier note to remove with it.
    /// When this fails the write must not be made.
    pub fn begin_write(
        &self,
        source: Option<&Source>,
        command: Option<&str>,
        clear: Option<i64>,
    ) -> Result<i64> {
        let mut db = self.db();
        let tx = db.transaction()?;
        if let Some(earlier) = clear {
            tx.execute("DELETE FROM ledger_pending WHERE id = ?1", [earlier])?;
        }
        tx.execute(
            "INSERT INTO ledger_pending (at, source_kind, source_name, command) VALUES (?1, ?2, ?3, ?4)",
            params![
                now(),
                source.map(|source| source.kind.as_str()),
                source.map(|source| source.name.as_str()),
                command
            ],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    /// Removes a note of a write that changed nothing, or whose entry is
    /// not this Ledger's to write.
    pub fn end_write(&self, pending: i64) -> Result<()> {
        self.db()
            .execute("DELETE FROM ledger_pending WHERE id = ?1", [pending])?;
        Ok(())
    }

    /// Appends one entry and, in the same transaction, removes the note of
    /// the write it records. Answers the entry's identifier.
    pub fn append(&self, entry: &NewEntry, pending: Option<i64>) -> Result<i64> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let id = append_in(&tx, entry)?;
        if let Some(pending) = pending {
            tx.execute("DELETE FROM ledger_pending WHERE id = ?1", [pending])?;
        }
        tx.commit()?;
        Ok(id)
    }

    /// The notes of writes that began and are not on record, oldest first.
    pub fn pending_writes(&self) -> Result<Vec<Pending>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT id, at, source_kind, source_name, command FROM ledger_pending ORDER BY id",
        )?;
        let pending = statement
            .query_map([], |row| {
                let kind: Option<String> = row.get(2)?;
                let name: Option<String> = row.get(3)?;
                Ok(Pending {
                    id: row.get(0)?,
                    at: row.get(1)?,
                    source: kind
                        .zip(name)
                        .map(|(kind, name)| Source::new(SourceKind::from_stored(&kind), name)),
                    command: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<Pending>>>()?;
        Ok(pending)
    }

    /// Turns every note of a write that never reached the record, other
    /// than `keep`, into an entry of `run` saying so (`entry` builds it),
    /// all in one transaction. Answers how many there were.
    pub fn settle_pending(
        &self,
        keep: Option<i64>,
        run: impl FnOnce() -> Result<RunId>,
        entry: impl Fn(&Pending, RunId) -> OwnedEntry,
    ) -> Result<usize> {
        let left: Vec<Pending> = self
            .pending_writes()?
            .into_iter()
            .filter(|pending| Some(pending.id) != keep)
            .collect();
        if left.is_empty() {
            return Ok(0);
        }
        let run = run()?;
        let mut db = self.db();
        let tx = db.transaction()?;
        for pending in &left {
            let owned = entry(pending, run);
            append_in(&tx, &owned.borrowed())?;
            tx.execute("DELETE FROM ledger_pending WHERE id = ?1", [pending.id])?;
        }
        tx.commit()?;
        Ok(left.len())
    }

    /// Ends every Run of `source` called `description` that was never
    /// ended: the ones a service that died left open.
    pub fn finish_abandoned_runs(
        &self,
        source: &Source,
        description: &str,
        outcome: &str,
    ) -> Result<usize> {
        Ok(self.db().execute(
            "UPDATE run SET finished_at = ?1, outcome = ?2
             WHERE finished_at IS NULL AND source_kind = ?3 AND source_name = ?4 AND description = ?5",
            params![now(), outcome, source.kind.as_str(), source.name, description],
        )?)
    }

    /// Runs, newest first, each with how many changes of each kind it made
    /// to the Library and to the Sidecar; and how many Runs there are.
    pub fn runs(&self, limit: usize, offset: usize) -> Result<(Vec<RunSummary>, usize)> {
        let db = self.db();
        let total: i64 = db.query_row("SELECT COUNT(*) FROM run", [], |row| row.get(0))?;
        let mut statement = db.prepare(&format!(
            "{RUN_COLUMNS} FROM run ORDER BY id DESC LIMIT ?1 OFFSET ?2"
        ))?;
        let runs = statement
            .query_map(params![to_i64(limit), to_i64(offset)], read_run)?
            .collect::<rusqlite::Result<Vec<Run>>>()?;
        let mut summaries = Vec::with_capacity(runs.len());
        for run in runs {
            let counts = counts_of(&db, run.id)?;
            summaries.push(RunSummary { run, counts });
        }
        Ok((summaries, usize::try_from(total).unwrap_or(0)))
    }

    /// One Run with its counts.
    pub fn run_summary(&self, run: RunId) -> Result<Option<RunSummary>> {
        let Some(run) = self.run(run)? else {
            return Ok(None);
        };
        let counts = counts_of(&self.db(), run.id)?;
        Ok(Some(RunSummary { run, counts }))
    }

    /// What `run` changed, in the order it happened: its edits to the
    /// Library from the Ledger and the Facts it recorded in the Sidecar,
    /// as one list, a page of it; and how many there are in all.
    pub fn run_entries(
        &self,
        run: RunId,
        limit: usize,
        offset: usize,
    ) -> Result<(Vec<Entry>, usize)> {
        let db = self.db();
        let total: i64 = db.query_row(
            "SELECT (SELECT COUNT(*) FROM ledger WHERE run_id = ?1)
                  + (SELECT COUNT(*) FROM fact WHERE run_id = ?1)",
            [run.0],
            |row| row.get(0),
        )?;
        let mut keys = db.prepare(
            "SELECT 0 AS store, id, at FROM ledger WHERE run_id = ?1
             UNION ALL
             SELECT 1, id, recorded_at FROM fact WHERE run_id = ?1
             ORDER BY 3, 1, 2 LIMIT ?2 OFFSET ?3",
        )?;
        let keys = keys
            .query_map(params![run.0, to_i64(limit), to_i64(offset)], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<(i64, i64)>>>()?;
        let mut entries = Vec::with_capacity(keys.len());
        for (store, id) in keys {
            if store == 0 {
                entries.push(db.query_row(
                    &format!("{ENTRY_COLUMNS} FROM ledger WHERE id = ?1"),
                    [id],
                    read_entry,
                )?);
            } else {
                let fact = db.query_row(&select("WHERE fact.id = ?1"), [id], read_fact)?;
                let replaced = db
                    .query_row(
                        &select("WHERE fact.superseded_by = ?1 ORDER BY fact.id DESC LIMIT 1"),
                        [id],
                        read_fact,
                    )
                    .optional()?;
                entries.push(Entry {
                    store: Store::Sidecar,
                    id,
                    run: fact.run,
                    at: fact.recorded_at,
                    source: fact.source.clone(),
                    action: Action::Record,
                    kind: fact.kind.clone(),
                    label: "Record Fact".to_owned(),
                    command: None,
                    reverses: None,
                    reversible: true,
                    format: 0,
                    payload: None,
                    fact: Some(fact),
                    replaced,
                });
            }
        }
        Ok((entries, usize::try_from(total).unwrap_or(0)))
    }

    /// One entry of the Ledger by its identifier.
    pub fn ledger_entry(&self, id: i64) -> Result<Option<Entry>> {
        Ok(self
            .db()
            .query_row(
                &format!("{ENTRY_COLUMNS} FROM ledger WHERE id = ?1"),
                [id],
                read_entry,
            )
            .optional()?)
    }
}

/// A [`NewEntry`] that owns what it holds.
#[derive(Debug, Clone)]
pub struct OwnedEntry {
    pub run: RunId,
    pub source: Source,
    pub action: Action,
    pub kind: String,
    pub label: String,
    pub command: Option<Value>,
}

impl OwnedEntry {
    fn borrowed(&self) -> NewEntry<'_> {
        NewEntry {
            run: self.run,
            source: &self.source,
            action: self.action,
            kind: &self.kind,
            label: &self.label,
            command: self.command.as_ref(),
            reverses: None,
            reversible: false,
            format: 0,
            payload: None,
        }
    }
}

fn counts_of(db: &rusqlite::Connection, run: RunId) -> Result<Vec<Count>> {
    let mut counts = db.prepare(
        "SELECT 0, kind, COUNT(*) FROM ledger WHERE run_id = ?1 GROUP BY kind
         UNION ALL
         SELECT 1, kind, COUNT(*) FROM fact WHERE run_id = ?1 GROUP BY kind
         ORDER BY 1, 2",
    )?;
    let counts = counts
        .query_map([run.0], |row| {
            Ok(Count {
                store: if row.get::<_, i64>(0)? == 0 {
                    Store::Library
                } else {
                    Store::Sidecar
                },
                kind: row.get(1)?,
                count: u64::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
            })
        })?
        .collect::<rusqlite::Result<Vec<Count>>>()?;
    Ok(counts)
}

fn append_in(tx: &rusqlite::Transaction, entry: &NewEntry) -> Result<i64> {
    tx.execute(
        "INSERT INTO ledger (run_id, at, source_kind, source_name, action, kind, label, command,
                             reverses, reversible, format, payload)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            entry.run.0,
            now(),
            entry.source.kind.as_str(),
            entry.source.name,
            entry.action.as_str(),
            entry.kind,
            entry.label,
            entry.command.map(Value::to_string),
            entry.reverses,
            entry.reversible,
            entry.format,
            entry.payload.map(Value::to_string),
        ],
    )?;
    Ok(tx.last_insert_rowid())
}

const RUN_COLUMNS: &str =
    "SELECT id, source_kind, source_name, description, started_at, finished_at, outcome";

fn read_run(row: &Row) -> rusqlite::Result<Run> {
    Ok(Run {
        id: RunId(row.get(0)?),
        source: Source {
            kind: SourceKind::from_stored(&row.get::<_, String>(1)?),
            name: row.get(2)?,
        },
        description: row.get(3)?,
        started_at: row.get(4)?,
        finished_at: row.get(5)?,
        outcome: row.get(6)?,
    })
}

const ENTRY_COLUMNS: &str = "SELECT id, run_id, at, source_kind, source_name, action, kind, label,
                                    command, reverses, reversible, format, payload";

fn read_entry(row: &Row) -> rusqlite::Result<Entry> {
    let json = |text: Option<String>| text.and_then(|text| serde_json::from_str(&text).ok());
    Ok(Entry {
        store: Store::Library,
        id: row.get(0)?,
        run: RunId(row.get(1)?),
        at: row.get(2)?,
        source: Source {
            kind: SourceKind::from_stored(&row.get::<_, String>(3)?),
            name: row.get(4)?,
        },
        action: Action::from_stored(&row.get::<_, String>(5)?),
        kind: row.get(6)?,
        label: row.get(7)?,
        command: json(row.get(8)?),
        reverses: row.get(9)?,
        reversible: row.get(10)?,
        format: row.get(11)?,
        payload: json(row.get(12)?),
        fact: None,
        replaced: None,
    })
}

fn to_i64(count: usize) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}
