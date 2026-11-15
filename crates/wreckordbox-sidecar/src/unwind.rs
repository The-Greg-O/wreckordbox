//! Read-only Unwind evidence and atomic Fact visibility changes. The original
//! Fact rows retain their source, confidence and Run. No schema change.
use crate::{ledger::Entry, Sidecar, Result};
use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Visibility {
    pub id: i64,
    pub superseded_at: Option<i64>,
    pub superseded_by: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactChange {
    pub before: Visibility,
    pub after: Visibility,
}

impl Sidecar {
    /// A successful Unwind is a durable receipt, even after restart. Fact
    /// identifiers and Ledger identifiers are different namespaces.
    pub fn was_unwound(&self, id: i64, fact: bool) -> Result<bool> {
        Ok(self.db().query_row(
            "SELECT EXISTS(SELECT 1 FROM ledger l, json_each(l.payload, '$.reverses') r
             WHERE l.kind='unwind' AND l.action='edit'
               AND json_extract(r.value,'$.id')=?1 AND json_extract(r.value,'$.fact')=?2)",
            params![id, fact], |r| r.get(0),
        )?)
    }

    pub fn fact_visibility(&self, id: i64) -> Result<Option<Visibility>> {
        visibility(&self.db(), id)
    }

    /// The original native edit's exact pre-edit visibility. A Fact made
    /// by that edit is withdrawn, rather than deleted from the Ledger.
    pub fn native_unwind_targets(&self, entry: &Entry) -> Result<Vec<Visibility>> {
        let db = self.db();
        let mut q = db.prepare("SELECT fact_id,existed,superseded_at,superseded_by FROM dj_fact_effect WHERE entry_id=?1 ORDER BY fact_id")?;
        let rows = q.query_map([entry.id], |r| {
            let existed: bool = r.get(1)?;
            Ok(Visibility { id:r.get(0)?, superseded_at:if existed { r.get(2)? } else { Some(crate::store::now()) }, superseded_by:if existed { r.get(3)? } else { None } })
        })?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Later native effects or same-source Fact replacements must not be
    /// silently withdrawn along with an older native edit.
    pub fn native_effects_changed(&self, id: i64) -> Result<bool> {
        let db = self.db();
        Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM dj_fact_effect a JOIN dj_fact_effect b ON a.fact_id=b.fact_id
                           WHERE a.entry_id=?1 AND b.entry_id>?1)
                OR EXISTS(SELECT 1 FROM dj_fact d JOIN fact f ON f.id=d.fact_id
                          WHERE d.ledger_id=?1 AND f.superseded_at IS NOT NULL)",
            [id], |r| r.get(0),
        )?)
    }
}

fn visibility(db: &rusqlite::Connection, id: i64) -> Result<Option<Visibility>> {
    Ok(db.query_row("SELECT superseded_at,superseded_by FROM fact WHERE id=?1", [id], |r|
        Ok(Visibility { id, superseded_at:r.get(0)?, superseded_by:r.get(1)? })
    ).optional()?)
}

pub(crate) fn apply_in(tx: &Transaction<'_>, entry: i64, payload: &Value) -> Result<()> {
    let changes: Vec<FactChange> = serde_json::from_value(payload["fact_changes"].clone()).map_err(|e|crate::SidecarError::Invalid(e.to_string()))?;
    for change in &changes {
        if visibility(tx, change.before.id)?.as_ref() != Some(&change.before) {
            return Err(crate::SidecarError::Invalid("Fact visibility changed while applying Unwind".into()));
        }
    }
    for change in changes {
        super::dj_facts::remember(tx, entry, change.before.id, true)?;
        tx.execute("UPDATE fact SET superseded_at=?1,superseded_by=?2 WHERE id=?3",
            params![change.after.superseded_at,change.after.superseded_by,change.after.id])?;
    }
    Ok(())
}
