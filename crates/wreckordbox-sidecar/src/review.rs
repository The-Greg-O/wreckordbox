//! The DJ's rejection of waiting Facts (wbx-27).
//!
//! A waiting Fact is a standing Fact Projection held back for the DJ. The
//! DJ may reject it: [`Sidecar::reject_facts`] records that as ONE Ledger
//! entry in the Sidecar's store, kind [`REJECTED_KIND`], in a Run of the
//! DJ's own made and finished in the same transaction, whose payload names
//! each rejected Fact as it was offered (`{"facts": [{fact, track, field,
//! value, source, confidence}, ...]}`). Nothing is removed or changed: the
//! Fact stays recorded, the Ledger is append-only, and the Library is not
//! touched. [`Sidecar::rejected_facts`] reads every Fact so rejected; a
//! rejected Fact is never offered again, and never projected.
use std::collections::HashSet;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ledger::{Action, NewEntry, Store};
use crate::{Result, RunId, Sidecar, Source, SourceKind};

/// The Ledger kind of the DJ's rejection of waiting Facts (Sidecar store).
pub const REJECTED_KIND: &str = "facts_rejected";
const REJECTED_LABEL: &str = "Reject waiting Facts";

/// The DJ's rejection, as recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rejection {
    /// The Ledger entry recording it.
    pub entry: i64,
    /// Its Run, the DJ's.
    pub run: i64,
    /// When, in milliseconds since the Unix epoch.
    pub at: i64,
}

/// Why a rejection was not recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotRejected {
    /// The source is not the DJ.
    NotTheDj,
    /// Nothing to reject.
    Nothing,
}

fn rejected(db: &Connection) -> Result<HashSet<i64>> {
    let mut statement = db.prepare(
        "SELECT json_extract(each.value, '$.fact')
         FROM ledger, json_each(ledger.payload, '$.facts') AS each
         WHERE ledger.store = 'sidecar' AND ledger.kind = ?1
           AND json_valid(ledger.payload)",
    )?;
    let ids = statement
        .query_map([REJECTED_KIND], |row| row.get::<_, Option<i64>>(0))?
        .filter_map(|id| id.transpose())
        .collect::<rusqlite::Result<HashSet<i64>>>()?;
    Ok(ids)
}

impl Sidecar {
    /// Every Fact the DJ has rejected. Writes nothing.
    pub fn rejected_facts(&self) -> Result<HashSet<i64>> {
        rejected(&self.db())
    }

    /// The DJ's rejection of `facts` (each as it was offered: at least its
    /// `fact` id), recorded as one Ledger entry in a Run of `source`'s own,
    /// with `command` (the request that asked). Refused, with nothing
    /// written, unless `source` is the DJ and there is something to reject.
    ///
    /// # Errors
    /// The Sidecar could not be written; nothing is kept.
    pub fn reject_facts(
        &self,
        facts: &[Value],
        source: &Source,
        command: &Value,
    ) -> Result<std::result::Result<Rejection, NotRejected>> {
        if source.kind != SourceKind::Dj {
            return Ok(Err(NotRejected::NotTheDj));
        }
        if facts.is_empty() {
            return Ok(Err(NotRejected::Nothing));
        }
        let mut db = self.db();
        let tx = db.transaction()?;
        let at = crate::store::now();
        let count = facts.len();
        tx.execute(
            "INSERT INTO run (source_kind, source_name, description, started_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                source.kind.as_str(),
                source.name,
                format!("Reject {count} waiting Fact(s)"),
                at
            ],
        )?;
        let run = RunId(tx.last_insert_rowid());
        let payload = json!({ "facts": facts });
        let entry = crate::ledger::append_in(
            &tx,
            &NewEntry {
                id: None,
                run,
                store: Store::Sidecar,
                source,
                at: Some(at),
                action: Action::Edit,
                kind: REJECTED_KIND,
                label: REJECTED_LABEL,
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
            params![run.0, at, format!("rejected {count} Fact(s)")],
        )?;
        tx.commit()?;
        Ok(Ok(Rejection {
            entry,
            run: run.0,
            at,
        }))
    }
}

impl crate::FactsReader {
    /// [`Sidecar::rejected_facts`] on a read-only snapshot.
    pub fn rejected_facts(&self) -> Result<HashSet<i64>> {
        rejected(self.connection())
    }
}
