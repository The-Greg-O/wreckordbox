//! The key writer alone may carry legacy evidence after verified publication.
use super::{bulk, Keeper, WindowRun};
use rbl_db::key_tag::prepared::{Prepared, Verified};
use std::{collections::BTreeMap, sync::Weak, time::Instant};
use wreckordbox_sidecar::{
    fingerprint_file, key_evidence as stored, Fact, Fingerprint, Nature, RunId, Sidecar,
};

struct Observation {
    before: Fingerprint,
    after: Fingerprint,
    facts: Vec<Fact>,
}
struct Publication {
    keeper: Weak<Keeper>,
    proof: Verified<Observation>,
}
pub(super) fn prepare(
    keeper: &Keeper,
    file: &Prepared,
    tracks: &[String],
) -> Result<Option<Box<dyn rbl_library::key_evidence::KeyEvidence>>, String> {
    let facts = crate::mutant_reads::inspect(&keeper.stems, |store| {
        let mut facts = Vec::new();
        for track in tracks {
            facts.extend(match &store {
                crate::mutant_reads::Store::Open(s) => s.facts_of_track(track)?,
                crate::mutant_reads::Store::Snapshot(s) => s.facts_of_track(track)?,
                crate::mutant_reads::Store::Absent => Vec::new(),
            });
        }
        Ok(facts)
    })
    .map_err(|e| e.message)?;
    let candidates: Vec<_> = facts
        .into_iter()
        .filter(|f| {
            f.is_live()
                && f.nature == Nature::Measurement
                && f.fingerprint.as_ref().is_some_and(|fp| {
                    fp.as_str().starts_with("a1:") || fp.as_str().starts_with("f1:")
                })
        })
        .collect();
    if candidates.is_empty() {
        return Ok(None);
    }
    let proof = file
        .inspect_verified(|before, after| {
            let read = |path| {
                fingerprint_file(path)
                    .map_err(|e| rbl_db::key_tag::KeyTagError::Verify(e.to_string()))
            };
            let before = read(before)?;
            let after = read(after)?;
            if before.as_str().starts_with("a2:")
                && after.as_str().starts_with("a2:")
                && before != after
            {
                return Err(rbl_db::key_tag::KeyTagError::Verify(
                    "decoding identity changed during the key rewrite".into(),
                ));
            }
            let facts = candidates
                .into_iter()
                .filter(|f| {
                    f.fingerprint
                        .as_ref()
                        .is_some_and(|fp| before.matches_stored(fp))
                })
                .collect();
            Ok(Observation {
                before: before.legacy(),
                after: after.legacy(),
                facts,
            })
        })
        .map_err(|e| e.to_string())?;
    let observed = proof.observed();
    if observed.before == observed.after || observed.facts.is_empty() {
        return Ok(None);
    }
    Ok(Some(Box::new(Publication {
        keeper: keeper.weak.clone(),
        proof,
    })))
}
impl rbl_library::key_evidence::KeyEvidence for Publication {
    fn record_key_evidence(&self, file: &Prepared) -> Result<(), String> {
        let observed = file
            .published_verification(&self.proof)
            .ok_or_else(|| "This key writer did not publish the verified audio".to_owned())?;
        let keeper = self
            .keeper
            .upgrade()
            .ok_or_else(|| "Library service stopped".to_owned())?;
        let _access = keeper.stems.bulk_access()?;
        let sidecar = keeper.stems.sidecar().map_err(|e| e.message)?;
        let mut run = key_run(&keeper, &sidecar)?;
        let in_bulk = bulk::active(&keeper).is_some();
        let mut changed: BTreeMap<(String, String), Vec<i64>> = BTreeMap::new();
        let mut failure = None;
        for chunk in observed.facts.chunks(stored::BATCH) {
            match sidecar.carry_key_evidence(run, &observed.before, &observed.after, chunk) {
                Ok((recorded_run, facts)) => {
                    run = recorded_run;
                    if !in_bulk {
                        if let Some(run) = run {
                            keeper.state().key_writer = Some(WindowRun {
                                run,
                                last_edit: Instant::now(),
                            });
                        }
                    }
                    for fact in facts {
                        changed
                            .entry((fact.track_id, fact.kind))
                            .or_default()
                            .push(fact.id);
                    }
                }
                Err(why) => {
                    failure = Some(why.to_string());
                    break;
                }
            }
        }
        for ((track, kind), ids) in changed {
            keeper.stems.publish(
                crate::stems::events::SIDECAR_CHANGED,
                serde_json::json!({
                "track_id":track,"kind":kind,"fact_id":ids.last(),"fact_ids":ids,"stale":[]}),
            );
        }
        failure.map_or(Ok(()), Err)
    }
}

/// Same Run as an active bulk operation. Hand edits have a separate tool Run,
/// coalesced like the window's Run; their DJ attribution is never inherited.
/// Called only by publication under the core's existing single writer gate.
fn key_run(keeper: &Keeper, sidecar: &Sidecar) -> Result<Option<RunId>, String> {
    if let Some(active) = bulk::active(keeper) {
        return active.import_target().0.map(Some);
    }
    let previous = {
        let mut state = keeper.state();
        if let Some(run) = state.key_writer.as_mut() {
            if run.last_edit.elapsed() < keeper.pause {
                run.last_edit = Instant::now();
                return Ok(Some(run.run));
            }
        }
        state.key_writer.take().map(|r| r.run)
    };
    if let Some(run) = previous {
        sidecar
            .finish_run(run, "key writer paused")
            .map_err(|e| e.to_string())?;
    }
    Ok(None)
}
