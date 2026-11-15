//! A lookup check as the Sidecar keeps it (wbx-37): the Decision that a
//! candidate found by an agent is, or is not, the track; its one answer;
//! and, when the candidate was accepted, the Facts its fields became, each
//! with the web address it came from as its source. All of it in one Run,
//! written in one transaction: a check that fails part way leaves nothing.
//!
//! No new table: a Fact carries its address in its source
//! (`web:<address>`), and the date of the lookup in its Run, whose
//! Decision holds the address and the date in its context and whose
//! description names both.

use rusqlite::OptionalExtension;
use serde_json::Value;

use crate::decisions::{
    answered, checked_answer, insert_answer, insert_question, read_decision, Answer, Decision,
    NewAnswer, NewDecision, RunOfAnswer,
};
use crate::store::{now, record_in};
use crate::{Fact, Fingerprint, Kind, NewFact, Result, RunId, Sidecar, SidecarError, Source};

/// A Fact an accepted candidate gives: one of its fields, about the
/// track, from the address the candidate came from.
#[derive(Debug, Clone)]
pub struct LookupFact<'a> {
    pub kind: Kind<'a>,
    pub value: Value,
    /// `web:<address>`.
    pub source: &'a Source,
    pub confidence: f64,
}

/// A checked candidate to keep.
#[derive(Debug, Clone)]
pub struct LookupCheck<'a> {
    /// The question as asked. Its `run` is left out: it is the check's.
    pub decision: NewDecision<'a>,
    /// Who checked (`tool:lookup-check`).
    pub decider: &'a Source,
    /// `accepted`, `held` or `rejected`.
    pub choice: &'a str,
    pub choice_probability: f64,
    pub reason: &'a str,
    /// The track the Facts are about, and its audio's fingerprint now.
    pub track_id: &'a str,
    pub fingerprint: Option<&'a Fingerprint>,
    /// Empty unless the candidate was accepted.
    pub facts: &'a [LookupFact<'a>],
    /// The Run it all belongs to: an open one, or one made for the check
    /// (by the decider) and ended with it.
    pub run: RunOfAnswer<'a>,
}

/// What [`Sidecar::record_lookup_check`] wrote.
#[derive(Debug, Clone)]
pub struct LookupRecorded {
    pub decision: Decision,
    pub answer: Answer,
    pub facts: Vec<Fact>,
    pub run: RunId,
}

impl Sidecar {
    /// Records a lookup check: the Decision, its answer and the accepted
    /// candidate's Facts, in one Run and one transaction. Any refusal
    /// (a Run that has ended, a value the store refuses) writes nothing.
    pub fn record_lookup_check(&self, check: &LookupCheck) -> Result<LookupRecorded> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let run = run_of_check(&tx, check.run, check.decider)?;
        let id = insert_question(
            &tx,
            &NewDecision {
                run: Some(run),
                ..check.decision.clone()
            },
            Some(check.decider),
        )?;
        let decision = read_decision(&tx, id)?.ok_or(SidecarError::NoDecision(id))?;
        let new = NewAnswer {
            decision: id,
            decider: check.decider,
            choice: check.choice,
            choice_probability: Some(check.choice_probability),
            probabilities: None,
            reason: Some(check.reason),
            fact_id: None,
            run: Some(run),
        };
        let checked = checked_answer(&decision, &new)?;
        let facts = check
            .facts
            .iter()
            .map(|fact| {
                record_in(
                    &tx,
                    &NewFact {
                        track_id: check.track_id,
                        fingerprint: check.fingerprint,
                        kind: fact.kind,
                        value: fact.value.clone(),
                        source: fact.source,
                        confidence: fact.confidence,
                        run,
                    },
                )
            })
            .collect::<Result<Vec<Fact>>>()?;
        let answer = insert_answer(&tx, &new, &checked, None, Some(run))?;
        if matches!(check.run, RunOfAnswer::Made { .. }) {
            let outcome = match facts.len() {
                0 => format!("checked: {}, no Facts recorded", check.choice),
                1 => format!("checked: {}, recorded 1 Fact", check.choice),
                n => format!("checked: {}, recorded {n} Facts", check.choice),
            };
            tx.execute(
                "UPDATE run SET finished_at = ?2, outcome = ?3 WHERE id = ?1",
                rusqlite::params![run.0, now(), outcome],
            )?;
        }
        let (decision, answer) = answered(&tx, id, answer)?;
        tx.commit()?;
        Ok(LookupRecorded {
            decision,
            answer,
            facts,
            run,
        })
    }
}

/// The Run a check belongs to, inside its transaction: the open Run
/// named, checked to be open, or a new one by `decider`.
fn run_of_check(tx: &rusqlite::Transaction, run: RunOfAnswer, decider: &Source) -> Result<RunId> {
    match run {
        RunOfAnswer::Open(run) => {
            let finished_at: Option<Option<i64>> = tx
                .query_row(
                    "SELECT finished_at FROM run WHERE id = ?1",
                    [run.0],
                    |row| row.get(0),
                )
                .optional()?;
            match finished_at {
                Some(None) => Ok(run),
                Some(Some(_)) => Err(SidecarError::Invalid(format!("Run {} has ended", run.0))),
                None => Err(SidecarError::Invalid(format!("there is no Run {}", run.0))),
            }
        }
        RunOfAnswer::Made { description } => {
            tx.execute(
                "INSERT INTO run (source_kind, source_name, description, started_at)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![decider.kind.as_str(), decider.name, description, now()],
            )?;
            Ok(RunId(tx.last_insert_rowid()))
        }
    }
}
