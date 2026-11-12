//! Mutants (ADR 0008): a playlist's own edited version of a track.
//!
//! A Mutant belongs to one playlist entry. It is keyed to the entry's own
//! identity in the Library (the identifier of the entry's row, not its
//! position, so reordering the playlist leaves it attached) and records the
//! playlist, the parent track, the fingerprint of the parent's audio and the
//! edits. Nothing of it is in the Library: the playlist's entry there stays
//! the parent track.
//!
//! ```text
//! mutant(id, playlist_id, entry_id, track_id, fingerprint,
//!        created_at, created_run, removed_at, removed_run)
//! mutant_change(id, mutant_id, action, edits, fingerprint,
//!               source_kind, source_name, run_id, recorded_at)
//! ```
//!
//! Mutants are not Facts and are not in the Fact table, but keep its
//! rules. A `mutant` row is written once; afterwards only `removed_at` and
//! `removed_run` are set, once. Everything that happens to a Mutant is a
//! `mutant_change` row, written once and never changed or deleted, carrying
//! the edits as they stand after it, the parent's fingerprint at the time,
//! who made it and the Run it was made in. A Mutant's edits are those of
//! its newest change. So the Ledger can list a Run's Mutant changes and
//! Unwind can put back the edits of the change before, with no change to
//! what is stored.
//!
//! An entry has at most one Mutant at a time. Removing a Mutant keeps its
//! rows; mutating the entry again makes a new Mutant with a new identifier.
//!
//! The edits are a JSON object. An empty object is "as the parent"; a key
//! that is absent means that aspect is the parent's. What the keys are is
//! for whoever defines the edits; this store keeps the object whole.

use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::store::{now, run_in, Run, RunId, Sidecar, Source, SourceKind};
use crate::{Fingerprint, Result, SidecarError};

/// What version 2 of the store adds.
pub(crate) const SCHEMA: &str = "
CREATE TABLE mutant (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    playlist_id TEXT NOT NULL,
    entry_id    TEXT NOT NULL,
    track_id    TEXT NOT NULL,
    fingerprint TEXT,
    created_at  INTEGER NOT NULL,
    created_run INTEGER NOT NULL REFERENCES run (id),
    removed_at  INTEGER,
    removed_run INTEGER REFERENCES run (id)
) STRICT;

CREATE UNIQUE INDEX mutant_live_entry ON mutant (entry_id) WHERE removed_at IS NULL;
CREATE INDEX mutant_by_playlist ON mutant (playlist_id);
CREATE INDEX mutant_by_track ON mutant (track_id);

CREATE TABLE mutant_change (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    mutant_id   INTEGER NOT NULL REFERENCES mutant (id),
    action      TEXT NOT NULL,
    edits       TEXT NOT NULL,
    fingerprint TEXT,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('dj', 'agent', 'tool', 'web')),
    source_name TEXT NOT NULL,
    run_id      INTEGER NOT NULL REFERENCES run (id),
    recorded_at INTEGER NOT NULL
) STRICT;

CREATE INDEX mutant_change_by_mutant ON mutant_change (mutant_id);
CREATE INDEX mutant_change_by_run ON mutant_change (run_id);
";

/// What a change to a Mutant did, as `mutant_change.action` holds it. The
/// column takes any text, so a later kind of change needs no new schema.
pub mod actions {
    /// The entry was mutated.
    pub const CREATED: &str = "created";
    /// The Mutant's edits were changed.
    pub const EDITED: &str = "edited";
    /// The Mutant was removed; the entry is the core track again.
    pub const REMOVED: &str = "removed";
}

/// A Mutant to make.
#[derive(Debug, Clone)]
pub struct NewMutant<'a> {
    /// The playlist's identifier in the Library.
    pub playlist_id: &'a str,
    /// The playlist entry's own identifier in the Library.
    pub entry_id: &'a str,
    /// The parent track's identifier in the Library.
    pub track_id: &'a str,
    /// The fingerprint of the parent's audio as it is now; `None` when its
    /// file is not at hand.
    pub fingerprint: Option<&'a Fingerprint>,
    /// The edits it starts with: a JSON object, empty for "as the parent".
    pub edits: Value,
    pub source: &'a Source,
    pub run: InRun<'a>,
}

/// The Run a change to a Mutant is made in. It is checked, made and ended
/// in the transaction that records the change, so a change is never kept
/// without its Run, nor made in a Run another caller has just ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InRun<'a> {
    /// A Run that is open now. Refused with [`SidecarError::NoRun`] or
    /// [`SidecarError::RunFinished`]; it stays open.
    Open(RunId),
    /// A Run made for this change alone, by the change's source, ended
    /// with the outcome `"<action> Mutant <id>"`.
    Own { description: &'a str },
}

impl From<RunId> for InRun<'_> {
    fn from(run: RunId) -> Self {
        Self::Open(run)
    }
}

/// A change made: the Mutant as it is after it, and its Run as it is then.
#[derive(Debug, Clone, PartialEq)]
pub struct Committed {
    pub mutant: Mutant,
    pub run: Run,
}

/// A change to a Mutant that exists: new edits, or its removal.
#[derive(Debug, Clone)]
pub struct MutantUpdate<'a> {
    /// The fingerprint of the parent's audio as it is now, when known.
    pub fingerprint: Option<&'a Fingerprint>,
    pub source: &'a Source,
    pub run: InRun<'a>,
}

/// A Mutant as the Sidecar holds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mutant {
    pub id: i64,
    pub playlist_id: String,
    pub entry_id: String,
    /// The parent track.
    pub track_id: String,
    /// The fingerprint of the parent's audio when the entry was mutated.
    pub fingerprint: Option<Fingerprint>,
    /// The edits as they stand: those of the newest change.
    pub edits: Value,
    /// Who made the newest change, and in which Run.
    pub source: Source,
    pub run: RunId,
    /// Milliseconds since the Unix epoch.
    pub created_at: i64,
    /// When the newest change was made.
    pub changed_at: i64,
    /// Set once the Mutant has been removed.
    pub removed_at: Option<i64>,
}

impl Mutant {
    /// Whether the entry still has this Mutant: it has not been removed.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.removed_at.is_none()
    }
}

/// One thing that happened to a Mutant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MutantChange {
    pub id: i64,
    pub mutant_id: i64,
    /// One of [`actions`].
    pub action: String,
    /// The Mutant's edits after this change.
    pub edits: Value,
    /// The fingerprint of the parent's audio when the change was made.
    pub fingerprint: Option<Fingerprint>,
    pub source: Source,
    pub run: RunId,
    /// Milliseconds since the Unix epoch.
    pub recorded_at: i64,
}

impl Sidecar {
    /// Makes a Mutant for a playlist entry. Refused with
    /// [`SidecarError::AlreadyMutant`] when the entry has one, and as
    /// [`InRun`] says when its Run cannot take it; a refusal keeps nothing.
    pub fn create_mutant(&self, new: &NewMutant) -> Result<Committed> {
        if new.playlist_id.is_empty() || new.entry_id.is_empty() || new.track_id.is_empty() {
            return Err(SidecarError::Invalid(
                "a Mutant needs a playlist, an entry and a parent track".to_owned(),
            ));
        }
        let edits = edits_text(&new.edits)?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let existing: Option<i64> = tx
            .query_row(
                "SELECT id FROM mutant WHERE entry_id = ?1 AND removed_at IS NULL",
                [new.entry_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(mutant) = existing {
            return Err(SidecarError::AlreadyMutant {
                entry_id: new.entry_id.to_owned(),
                mutant,
            });
        }
        let (run, own) = take_run(&tx, new.run, new.source)?;
        let at = now();
        let fingerprint = new.fingerprint.map(Fingerprint::as_str);
        tx.execute(
            "INSERT INTO mutant (playlist_id, entry_id, track_id, fingerprint, created_at, created_run)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                new.playlist_id,
                new.entry_id,
                new.track_id,
                fingerprint,
                at,
                run.0
            ],
        )?;
        let id = tx.last_insert_rowid();
        insert_change(
            &tx,
            id,
            actions::CREATED,
            &edits,
            fingerprint,
            new.source,
            run,
            at,
        )?;
        let committed = settle(&tx, id, actions::CREATED, run, own)?;
        tx.commit()?;
        Ok(committed)
    }

    /// Gives a live Mutant new edits: a JSON object that replaces the edits
    /// it had. The edits before are kept in its history.
    pub fn edit_mutant(&self, id: i64, edits: &Value, update: &MutantUpdate) -> Result<Committed> {
        let edits = edits_text(edits)?;
        self.change(id, actions::EDITED, Some(&edits), update)
    }

    /// Removes a live Mutant: its entry is the core track again. Its rows
    /// are kept, marked removed.
    pub fn remove_mutant(&self, id: i64, update: &MutantUpdate) -> Result<Committed> {
        self.change(id, actions::REMOVED, None, update)
    }

    /// Read the stored Mutant archive without changing it.
    pub fn mutant(&self, id: i64) -> Result<Option<Mutant>> {
        MutantReader::new(&self.db()).mutant(id)
    }

    /// Read the stored Mutant archive without changing it.
    pub fn mutant_of_entry(&self, entry_id: &str) -> Result<Option<Mutant>> {
        MutantReader::new(&self.db()).mutant_of_entry(entry_id)
    }

    /// Read the stored Mutant archive without changing it.
    pub fn mutants(&self, playlist_id: Option<&str>) -> Result<Vec<Mutant>> {
        MutantReader::new(&self.db()).mutants(playlist_id)
    }

    /// Read the stored Mutant archive without changing it.
    pub fn mutants_of_track(&self, track_id: &str) -> Result<Vec<Mutant>> {
        MutantReader::new(&self.db()).mutants_of_track(track_id)
    }

    /// Read the stored Mutant archive without changing it.
    pub fn mutant_changes(&self, id: i64) -> Result<Vec<MutantChange>> {
        MutantReader::new(&self.db()).mutant_changes(id)
    }

    /// Read the stored Mutant archive without changing it.
    pub fn mutant_changes_in_run(&self, run: RunId) -> Result<Vec<MutantChange>> {
        MutantReader::new(&self.db()).mutant_changes_in_run(run)
    }

    /// One more change to a live Mutant. `edits` of `None` keeps the edits
    /// it has (a removal records what was removed).
    fn change(
        &self,
        id: i64,
        action: &str,
        edits: Option<&str>,
        update: &MutantUpdate,
    ) -> Result<Committed> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let current: Option<String> = tx
            .query_row(
                "SELECT (SELECT edits FROM mutant_change WHERE mutant_id = mutant.id
                         ORDER BY id DESC LIMIT 1)
                 FROM mutant WHERE id = ?1 AND removed_at IS NULL",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(current) = current else {
            return Err(SidecarError::NoMutant(id));
        };
        let (run, own) = take_run(&tx, update.run, update.source)?;
        let at = now();
        insert_change(
            &tx,
            id,
            action,
            edits.unwrap_or(&current),
            update.fingerprint.map(Fingerprint::as_str),
            update.source,
            run,
            at,
        )?;
        if action == actions::REMOVED {
            tx.execute(
                "UPDATE mutant SET removed_at = ?2, removed_run = ?3 WHERE id = ?1",
                params![id, at, run.0],
            )?;
        }
        let committed = settle(&tx, id, action, run, own)?;
        tx.commit()?;
        Ok(committed)
    }
}

/// The Run a change goes in, inside its transaction: an open one checked,
/// or one made now. Answers whether it was made.
fn take_run(tx: &rusqlite::Transaction, run: InRun, source: &Source) -> Result<(RunId, bool)> {
    match run {
        InRun::Open(run) => {
            let finished: Option<Option<i64>> = tx
                .query_row(
                    "SELECT finished_at FROM run WHERE id = ?1",
                    [run.0],
                    |row| row.get(0),
                )
                .optional()?;
            match finished {
                None => Err(SidecarError::NoRun(run)),
                Some(Some(_)) => Err(SidecarError::RunFinished(run)),
                Some(None) => Ok((run, false)),
            }
        }
        InRun::Own { description } => {
            tx.execute(
                "INSERT INTO run (source_kind, source_name, description, started_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![source.kind.as_str(), source.name, description, now()],
            )?;
            Ok((RunId(tx.last_insert_rowid()), true))
        }
    }
}

/// Ends a Run made for one change, and reads back what the change left,
/// still inside its transaction.
fn settle(
    tx: &rusqlite::Transaction,
    mutant: i64,
    action: &str,
    run: RunId,
    own: bool,
) -> Result<Committed> {
    if own {
        tx.execute(
            "UPDATE run SET finished_at = ?2, outcome = ?3 WHERE id = ?1",
            params![run.0, now(), format!("{action} Mutant {mutant}")],
        )?;
    }
    let read = tx.query_row(&select("WHERE mutant.id = ?1"), [mutant], read_mutant)?;
    let run = run_in(tx, run)?.ok_or(SidecarError::NoRun(run))?;
    Ok(Committed { mutant: read, run })
}

#[allow(
    clippy::too_many_arguments,
    reason = "one row's columns, written in one place"
)]
fn insert_change(
    tx: &rusqlite::Transaction,
    mutant: i64,
    action: &str,
    edits: &str,
    fingerprint: Option<&str>,
    source: &Source,
    run: RunId,
    at: i64,
) -> Result<()> {
    tx.execute(
        "INSERT INTO mutant_change
            (mutant_id, action, edits, fingerprint, source_kind, source_name, run_id, recorded_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            mutant,
            action,
            edits,
            fingerprint,
            source.kind.as_str(),
            source.name,
            run.0,
            at
        ],
    )?;
    Ok(())
}

/// The edits as stored: a JSON object and nothing else.
fn edits_text(edits: &Value) -> Result<String> {
    if !edits.is_object() {
        return Err(SidecarError::Invalid(format!(
            "a Mutant's edits are a JSON object, not {edits}"
        )));
    }
    Ok(edits.to_string())
}

fn stored_edits(text: &str) -> rusqlite::Result<Value> {
    let object: serde_json::Map<String, Value> = serde_json::from_str(text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(Value::Object(object))
}

/// A Mutant with its newest change beside it.
fn select(rest: &str) -> String {
    format!(
        "SELECT mutant.id, mutant.playlist_id, mutant.entry_id, mutant.track_id,
                mutant.fingerprint, mutant.created_at, mutant.removed_at,
                latest.edits, latest.source_kind, latest.source_name, latest.run_id,
                latest.recorded_at
         FROM mutant
         JOIN mutant_change AS latest ON latest.id =
              (SELECT max(id) FROM mutant_change WHERE mutant_id = mutant.id)
         {rest}"
    )
}

fn read_mutant(row: &Row) -> rusqlite::Result<Mutant> {
    Ok(Mutant {
        id: row.get(0)?,
        playlist_id: row.get(1)?,
        entry_id: row.get(2)?,
        track_id: row.get(3)?,
        fingerprint: row
            .get::<_, Option<String>>(4)?
            .map(Fingerprint::from_stored),
        created_at: row.get(5)?,
        removed_at: row.get(6)?,
        edits: stored_edits(&row.get::<_, String>(7)?)?,
        source: Source {
            kind: SourceKind::from_stored(&row.get::<_, String>(8)?),
            name: row.get(9)?,
        },
        run: RunId(row.get(10)?),
        changed_at: row.get(11)?,
    })
}

/// A borrowed, read-only view of the Mutant archive. It never opens a writer.
pub struct MutantReader<'a> {
    db: &'a rusqlite::Connection,
}
impl<'a> MutantReader<'a> {
    pub(crate) fn new(db: &'a rusqlite::Connection) -> Self {
        Self { db }
    }
    /// One Mutant by its identifier, removed ones included.
    pub fn mutant(&self, id: i64) -> Result<Option<Mutant>> {
        Ok(self
            .db
            .query_row(&select("WHERE mutant.id = ?1"), [id], read_mutant)
            .optional()?)
    }

    /// The Mutant a playlist entry has now, if it has one.
    pub fn mutant_of_entry(&self, entry_id: &str) -> Result<Option<Mutant>> {
        Ok(self
            .db
            .query_row(
                &select("WHERE mutant.entry_id = ?1 AND mutant.removed_at IS NULL"),
                [entry_id],
                read_mutant,
            )
            .optional()?)
    }

    /// The live Mutants of one playlist, or of every playlist, oldest
    /// first.
    pub fn mutants(&self, playlist_id: Option<&str>) -> Result<Vec<Mutant>> {
        self.live("mutant.playlist_id", playlist_id)
    }

    /// The live Mutants whose parent is this track, oldest first.
    pub fn mutants_of_track(&self, track_id: &str) -> Result<Vec<Mutant>> {
        self.live("mutant.track_id", Some(track_id))
    }

    /// Everything that has happened to a Mutant, oldest first.
    pub fn mutant_changes(&self, id: i64) -> Result<Vec<MutantChange>> {
        self.changes("WHERE mutant_id = ?1 ORDER BY id", id)
    }

    /// Every change to any Mutant made in a Run, oldest first.
    pub fn mutant_changes_in_run(&self, run: RunId) -> Result<Vec<MutantChange>> {
        self.changes("WHERE run_id = ?1 ORDER BY id", run.0)
    }

    fn live(&self, column: &str, value: Option<&str>) -> Result<Vec<Mutant>> {
        let db = self.db;
        let mutants = if let Some(value) = value {
            db.prepare(&select(&format!(
                "WHERE {column} = ?1 AND mutant.removed_at IS NULL ORDER BY mutant.id"
            )))?
            .query_map([value], read_mutant)?
            .collect::<rusqlite::Result<Vec<Mutant>>>()?
        } else {
            db.prepare(&select(
                "WHERE mutant.removed_at IS NULL ORDER BY mutant.id",
            ))?
            .query_map([], read_mutant)?
            .collect::<rusqlite::Result<Vec<Mutant>>>()?
        };
        Ok(mutants)
    }

    fn changes(&self, rest: &str, key: i64) -> Result<Vec<MutantChange>> {
        let db = self.db;
        let changes = db
            .prepare(&format!(
                "SELECT id, mutant_id, action, edits, fingerprint, source_kind, source_name,
                        run_id, recorded_at
                 FROM mutant_change {rest}"
            ))?
            .query_map([key], |row| {
                Ok(MutantChange {
                    id: row.get(0)?,
                    mutant_id: row.get(1)?,
                    action: row.get(2)?,
                    edits: stored_edits(&row.get::<_, String>(3)?)?,
                    fingerprint: row
                        .get::<_, Option<String>>(4)?
                        .map(Fingerprint::from_stored),
                    source: Source {
                        kind: SourceKind::from_stored(&row.get::<_, String>(5)?),
                        name: row.get(6)?,
                    },
                    run: RunId(row.get(7)?),
                    recorded_at: row.get(8)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<MutantChange>>>()?;
        Ok(changes)
    }
}
