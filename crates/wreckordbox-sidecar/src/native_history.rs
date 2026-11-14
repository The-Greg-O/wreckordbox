//! Put existing append-only Sidecar history in the shared Ledger order.
//! No second logical result is counted: inspection reads the Ledger reference,
//! never a union that counts both it and the native history row. Catch-up is a
//! bounded, resumable data migration on the writer's open, not on any reader.
use crate::{
    ledger::{Action, NewEntry, Store},
    Result, RunId, Source, SourceKind,
};
use rusqlite::{params, OptionalExtension, Transaction};
use serde_json::{json, Value};

#[derive(Clone, Copy)]
pub(crate) enum Table {
    Mutant,
    Decision,
    Answer,
}
impl Table {
    fn name(self) -> &'static str {
        match self {
            Self::Mutant => "mutant_change",
            Self::Decision => "decision",
            Self::Answer => "decision_answer",
        }
    }
    fn key(self) -> String {
        format!("ledger_native_cursor_{}", self.name())
    }
}
fn row(db: &rusqlite::Connection, table: Table, id: i64) -> Result<Value> {
    let mut query = db.prepare(&format!("SELECT * FROM {} WHERE id=?1", table.name()))?;
    let names = query
        .column_names()
        .iter()
        .map(|s| (*s).to_owned())
        .collect::<Vec<_>>();
    Ok(query.query_row([id], |row| {
        let mut object = serde_json::Map::new();
        for (column, name) in names.iter().enumerate() {
            use rusqlite::types::ValueRef;
            let value = match row.get_ref(column)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(n) => json!(n),
                ValueRef::Real(n) => json!(n),
                ValueRef::Text(bytes) => {
                    let text = std::str::from_utf8(bytes).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            column,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;
                    if matches!(
                        name.as_str(),
                        "edits" | "context" | "candidates" | "probabilities"
                    ) {
                        serde_json::from_str(text).unwrap_or_else(|_| json!(text))
                    } else {
                        json!(text)
                    }
                }
                ValueRef::Blob(bytes) => json!({"bytes":bytes}),
            };
            object.insert(name.clone(), value);
        }
        Ok(Value::Object(object))
    })?)
}
pub(crate) fn record(tx: &Transaction<'_>, table: Table, id: i64) -> Result<()> {
    capture(tx, table, id, false)
}
fn capture(tx: &Transaction<'_>, table: Table, id: i64, historical: bool) -> Result<()> {
    let row = row(tx, table, id)?;
    if let Some(run) = row["run_id"].as_i64() {
        let (source, at, label) = match table {
            Table::Mutant => (
                Source::new(
                    SourceKind::from_stored(row["source_kind"].as_str().unwrap_or("tool")),
                    row["source_name"].as_str().unwrap_or(""),
                ),
                row["recorded_at"].as_i64().unwrap_or(0),
                format!(
                    "{} Mutant {}",
                    row["action"].as_str().unwrap_or("Changed"),
                    row["mutant_id"]
                ),
            ),
            Table::Decision => {
                let run = crate::store::run_in(tx, RunId(run))?
                    .ok_or(crate::SidecarError::NoRun(RunId(run)))?;
                (
                    run.source,
                    row["created_at"].as_i64().unwrap_or(0),
                    format!("Ask Decision: {}", row["question"].as_str().unwrap_or("")),
                )
            }
            Table::Answer => (
                Source::new(
                    SourceKind::from_stored(row["decider_kind"].as_str().unwrap_or("tool")),
                    row["decider_name"].as_str().unwrap_or(""),
                ),
                row["answered_at"].as_i64().unwrap_or(0),
                format!("Answer Decision {}", row["decision_id"]),
            ),
        };
        let command = json!({"name":"sidecar.history","args":{"table":table.name(),"id":id}});
        let payload = json!({"table":table.name(),"row":row,"historical_order":historical});
        crate::ledger::append_in(
            tx,
            &NewEntry {
                id: None,
                run: RunId(run),
                store: Store::Sidecar,
                source: &source,
                at: Some(at),
                action: Action::Write,
                kind: table.name(),
                label: &label,
                command: Some(&command),
                reverses: None,
                part_of: None,
                reversible: false,
                format: 1,
                payload: Some(&payload),
            },
        )?;
    }
    tx.execute("INSERT INTO meta(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=CAST(MAX(CAST(value AS INTEGER),CAST(excluded.value AS INTEGER)) AS TEXT)",params![table.key(),id.to_string()])?;
    Ok(())
}
pub(crate) fn catch_up(db: &mut rusqlite::Connection) -> Result<()> {
    for table in [Table::Mutant, Table::Decision, Table::Answer] {
        loop {
            let tx = db.transaction()?;
            let cursor: Option<String> = tx
                .query_row("SELECT value FROM meta WHERE key=?1", [table.key()], |r| {
                    r.get(0)
                })
                .optional()?;
            let cursor = cursor.and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
            let ids = tx
                .prepare(&format!(
                    "SELECT id FROM {} WHERE id>?1 ORDER BY id LIMIT 128",
                    table.name()
                ))?
                .query_map([cursor], |r| r.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if ids.is_empty() {
                break;
            }
            for id in ids {
                capture(&tx, table, id, true)?;
            }
            tx.commit()?;
        }
    }
    Ok(())
}

// Accepted entries retain the shared sequence even if the clock moves. Legacy
// native rows had no place in that sequence: place them after the last known
// entry no later than their original timestamp, with deterministic timestamp/id
// ties. Their payload explicitly identifies this weaker historical ordering.
pub(crate) const PAGE_QUERY:&str="
SELECT store,id,replaced FROM (
 SELECT 0 AS store,ledger.id AS id,NULL AS replaced,
 CASE WHEN json_extract(ledger.payload,'$.historical_order')=1 THEN
  MAX(COALESCE((SELECT MAX(prior.id) FROM ledger AS prior
   WHERE prior.run_id=?1 AND prior.at<=ledger.at
   AND COALESCE(json_extract(prior.payload,'$.historical_order'),0)=0),0),
   COALESCE((SELECT MAX(ledger_order.seq) FROM fact JOIN ledger_order ON ledger_order.fact_id=fact.id
   WHERE fact.run_id=?1 AND fact.recorded_at<=ledger.at AND fact.id NOT IN (SELECT fact_id FROM dj_fact)),0))
 ELSE ledger.id END AS seq,
 COALESCE(json_extract(ledger.payload,'$.historical_order'),0) AS historical,
 ledger.at AS historical_at
 FROM ledger WHERE ledger.run_id=?1
 UNION ALL
 SELECT 1,fact.id,(SELECT before.id FROM fact AS before
  WHERE before.superseded_by=fact.id ORDER BY before.id DESC LIMIT 1),ledger_order.seq,0,0
 FROM fact JOIN ledger_order ON ledger_order.fact_id=fact.id
 WHERE fact.run_id=?1 AND fact.id NOT IN (SELECT fact_id FROM dj_fact))
 ORDER BY seq,historical,historical_at,id LIMIT ?2 OFFSET ?3";
