//! What Projection (wbx-26) reads from the Sidecar: every Fact of one kind
//! at once, and who made the latest recorded write to each native field,
//! from the Ledger. Each is one query whatever the size of the Library, so
//! a projection never asks the store once per track. Nothing here writes.
//!
//! Provenance is read field by field from `track` entries (the payload of
//! every rating, colour and other track edit names its track and column),
//! following Undo and Redo back to the edit they step: an Undo restores the
//! state before the edit it takes back, a Redo puts that edit's owner and
//! value back. A chain that cannot be followed (its edit is not in the
//! Ledger) leaves the field [`Owner::Unknown`].
use std::collections::HashMap;

use rusqlite::{params, Connection};
use serde_json::Value;

use crate::store::{read_fact, select};
use crate::{Fact, FactsReader, Result, Sidecar, Source, SourceKind};

/// Who made the write that left a field as it is.
#[derive(Debug, Clone, PartialEq)]
pub enum Owner {
    /// A `projection.apply` (or a Redo of one).
    Projection,
    /// The DJ.
    Dj,
    /// Anyone else the Ledger names: an agent, a tool, a player.
    Other(Source),
    /// What the field held before any recorded write, back after an Undo.
    Original,
    /// The Ledger cannot say (an Undo of an edit it does not hold).
    Unknown,
}

/// The effective latest recorded write to one field.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldState {
    pub owner: Owner,
    /// The value it left, as the Ledger stores values (`null`, a whole
    /// number, text). `null` for [`Owner::Original`] and [`Owner::Unknown`].
    pub value: Value,
    /// When the entry that left it was recorded, in Unix milliseconds:
    /// after its write was made, so a row written later is a later write.
    pub at: i64,
}

/// The provenance of native fields, from the Ledger.
#[derive(Debug, Clone, Default)]
pub struct Provenance {
    /// By `(track, column)`: only fields some `track` entry wrote.
    pub fields: HashMap<(String, String), FieldState>,
    /// By track: when the latest `track` entry naming it was recorded.
    pub latest: HashMap<String, i64>,
    /// The highest Ledger entry read: anything after it is newer than this
    /// provenance.
    pub watermark: i64,
}

/// The service command whose edits are a projection's.
const PROJECTION_COMMAND: &str = "projection.apply";

fn facts_of_kind(db: &Connection, kind: &str) -> Result<Vec<Fact>> {
    let mut statement = db.prepare_cached(&select("WHERE fact.kind = ?1 ORDER BY fact.id DESC"))?;
    let facts = statement
        .query_map(params![kind], read_fact)?
        .collect::<rusqlite::Result<Vec<Fact>>>()?;
    Ok(facts)
}

/// The states each step of one field replaced, by the edit it steps.
type Steps = Vec<(i64, Option<FieldState>)>;

struct Row {
    id: i64,
    at: i64,
    action: String,
    source: Source,
    command: Option<String>,
    reverses: Option<i64>,
    payload: Option<String>,
}

fn provenance(db: &Connection, columns: &[&str]) -> Result<Provenance> {
    let watermark: i64 = db.query_row("SELECT COALESCE(MAX(id), 0) FROM ledger", [], |row| {
        row.get(0)
    })?;
    let mut statement = db.prepare_cached(
        "SELECT id, at, action, source_kind, source_name,
                json_extract(command, '$.name'), reverses, payload
         FROM ledger WHERE store = 'library' AND kind = 'track' ORDER BY id",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok(Row {
                id: row.get(0)?,
                at: row.get(1)?,
                action: row.get(2)?,
                source: Source {
                    kind: SourceKind::from_stored(&row.get::<_, String>(3)?),
                    name: row.get(4)?,
                },
                command: row.get(5)?,
                reverses: row.get(6)?,
                payload: row.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<Row>>>()?;
    let mut provenance = Provenance {
        watermark,
        ..Provenance::default()
    };
    // id -> (action, reverses, owner when it is an edit)
    let mut steps: HashMap<i64, (String, Option<i64>, Owner)> = HashMap::new();
    // Per field: the states each step replaced, to go back to on Undo.
    let mut stacks: HashMap<(String, String), Steps> = HashMap::new();
    for row in rows {
        let owner = if row.command.as_deref() == Some(PROJECTION_COMMAND) {
            Owner::Projection
        } else if row.source.kind == SourceKind::Dj {
            Owner::Dj
        } else {
            Owner::Other(row.source.clone())
        };
        steps.insert(row.id, (row.action.clone(), row.reverses, owner.clone()));
        // The edit a step steps, through Undo and Redo, and its owner.
        let origin = {
            let mut at = row.id;
            let mut hops = 0;
            loop {
                match steps.get(&at) {
                    Some((action, Some(reverses), _)) if action != "edit" && hops < 10_000 => {
                        at = *reverses;
                        hops += 1;
                    }
                    Some((action, _, owner)) if action == "edit" => {
                        break Some((at, owner.clone()))
                    }
                    _ => break None,
                }
            }
        };
        let Some(payload) = row
            .payload
            .as_deref()
            .and_then(|p| serde_json::from_str::<Value>(p).ok())
        else {
            continue;
        };
        for edit in payload["edits"].as_array().into_iter().flatten() {
            let (Some(track), Some(column)) = (edit["track"].as_str(), edit["column"].as_str())
            else {
                continue;
            };
            let latest = provenance.latest.entry(track.to_owned()).or_insert(row.at);
            *latest = (*latest).max(row.at);
            if !columns.contains(&column) {
                continue;
            }
            let key = (track.to_owned(), column.to_owned());
            let current = provenance.fields.get(&key).cloned();
            let stack = stacks.entry(key.clone()).or_default();
            let next = match (row.action.as_str(), &origin) {
                ("undo", Some((origin, _))) => {
                    match stack.iter().rposition(|(step, _)| step == origin) {
                        Some(at) => {
                            let before = stack[at].1.clone();
                            stack.truncate(at);
                            match before {
                                Some(state) => FieldState {
                                    at: row.at,
                                    ..state
                                },
                                None => FieldState {
                                    owner: Owner::Original,
                                    value: Value::Null,
                                    at: row.at,
                                },
                            }
                        }
                        None => FieldState {
                            owner: Owner::Unknown,
                            value: Value::Null,
                            at: row.at,
                        },
                    }
                }
                ("edit" | "redo", Some((origin, owner))) => {
                    stack.push((*origin, current));
                    FieldState {
                        owner: owner.clone(),
                        value: edit["after"].clone(),
                        at: row.at,
                    }
                }
                _ => FieldState {
                    owner: Owner::Unknown,
                    value: Value::Null,
                    at: row.at,
                },
            };
            provenance.fields.insert(key, next);
        }
    }
    Ok(provenance)
}

/// Whether any `track` entry after `watermark` names one of `tracks`.
fn written_since(db: &Connection, watermark: i64, tracks: &[String]) -> Result<bool> {
    let mut statement = db.prepare_cached(
        "SELECT payload FROM ledger
         WHERE id > ?1 AND store = 'library' AND kind = 'track'",
    )?;
    let payloads = statement
        .query_map(params![watermark], |row| row.get::<_, Option<String>>(0))?
        .collect::<rusqlite::Result<Vec<Option<String>>>>()?;
    Ok(payloads.into_iter().any(|payload| {
        payload
            .and_then(|p| serde_json::from_str::<Value>(&p).ok())
            .is_some_and(|payload| {
                payload["edits"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|edit| {
                        edit["track"]
                            .as_str()
                            .is_some_and(|track| tracks.iter().any(|t| t == track))
                    })
            })
    }))
}

impl Sidecar {
    /// Every Fact of `kind`, of every track, newest first: archived and
    /// stale ones included, for the caller to rank.
    pub fn facts_of_kind(&self, kind: &str) -> Result<Vec<Fact>> {
        facts_of_kind(&self.db(), kind)
    }

    /// Who made the latest recorded write to each of `columns`, per track
    /// ([`Provenance`]).
    pub fn provenance(&self, columns: &[&str]) -> Result<Provenance> {
        provenance(&self.db(), columns)
    }

    /// Whether a `track` entry newer than `watermark` names one of `tracks`.
    pub fn written_since(&self, watermark: i64, tracks: &[String]) -> Result<bool> {
        written_since(&self.db(), watermark, tracks)
    }
}

impl FactsReader {
    /// [`Sidecar::facts_of_kind`] on a read-only snapshot.
    pub fn facts_of_kind(&self, kind: &str) -> Result<Vec<Fact>> {
        facts_of_kind(self.connection(), kind)
    }

    /// [`Sidecar::provenance`] on a read-only snapshot.
    pub fn provenance(&self, columns: &[&str]) -> Result<Provenance> {
        provenance(self.connection(), columns)
    }
}
