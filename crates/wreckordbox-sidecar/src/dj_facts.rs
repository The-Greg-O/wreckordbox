//! Facts derived from native DJ edits, committed with their Ledger entry.
use crate::attachments::IdentifyingFields;
use crate::ledger::{append_in, NewEntry};
use crate::store::record_in;
use crate::{Fingerprint, Kind, Nature, NewFact, Result, Sidecar, SourceKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const SCHEMA: &str = "
CREATE TABLE dj_intent (id INTEGER PRIMARY KEY REFERENCES ledger_intent(id) ON DELETE CASCADE, snapshot TEXT NOT NULL);
CREATE TABLE dj_fact (fact_id INTEGER PRIMARY KEY REFERENCES fact(id), ledger_id INTEGER NOT NULL REFERENCES ledger(id), reconciled INTEGER NOT NULL);
CREATE INDEX dj_fact_entry ON dj_fact(ledger_id);
";

/// One native value changed by a DJ edit. Empty native scalars are null;
/// empty dimensions are empty lists, so clearing supersedes earlier claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeFact {
    pub track: String,
    pub kind: String,
    pub value: Value,
}
/// Audio and identity captured before taking the history/Sidecar locks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeAudio {
    pub fingerprint: Option<Fingerprint>,
    pub identity: IdentifyingFields,
}
/// An edit's native values and the Library snapshot used for observation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NativeFacts {
    pub values: Vec<NativeFact>,
    pub audio: BTreeMap<String, NativeAudio>,
    pub present: BTreeSet<String>,
}
impl Sidecar {
    /// Append the entry, its DJ Facts and their provenance together, and
    /// clear the intent. Replaying a reserved identifier is a complete no-op.
    pub fn append_with_dj_facts(
        &self,
        entry: &NewEntry,
        facts: &NativeFacts,
        reconciled: bool,
    ) -> Result<i64> {
        let mut db = self.db();
        let tx = db.transaction()?;
        if let Some(id) = entry.id {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM ledger WHERE id=?1)",
                [id],
                |row| row.get(0),
            )?;
            if exists {
                return Ok(id);
            }
        }
        let id = append_in(&tx, entry)?;
        if entry.source.kind == SourceKind::Dj {
            for (track, audio) in &facts.audio {
                crate::attachments::observe_in(
                    &tx,
                    track,
                    audio.fingerprint.as_ref(),
                    &audio.identity,
                    |id| facts.present.contains(id),
                    Some(entry.run),
                )?;
                if let Some(fingerprint) = &audio.fingerprint {
                    crate::store::mark_stale_in(&tx, track, fingerprint)?;
                }
            }
            for native in &facts.values {
                let fact = record_in(
                    &tx,
                    &NewFact {
                        track_id: &native.track,
                        fingerprint: facts
                            .audio
                            .get(&native.track)
                            .and_then(|audio| audio.fingerprint.as_ref()),
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
                tx.execute(
                    "INSERT INTO dj_fact(fact_id,ledger_id,reconciled) VALUES(?1,?2,?3)",
                    rusqlite::params![fact.id, id, reconciled],
                )?;
            }
        }
        tx.commit()?;
        Ok(id)
    }
}

/// Never serve old standing data while an admitted DJ edit still needs its
/// entry. This checks only; it does not replay, clear, or recover anything.
pub(crate) fn ready(db: &rusqlite::Connection) -> Result<()> {
    let pending: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM dj_intent)", [], |row| {
        row.get(0)
    })?;
    if pending {
        return Err(crate::SidecarError::Invalid("a DJ edit is pending in the Ledger; its Facts are unavailable until recovery or the next successful edit".to_owned()));
    }
    Ok(())
}
