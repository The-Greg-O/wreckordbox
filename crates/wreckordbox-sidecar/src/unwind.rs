//! Read-only Unwind evidence and atomic Fact visibility changes. The original
//! Fact rows retain their source, confidence and Run. No schema change.
use crate::{ledger::Entry, Result, Sidecar};
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
            params![id, fact],
            |r| r.get(0),
        )?)
    }

    /// A window Undo already took this entry back. Follow Redo links too;
    /// entries within the Run are handled by the reverse-order planner itself.
    pub fn undone_outside_run(&self, entry: &Entry) -> Result<bool> {
        let db = self.db();
        let mut at = entry.id;
        let mut reversed = false;
        let mut last_run = entry.run.0;
        loop {
            let next: Option<(i64, i64)> = db.query_row(
                "SELECT id,run_id FROM ledger WHERE reverses=?1 AND id>?1 AND action IN ('undo','redo') ORDER BY id DESC LIMIT 1",
                [at], |r| Ok((r.get(0)?,r.get(1)?)),
            ).optional()?;
            let Some((id, run)) = next else {
                break;
            };
            at = id;
            last_run = run;
            reversed = !reversed;
        }
        Ok(reversed && last_run != entry.run.0)
    }

    pub fn fact_visibility(&self, id: i64) -> Result<Option<Visibility>> {
        visibility(&self.db(), id)
    }

    /// The original native edit's exact pre-edit visibility. A Fact made
    /// by that edit is withdrawn, rather than deleted from the Ledger.
    pub fn native_unwind_targets(&self, entry: &Entry) -> Result<Vec<Visibility>> {
        let db = self.db();
        let mut q = db.prepare("SELECT fact_id,existed,superseded_at,superseded_by FROM dj_fact_effect WHERE entry_id=?1 ORDER BY fact_id")?;
        let rows = q
            .query_map([entry.id], |r| {
                let existed: bool = r.get(1)?;
                Ok(Visibility {
                    id: r.get(0)?,
                    superseded_at: if existed {
                        r.get(2)?
                    } else {
                        Some(crate::store::now())
                    },
                    superseded_by: if existed { r.get(3)? } else { None },
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Current opinions in the same slots as an entry's native Fact effects.
    pub fn native_unwind_facts(&self, id: i64) -> Result<Vec<crate::Fact>> {
        let db = self.db();
        let mut q = db.prepare(&crate::store::select("WHERE fact.id IN (
            SELECT newer.id FROM dj_fact_effect e
            JOIN attached_fact old ON old.id=e.fact_id
            JOIN attached_fact newer ON newer.effective_track_id=old.effective_track_id AND newer.kind=old.kind
                AND newer.source_kind=old.source_kind AND newer.source_name=old.source_name
            WHERE e.entry_id=?1)"))?;
        let facts = q
            .query_map([id], crate::store::read_fact)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(facts)
    }

    /// All recorded opinions sharing the changed Facts' attachment, kind and source.
    pub fn unwind_related_facts(&self, ids: &[i64]) -> Result<Vec<crate::Fact>> {
        let db = self.db();
        let mut q = db.prepare(&crate::store::select("WHERE fact.id IN (
            SELECT newer.id FROM attached_fact old
            JOIN attached_fact newer ON newer.effective_track_id=old.effective_track_id AND newer.kind=old.kind
                AND newer.source_kind=old.source_kind AND newer.source_name=old.source_name
            WHERE old.id IN (SELECT value FROM json_each(?1)))"))?;
        let facts = q
            .query_map(
                [serde_json::json!(ids).to_string()],
                crate::store::read_fact,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(facts)
    }

    /// Window history must protect Facts just as it protects native fields.
    pub fn check_unwind_history(&self, payload: &Value, undo: bool) -> Result<()> {
        let changes: Vec<FactChange> = serde_json::from_value(payload["fact_changes"].clone())
            .map_err(|e| crate::SidecarError::Invalid(e.to_string()))?;
        let conflict = || {
            crate::SidecarError::Invalid(
                "An Unwind Fact changed since; kept. Inspect the Run before trying again.".into(),
            )
        };
        let ids: Vec<_> = changes.iter().map(|change| change.before.id).collect();
        for change in &changes {
            let expected = if undo { &change.after } else { &change.before };
            if self.fact_visibility(expected.id)?.as_ref() != Some(expected) {
                return Err(conflict());
            }
        }
        if self
            .unwind_related_facts(&ids)?
            .iter()
            .any(|fact| fact.is_live() && !ids.contains(&fact.id))
        {
            return Err(conflict());
        }
        Ok(())
    }

    /// Later native effects or same-source Fact replacements must not be
    /// silently withdrawn along with an older native edit.
    pub fn native_effects_changed(
        &self,
        id: i64,
        reversed: &[i64],
        virtuals: &std::collections::BTreeMap<i64, FactChange>,
    ) -> Result<bool> {
        let db = self.db();
        let mut q = db.prepare("SELECT DISTINCT b.entry_id FROM dj_fact_effect a JOIN dj_fact_effect b ON a.fact_id=b.fact_id WHERE a.entry_id=?1 AND b.entry_id>?1")?;
        let later = q
            .query_map([id], |r| r.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if later.iter().any(|entry| !reversed.contains(entry)) {
            return Ok(true);
        }
        let mut q = db.prepare("SELECT fact_id FROM dj_fact WHERE ledger_id=?1")?;
        for fact in q.query_map([id], |r| r.get::<_, i64>(0))? {
            let fact = fact?;
            let current = match virtuals.get(&fact) {
                Some(change) => Some(change.after.clone()),
                None => visibility(&db, fact)?,
            };
            if current.is_none_or(|v| v.superseded_at.is_some()) {
                return Ok(true);
            }
        }
        // A direct Fact recorded after a native clear has no native effect
        // journal row. It still must prevent resurrecting the cleared opinion.
        let mut q = db.prepare("SELECT DISTINCT newer.id FROM dj_fact_effect e
            JOIN fact old ON old.id=e.fact_id
            JOIN fact newer ON newer.track_id=old.track_id AND newer.kind=old.kind
                AND newer.source_kind=old.source_kind AND newer.source_name=old.source_name
            WHERE e.entry_id=?1 AND newer.id>old.id AND newer.superseded_at IS NULL
                AND NOT EXISTS(SELECT 1 FROM dj_fact d WHERE d.fact_id=newer.id AND d.ledger_id=?1)")?;
        for fact in q.query_map([id], |r| r.get::<_, i64>(0))? {
            let fact = fact?;
            if virtuals
                .get(&fact)
                .is_none_or(|change| change.after.superseded_at.is_none())
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

fn visibility(db: &rusqlite::Connection, id: i64) -> Result<Option<Visibility>> {
    Ok(db
        .query_row(
            "SELECT superseded_at,superseded_by FROM fact WHERE id=?1",
            [id],
            |r| {
                Ok(Visibility {
                    id,
                    superseded_at: r.get(0)?,
                    superseded_by: r.get(1)?,
                })
            },
        )
        .optional()?)
}

pub(crate) fn apply_in(tx: &Transaction<'_>, entry: i64, payload: &Value) -> Result<()> {
    let changes: Vec<FactChange> = serde_json::from_value(payload["fact_changes"].clone())
        .map_err(|e| crate::SidecarError::Invalid(e.to_string()))?;
    for change in &changes {
        if visibility(tx, change.before.id)?.as_ref() != Some(&change.before) {
            return Err(crate::SidecarError::Invalid(
                "Fact visibility changed while applying Unwind".into(),
            ));
        }
    }
    for change in changes {
        super::dj_facts::remember(tx, entry, change.before.id, true)?;
        tx.execute(
            "UPDATE fact SET superseded_at=?1,superseded_by=?2 WHERE id=?3",
            params![
                change.after.superseded_at,
                change.after.superseded_by,
                change.after.id
            ],
        )?;
    }
    Ok(())
}
