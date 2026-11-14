//! Projection's barriers as the DJ sees them, and the DJ's clearing of one
//! (wbx-xme).
//!
//! A barrier ([`crate::projection::Barrier`]) is a write the Ledger cannot
//! account for: an `unconfirmed` entry (begun and never confirmed, written
//! by recovery), a `track` entry whose payload cannot be read, or an intent
//! still open (a write in progress). While one stands, Projection fills no
//! empty field it could concern, and replaces no value it wrote before it.
//!
//! [`Sidecar::barrier_list`] says, for each, which write it was (its Run,
//! source, label and command), when, which tracks it could concern (or that
//! it cannot say), and whether the DJ has cleared it. It is a read: it
//! writes nothing.
//!
//! [`Sidecar::clear_barrier`] is the DJ's deliberate act of clearing one:
//! one Ledger entry in the Sidecar's store, kind [`CLEARED_KIND`], in a Run
//! of the DJ's of its own, whose payload is the barrier as it was listed.
//! Nothing is removed or changed: the Ledger is append-only, and the
//! barrier stays listed, marked cleared. A write in progress cannot be
//! cleared, nor one already cleared. The caller decides who the DJ is; this
//! module refuses any other source.
//!
//! **What a cleared barrier allows:** an EMPTY field it concerned, which no
//! recorded write touched (or an Undo returned to before any), may be filled
//! again. Nothing else: a value a projection wrote before the barrier is
//! still not the projection's to replace (the cleared write may have been
//! made after it), and a value with no record (which the cleared write may
//! itself have left) stays the DJ's, as it always was.
use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ledger::{Action, NewEntry, Store};
use crate::{Result, RunId, Sidecar, Source, SourceKind};

/// The Ledger kind of the DJ's clearing of a barrier (Sidecar store).
pub const CLEARED_KIND: &str = "barrier_cleared";
/// What Undo-style listings call a clearing.
const CLEARED_LABEL: &str = "Clear a Projection barrier";

/// What a barrier is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BarrierKind {
    /// A write begun and never confirmed (recovered as `unconfirmed`).
    Unconfirmed,
    /// A recorded `track` edit whose payload cannot be read.
    Unreadable,
    /// An intent still open: a write in progress (or one the next service
    /// start will recover as unconfirmed).
    InProgress,
}

/// The DJ's clearing of a barrier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clearing {
    /// The Ledger entry recording it.
    pub entry: i64,
    /// Its Run.
    pub run: i64,
    /// When, in milliseconds since the Unix epoch.
    pub at: i64,
}

/// One barrier, as the DJ sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BarrierRecord {
    /// Its place in the Ledger's order: what to name to clear it.
    pub seq: i64,
    pub what: BarrierKind,
    /// When the write was begun or recorded (ms since the Unix epoch).
    pub at: i64,
    /// The write's Run, when it had one.
    pub run: Option<i64>,
    /// Who the write was by.
    pub source: Source,
    /// The write's label or description.
    pub label: String,
    /// The service command that made it (`projection.apply`), when known.
    pub command: Option<String>,
    /// The tracks it could concern; `None`: it cannot say, so every track.
    pub tracks: Option<Vec<String>>,
    /// The DJ's clearing, if they cleared it.
    pub cleared: Option<Clearing>,
}

/// Why a barrier cannot be cleared.
#[derive(Debug, Clone, PartialEq)]
pub enum NotCleared {
    /// No barrier is at that place in the Ledger.
    NotFound,
    /// A write still in progress.
    InProgress,
    /// Already cleared, by this entry.
    AlreadyCleared(i64),
    /// Only the DJ clears a barrier.
    NotTheDj,
}

/// The barrier each clearing entry names, by barrier: the clearing entry.
pub(crate) fn cleared(db: &Connection) -> Result<HashMap<i64, i64>> {
    let mut statement = db.prepare_cached(
        "SELECT json_extract(payload, '$.barrier.seq'), id FROM ledger
         WHERE store = 'sidecar' AND kind = ?1 ORDER BY id",
    )?;
    let rows = statement
        .query_map([CLEARED_KIND], |row| {
            Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut found = HashMap::new();
    for (seq, entry) in rows {
        if let Some(seq) = seq {
            found.entry(seq).or_insert(entry);
        }
    }
    Ok(found)
}

fn source_of(kind: &str, name: String) -> Source {
    Source::new(SourceKind::from_stored(kind), name)
}

fn list(db: &Connection) -> Result<Vec<BarrierRecord>> {
    let barriers = crate::projection::barriers(db)?;
    let mut out = Vec::with_capacity(barriers.len());
    for barrier in barriers {
        let entry = db
            .query_row(
                "SELECT kind, run_id, at, source_kind, source_name, label,
                        json_extract(command, '$.name')
                 FROM ledger WHERE id = ?1",
                [barrier.seq],
                |row| {
                    let kind: String = row.get(0)?;
                    Ok((
                        if kind == "unconfirmed" {
                            BarrierKind::Unconfirmed
                        } else {
                            BarrierKind::Unreadable
                        },
                        Some(row.get::<_, i64>(1)?),
                        row.get::<_, i64>(2)?,
                        source_of(&row.get::<_, String>(3)?, row.get(4)?),
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?,
                    ))
                },
            )
            .optional()?;
        let found = match entry {
            Some(found) => Some(found),
            None => db
                .query_row(
                    "SELECT run_id, at, source_kind, source_name, description,
                            json_extract(command, '$.name')
                     FROM ledger_intent WHERE id = ?1",
                    [barrier.seq],
                    |row| {
                        Ok((
                            BarrierKind::InProgress,
                            row.get::<_, Option<i64>>(0)?,
                            row.get::<_, i64>(1)?,
                            source_of(&row.get::<_, String>(2)?, row.get(3)?),
                            row.get::<_, String>(4)?,
                            row.get::<_, Option<String>>(5)?,
                        ))
                    },
                )
                .optional()?,
        };
        let Some((what, run, at, source, label, command)) = found else {
            continue;
        };
        let cleared = match barrier.cleared {
            Some(entry) => db
                .query_row(
                    "SELECT run_id, at FROM ledger WHERE id = ?1",
                    [entry],
                    |row| {
                        Ok(Clearing {
                            entry,
                            run: row.get(0)?,
                            at: row.get(1)?,
                        })
                    },
                )
                .optional()?,
            None => None,
        };
        out.push(BarrierRecord {
            seq: barrier.seq,
            what,
            at,
            run,
            source,
            label,
            command,
            tracks: barrier.tracks,
            cleared,
        });
    }
    Ok(out)
}

impl Sidecar {
    /// Every barrier, oldest first, cleared ones included and marked.
    /// Writes nothing.
    pub fn barrier_list(&self) -> Result<Vec<BarrierRecord>> {
        list(&self.db())
    }

    /// The DJ's clearing of the barrier at `seq`, recorded as one Ledger
    /// entry in a Run of `source`'s own, with `command` (the request that
    /// asked) and the barrier as it was listed. Refused, with nothing
    /// written, unless `source` is the DJ and `seq` is a barrier not in
    /// progress and not already cleared. Checked and written in one
    /// transaction.
    ///
    /// # Errors
    /// The Sidecar could not be read or written; nothing is kept.
    pub fn clear_barrier(
        &self,
        seq: i64,
        source: &Source,
        command: &Value,
    ) -> Result<std::result::Result<Clearing, NotCleared>> {
        if source.kind != SourceKind::Dj {
            return Ok(Err(NotCleared::NotTheDj));
        }
        let mut db = self.db();
        let tx = db.transaction()?;
        let Some(barrier) = list(&tx)?.into_iter().find(|b| b.seq == seq) else {
            return Ok(Err(NotCleared::NotFound));
        };
        if let Some(cleared) = &barrier.cleared {
            return Ok(Err(NotCleared::AlreadyCleared(cleared.entry)));
        }
        if barrier.what == BarrierKind::InProgress {
            return Ok(Err(NotCleared::InProgress));
        }
        let at = crate::store::now();
        tx.execute(
            "INSERT INTO run (source_kind, source_name, description, started_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                source.kind.as_str(),
                source.name,
                format!("Clear Projection barrier {seq}"),
                at
            ],
        )?;
        let run = RunId(tx.last_insert_rowid());
        let payload = json!({
            "barrier": barrier,
            "allows": "empty fields it concerned may be filled again; nothing becomes a projection's",
        });
        let entry = crate::ledger::append_in(
            &tx,
            &NewEntry {
                id: None,
                run,
                store: Store::Sidecar,
                source,
                at: Some(at),
                action: Action::Edit,
                kind: CLEARED_KIND,
                label: CLEARED_LABEL,
                command: Some(command),
                reverses: None,
                part_of: None,
                reversible: false,
                format: 1,
                payload: Some(&payload),
            },
        )?;
        tx.execute(
            "UPDATE run SET finished_at = ?2, outcome = ?3 WHERE id = ?1",
            params![run.0, at, "cleared 1 barrier"],
        )?;
        tx.commit()?;
        Ok(Ok(Clearing {
            entry,
            run: run.0,
            at,
        }))
    }
}

impl crate::FactsReader {
    /// [`Sidecar::barrier_list`] on a read-only snapshot.
    pub fn barrier_list(&self) -> Result<Vec<BarrierRecord>> {
        list(self.connection())
    }
}
