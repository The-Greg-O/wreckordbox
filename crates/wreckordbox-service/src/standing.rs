//! Read-only standing Facts: no lazy writer opening, migration, attachment
//! update, stale marking or event. Measurements are verified against audio
//! for this answer only, using the same fingerprint as other Fact consumers.
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use wreckordbox_sidecar::precedence::{Reason, Rules, RULES_FILE};
use wreckordbox_sidecar::{FactsSnapshot, Kinds, Nature};

use crate::facts::{self, Fact};
use crate::protocol::{Request, ServiceError};
use crate::stems::{self, Parent, ParentAudio, Stems};

/// The standing Fact of one track and kind, plus the reason and alternatives.
pub const FACTS_STANDING: &str = "facts.standing";
/// Malformed or unreadable source rules; never silently fall back.
pub const RULES_INVALID: &str = "source_precedence_invalid";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    pub id: String,
    pub kind: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Explanation {
    pub code: Reason,
    pub message: String,
    pub source_priority: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Standing {
    pub track: Parent,
    pub kind: String,
    pub standing: Option<Fact>,
    pub reason: Explanation,
    /// Includes original history and current answers outranked by the winner.
    /// For the latter, superseded_by is computed, not persisted.
    pub superseded: Vec<Fact>,
    /// Measurements whose fingerprints do not match current audio.
    pub stale: Vec<Fact>,
    pub rules_file: PathBuf,
}

pub(crate) fn answer(
    stems: &Stems,
    request: &Request,
    lookup: &dyn Fn(&str) -> Result<ParentAudio, ServiceError>,
) -> Result<Value, ServiceError> {
    let query: Query = facts::arguments(request)?;
    let found = lookup(&query.id)?;
    let rules =
        Rules::load(stems.sidecar_dir()).map_err(|why| ServiceError::new(RULES_INVALID, why))?;
    let kinds = Kinds::load(stems.sidecar_dir()).map_err(|e| facts::sidecar_error(&e))?;
    let declaration = kinds.get(&query.kind);
    if declaration.is_none() && !kinds.schema_is_broken() {
        return Err(ServiceError::new(
            facts::codes::KIND_NOT_DECLARED,
            format!("{} is not a declared kind of Fact", query.kind),
        ));
    }
    let mut stored = stems
        .with_current_sidecar(|open| match open {
            Some(sidecar) => sidecar
                .facts(&query.id, &query.kind)
                .map_err(|e| e.to_string()),
            None => match FactsSnapshot::open(stems.sidecar_dir()).map_err(|e| e.to_string())? {
                FactsSnapshot::Absent => Ok(Vec::new()),
                FactsSnapshot::Unknown(why) => Err(why),
                FactsSnapshot::Open(reader) => reader
                    .facts(&query.id, &query.kind)
                    .map_err(|e| e.to_string()),
            },
        })
        .unwrap_or_else(|| Err("the Sidecar is being opened; try again".to_owned()))
        .map_err(|why| ServiceError::new(stems::codes::SIDECAR_FAILED, why))?;
    if stored
        .iter()
        .any(|fact| fact.nature == Nature::Measurement && fact.is_live())
    {
        // Failure is visible; an old successful check never substitutes for
        // audio that is missing or unreadable now. This only reads the file.
        let fingerprint = stems::fingerprint(found.parent(), found.path())?;
        for fact in &mut stored {
            if fact.nature == Nature::Measurement {
                fact.stale_at = (fact.fingerprint.as_ref() != Some(&fingerprint)).then_some(0);
            }
        }
    }
    let (winner, reason) = rules.standing(&stored);
    let winner_id = winner.map(|fact| fact.id);
    let convert = |fact: wreckordbox_sidecar::Fact| {
        let mut fact = Fact::from(fact);
        if let Some(declared) = declaration {
            fact.value = declared.normalise(fact.value);
        }
        fact
    };
    let standing = winner.cloned().map(convert);
    let priority = winner.map(|fact| rules.priority(&fact.source));
    let mut superseded = Vec::new();
    let mut stale = Vec::new();
    for fact in stored {
        if Some(fact.id) == winner_id {
            continue;
        }
        let mut fact = convert(fact);
        if fact.stale {
            stale.push(fact);
        } else {
            if !fact.superseded {
                fact.superseded = true;
                fact.superseded_by = winner_id;
            }
            superseded.push(fact);
        }
    }
    serde_json::to_value(Standing {
        track: found.parent().clone(),
        kind: query.kind,
        standing,
        reason: Explanation {
            code: reason,
            message: reason.message().to_owned(),
            source_priority: priority,
        },
        superseded,
        stale,
        rules_file: stems.sidecar_dir().join(RULES_FILE),
    })
    .map_err(|e| ServiceError::new(crate::codes::SERVICE_FAILED, e.to_string()))
}
