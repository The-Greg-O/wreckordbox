//! Decisions: recorded judgements between fixed candidates, and the answers
//! Deciders gave them.
//!
//! ```text
//! decision(id, question_type, question, context, candidates, track_id, run_id, created_at)
//! decision_answer(id, decision_id, decider_kind, decider_name, choice,
//!                 choice_probability, probabilities, reason, fact_id, run_id, answered_at)
//! ```
//!
//! Both tables are append-only: a row is written once and never changed or
//! deleted. A Decision is the question as it was asked (its type, its
//! text, the context supplied and the candidates). An answer is one
//! Decider's choice among those candidates. A Decision with no answer is
//! pending. The same Decision can be answered again, by another Decider or
//! by the same one, and every answer is kept, which is what asking old
//! Decisions of a different Decider and comparing them needs.
//!
//! # How sure: a probability, not a confidence
//!
//! `choice_probability` is the probability the Decider gives to the
//! candidate it chose being the right one, from 0 to 1. `probabilities` is
//! the Decider's probability for each candidate, when it supplied them.
//! Neither is called a confidence, on purpose: decision models report a
//! field of that name which measures how concentrated their probabilities
//! are, not how likely the answer is to be right. A threshold reads
//! `choice_probability`.
//!
//! # Question types
//!
//! - [`QuestionType::Choice`]: one of two or more named candidates.
//! - [`QuestionType::YesNo`]: whether the question, a statement, is true.
//!   Its candidates are always [`YES`] and [`NO`], and its answers always
//!   carry both probabilities, so `probabilities.yes` is the probability
//!   that the statement is true.

use std::collections::BTreeMap;

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::store::now;
use crate::{Result, RunId, Sidecar, SidecarError, Source, SourceKind};

/// The tables Decisions add to the store.
pub(crate) const SCHEMA: &str = "
CREATE TABLE decision (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    question_type TEXT NOT NULL CHECK (question_type IN ('choice', 'yes_no')),
    question      TEXT NOT NULL,
    context       TEXT NOT NULL,
    candidates    TEXT NOT NULL,
    track_id      TEXT,
    run_id        INTEGER REFERENCES run (id),
    created_at    INTEGER NOT NULL
) STRICT;

CREATE TABLE decision_answer (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    decision_id        INTEGER NOT NULL REFERENCES decision (id),
    decider_kind       TEXT NOT NULL CHECK (decider_kind IN ('dj', 'agent', 'tool', 'web')),
    decider_name       TEXT NOT NULL,
    choice             TEXT NOT NULL,
    choice_probability REAL NOT NULL CHECK (choice_probability >= 0 AND choice_probability <= 1),
    probabilities      TEXT,
    reason             TEXT,
    fact_id            INTEGER REFERENCES fact (id),
    run_id             INTEGER REFERENCES run (id),
    answered_at        INTEGER NOT NULL
) STRICT;

CREATE INDEX decision_by_track ON decision (track_id);
CREATE INDEX decision_answer_by_decision ON decision_answer (decision_id);
CREATE INDEX decision_answer_by_decider ON decision_answer (decider_kind, decider_name);
";

/// The candidate of a yes-or-no question that says the statement is true.
pub const YES: &str = "yes";
/// The candidate of a yes-or-no question that says the statement is false.
pub const NO: &str = "no";

/// How far the probabilities of an answer may add up past 1, and how far a
/// stated probability of the choice may differ from the one among them,
/// before it is refused: room for rounding, nothing more.
const TOLERANCE: f64 = 1e-6;

/// What kind of question a Decision asks, which fixes what its candidates
/// are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionType {
    /// One of two or more named candidates.
    Choice,
    /// Whether a statement is true: the candidates are `yes` and `no`.
    YesNo,
}

impl QuestionType {
    /// The name as stored and as written on the wire.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Choice => "choice",
            Self::YesNo => "yes_no",
        }
    }

    fn from_stored(text: &str) -> Self {
        if text == "yes_no" {
            Self::YesNo
        } else {
            Self::Choice
        }
    }
}

/// One of the fixed things a Decision chooses between.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    /// What an answer names to choose it. Compared exactly.
    pub name: String,
    /// What the candidate means, for a Decider that needs telling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Candidate {
    #[must_use]
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
        }
    }
}

/// A Decision to ask.
#[derive(Debug, Clone)]
pub struct NewDecision<'a> {
    pub question_type: QuestionType,
    /// The question, or for [`QuestionType::YesNo`] the statement.
    pub question: &'a str,
    /// What a Decider is given to answer from: a JSON object.
    pub context: &'a Value,
    /// Two or more, each named once. Leave empty for
    /// [`QuestionType::YesNo`]: they are `yes` and `no`.
    pub candidates: &'a [Candidate],
    /// The track the question is about, when it is about one.
    pub track_id: Option<&'a str>,
    /// The Run the question was asked in, when it was asked in one.
    pub run: Option<RunId>,
}

/// One Decider's answer to a Decision.
#[derive(Debug, Clone)]
pub struct NewAnswer<'a> {
    /// The Decision's identifier.
    pub decision: i64,
    /// Who answers.
    pub decider: &'a Source,
    /// The name of one of the Decision's candidates.
    pub choice: &'a str,
    /// The probability that `choice` is right, 0 to 1. May be left out
    /// when `probabilities` holds it.
    pub choice_probability: Option<f64>,
    /// The Decider's probability for each candidate, when it has them.
    /// Every name must be a candidate; candidates may be missing.
    pub probabilities: Option<&'a BTreeMap<String, f64>>,
    /// Why, in the Decider's words.
    pub reason: Option<&'a str>,
    /// The Fact this answer produced, when it produced one.
    pub fact_id: Option<i64>,
    /// The Run the answer was given in, when it was given in one.
    pub run: Option<RunId>,
}

/// An answer as the Sidecar holds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    pub id: i64,
    pub decision_id: i64,
    /// Who answered.
    pub decider: Source,
    /// The candidate chosen.
    pub choice: String,
    /// The probability the Decider gives to `choice` being right, 0 to 1.
    /// Not a model's "confidence"; see the module documentation.
    pub choice_probability: f64,
    /// The Decider's probability for each candidate it gave one for, by
    /// candidate name. Always both for a yes-or-no question.
    pub probabilities: Option<BTreeMap<String, f64>>,
    pub reason: Option<String>,
    /// The Fact this answer produced.
    pub fact_id: Option<i64>,
    pub run: Option<RunId>,
    /// Milliseconds since the Unix epoch.
    pub answered_at: i64,
}

/// A Decision as the Sidecar holds it, with every answer it has been given.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub id: i64,
    pub question_type: QuestionType,
    pub question: String,
    pub context: Value,
    pub candidates: Vec<Candidate>,
    pub track_id: Option<String>,
    pub run: Option<RunId>,
    /// Milliseconds since the Unix epoch.
    pub created_at: i64,
    /// No Decider has answered yet.
    pub pending: bool,
    /// Every answer, oldest first.
    pub answers: Vec<Answer>,
}

/// Which Decisions a listing wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionState {
    /// Not answered.
    #[default]
    Pending,
    Answered,
    All,
}

/// A listing of Decisions, oldest first.
#[derive(Debug, Clone, Default)]
pub struct DecisionQuery {
    pub state: DecisionState,
    /// With a Decider, "answered" and "pending" mean answered, or not yet
    /// answered, by that Decider; without, by anyone.
    pub decider: Option<Source>,
    /// Only Decisions about this track.
    pub track_id: Option<String>,
    pub limit: usize,
    pub offset: usize,
}

impl Sidecar {
    /// Records a Decision to be answered. It is pending until
    /// [`answer_decision`](Self::answer_decision) is called for it.
    pub fn create_decision(&self, new: &NewDecision) -> Result<Decision> {
        if new.question.trim().is_empty() {
            return Err(SidecarError::Decision(
                "a Decision needs a question".to_owned(),
            ));
        }
        if !new.context.is_object() {
            return Err(SidecarError::Decision(
                "a Decision's context is a JSON object: named things a Decider answers from"
                    .to_owned(),
            ));
        }
        let candidates = candidates_of(new.question_type, new.candidates)?;
        let db = self.db();
        db.execute(
            "INSERT INTO decision (question_type, question, context, candidates, track_id, run_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                new.question_type.as_str(),
                new.question.trim(),
                new.context.to_string(),
                serde_json::to_string(&candidates).unwrap_or_default(),
                new.track_id,
                new.run.map(|run| run.0),
                now(),
            ],
        )?;
        let id = db.last_insert_rowid();
        read_decision(&db, id)?.ok_or(SidecarError::NoDecision(id))
    }

    /// Records one Decider's answer to a Decision and answers the Decision
    /// with it. Refused, recording nothing, when the choice is not one of
    /// the candidates or a probability is not one. Earlier answers are
    /// kept beside it.
    pub fn answer_decision(&self, new: &NewAnswer) -> Result<(Decision, Answer)> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let decision =
            read_decision(&tx, new.decision)?.ok_or(SidecarError::NoDecision(new.decision))?;
        let (choice_probability, probabilities) = checked_answer(&decision, new)?;
        tx.execute(
            "INSERT INTO decision_answer (decision_id, decider_kind, decider_name, choice,
                                          choice_probability, probabilities, reason, fact_id, run_id, answered_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                new.decision,
                new.decider.kind.as_str(),
                new.decider.name,
                new.choice,
                choice_probability,
                probabilities
                    .as_ref()
                    .map(|all| serde_json::to_string(all).unwrap_or_default()),
                new.reason.map(str::trim).filter(|reason| !reason.is_empty()),
                new.fact_id,
                new.run.map(|run| run.0),
                now(),
            ],
        )?;
        let id = tx.last_insert_rowid();
        let decision =
            read_decision(&tx, new.decision)?.ok_or(SidecarError::NoDecision(new.decision))?;
        tx.commit()?;
        let answer = decision
            .answers
            .iter()
            .find(|answer| answer.id == id)
            .cloned()
            .ok_or(SidecarError::NoDecision(new.decision))?;
        Ok((decision, answer))
    }

    /// Checks an answer as [`answer_decision`](Self::answer_decision)
    /// would, recording nothing: the probability of the choice and the
    /// probabilities that would be stored.
    pub fn check_answer(&self, new: &NewAnswer) -> Result<f64> {
        let decision = self
            .decision(new.decision)?
            .ok_or(SidecarError::NoDecision(new.decision))?;
        Ok(checked_answer(&decision, new)?.0)
    }

    /// One Decision by its identifier, with its answers.
    pub fn decision(&self, id: i64) -> Result<Option<Decision>> {
        read_decision(&self.db(), id)
    }

    /// A page of Decisions, oldest first, and how many there are before
    /// the limit.
    pub fn decisions(&self, query: &DecisionQuery) -> Result<(Vec<Decision>, usize)> {
        // ?1 track, ?2 and ?3 the Decider; each may be NULL for "any".
        let answered = "EXISTS (SELECT 1 FROM decision_answer AS answer
                                WHERE answer.decision_id = decision.id
                                  AND (?2 IS NULL OR (answer.decider_kind = ?2 AND answer.decider_name = ?3)))";
        let state = match query.state {
            DecisionState::Pending => format!("NOT {answered}"),
            DecisionState::Answered => answered.to_owned(),
            // The Decider still binds, so the statement always has three
            // parameters.
            DecisionState::All => "(?2 IS NULL OR ?3 IS NULL OR 1)".to_owned(),
        };
        let filter = format!("WHERE (?1 IS NULL OR decision.track_id = ?1) AND {state}");
        let decider_kind = query.decider.as_ref().map(|decider| decider.kind.as_str());
        let decider_name = query.decider.as_ref().map(|decider| decider.name.as_str());
        let bound = params![query.track_id, decider_kind, decider_name];
        let db = self.db();
        let total: i64 = db.query_row(
            &format!("SELECT COUNT(*) FROM decision {filter}"),
            bound,
            |row| row.get(0),
        )?;
        let mut statement = db.prepare(&format!(
            "SELECT id FROM decision {filter} ORDER BY id LIMIT {} OFFSET {}",
            i64::try_from(query.limit).unwrap_or(i64::MAX),
            i64::try_from(query.offset).unwrap_or(i64::MAX),
        ))?;
        let ids = statement
            .query_map(bound, |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<i64>>>()?;
        let mut decisions = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(decision) = read_decision(&db, id)? {
                decisions.push(decision);
            }
        }
        Ok((decisions, usize::try_from(total).unwrap_or(0)))
    }
}

/// The candidates a Decision of this type is stored with.
fn candidates_of(question_type: QuestionType, given: &[Candidate]) -> Result<Vec<Candidate>> {
    if question_type == QuestionType::YesNo {
        let names: Vec<&str> = given.iter().map(|one| one.name.as_str()).collect();
        if !(names.is_empty() || names == [YES, NO]) {
            return Err(SidecarError::Candidates(format!(
                "a yes-or-no question has the candidates {YES} and {NO} and takes no others"
            )));
        }
        return Ok(vec![Candidate::named(YES), Candidate::named(NO)]);
    }
    if given.len() < 2 {
        return Err(SidecarError::Candidates(format!(
            "a choice is between two or more candidates, not {}",
            given.len()
        )));
    }
    let mut seen: Vec<&str> = Vec::with_capacity(given.len());
    for candidate in given {
        let name = candidate.name.as_str();
        if name.trim().is_empty() || name.trim() != name {
            return Err(SidecarError::Candidates(
                "a candidate needs a name, with no space before or after it".to_owned(),
            ));
        }
        if seen.contains(&name) {
            return Err(SidecarError::Candidates(format!(
                "the candidate \"{name}\" is given twice"
            )));
        }
        seen.push(name);
    }
    Ok(given.to_vec())
}

/// Checks an answer against its Decision: the probability of the choice and
/// the probabilities to store.
fn checked_answer(
    decision: &Decision,
    new: &NewAnswer,
) -> Result<(f64, Option<BTreeMap<String, f64>>)> {
    let is_candidate = |name: &str| decision.candidates.iter().any(|one| one.name == name);
    if !is_candidate(new.choice) {
        return Err(SidecarError::NotACandidate {
            decision: decision.id,
            choice: new.choice.to_owned(),
            candidates: decision
                .candidates
                .iter()
                .map(|one| one.name.clone())
                .collect(),
        });
    }
    let unit = |what: &str, probability: f64| {
        if (0.0..=1.0).contains(&probability) {
            Ok(())
        } else {
            Err(SidecarError::Probability(format!(
                "the probability of {what} is from 0 to 1, not {probability}"
            )))
        }
    };
    let mut probabilities = new.probabilities.cloned();
    if let Some(all) = &probabilities {
        for (name, probability) in all {
            if !is_candidate(name) {
                return Err(SidecarError::Probability(format!(
                    "a probability is given for \"{name}\", which is not a candidate of Decision {}",
                    decision.id
                )));
            }
            unit(&format!("\"{name}\""), *probability)?;
        }
        let sum: f64 = all.values().sum();
        if sum > 1.0 + TOLERANCE {
            return Err(SidecarError::Probability(format!(
                "the probabilities of the candidates add up to {sum}, which is more than 1"
            )));
        }
    }
    let among = probabilities
        .as_ref()
        .and_then(|all| all.get(new.choice).copied());
    let choice_probability = match (new.choice_probability, among) {
        (Some(stated), Some(among)) if (stated - among).abs() > TOLERANCE => {
            return Err(SidecarError::Probability(format!(
                "the probability of the choice is given as {stated} and, among the candidates', as {among}"
            )));
        }
        (Some(stated), _) => stated,
        (None, Some(among)) => among,
        (None, None) => {
            return Err(SidecarError::Probability(
                "say how sure the Decider is: the probability, from 0 to 1, that its choice is right"
                    .to_owned(),
            ));
        }
    };
    unit("the choice", choice_probability)?;
    if decision.question_type == QuestionType::YesNo {
        // Two candidates that exclude each other: one probability fixes
        // the other, so both are always stored.
        let all = probabilities.get_or_insert_with(BTreeMap::new);
        all.insert(new.choice.to_owned(), choice_probability);
        let other = if new.choice == YES { NO } else { YES };
        let rest = (1.0 - choice_probability).clamp(0.0, 1.0);
        match all.get(other) {
            Some(given) if (given - rest).abs() > TOLERANCE => {
                return Err(SidecarError::Probability(format!(
                    "the probabilities of {YES} and {NO} must add up to 1; they add up to {}",
                    given + choice_probability
                )));
            }
            Some(_) => {}
            None => {
                all.insert(other.to_owned(), rest);
            }
        }
    }
    Ok((choice_probability, probabilities))
}

fn read_decision(db: &Connection, id: i64) -> Result<Option<Decision>> {
    let Some(mut decision) = db
        .query_row(
            "SELECT id, question_type, question, context, candidates, track_id, run_id, created_at
             FROM decision WHERE id = ?1",
            [id],
            |row| {
                Ok(Decision {
                    id: row.get(0)?,
                    question_type: QuestionType::from_stored(&row.get::<_, String>(1)?),
                    question: row.get(2)?,
                    context: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or(Value::Null),
                    candidates: serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or_default(),
                    track_id: row.get(5)?,
                    run: row.get::<_, Option<i64>>(6)?.map(RunId),
                    created_at: row.get(7)?,
                    pending: true,
                    answers: Vec::new(),
                })
            },
        )
        .optional()?
    else {
        return Ok(None);
    };
    let mut statement = db.prepare(
        "SELECT id, decision_id, decider_kind, decider_name, choice, choice_probability,
                probabilities, reason, fact_id, run_id, answered_at
         FROM decision_answer WHERE decision_id = ?1 ORDER BY id",
    )?;
    decision.answers = statement
        .query_map([id], read_answer)?
        .collect::<rusqlite::Result<Vec<Answer>>>()?;
    decision.pending = decision.answers.is_empty();
    Ok(Some(decision))
}

fn read_answer(row: &Row) -> rusqlite::Result<Answer> {
    Ok(Answer {
        id: row.get(0)?,
        decision_id: row.get(1)?,
        decider: Source {
            kind: SourceKind::from_stored(&row.get::<_, String>(2)?),
            name: row.get(3)?,
        },
        choice: row.get(4)?,
        choice_probability: row.get(5)?,
        probabilities: row
            .get::<_, Option<String>>(6)?
            .and_then(|all| serde_json::from_str(&all).ok()),
        reason: row.get(7)?,
        fact_id: row.get(8)?,
        run: row.get::<_, Option<i64>>(9)?.map(RunId),
        answered_at: row.get(10)?,
    })
}
