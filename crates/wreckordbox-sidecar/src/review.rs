//! The DJ's rejection of waiting Facts, and the DJ's reopening of one
//! (wbx-27).
//!
//! A waiting Fact is a standing Fact Projection held back for the DJ. The
//! DJ rejects the SUGGESTION it makes, not the one Fact: the same track,
//! field, canonical value (Energy's stars, a Prep colour's id) and source
//! ([`Suggestion`]). The same answer recorded again by the same source is a
//! new Fact but the same suggestion, and stays rejected; a different value
//! from that source, or the same value from another, is a new suggestion.
//!
//! [`Sidecar::reject`] records a rejection as ONE Ledger entry in the
//! Sidecar's store, kind [`REJECTED_KIND`], in a Run of the DJ's own made
//! and finished in the same transaction; its payload names each suggestion
//! as it was offered (`{"suggestions": [{fact, track, field, value,
//! canonical, source, confidence}]}`). [`Sidecar::reopen`] is the DJ's
//! reversal, kind [`REOPENED_KIND`], the same shape. Nothing is ever
//! removed: the Ledger is append-only, and which suggestions stand rejected
//! is the replay of both kinds in Ledger order. Neither touches the
//! Library.
use std::collections::{BTreeMap, HashSet};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ledger::{Action, NewEntry, Store};
use crate::{Result, RunId, Sidecar, Source, SourceKind};

/// The Ledger kind of the DJ's rejection of waiting Facts (Sidecar store).
pub const REJECTED_KIND: &str = "facts_rejected";
/// The Ledger kind of the DJ's reopening of rejected suggestions.
pub const REOPENED_KIND: &str = "facts_reopened";

/// What a rejection is of: a track, a field, a canonical value and who
/// suggests it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Suggestion {
    pub track: String,
    pub field: String,
    /// Energy's stars, or a Prep colour's id.
    pub canonical: i64,
    /// The source's kind (`agent`, `tool`, ...) and name.
    pub source: (String, String),
}

impl Suggestion {
    #[must_use]
    pub fn new(track: &str, field: &str, canonical: i64, source: &Source) -> Self {
        Self {
            track: track.to_owned(),
            field: field.to_owned(),
            canonical,
            source: (source.kind.as_str().to_owned(), source.name.clone()),
        }
    }
}

/// One suggestion as the DJ decided on it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Offered {
    /// The Fact that made it, when decided.
    pub fact: i64,
    pub track: String,
    pub field: String,
    /// As the review showed it.
    pub value: Value,
    pub canonical: i64,
    pub source: Source,
    pub confidence: f64,
}

impl Offered {
    #[must_use]
    pub fn suggestion(&self) -> Suggestion {
        Suggestion::new(&self.track, &self.field, self.canonical, &self.source)
    }
}

/// A Ledger entry recording the DJ's decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub entry: i64,
    /// Its Run, the DJ's.
    pub run: i64,
    /// When, in milliseconds since the Unix epoch.
    pub at: i64,
}

/// A suggestion that stands rejected, with the rejection that made it so.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rejected {
    #[serde(flatten)]
    pub offered: Offered,
    pub rejection: Decision,
}

/// Why a decision was not recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotRecorded {
    /// The source is not the DJ.
    NotTheDj,
    /// Nothing to decide.
    Nothing,
}

/// Every suggestion standing rejected, in the order rejected.
fn standing(db: &Connection) -> Result<Vec<Rejected>> {
    let mut statement = db.prepare(
        "SELECT id, run_id, at, kind, payload FROM ledger
         WHERE store = 'sidecar' AND kind IN (?1, ?2) ORDER BY id",
    )?;
    let rows = statement.query_map([REJECTED_KIND, REOPENED_KIND], |row| {
        Ok((
            Decision {
                entry: row.get(0)?,
                run: row.get(1)?,
                at: row.get(2)?,
            },
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;
    let mut rejected: BTreeMap<Suggestion, Rejected> = BTreeMap::new();
    let mut order: Vec<Suggestion> = Vec::new();
    for row in rows {
        let (decision, kind, payload) = row?;
        let offered: Vec<Offered> = payload
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|payload| serde_json::from_value(payload["suggestions"].clone()).ok())
            .unwrap_or_default();
        for one in offered {
            let key = one.suggestion();
            if kind == REJECTED_KIND {
                if !rejected.contains_key(&key) {
                    order.push(key.clone());
                }
                rejected.insert(
                    key,
                    Rejected {
                        offered: one,
                        rejection: decision.clone(),
                    },
                );
            } else {
                rejected.remove(&key);
            }
        }
    }
    Ok(order
        .into_iter()
        .filter_map(|key| rejected.remove(&key))
        .collect())
}

impl Sidecar {
    /// Every suggestion standing rejected. Writes nothing.
    pub fn rejections(&self) -> Result<Vec<Rejected>> {
        standing(&self.db())
    }

    /// The suggestions standing rejected, for checking. Writes nothing.
    pub fn rejected_suggestions(&self) -> Result<HashSet<Suggestion>> {
        Ok(self
            .rejections()?
            .into_iter()
            .map(|r| r.offered.suggestion())
            .collect())
    }

    /// The DJ's rejection of `offered`, one Ledger entry in a Run of
    /// `source`'s own, with `command` (the request that asked). Refused,
    /// with nothing written, unless `source` is the DJ and there is
    /// something to reject.
    ///
    /// # Errors
    /// The Sidecar could not be written; nothing is kept.
    pub fn reject(
        &self,
        offered: &[Offered],
        source: &Source,
        command: &Value,
    ) -> Result<std::result::Result<Decision, NotRecorded>> {
        self.decide(
            REJECTED_KIND,
            "Reject waiting Facts",
            offered,
            source,
            command,
        )
    }

    /// The DJ's reopening of rejected suggestions: they may be offered
    /// again. The rejections stay in the Ledger as they were.
    ///
    /// # Errors
    /// The Sidecar could not be written; nothing is kept.
    pub fn reopen(
        &self,
        offered: &[Offered],
        source: &Source,
        command: &Value,
    ) -> Result<std::result::Result<Decision, NotRecorded>> {
        self.decide(
            REOPENED_KIND,
            "Reopen rejected suggestions",
            offered,
            source,
            command,
        )
    }

    fn decide(
        &self,
        kind: &str,
        label: &str,
        offered: &[Offered],
        source: &Source,
        command: &Value,
    ) -> Result<std::result::Result<Decision, NotRecorded>> {
        if source.kind != SourceKind::Dj {
            return Ok(Err(NotRecorded::NotTheDj));
        }
        if offered.is_empty() {
            return Ok(Err(NotRecorded::Nothing));
        }
        let mut db = self.db();
        let tx = db.transaction()?;
        let at = crate::store::now();
        let count = offered.len();
        tx.execute(
            "INSERT INTO run (source_kind, source_name, description, started_at) VALUES (?1, ?2, ?3, ?4)",
            params![source.kind.as_str(), source.name, format!("{label} ({count})"), at],
        )?;
        let run = RunId(tx.last_insert_rowid());
        let payload = json!({ "suggestions": offered });
        let entry = crate::ledger::append_in(
            &tx,
            &NewEntry {
                id: None,
                run,
                store: Store::Sidecar,
                source,
                at: Some(at),
                action: Action::Edit,
                kind,
                label,
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
            params![run.0, at, format!("{count} suggestion(s)")],
        )?;
        tx.commit()?;
        Ok(Ok(Decision {
            entry,
            run: run.0,
            at,
        }))
    }
}

impl crate::FactsReader {
    /// [`Sidecar::rejections`] on a read-only snapshot.
    pub fn rejections(&self) -> Result<Vec<Rejected>> {
        standing(self.connection())
    }

    /// [`Sidecar::rejected_suggestions`] on a read-only snapshot.
    pub fn rejected_suggestions(&self) -> Result<HashSet<Suggestion>> {
        Ok(self
            .rejections()?
            .into_iter()
            .map(|r| r.offered.suggestion())
            .collect())
    }
}
