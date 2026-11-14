//! A key writer re-attests existing legacy Measurements; it does not analyze.
use crate::{
    ledger::{append_in, Action, NewEntry, Store},
    store::{read_fact, record_in, select},
    Fact, Fingerprint, Kind, Nature, NewFact, Result, RunId, Sidecar, SidecarError, Source,
    SourceKind,
};
use rusqlite::OptionalExtension;
use serde_json::json;

pub const SOURCE: &str = "wreckordbox-key-tag";
pub const KIND: &str = "key_evidence_carry";
pub const BATCH: usize = 128;
pub fn source() -> Source {
    Source::new(SourceKind::Tool, SOURCE)
}

impl Sidecar {
    /// Caller is the key writer with a just-published byte-verified copy. Only
    /// these captured, still-live Facts can carry: never a track-wide alias.
    /// Copies keep original measurement provenance/value/confidence and a new
    /// *legacy* fingerprint. The explicit entry attributes the copying act to
    /// the writer and names each old/new Fact. Original rows are not rewritten.
    /// The copies, supersessions and entry commit together, or none do.
    pub fn carry_key_evidence(
        &self,
        mut run: Option<RunId>,
        before: &Fingerprint,
        after: &Fingerprint,
        candidates: &[Fact],
    ) -> Result<(Option<RunId>, Vec<Fact>)> {
        if candidates.len() > BATCH
            || before == after
            || ![before, after]
                .iter()
                .all(|fp| fp.as_str().starts_with("a1:") || fp.as_str().starts_with("f1:"))
        {
            return Err(SidecarError::Invalid(
                "Key evidence needs a bounded batch and distinct legacy fingerprints".into(),
            ));
        }
        let mut db = self.db();
        let tx = db.transaction()?;
        let mut copies = Vec::new();
        let mut links = Vec::new();
        for captured in candidates {
            let Some(old) = tx
                .query_row(&select("WHERE fact.id=?1"), [captured.id], read_fact)
                .optional()?
            else {
                continue;
            };
            if !old.is_live()
                || old.nature != Nature::Measurement
                || old.track_id != captured.track_id
                || old.fingerprint.as_ref() != Some(before)
            {
                continue;
            }
            let recorded_run = if let Some(run) = run {
                run
            } else {
                let source = source();
                tx.execute("INSERT INTO run (source_kind,source_name,description,started_at) VALUES (?1,?2,?3,?4)",
                    rusqlite::params![source.kind.as_str(),source.name,"Key writer verified unchanged audio evidence",crate::store::now()])?;
                let id = RunId(tx.last_insert_rowid());
                tx.execute("INSERT INTO ledger_owned_run(run_id) VALUES (?1)", [id.0])?;
                run = Some(id);
                id
            };
            let new = record_in(
                &tx,
                &NewFact {
                    track_id: &old.track_id,
                    fingerprint: Some(after),
                    kind: Kind {
                        name: &old.kind,
                        nature: Nature::Measurement,
                    },
                    value: old.value.clone(),
                    source: &old.source,
                    confidence: old.confidence,
                    run: recorded_run,
                },
            )?;
            links.push(
                json!({"track":old.track_id,"kind":old.kind,"from_fact":old.id,
                "to_fact":new.id,"measurement_source":old.source,"measurement_run":old.run}),
            );
            copies.push(new);
        }
        if let Some(run) = run.filter(|_| !copies.is_empty()) {
            let payload = json!({"verification":"key_writer_byte_for_byte", "before":before,
                "after":after,"evidence_version":1,"facts":links,
                "not_reversible_reason":"Carry-over preserves measurement provenance; no new analysis or version-2 evidence was produced"});
            append_in(
                &tx,
                &NewEntry {
                    id: None,
                    run,
                    store: Store::Sidecar,
                    source: &source(),
                    at: None,
                    action: Action::Write,
                    kind: KIND,
                    label: "Key writer carried verified unchanged audio evidence",
                    command: None,
                    reverses: None,
                    part_of: None,
                    reversible: false,
                    format: 1,
                    payload: Some(&payload),
                },
            )?;
        }
        tx.commit()?;
        Ok((run, copies))
    }
}
