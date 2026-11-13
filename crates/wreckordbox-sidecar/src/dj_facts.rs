//! Native edit Facts and their reversible effects, at the Ledger boundary.
use crate::ledger::{append_in, Action, NewEntry};
use crate::store::record_in;
use crate::{Kind, Nature, NewFact, Result, Sidecar, SourceKind};
use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub(crate) const SCHEMA: &str = "
CREATE TABLE dj_intent (id INTEGER PRIMARY KEY REFERENCES ledger_intent(id) ON DELETE CASCADE, snapshot TEXT NOT NULL);
CREATE TABLE dj_fact (fact_id INTEGER PRIMARY KEY REFERENCES fact(id), ledger_id INTEGER NOT NULL REFERENCES ledger(id), reconciled INTEGER NOT NULL);
CREATE INDEX dj_fact_entry ON dj_fact(ledger_id);
";
pub(crate) const EFFECTS_SCHEMA: &str = "
CREATE TABLE dj_fact_effect (
 entry_id INTEGER NOT NULL REFERENCES ledger(id), fact_id INTEGER NOT NULL REFERENCES fact(id),
 existed INTEGER NOT NULL, superseded_at INTEGER, superseded_by INTEGER,
 PRIMARY KEY(entry_id,fact_id)
);
";
/// One native judgement; null withdraws an opinion instead of creating a null Fact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeFact {
    pub track: String,
    pub kind: String,
    pub value: Value,
    /// For membership edits: unchanged terms need existing DJ provenance.
    #[serde(default)]
    pub before: Option<Value>,
}
/// A capture failure is part of the entry, never a reason to refuse the edit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Omission {
    pub track: String,
    pub kind: String,
    pub reason: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NativeFacts {
    pub values: Vec<NativeFact>,
    #[serde(default)]
    pub skipped: Vec<Omission>,
    #[serde(default)]
    pub undo_of: Option<i64>,
    #[serde(default)]
    pub redo_of: Option<i64>,
}
impl NativeFacts {
    pub fn skip(
        &mut self,
        track: impl Into<String>,
        kind: impl Into<String>,
        reason: impl Into<String>,
    ) {
        self.skipped.push(Omission {
            track: track.into(),
            kind: kind.into(),
            reason: reason.into(),
        });
    }
}
impl Sidecar {
    /// Add bounded evidence to the already committed intent, before the Library write.
    pub fn capture_dj_intent(&self, id: i64, snapshot: &Value) -> Result<()> {
        self.db().execute(
            "INSERT OR REPLACE INTO dj_intent(id,snapshot) VALUES(?1,?2)",
            params![id, snapshot.to_string()],
        )?;
        Ok(())
    }
    /// Commit entry and successful Fact work together. A failed Fact batch rolls
    /// back to the savepoint and becomes an entry with explicit omissions; it
    /// cannot spill or block the Ledger. Failure of the Ledger itself still spills.
    pub fn append_with_dj_facts(
        &self,
        entry: &NewEntry,
        facts: &NativeFacts,
        reconciled: bool,
    ) -> Result<i64> {
        self.append_in_one(
            entry,
            facts,
            reconciled,
            &crate::mutant_follow::Follow::default(),
        )
        .map(|(id, _)| id)
    }

    /// [`Self::append_with_dj_facts`] with the Mutant consequences of the
    /// change in the same transaction (wbx-80, [`crate::mutant_follow`]).
    pub(crate) fn append_in_one(
        &self,
        entry: &NewEntry,
        facts: &NativeFacts,
        reconciled: bool,
        follow: &crate::mutant_follow::Follow,
    ) -> Result<(i64, Vec<crate::mutant_follow::Followed>)> {
        let mut db = self.db();
        let tx = db.transaction()?;
        if let Some(id) = entry.id {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM ledger WHERE id=?1)",
                [id],
                |r| r.get(0),
            )?;
            if exists {
                let id = append_in(&tx, entry)?;
                tx.commit()?;
                return Ok((id, Vec::new()));
            }
        }
        // The Mutants' consequences: with the entry or not at all. A
        // failure fails the append, so the Ledger keeps both and writes
        // them later together (wbx-80).
        let followed = if follow.is_empty() {
            Vec::new()
        } else {
            let at = entry.at.unwrap_or_else(crate::store::now);
            crate::mutant_follow::apply_in(&tx, follow, entry.source, entry.run, at)?
        };
        let mut facts = facts.clone();
        // Nothing from an agent/Projection is promoted by this observer.
        if entry.source.kind != SourceKind::Dj {
            facts = NativeFacts::default();
        }
        let redo = facts.redo_of.or_else(|| {
            (entry.action == Action::Redo)
                .then_some(entry.reverses)
                .flatten()
        });
        if let Some(undo) = redo {
            // A Redo re-asserts the edit it steps only when that edit, at
            // the origin of the whole Undo/Redo chain, was the DJ's; a chain
            // that cannot be followed to an edit promotes nothing (wbx-26).
            if originating_source(&tx, undo)?.as_deref() != Some("dj") {
                facts.values.clear();
            }
        }
        let mut allowed = Vec::new();
        for fact in std::mem::take(&mut facts.values) {
            match attributable(&tx,&fact,entry.source.name.as_str()) {
                Ok(true)=>allowed.push(fact),
                Ok(false)=>facts.skip(&fact.track,&fact.kind,"unchanged native terms lack DJ provenance; explicitly assert the whole dimension with facts record or schema convert"),
                Err(e)=>facts.skip(&fact.track,&fact.kind,format!("could not establish DJ provenance: {e}")),
            }
        }
        facts.values = allowed;
        let payload = with_omissions(entry, &facts.skipped);
        let candidate = NewEntry {
            payload: payload.as_ref(),
            ..*entry
        };
        tx.execute_batch("SAVEPOINT native_facts")?;
        let id = append_in(&tx, &candidate)?;
        let work = (|| {
            if let Some(reversed) = facts.undo_of.or_else(|| {
                (entry.action == Action::Undo)
                    .then_some(entry.reverses)
                    .flatten()
            }) {
                withdraw_edit(&tx, id, reversed)?;
            } else {
                for native in &facts.values {
                    record_native(&tx, id, entry, native, reconciled)?;
                }
            }
            Ok::<_, crate::SidecarError>(())
        })();
        let id = match work {
            Ok(()) => {
                tx.execute_batch("RELEASE native_facts")?;
                id
            }
            Err(error) => {
                tx.execute_batch("ROLLBACK TO native_facts; RELEASE native_facts")?;
                if facts.values.is_empty() {
                    facts.skip("", "undo", format!("Fact withdrawal not recorded: {error}"));
                } else {
                    for native in facts.values.clone() {
                        facts.skip(
                            native.track,
                            native.kind,
                            format!("Fact batch not recorded: {error}"),
                        );
                    }
                }
                let payload = with_omissions(entry, &facts.skipped);
                append_in(
                    &tx,
                    &NewEntry {
                        payload: payload.as_ref(),
                        ..*entry
                    },
                )?
            }
        };
        tx.commit()?;
        Ok((id, followed))
    }
}
/// The source of the edit at the origin of the Undo/Redo chain that `step`
/// belongs to: from `step`, follow `reverses` through every Undo and Redo,
/// however many, to the first entry that is an edit. `None` when the chain
/// breaks (an entry missing, or no edit at its end).
fn originating_source(tx: &rusqlite::Transaction<'_>, step: i64) -> Result<Option<String>> {
    let mut at = step;
    for _ in 0..100_000 {
        let found: Option<(String, Option<i64>, String)> = tx
            .query_row(
                "SELECT action, reverses, source_kind FROM ledger WHERE id = ?1",
                [at],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        match found {
            Some((action, _, source)) if action == "edit" => return Ok(Some(source)),
            Some((_, Some(reverses), _)) => at = reverses,
            _ => return Ok(None),
        }
    }
    Ok(None)
}

fn with_omissions(entry: &NewEntry, skipped: &[Omission]) -> Option<Value> {
    if skipped.is_empty() {
        return entry.payload.cloned();
    }
    let mut payload = entry.payload.cloned().unwrap_or_else(|| json!({}));
    payload["facts_not_recorded"] = json!(skipped);
    Some(payload)
}
fn terms(value: &Value) -> Vec<&str> {
    match value {
        Value::String(s) => vec![s],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}
fn attributable(tx: &Transaction<'_>, fact: &NativeFact, source: &str) -> Result<bool> {
    let Some(before) = &fact.before else {
        return Ok(true);
    };
    let after = terms(&fact.value);
    let unchanged: Vec<_> = terms(before)
        .into_iter()
        .filter(|term| after.contains(term))
        .collect();
    if unchanged.is_empty() {
        return Ok(true);
    }
    let existing:Option<String>=tx.query_row("SELECT value FROM attached_fact WHERE effective_track_id=?1 AND kind=?2 AND source_kind='dj' AND source_name=?3 AND superseded_at IS NULL ORDER BY id DESC LIMIT 1",params![fact.track,fact.kind,source],|r|r.get(0)).optional()?;
    let existing = existing
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or(Value::Null);
    let owned = terms(&existing);
    Ok(unchanged.iter().all(|term| owned.contains(term)))
}
fn remember(tx: &Transaction<'_>, entry: i64, fact: i64, existed: bool) -> Result<()> {
    tx.execute("INSERT OR IGNORE INTO dj_fact_effect(entry_id,fact_id,existed,superseded_at,superseded_by) SELECT ?1,id,?3,superseded_at,superseded_by FROM fact WHERE id=?2",params![entry,fact,existed])?;
    Ok(())
}
fn record_native(
    tx: &Transaction<'_>,
    id: i64,
    entry: &NewEntry,
    native: &NativeFact,
    reconciled: bool,
) -> Result<()> {
    let previous = {
        let mut q=tx.prepare("SELECT id FROM attached_fact WHERE effective_track_id=?1 AND kind=?2 AND source_kind='dj' AND source_name=?3 AND superseded_at IS NULL")?;
        let rows = q
            .query_map(params![native.track, native.kind, entry.source.name], |r| {
                r.get::<_, i64>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for fact in &previous {
        remember(tx, id, *fact, true)?;
    }
    if native.value.is_null() {
        for fact in previous {
            tx.execute(
                "UPDATE fact SET superseded_at=?1,superseded_by=NULL WHERE id=?2",
                params![crate::store::now(), fact],
            )?;
        }
        return Ok(());
    }
    let fact = record_in(
        tx,
        &NewFact {
            track_id: &native.track,
            fingerprint: None,
            kind: Kind {
                name: &native.kind,
                nature: Nature::Judgement,
            },
            value: native.value.clone(),
            source: entry.source,
            confidence: 1.0,
            run: entry.run,
        },
    )?;
    remember(tx, id, fact.id, false)?;
    tx.execute(
        "INSERT INTO dj_fact(fact_id,ledger_id,reconciled) VALUES(?1,?2,?3)",
        params![fact.id, id, reconciled],
    )?;
    Ok(())
}
/// Restore the original rows' visibility, retaining their IDs, source and Run.
fn withdraw_edit(tx: &Transaction<'_>, id: i64, reversed: i64) -> Result<()> {
    let effects = {
        let mut q=tx.prepare("SELECT fact_id,existed,superseded_at,superseded_by FROM dj_fact_effect WHERE entry_id=?1 ORDER BY fact_id DESC")?;
        let rows = q
            .query_map([reversed], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, bool>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    if effects.is_empty() {
        // Schema-7 entries had links but no effect journal. Preserve their history too.
        let made = {
            let mut q = tx.prepare("SELECT fact_id FROM dj_fact WHERE ledger_id=?1")?;
            let rows = q
                .query_map([reversed], |r| r.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        for fact in made {
            let live: bool = tx.query_row(
                "SELECT superseded_at IS NULL FROM fact WHERE id=?1",
                [fact],
                |r| r.get(0),
            )?;
            remember(tx, id, fact, true)?;
            if live {
                tx.execute(
                    "UPDATE fact SET superseded_at=NULL,superseded_by=NULL WHERE superseded_by=?1",
                    [fact],
                )?;
            }
            tx.execute(
                "UPDATE fact SET superseded_at=?1,superseded_by=NULL WHERE id=?2",
                params![crate::store::now(), fact],
            )?;
        }
        return Ok(());
    }
    for (fact, existed, at, by) in effects {
        remember(tx, id, fact, true)?;
        if existed {
            tx.execute(
                "UPDATE fact SET superseded_at=?1,superseded_by=?2 WHERE id=?3",
                params![at, by, fact],
            )?;
        } else {
            tx.execute(
                "UPDATE fact SET superseded_at=?1,superseded_by=NULL WHERE id=?2",
                params![crate::store::now(), fact],
            )?;
        }
    }
    Ok(())
}
