//! Archive queries share the immutable snapshot path; verification only
//! changes the answer. Producers alone observe/re-attach under Fact coordination.
use crate::{
    decisions::{self, DecisionPage, GetDecision, ListDecisions, DEFAULT_DECISION_LIMIT},
    facts::{self, DeclaredKinds, Fact, FactsOfTrack, TrackFacts},
    mutant_reads::{inspect, Store},
    protocol::{Request, ServiceError},
    stems::{self, ParentAudio, Stems},
};
use serde_json::Value;
use wreckordbox_sidecar::{Kinds, Nature, SidecarError};

pub(crate) fn facts(
    stems: &Stems,
    request: &Request,
    lookup: &dyn Fn(&str) -> Result<ParentAudio, ServiceError>,
) -> Result<Value, ServiceError> {
    let kinds = Kinds::load(stems.sidecar_dir()).map_err(|e| facts::sidecar_error(&e));
    if request.command == facts::commands::FACTS_KINDS {
        let kinds = kinds?;
        return encode(DeclaredKinds {
            kinds: kinds.all().to_vec(),
            file: kinds.file().to_path_buf(),
        });
    }
    let wanted: FactsOfTrack = facts::arguments(request)?;
    let found = lookup(&wanted.id)?;
    if let Some(kind) = &wanted.kind {
        let kinds = kinds.as_ref().map_err(Clone::clone)?;
        if kinds.get(kind).is_none() && !kinds.schema_is_broken() {
            return Err(facts::not_declared(kind, kinds));
        }
    }
    let audio = if wanted.check_audio {
        Some(stems::fingerprint(found.parent(), found.path())?)
    } else {
        None
    };
    let (stored, review) = inspect(stems, |store| {
        let stored = match &wanted.kind {
            Some(kind) => store.facts(&wanted.id, kind)?,
            None => match store {
                Store::Open(s) => s.facts_of_track(&wanted.id)?,
                Store::Snapshot(s) => s.facts_of_track(&wanted.id)?,
                Store::Absent => Vec::new(),
            },
        };
        let review = match store {
            Store::Open(s) => s.attachment_reviews(&wanted.id)?,
            Store::Snapshot(s) => s.attachment_reviews(&wanted.id)?,
            Store::Absent => Vec::new(),
        };
        Ok((stored, review))
    })?;
    let facts: Vec<Fact> = stored
        .into_iter()
        .map(|stored| {
            let mut fact = Fact::from(stored);
            if let Some(audio) = &audio {
                if fact.nature == Nature::Measurement {
                    fact.stale = !fact
                        .fingerprint
                        .as_ref()
                        .is_some_and(|stored| audio.matches_stored(stored));
                }
            }
            if let Some(kind) = kinds.as_ref().ok().and_then(|k| k.get(&fact.kind)) {
                fact.value = kind.normalise(fact.value);
            }
            fact
        })
        .collect();
    encode(TrackFacts {
        track: found.parent().clone(),
        stale: facts.iter().filter(|f| f.stale).cloned().collect(),
        unverified: facts
            .iter()
            .filter(|f| !f.stale && f.nature == Nature::Measurement && audio.is_none())
            .cloned()
            .collect(),
        facts: facts
            .into_iter()
            .filter(|f| {
                wanted.all
                    || (!f.superseded
                        && !f.stale
                        && (f.nature == Nature::Judgement || audio.is_some()))
            })
            .collect(),
        reattached: Vec::new(),
        review,
    })
}

pub(crate) fn decisions(stems: &Stems, request: &Request) -> Result<Value, ServiceError> {
    if request.command == decisions::commands::DECISIONS_GET {
        let wanted: GetDecision = facts::arguments(request)?;
        let found = inspect(stems, |store| match store {
            Store::Open(s) => s.decision(wanted.id),
            Store::Snapshot(s) => s.decision(wanted.id),
            Store::Absent => Ok(None),
        })?;
        return encode(
            found.ok_or_else(|| decisions::sidecar_error(&SidecarError::NoDecision(wanted.id)))?,
        );
    }
    let wanted: ListDecisions = facts::arguments(request)?;
    let limit = wanted.limit.unwrap_or(DEFAULT_DECISION_LIMIT);
    let offset = wanted.offset.unwrap_or(0);
    let query = wreckordbox_sidecar::decisions::DecisionQuery {
        state: wanted.state,
        decider: wanted.decider.as_ref().map(facts::source).transpose()?,
        track_id: wanted.track,
        topic: wanted.topic,
        limit,
        offset,
    };
    let (decisions, total) = inspect(stems, |store| match store {
        Store::Open(s) => s.decisions(&query),
        Store::Snapshot(s) => s.decisions(&query),
        Store::Absent => Ok((Vec::new(), 0)),
    })?;
    let end = offset.saturating_add(decisions.len());
    encode(DecisionPage {
        next_offset: (end < total && !decisions.is_empty()).then_some(end),
        decisions,
        total,
        offset,
        limit,
    })
}
fn encode(value: impl serde::Serialize) -> Result<Value, ServiceError> {
    serde_json::to_value(value)
        .map_err(|e| ServiceError::new(crate::codes::SERVICE_FAILED, e.to_string()))
}
