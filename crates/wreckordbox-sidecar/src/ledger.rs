//! The Ledger (ADR 0004): the durable, append-only record of every change
//! to the Library and to the Sidecar, kept in the Sidecar's database beside
//! the Facts, and the reading of a Run's changes to both.
//!
//! ```text
//! ledger(id, run_id, at, store, source_kind, source_name, action, kind,
//!        label, command, reverses, reversible, format, payload)
//! ```
//!
//! A `ledger` row is one change: which Run, which source, which `store`
//! (`library` or `sidecar`), what it was (`action`, `kind`, `label`, and
//! the service `command` that made it when one did), and in `payload` the
//! exact before and after values, as JSON whose shape `format` versions.
//! The Sidecar does not look inside a Library payload; whoever writes
//! Library edits owns its shape. Rows are never changed or removed: two
//! triggers refuse it. An Undo, a Redo or an Unwind is a new row whose
//! `reverses` names the row it steps.
//!
//! A Fact recorded in the Sidecar is not copied here: the `fact` row is
//! its own entry. It is written once, carries its Run, its source and its
//! time, and the Fact its source had before it is kept and points at it,
//! so [`Sidecar::run_entries`] reads a Run's Facts beside its `ledger`
//! rows as entries of one list. The one other change the Sidecar makes to
//! a Fact, marking a Measurement stale, is a `ledger` row of kind
//! [`STALE`], written in the transaction that marks it.
//!
//! # One order for both stores
//!
//! Every entry has a place in one durable sequence, `ledger_order`, shared
//! by `ledger` rows and Facts: a `ledger` row's `id` is its place, and a
//! Fact is given one by a trigger as it is inserted. A Run's entries are
//! read in that order, so two made in the same millisecond still read in
//! the order they were made.
//!
//! # Intents: no Library edit without a trace
//!
//! The Library and the Sidecar are two databases with no transaction in
//! common. So before a Library write, whoever writes it reserves the
//! entry's place with an *intent* ([`Sidecar::begin_intent`]), committed
//! before the Library is touched. The entry is later appended under that
//! identifier ([`NewEntry::id`]) and its intent is removed in the same
//! transaction; appending it again changes nothing, so a replay after a
//! crash cannot duplicate it. An intent still open after a restart
//! ([`Sidecar::open_intents`]) is a change that was begun and never
//! confirmed. The `write` action is for entries that say only that.

use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::store::{now, read_fact, select, SourceKind};
use crate::{Fact, Result, Run, RunId, Sidecar, Source};

pub(crate) const SCHEMA: &str = "
CREATE TABLE ledger_order (
    seq     INTEGER PRIMARY KEY AUTOINCREMENT,
    fact_id INTEGER UNIQUE REFERENCES fact (id)
) STRICT;

INSERT INTO ledger_order (fact_id) SELECT id FROM fact ORDER BY recorded_at, id;

CREATE TRIGGER fact_takes_its_place_in_order AFTER INSERT ON fact
BEGIN INSERT INTO ledger_order (fact_id) VALUES (new.id); END;

CREATE INDEX fact_by_successor ON fact (superseded_by);

CREATE TABLE ledger (
    id          INTEGER PRIMARY KEY REFERENCES ledger_order (seq),
    run_id      INTEGER NOT NULL REFERENCES run (id),
    at          INTEGER NOT NULL,
    store       TEXT NOT NULL CHECK (store IN ('library', 'sidecar')),
    source_kind TEXT NOT NULL CHECK (source_kind IN ('dj', 'agent', 'tool', 'web')),
    source_name TEXT NOT NULL,
    action      TEXT NOT NULL CHECK (action IN ('edit', 'undo', 'redo', 'write')),
    kind        TEXT NOT NULL,
    label       TEXT NOT NULL,
    command     TEXT,
    reverses    INTEGER,
    part_of     INTEGER,
    reversible  INTEGER NOT NULL CHECK (reversible IN (0, 1)),
    format      INTEGER NOT NULL,
    payload     TEXT
) STRICT;

CREATE INDEX ledger_by_run ON ledger (run_id);
CREATE INDEX ledger_by_reversed ON ledger (reverses);
CREATE INDEX ledger_by_whole ON ledger (part_of);

CREATE TABLE ledger_intent (
    id          INTEGER PRIMARY KEY REFERENCES ledger_order (seq),
    run_id      INTEGER REFERENCES run (id),
    at          INTEGER NOT NULL,
    source_kind TEXT NOT NULL,
    source_name TEXT NOT NULL,
    description TEXT NOT NULL,
    command     TEXT
) STRICT;

CREATE TABLE ledger_owned_run (
    run_id INTEGER PRIMARY KEY REFERENCES run (id)
) STRICT;

CREATE TRIGGER ledger_is_never_changed BEFORE UPDATE ON ledger
BEGIN SELECT RAISE(ABORT, 'the Ledger is append-only: an entry is never changed'); END;

CREATE TRIGGER ledger_is_never_removed BEFORE DELETE ON ledger
BEGIN SELECT RAISE(ABORT, 'the Ledger is append-only: an entry is never removed'); END;
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

impl Store {
    fn as_str(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::Sidecar => "sidecar",
        }
    }
}

/// The kind of the entry that says Measurements were marked stale. Its
/// payload (format 1) is `{"track", "fingerprint", "facts": [ids]}`: the
/// track, the fingerprint its audio has now, and the Facts marked.
pub const STALE: &str = "stale";

/// Who marks Measurements stale: wreckordbox itself, having found a
/// track's audio changed.
pub const HOUSEKEEPING: &str = "wreckordbox";

/// A change, to append.
#[derive(Debug, Clone)]
pub struct NewEntry<'a> {
    /// The place reserved for it by [`Sidecar::begin_intent`], or `None`
    /// for the next. An entry already there under it is kept as it is.
    pub id: Option<i64>,
    pub run: RunId,
    pub store: Store,
    pub source: &'a Source,
    /// When the change was made, in milliseconds since the Unix epoch;
    /// `None` for now.
    pub at: Option<i64>,
    pub action: Action,
    /// What kind of edit, in a stable word: `track`, `track_tags`, ...
    pub kind: &'a str,
    /// The edit's name for a person, as Undo shows it.
    pub label: &'a str,
    /// The service command that made the edit, `{"name", "args"}`.
    pub command: Option<&'a Value>,
    /// The entry this one takes back or makes again.
    pub reverses: Option<i64>,
    /// The entry whose change this one carries on (a later slice of one
    /// Add Tracks to Playlist names the first). An entry that reverses the
    /// first reverses every entry `part_of` it.
    pub part_of: Option<i64>,
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
    /// The `ledger` row's identifier, or for a recorded Fact
    /// ([`Action::Record`]) the Fact's: unique among entries of its store
    /// and action.
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
    pub part_of: Option<i64>,
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

/// A change about to be made to the Library, said before it is made.
#[derive(Debug, Clone)]
pub struct Intent<'a> {
    pub source: &'a Source,
    /// Its Run, when it has one already.
    pub run: Option<RunId>,
    /// What to call a Run made for it, when it has none.
    pub description: &'a str,
    /// The service command making it, `{"name", "args"}`.
    pub command: Option<&'a Value>,
    /// Milliseconds since the Unix epoch.
    pub at: i64,
}

/// An intent no entry has settled.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenIntent {
    /// The entry's reserved identifier.
    pub id: i64,
    pub run: Option<RunId>,
    pub at: i64,
    pub source: Source,
    pub description: String,
    pub command: Option<Value>,
    /// Native values before a DJ write, for recovery only.
    pub dj_snapshot: Option<Value>,
    /// What a write that replaces or copies playlist entries planned for
    /// their Mutants (wbx-80), for recovery only.
    pub mutant_plan: Option<Value>,
}

impl Sidecar {
    /// Says, durably, that a change to the Library is about to be made,
    /// and answers the identifier its entry will have. In the same
    /// transaction the intents in `settled` are removed: changes known to
    /// have needed no entry. An error means nothing can be recorded now.
    pub fn begin_intent(&self, intent: &Intent, settled: &[i64]) -> Result<i64> {
        self.begin_intent_with_snapshot(intent, settled, None)
    }

    /// Reserve an entry and retain the native pre-edit values in the same transaction.
    pub fn begin_intent_with_snapshot(
        &self,
        intent: &Intent,
        settled: &[i64],
        snapshot: Option<&Value>,
    ) -> Result<i64> {
        let mut db = self.db();
        let tx = db.transaction()?;
        settle_in(&tx, settled)?;
        tx.execute("INSERT INTO ledger_order DEFAULT VALUES", [])?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO ledger_intent (id, run_id, at, source_kind, source_name, description, command)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                intent.run.map(|run| run.0),
                intent.at,
                intent.source.kind.as_str(),
                intent.source.name,
                intent.description,
                intent.command.map(Value::to_string),
            ],
        )?;
        if let Some(snapshot) = snapshot {
            tx.execute(
                "INSERT INTO dj_intent(id,snapshot) VALUES(?1,?2)",
                params![id, snapshot.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    /// Removes intents that needed no entry.
    pub fn settle_intents(&self, settled: &[i64]) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        settle_in(&tx, settled)?;
        tx.commit()?;
        Ok(())
    }

    /// The intents no entry has settled, oldest first.
    pub fn open_intents(&self) -> Result<Vec<OpenIntent>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT id, run_id, at, source_kind, source_name, description, command, (SELECT snapshot FROM dj_intent WHERE dj_intent.id = ledger_intent.id),
                    (SELECT plan FROM mutant_intent WHERE mutant_intent.id = ledger_intent.id)
             FROM ledger_intent ORDER BY id",
        )?;
        let open = statement
            .query_map([], |row| {
                Ok(OpenIntent {
                    id: row.get(0)?,
                    run: row.get::<_, Option<i64>>(1)?.map(RunId),
                    at: row.get(2)?,
                    source: Source {
                        kind: SourceKind::from_stored(&row.get::<_, String>(3)?),
                        name: row.get(4)?,
                    },
                    description: row.get(5)?,
                    dj_snapshot: row
                        .get::<_, Option<String>>(7)?
                        .map(|text| {
                            serde_json::from_str(&text).map_err(|e| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    7,
                                    rusqlite::types::Type::Text,
                                    Box::new(e),
                                )
                            })
                        })
                        .transpose()?,
                    command: row
                        .get::<_, Option<String>>(6)?
                        .and_then(|text| serde_json::from_str(&text).ok()),
                    mutant_plan: row
                        .get::<_, Option<String>>(8)?
                        .map(|text| {
                            serde_json::from_str(&text).map_err(|e| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    8,
                                    rusqlite::types::Type::Text,
                                    Box::new(e),
                                )
                            })
                        })
                        .transpose()?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(open)
    }

    /// Appends one entry and answers its identifier. With an identifier
    /// reserved by [`Self::begin_intent`], its intent is settled in the
    /// same transaction, and an entry already there under it is left as it
    /// is: appending it again is a replay, not a second change.
    pub fn append(&self, entry: &NewEntry) -> Result<i64> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let id = append_in(&tx, entry)?;
        tx.commit()?;
        Ok(id)
    }

    /// Starts a Run that wreckordbox itself owns and ends (the window's,
    /// or a command's own), so that one a process that died left open can
    /// be told from a client's and ended ([`Self::finish_owned_runs`]).
    pub fn begin_owned_run(&self, source: &Source, description: &str) -> Result<RunId> {
        let mut db = self.db();
        let tx = db.transaction()?;
        tx.execute(
            "INSERT INTO run (source_kind, source_name, description, started_at) VALUES (?1, ?2, ?3, ?4)",
            params![source.kind.as_str(), source.name, description, now()],
        )?;
        let run = tx.last_insert_rowid();
        tx.execute("INSERT INTO ledger_owned_run (run_id) VALUES (?1)", [run])?;
        tx.commit()?;
        Ok(RunId(run))
    }

    /// Ends every Run wreckordbox owned and never ended: the ones a
    /// service that died left open. Runs clients started are left alone.
    pub fn finish_owned_runs(&self, outcome: &str) -> Result<usize> {
        Ok(self.db().execute(
            "UPDATE run SET finished_at = ?1, outcome = ?2
             WHERE finished_at IS NULL AND id IN (SELECT run_id FROM ledger_owned_run)",
            params![now(), outcome],
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
                  + (SELECT COUNT(*) FROM fact WHERE run_id = ?1 AND id NOT IN (SELECT fact_id FROM dj_fact))",
            [run.0],
            |row| row.get(0),
        )?;
        let mut keys = db.prepare(PAGE_QUERY)?;
        let keys = keys
            .query_map(params![run.0, to_i64(limit), to_i64(offset)], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<(i64, i64, Option<i64>)>>>()?;
        let mut entries = Vec::with_capacity(keys.len());
        for (store, id, replaced) in keys {
            if store == 0 {
                entries.push(db.query_row(
                    &format!("{ENTRY_COLUMNS} FROM ledger WHERE id = ?1"),
                    [id],
                    read_entry,
                )?);
            } else {
                let fact = db.query_row(&select("WHERE fact.id = ?1"), [id], read_fact)?;
                let replaced = if let Some(replaced) = replaced {
                    Some(db.query_row(&select("WHERE fact.id = ?1"), [replaced], read_fact)?)
                } else {
                    None
                };
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
                    part_of: None,
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

/// A page of a Run's entries, in the order made: `(store, id, replaced)`
/// for `?1` the Run, `?2` the limit and `?3` the offset. `store` is 0 for a
/// `ledger` row, 1 for a Fact, whose `replaced` is the Fact it took the
/// place of. Every step uses an index.
pub const PAGE_QUERY: &str = "
    SELECT store, id, replaced FROM (
        SELECT 0 AS store, ledger.id AS id, NULL AS replaced, ledger.id AS seq
        FROM ledger WHERE ledger.run_id = ?1
        UNION ALL
        SELECT 1, fact.id,
               (SELECT before.id FROM fact AS before
                WHERE before.superseded_by = fact.id ORDER BY before.id DESC LIMIT 1),
               ledger_order.seq
        FROM fact JOIN ledger_order ON ledger_order.fact_id = fact.id
        WHERE fact.run_id = ?1 AND fact.id NOT IN (SELECT fact_id FROM dj_fact))
    ORDER BY seq LIMIT ?2 OFFSET ?3";

fn settle_in(tx: &rusqlite::Transaction, settled: &[i64]) -> Result<()> {
    let mut remove = tx.prepare_cached("DELETE FROM ledger_intent WHERE id = ?1")?;
    for id in settled {
        remove.execute([id])?;
    }
    Ok(())
}

fn counts_of(db: &rusqlite::Connection, run: RunId) -> Result<Vec<Count>> {
    let mut counts = db.prepare(
        "SELECT store, kind, SUM(count) FROM (
             SELECT store, kind, COUNT(*) AS count FROM ledger WHERE run_id = ?1 GROUP BY store, kind
             UNION ALL
             SELECT 'sidecar', kind, COUNT(*) FROM fact WHERE run_id = ?1 AND id NOT IN (SELECT fact_id FROM dj_fact) GROUP BY kind)
         GROUP BY store, kind ORDER BY store, kind",
    )?;
    let counts = counts
        .query_map([run.0], |row| {
            Ok(Count {
                store: store_from(&row.get::<_, String>(0)?),
                kind: row.get(1)?,
                count: u64::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
            })
        })?
        .collect::<rusqlite::Result<Vec<Count>>>()?;
    Ok(counts)
}

fn store_from(text: &str) -> Store {
    if text == "sidecar" {
        Store::Sidecar
    } else {
        Store::Library
    }
}

pub(crate) fn append_in(tx: &rusqlite::Transaction, entry: &NewEntry) -> Result<i64> {
    let id = if let Some(id) = entry.id {
        let there: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM ledger WHERE id = ?1)",
            [id],
            |row| row.get(0),
        )?;
        if there {
            settle_in(tx, &[id])?;
            return Ok(id);
        }
        // Reserved by an intent, or by one in a store this one was
        // copied from: make sure its place exists.
        tx.execute("INSERT OR IGNORE INTO ledger_order (seq) VALUES (?1)", [id])?;
        id
    } else {
        tx.execute("INSERT INTO ledger_order DEFAULT VALUES", [])?;
        tx.last_insert_rowid()
    };
    tx.execute(
        "INSERT INTO ledger (id, run_id, at, source_kind, source_name, action, kind, label, command,
                             reverses, part_of, reversible, format, payload, store)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            id,
            entry.run.0,
            entry.at.unwrap_or_else(now),
            entry.source.kind.as_str(),
            entry.source.name,
            entry.action.as_str(),
            entry.kind,
            entry.label,
            entry.command.map(Value::to_string),
            entry.reverses,
            entry.part_of,
            entry.reversible,
            entry.format,
            entry.payload.map(Value::to_string),
            entry.store.as_str(),
        ],
    )?;
    settle_in(tx, &[id])?;
    Ok(id)
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
                                    command, reverses, reversible, format, payload, store, part_of";

fn read_entry(row: &Row) -> rusqlite::Result<Entry> {
    let json = |text: Option<String>| text.and_then(|text| serde_json::from_str(&text).ok());
    Ok(Entry {
        store: store_from(&row.get::<_, String>(13)?),
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
        part_of: row.get(14)?,
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
