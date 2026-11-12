//! What Projection (wbx-26) reads from the Sidecar: every Fact of one kind
//! at once, and the native values earlier Projections wrote, from the
//! Ledger. Both are one query each, whatever the size of the Library, so a
//! projection never asks the store once per track. Nothing here writes.
use std::collections::HashMap;

use rusqlite::{params, Connection};
use serde_json::Value;

use crate::store::{read_fact, select};
use crate::{Fact, FactsReader, Result, Sidecar};

/// Native values the Ledger says a command wrote, keyed by
/// `(track, column)`: the `after` of the latest such entry for each, as the
/// Ledger stores values (`null`, a whole number, text).
pub type Projected = HashMap<(String, String), Value>;

fn facts_of_kind(db: &Connection, kind: &str) -> Result<Vec<Fact>> {
    let mut statement = db.prepare_cached(&select("WHERE fact.kind = ?1 ORDER BY fact.id DESC"))?;
    let facts = statement
        .query_map(params![kind], read_fact)?
        .collect::<rusqlite::Result<Vec<Fact>>>()?;
    Ok(facts)
}

fn projected_by(db: &Connection, command: &str) -> Result<Projected> {
    let mut statement = db.prepare_cached(
        "SELECT payload FROM ledger
         WHERE store = 'library' AND kind = 'track' AND action = 'edit'
           AND payload IS NOT NULL AND json_extract(command, '$.name') = ?1
         ORDER BY id",
    )?;
    let payloads = statement
        .query_map(params![command], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    let mut projected = Projected::new();
    for payload in payloads {
        let Ok(payload) = serde_json::from_str::<Value>(&payload) else {
            continue;
        };
        for edit in payload["edits"].as_array().into_iter().flatten() {
            if let (Some(track), Some(column)) = (edit["track"].as_str(), edit["column"].as_str()) {
                projected.insert((track.to_owned(), column.to_owned()), edit["after"].clone());
            }
        }
    }
    Ok(projected)
}

impl Sidecar {
    /// Every Fact of `kind`, of every track, newest first: archived and
    /// stale ones included, for the caller to rank.
    pub fn facts_of_kind(&self, kind: &str) -> Result<Vec<Fact>> {
        facts_of_kind(&self.db(), kind)
    }

    /// The native values the Library edits made by service command
    /// `command` left, per track and column ([`Projected`]).
    pub fn projected_by(&self, command: &str) -> Result<Projected> {
        projected_by(&self.db(), command)
    }
}

impl FactsReader {
    /// [`Sidecar::facts_of_kind`] on a read-only snapshot.
    pub fn facts_of_kind(&self, kind: &str) -> Result<Vec<Fact>> {
        facts_of_kind(self.connection(), kind)
    }

    /// [`Sidecar::projected_by`] on a read-only snapshot.
    pub fn projected_by(&self, command: &str) -> Result<Projected> {
        projected_by(self.connection(), command)
    }
}
