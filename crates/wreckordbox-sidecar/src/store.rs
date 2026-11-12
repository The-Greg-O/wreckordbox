//! The Sidecar's database: Runs, kinds and Facts.
//!
//! ```text
//! meta(key, value)                      schema_version
//! kind(name, nature)                    each kind, declared once
//! run(id, source_kind, source_name, description, started_at, finished_at, outcome)
//! fact(id, track_id, fingerprint, kind, value, source_kind, source_name,
//!      confidence, run_id, recorded_at, stale_at, superseded_at, superseded_by)
//! ```
//!
//! Times are milliseconds since the Unix epoch, UTC. `value` is JSON. A
//! Fact row is written once; afterwards only `stale_at` and the two
//! `superseded_*` columns are ever set, each once.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{io, Fingerprint, Result, SidecarError};

/// Raised when a stored shape changes. A store from a newer wreckordbox is
/// refused rather than guessed at; an older one is brought up to date by
/// [`MIGRATIONS`] when it is opened.
pub const SCHEMA_VERSION: u32 = 2;

/// What each version after the first adds, in order: the version a store is
/// at once the statements have run, and the statements. A new store is made
/// at version 1 by [`SCHEMA`] and then taken through every one of these, so
/// a new store and an old one brought up to date hold the same tables. Add
/// a version at the end; never change one that has shipped.
const MIGRATIONS: &[(u32, &str)] = &[(2, crate::ledger::SCHEMA)];

const DATABASE_FILE: &str = "sidecar.db";
/// Where a tool writes before its result is accepted. Emptied on open.
const WORK_FOLDER: &str = "work";

const SCHEMA: &str = "
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;

CREATE TABLE kind (
    name   TEXT PRIMARY KEY,
    nature TEXT NOT NULL CHECK (nature IN ('judgement', 'measurement'))
) STRICT;

CREATE TABLE run (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('dj', 'agent', 'tool', 'web')),
    source_name TEXT NOT NULL,
    description TEXT NOT NULL,
    started_at  INTEGER NOT NULL,
    finished_at INTEGER,
    outcome     TEXT
) STRICT;

CREATE TABLE fact (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id      TEXT NOT NULL,
    fingerprint   TEXT,
    kind          TEXT NOT NULL REFERENCES kind (name),
    value         TEXT NOT NULL,
    source_kind   TEXT NOT NULL CHECK (source_kind IN ('dj', 'agent', 'tool', 'web')),
    source_name   TEXT NOT NULL,
    confidence    REAL NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
    run_id        INTEGER NOT NULL REFERENCES run (id),
    recorded_at   INTEGER NOT NULL,
    stale_at      INTEGER,
    superseded_at INTEGER,
    superseded_by INTEGER REFERENCES fact (id)
) STRICT;

CREATE INDEX fact_by_track ON fact (track_id, kind);
CREATE INDEX fact_by_fingerprint ON fact (fingerprint, kind);
CREATE INDEX fact_by_run ON fact (run_id);
";

/// Whether a kind of Fact follows the music or is tied to the exact audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Nature {
    /// About the music itself (genre, mood, Energy). It follows the track
    /// when its audio file is replaced, moved or re-added, so a change of
    /// fingerprint does not make it stale.
    Judgement,
    /// Computed from one audio file's samples (Stems, Intensity). Stale
    /// once that file's audio changes.
    Measurement,
}

impl Nature {
    fn as_str(self) -> &'static str {
        match self {
            Self::Judgement => "judgement",
            Self::Measurement => "measurement",
        }
    }

    fn from_stored(text: &str) -> Self {
        if text == "judgement" {
            Self::Judgement
        } else {
            Self::Measurement
        }
    }
}

impl std::fmt::Display for Nature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Judgement => "Judgement",
            Self::Measurement => "Measurement",
        })
    }
}

/// A kind of Fact and how it is declared. Declare one as a constant beside
/// the code that records it (`stems::STEMS` is the pattern), or build one
/// from a name known only at run time. The first Fact of a kind writes the
/// declaration into the store; recording the same name the other way is
/// refused from then on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kind<'a> {
    pub name: &'a str,
    pub nature: Nature,
}

impl<'a> Kind<'a> {
    #[must_use]
    pub const fn judgement(name: &'a str) -> Self {
        Self {
            name,
            nature: Nature::Judgement,
        }
    }

    #[must_use]
    pub const fn measurement(name: &'a str) -> Self {
        Self {
            name,
            nature: Nature::Measurement,
        }
    }
}

/// Who a Fact, or a Run, came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// The DJ, by hand. Outranks every other source.
    Dj,
    Agent,
    Tool,
    /// A web lookup.
    Web,
}

impl SourceKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Dj => "dj",
            Self::Agent => "agent",
            Self::Tool => "tool",
            Self::Web => "web",
        }
    }

    pub(crate) fn from_stored(text: &str) -> Self {
        match text {
            "dj" => Self::Dj,
            "agent" => Self::Agent,
            "web" => Self::Web,
            _ => Self::Tool,
        }
    }
}

/// A source: what kind of thing it is and which one, for example the tool
/// `separation-engine`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub kind: SourceKind,
    pub name: String,
}

impl Source {
    #[must_use]
    pub fn new(kind: SourceKind, name: impl Into<String>) -> Self {
        Self {
            kind,
            name: name.into(),
        }
    }
}

/// One Run in this Sidecar: a piece of work whose Facts belong together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunId(pub i64);

/// A Run as the Sidecar holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub id: RunId,
    /// Who the work is by.
    pub source: Source,
    /// What the work is, for a person.
    pub description: String,
    /// Milliseconds since the Unix epoch.
    pub started_at: i64,
    /// Set once the Run has ended.
    pub finished_at: Option<i64>,
    /// How it went, in a word or a sentence. Set with `finished_at`.
    pub outcome: Option<String>,
}

impl Run {
    /// Whether the Run has ended: nothing more is recorded in it.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.finished_at.is_some()
    }
}

/// A Fact to record.
#[derive(Debug, Clone)]
pub struct NewFact<'a> {
    /// The track's identifier in the Library.
    pub track_id: &'a str,
    /// The fingerprint of the track's audio as it is now. A Measurement
    /// must have one; a Judgement may be recorded without (a track whose
    /// file is not at hand).
    pub fingerprint: Option<&'a Fingerprint>,
    pub kind: Kind<'a>,
    /// Whatever the kind keeps, as JSON.
    pub value: Value,
    pub source: &'a Source,
    /// 0 to 1.
    pub confidence: f64,
    /// The Run this belongs to, from [`Sidecar::begin_run`].
    pub run: RunId,
}

/// A Fact as the Sidecar holds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    pub id: i64,
    pub track_id: String,
    /// The fingerprint of the track's audio when this was recorded.
    pub fingerprint: Option<Fingerprint>,
    pub kind: String,
    pub nature: Nature,
    pub value: Value,
    pub source: Source,
    pub confidence: f64,
    pub run: RunId,
    /// Milliseconds since the Unix epoch.
    pub recorded_at: i64,
    /// Set when a Measurement's audio was found to have changed.
    pub stale_at: Option<i64>,
    /// Set when a later Fact of the same kind from the same source took
    /// this one's place; `superseded_by` is that Fact.
    pub superseded_at: Option<i64>,
    pub superseded_by: Option<i64>,
}

impl Fact {
    /// Whether this is still what its source says: not superseded.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.superseded_at.is_none()
    }

    #[must_use]
    pub fn is_stale(&self) -> bool {
        self.stale_at.is_some()
    }
}

/// An open Sidecar. One process owns it: the Library service.
pub struct Sidecar {
    dir: PathBuf,
    db: Mutex<Connection>,
}

impl std::fmt::Debug for Sidecar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sidecar")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

impl Sidecar {
    /// Opens the Sidecar in `dir`, making the folder and an empty store
    /// when there is none. Anything a tool left half-written in the work
    /// folder by a service that died is cleared.
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir).map_err(io("could not make the Sidecar folder", dir))?;
        let dir = dir
            .canonicalize()
            .map_err(io("could not find the Sidecar folder", dir))?;
        let work = dir.join(WORK_FOLDER);
        if work.exists() {
            std::fs::remove_dir_all(&work).map_err(io("could not clear", &work))?;
        }
        let path = dir.join(DATABASE_FILE);
        let mut db = Connection::open(&path)?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "foreign_keys", "ON")?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        let has_meta: bool = db.query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'meta')",
            [],
            |row| row.get(0),
        )?;
        let found: u32 = if has_meta {
            db.query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .and_then(|version| version.parse().ok())
            .unwrap_or(0)
        } else {
            0
        };
        if found > SCHEMA_VERSION {
            return Err(SidecarError::Newer {
                path,
                found,
                supported: SCHEMA_VERSION,
            });
        }
        if found < SCHEMA_VERSION {
            // All of it or none: a store is never left between versions.
            let tx = db.transaction()?;
            if !has_meta {
                tx.execute_batch(SCHEMA)?;
            }
            for (version, statements) in MIGRATIONS {
                if *version > found {
                    tx.execute_batch(statements)?;
                }
            }
            tx.execute(
                "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                [SCHEMA_VERSION.to_string()],
            )?;
            tx.commit()?;
        }
        Ok(Self {
            dir,
            db: Mutex::new(db),
        })
    }

    /// The Sidecar's folder.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Starts a Run: one piece of work by `source`, described for a person.
    pub fn begin_run(&self, source: &Source, description: &str) -> Result<RunId> {
        let db = self.db();
        db.execute(
            "INSERT INTO run (source_kind, source_name, description, started_at) VALUES (?1, ?2, ?3, ?4)",
            params![source.kind.as_str(), source.name, description, now()],
        )?;
        Ok(RunId(db.last_insert_rowid()))
    }

    /// Ends a Run, saying in a word or a sentence how it went.
    pub fn finish_run(&self, run: RunId, outcome: &str) -> Result<()> {
        self.db().execute(
            "UPDATE run SET finished_at = ?2, outcome = ?3 WHERE id = ?1",
            params![run.0, now(), outcome],
        )?;
        Ok(())
    }

    /// One Run by its identifier.
    pub fn run(&self, run: RunId) -> Result<Option<Run>> {
        Ok(self
            .db()
            .query_row(
                "SELECT id, source_kind, source_name, description, started_at, finished_at, outcome
                 FROM run WHERE id = ?1",
                [run.0],
                |row| {
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
                },
            )
            .optional()?)
    }

    /// An empty folder for `run` to write into before its result is
    /// accepted. Inside the Sidecar, so accepting a file is a rename.
    pub fn workspace(&self, run: RunId) -> Result<PathBuf> {
        let dir = self.dir.join(WORK_FOLDER).join(format!("run-{}", run.0));
        std::fs::create_dir_all(&dir).map_err(io("could not make", &dir))?;
        Ok(dir)
    }

    /// Removes a Run's workspace and whatever is left in it.
    pub fn discard_workspace(&self, run: RunId) {
        let _ = std::fs::remove_dir_all(self.dir.join(WORK_FOLDER).join(format!("run-{}", run.0)));
    }

    /// Records a Fact. In one step: the kind is declared if it is new (and
    /// refused if it was declared the other way), the Fact is written, and
    /// any earlier Fact of the same kind for the same track from the same
    /// source is marked superseded by it. Facts from other sources stand
    /// beside it untouched.
    pub fn record(&self, new: &NewFact) -> Result<Fact> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let fact = record_in(&tx, new)?;
        tx.commit()?;
        Ok(fact)
    }

    /// Records several Facts as [`record`](Self::record) records one, all
    /// of them or, if any is refused, none. They are answered in the order
    /// given.
    pub fn record_many(&self, new: &[NewFact]) -> Result<Vec<Fact>> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let facts = new
            .iter()
            .map(|new| record_in(&tx, new))
            .collect::<Result<Vec<Fact>>>()?;
        tx.commit()?;
        Ok(facts)
    }

    /// Every Fact of `kind` ever recorded for the track, newest first,
    /// superseded and stale ones included. Use [`Fact::is_live`] and
    /// [`Fact::is_stale`] to tell them apart.
    pub fn facts(&self, track_id: &str, kind: &str) -> Result<Vec<Fact>> {
        let db = self.db();
        let mut statement = db.prepare(&select(
            "WHERE fact.track_id = ?1 AND fact.kind = ?2 ORDER BY fact.id DESC",
        ))?;
        let facts = statement
            .query_map(params![track_id, kind], read_fact)?
            .collect::<rusqlite::Result<Vec<Fact>>>()?;
        Ok(facts)
    }

    /// Every Fact of every kind recorded for the track, newest first.
    pub fn facts_of_track(&self, track_id: &str) -> Result<Vec<Fact>> {
        let db = self.db();
        let mut statement =
            db.prepare(&select("WHERE fact.track_id = ?1 ORDER BY fact.id DESC"))?;
        let facts = statement
            .query_map([track_id], read_fact)?
            .collect::<rusqlite::Result<Vec<Fact>>>()?;
        Ok(facts)
    }

    /// One Fact by its identifier.
    pub fn fact(&self, id: i64) -> Result<Option<Fact>> {
        Ok(self
            .db()
            .query_row(&select("WHERE fact.id = ?1"), [id], read_fact)
            .optional()?)
    }

    /// Says the track's audio is now `current`: every Measurement recorded
    /// for the track from different audio is marked stale, and their
    /// identifiers are answered. Judgements are left alone; they follow the
    /// music. A stale Fact is kept, not removed.
    pub fn mark_stale(&self, track_id: &str, current: &Fingerprint) -> Result<Vec<i64>> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let mut ids = {
            let mut statement = tx.prepare(
                "UPDATE fact SET stale_at = ?1
                 WHERE track_id = ?2 AND stale_at IS NULL
                   AND fingerprint IS NOT NULL AND fingerprint <> ?3
                   AND kind IN (SELECT name FROM kind WHERE nature = 'measurement')
                 RETURNING id",
            )?;
            let ids = statement
                .query_map(params![now(), track_id, current.as_str()], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<i64>>>()?;
            ids
        };
        ids.sort_unstable();
        if !ids.is_empty() {
            // The Ledger's entry for it, in a Run of wreckordbox's own:
            // marked and recorded together or not at all.
            let source = Source::new(SourceKind::Tool, crate::ledger::HOUSEKEEPING);
            let at = now();
            tx.execute(
                "INSERT INTO run (source_kind, source_name, description, started_at, finished_at, outcome)
                 VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
                params![
                    source.kind.as_str(),
                    source.name,
                    format!("The audio of track {track_id} changed"),
                    at,
                    format!("marked {} stale", ids.len()),
                ],
            )?;
            let run = RunId(tx.last_insert_rowid());
            let payload = serde_json::json!({
                "track": track_id,
                "fingerprint": current.as_str(),
                "facts": ids,
            });
            crate::ledger::append_in(
                &tx,
                &crate::ledger::NewEntry {
                    run,
                    store: crate::ledger::Store::Sidecar,
                    source: &source,
                    at: Some(at),
                    action: crate::ledger::Action::Edit,
                    kind: crate::ledger::STALE,
                    label: "Mark Measurements stale",
                    command: None,
                    reverses: None,
                    reversible: true,
                    format: 1,
                    payload: Some(&payload),
                },
            )?;
        }
        tx.commit()?;
        Ok(ids)
    }

    pub(crate) fn db(&self) -> MutexGuard<'_, Connection> {
        // Nothing is left half-done under this lock: a write is one
        // statement or one transaction.
        self.db.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One Fact, inside a transaction: declares the kind, writes the Fact and
/// supersedes the same source's earlier ones.
fn record_in(tx: &rusqlite::Transaction, new: &NewFact) -> Result<Fact> {
    if !(0.0..=1.0).contains(&new.confidence) {
        return Err(SidecarError::Invalid(format!(
            "a confidence is from 0 to 1, not {}",
            new.confidence
        )));
    }
    if new.kind.name.is_empty() || new.track_id.is_empty() {
        return Err(SidecarError::Invalid(
            "a Fact needs a kind and a track".to_owned(),
        ));
    }
    if new.kind.nature == Nature::Measurement && new.fingerprint.is_none() {
        return Err(SidecarError::Invalid(format!(
            "{} is a Measurement, so it needs the fingerprint of the audio it was made from",
            new.kind.name
        )));
    }
    let declared: Option<String> = tx
        .query_row(
            "SELECT nature FROM kind WHERE name = ?1",
            [new.kind.name],
            |row| row.get(0),
        )
        .optional()?;
    match declared.as_deref().map(Nature::from_stored) {
        None => {
            tx.execute(
                "INSERT INTO kind (name, nature) VALUES (?1, ?2)",
                params![new.kind.name, new.kind.nature.as_str()],
            )?;
        }
        Some(declared) if declared != new.kind.nature => {
            return Err(SidecarError::KindConflict {
                kind: new.kind.name.to_owned(),
                declared,
                asked: new.kind.nature,
            });
        }
        Some(_) => {}
    }
    let at = now();
    tx.execute(
        "INSERT INTO fact (track_id, fingerprint, kind, value, source_kind, source_name, confidence, run_id, recorded_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            new.track_id,
            new.fingerprint.map(Fingerprint::as_str),
            new.kind.name,
            new.value.to_string(),
            new.source.kind.as_str(),
            new.source.name,
            new.confidence,
            new.run.0,
            at,
        ],
    )?;
    let id = tx.last_insert_rowid();
    tx.execute(
        "UPDATE fact SET superseded_at = ?1, superseded_by = ?2
         WHERE track_id = ?3 AND kind = ?4 AND source_kind = ?5 AND source_name = ?6
           AND superseded_at IS NULL AND id <> ?2",
        params![
            at,
            id,
            new.track_id,
            new.kind.name,
            new.source.kind.as_str(),
            new.source.name
        ],
    )?;
    let fact = tx.query_row(&select("WHERE fact.id = ?1"), [id], read_fact)?;
    Ok(fact)
}

pub(crate) fn select(rest: &str) -> String {
    format!(
        "SELECT fact.id, fact.track_id, fact.fingerprint, fact.kind, kind.nature, fact.value,
                fact.source_kind, fact.source_name, fact.confidence, fact.run_id, fact.recorded_at,
                fact.stale_at, fact.superseded_at, fact.superseded_by
         FROM fact JOIN kind ON kind.name = fact.kind {rest}"
    )
}

pub(crate) fn read_fact(row: &Row) -> rusqlite::Result<Fact> {
    let value: String = row.get(5)?;
    Ok(Fact {
        id: row.get(0)?,
        track_id: row.get(1)?,
        fingerprint: row
            .get::<_, Option<String>>(2)?
            .map(Fingerprint::from_stored),
        kind: row.get(3)?,
        nature: Nature::from_stored(&row.get::<_, String>(4)?),
        value: serde_json::from_str(&value).unwrap_or(Value::Null),
        source: Source {
            kind: SourceKind::from_stored(&row.get::<_, String>(6)?),
            name: row.get(7)?,
        },
        confidence: row.get(8)?,
        run: RunId(row.get(9)?),
        recorded_at: row.get(10)?,
        stale_at: row.get(11)?,
        superseded_at: row.get(12)?,
        superseded_by: row.get(13)?,
    })
}

pub(crate) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|since| i64::try_from(since.as_millis()).ok())
        .unwrap_or(0)
}
