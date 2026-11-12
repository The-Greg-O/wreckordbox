//! Decisions: recorded judgements between fixed candidates, kept in the
//! Sidecar, and the Deciders that answer them.
//!
//! A Decision is the question as asked: its type, its text, the context
//! supplied and the candidates. It is pending until a Decider answers it.
//! A Decider is anything that answers that shape, identified as a source
//! (`agent:tagger`, `tool:rule-based`, `dj`): an agent answers through
//! `decisions.answer` under its own name, and the rule-based Decider
//! ([`crate::decider`]) through `decisions.decide`. Nothing registers a
//! Decider: its name on an answer is what attributes the answer to it.
//!
//! Decisions and answers are append-only. A Decision that has an answer can
//! be answered again, by another Decider or the same one, and every answer
//! is kept.
//!
//! How sure a Decider is, is `choice_probability`: the probability it gives
//! to its choice being right. It is not called a confidence, because the
//! field decision models call that measures how concentrated their
//! probabilities are, not how likely the answer is to be right. When a
//! Decider gives a probability for each candidate they are kept too, in
//! `probabilities`.
//!
//! An answer may also produce a Fact whose source is the Decider, when the
//! command says which kind and for which track. Whether that Fact is ever
//! projected is not decided here.
//!
//! Nothing here writes to the Library: no `library_changed`, no generation
//! change. A change is announced as
//! [`SIDECAR_CHANGED`](crate::stems::events::SIDECAR_CHANGED).

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
pub use wreckordbox_sidecar::decisions::{
    Answer, Candidate, Decision, DecisionState, QuestionType, NO, YES,
};
use wreckordbox_sidecar::decisions::{
    DecisionQuery, FactOfAnswer, NewAnswer, NewDecision, RunOfAnswer,
};
use wreckordbox_sidecar::{Kinds, RunId, Sidecar, SidecarError};

pub use crate::decider::Rule;
use crate::decider::{self, NoRuling};
use crate::facts::{self, Fact, FactToRecord, Run, Source, SourceArg, SourceKind, ValueType};
use crate::protocol::{codes as service_codes, Request, ServiceError};
use crate::stems::{self, ParentAudio, Stems};

/// The names of the Decisions commands.
pub mod commands {
    /// Asks a Decision. Takes a [`CreateDecision`](super::CreateDecision),
    /// answers the [`Decision`](super::Decision), pending.
    pub const DECISIONS_CREATE: &str = "decisions.create";
    /// A page of Decisions, oldest first. Takes a
    /// [`ListDecisions`](super::ListDecisions), answers a
    /// [`DecisionPage`](super::DecisionPage).
    pub const DECISIONS_LIST: &str = "decisions.list";
    /// One Decision with its answers. Takes a
    /// [`GetDecision`](super::GetDecision), answers the
    /// [`Decision`](super::Decision).
    pub const DECISIONS_GET: &str = "decisions.get";
    /// Answers a Decision as a named Decider. Takes an
    /// [`AnswerDecision`](super::AnswerDecision), answers an
    /// [`Answered`](super::Answered).
    pub const DECISIONS_ANSWER: &str = "decisions.answer";
    /// Has the rule-based Decider answer a Decision. Takes a
    /// [`DecideByRule`](super::DecideByRule), answers an
    /// [`Answered`](super::Answered).
    pub const DECISIONS_DECIDE: &str = "decisions.decide";
}

/// The error codes of the Decisions commands, beside the service's own and
/// those of [`facts::codes`]. Nothing is recorded when any is answered.
pub mod codes {
    /// There is no Decision with that identifier.
    pub const DECISION_NOT_FOUND: &str = "decision_not_found";
    /// The Decision cannot be asked as given: no question, or a context
    /// that is not a JSON object.
    pub const INVALID_DECISION: &str = "invalid_decision";
    /// The candidates cannot be asked between: fewer than two, one named
    /// twice or not at all, or candidates given to a yes-or-no question.
    pub const INVALID_CANDIDATES: &str = "invalid_candidates";
    /// The answer names none of the Decision's candidates.
    pub const ANSWER_NOT_A_CANDIDATE: &str = "answer_not_a_candidate";
    /// A probability is missing, is not from 0 to 1, is given for
    /// something that is not a candidate, or the probabilities disagree
    /// or add up to more than 1.
    pub const INVALID_PROBABILITY: &str = "invalid_probability";
    /// The rule cannot answer this Decision; it is left as it was.
    pub const RULE_NOT_APPLICABLE: &str = "rule_not_applicable";
    /// A Fact was asked for and neither the command nor the Decision says
    /// which track it is about.
    pub const FACT_NEEDS_TRACK: &str = "fact_needs_track";
}

/// The kind a [`SIDECAR_CHANGED`](crate::stems::events::SIDECAR_CHANGED)
/// event carries when what changed is a Decision, not a Fact.
pub const CHANGED_KIND: &str = "decision";

/// How many Decisions a listing answers when no limit is given.
pub const DEFAULT_DECISION_LIMIT: usize = 50;

/// A candidate as a command takes it: its name alone, or
/// `{"name": "Tech House", "description": "..."}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CandidateArg {
    Name(String),
    Described(Candidate),
}

impl From<CandidateArg> for Candidate {
    fn from(given: CandidateArg) -> Self {
        match given {
            CandidateArg::Name(name) => Candidate::named(name),
            CandidateArg::Described(candidate) => candidate,
        }
    }
}

/// The arguments of `decisions.create`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateDecision {
    pub question_type: QuestionType,
    /// What kind of question this is, in the asker's own word (`genre`,
    /// `lookup_match`): lower-case letters, digits and underscores.
    /// Decisions are listed, and Deciders compared, by it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    /// The question; for a yes-or-no question, the statement that is true
    /// or not.
    pub question: String,
    /// What a Decider answers from: a JSON object. Left out: `{}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<Value>,
    /// The candidates of a choice, two or more. Left out for a yes-or-no
    /// question, whose candidates are `yes` and `no`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<CandidateArg>,
    /// The track the question is about, when it is about one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<String>,
    /// The open Run the question is asked in, when it is asked in one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<i64>,
}

/// The arguments of `decisions.list`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListDecisions {
    /// `pending` (the default), `answered` or `all`.
    #[serde(default)]
    pub state: DecisionState,
    /// With a Decider, `answered` and `pending` mean answered, or not yet
    /// answered, by that Decider; without, by anyone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decider: Option<SourceArg>,
    /// Only Decisions about this track.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<String>,
    /// Only Decisions of this topic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    /// The most Decisions to answer; [`DEFAULT_DECISION_LIMIT`] when left
    /// out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
}

/// What `decisions.list` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionPage {
    /// Oldest first.
    pub decisions: Vec<Decision>,
    /// How many there are before the limit.
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
    /// The `offset` of the next page; `null` on the last.
    pub next_offset: Option<usize>,
}

/// The arguments of `decisions.get`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetDecision {
    pub id: i64,
}

/// A Fact an answer is to produce: the Decider's choice, recorded about a
/// track with the Decider as its source and the probability of the choice
/// as its confidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactFromAnswer {
    /// The kind of Fact, declared as any kind of Fact is.
    pub kind: String,
    /// The track it is about. Left out: the Decision's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<String>,
    /// The value to record. Left out, the choice read as the kind's type:
    /// its name for a text kind, a list of that one name for a list kind,
    /// `true` or `false` for a yes-or-no answer and a boolean kind, the
    /// number the name spells for a number kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

/// The arguments of `decisions.answer`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerDecision {
    pub id: i64,
    /// Who answers.
    pub decider: SourceArg,
    /// The name of one of the Decision's candidates, exactly.
    pub choice: String,
    /// The probability, 0 to 1, that the choice is right. May be left out
    /// when `probabilities` holds it, and when the Decider is the DJ (1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probability: Option<f64>,
    /// The Decider's probability for each candidate, by name, when it has
    /// them. Candidates may be missing; together they are at most 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probabilities: Option<BTreeMap<String, f64>>,
    /// Why, in the Decider's words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Record the choice as a Fact too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fact: Option<FactFromAnswer>,
    /// The open Run the answer (and its Fact) belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<i64>,
    /// What the Run made for the Fact is called. Only with `fact`, and not
    /// with `run`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// The arguments of `decisions.decide`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecideByRule {
    pub id: i64,
    /// The rule to decide by.
    pub rule: Rule,
    /// Record the choice as a Fact too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fact: Option<FactFromAnswer>,
    /// The open Run the answer (and its Fact) belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<i64>,
    /// What the Run made for the Fact is called. Only with `fact`, and not
    /// with `run`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// What `decisions.answer` and `decisions.decide` answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Answered {
    /// The Decision, with this answer among its answers.
    pub decision: Decision,
    /// The answer just given.
    pub answer: Answer,
    /// The Fact the answer produced, when one was asked for.
    pub fact: Option<Fact>,
    /// The Run the answer belongs to: the one given, or the one made for
    /// the Fact. `null` for an answer outside any Run.
    pub run: Option<Run>,
}

/// Whether `command` is one of this module's.
pub(crate) fn answers(command: &str) -> bool {
    matches!(
        command,
        commands::DECISIONS_CREATE
            | commands::DECISIONS_LIST
            | commands::DECISIONS_GET
            | commands::DECISIONS_ANSWER
            | commands::DECISIONS_DECIDE
    )
}

/// Answers one of this module's commands. `lookup` finds a track in the
/// Library; it is the only thing here that takes the Library's lock, and
/// only to read.
pub(crate) fn answer(
    stems: &Arc<Stems>,
    request: &Request,
    lookup: &dyn Fn(&str) -> Result<ParentAudio, ServiceError>,
) -> Result<Value, ServiceError> {
    let decisions = Decisions::new(stems, lookup)?;
    let answer = match request.command.as_str() {
        commands::DECISIONS_CREATE => {
            serde_json::to_value(decisions.create(facts::arguments(request)?)?)
        }
        commands::DECISIONS_LIST => {
            serde_json::to_value(decisions.list(&facts::arguments(request)?)?)
        }
        commands::DECISIONS_GET => {
            let wanted: GetDecision = facts::arguments(request)?;
            serde_json::to_value(decisions.existing(wanted.id)?)
        }
        commands::DECISIONS_ANSWER => {
            serde_json::to_value(decisions.answer(facts::arguments(request)?)?)
        }
        _ => serde_json::to_value(decisions.decide(facts::arguments(request)?)?),
    };
    answer.map_err(|e| ServiceError::new(service_codes::SERVICE_FAILED, e.to_string()))
}

/// The Decisions of one Sidecar, for the commands here and for any other
/// part of the service that records its own judgement as a Decision.
pub(crate) struct Decisions<'a> {
    sidecar: Arc<Sidecar>,
    stems: &'a Arc<Stems>,
    lookup: &'a dyn Fn(&str) -> Result<ParentAudio, ServiceError>,
}

/// What an answer also does, for both ways of answering.
struct Then {
    fact: Option<FactFromAnswer>,
    run: Option<i64>,
    description: Option<String>,
}

/// An answer about to be given, whoever gives it.
struct Given<'a> {
    decision: &'a Decision,
    decider: &'a Source,
    choice: &'a str,
    probability: Option<f64>,
    probabilities: Option<&'a BTreeMap<String, f64>>,
    reason: Option<&'a str>,
    then: &'a Then,
}

impl<'a> Decisions<'a> {
    /// The Decisions of the service's Sidecar, opening it if need be.
    /// `lookup` finds a track in the Library.
    pub(crate) fn new(
        stems: &'a Arc<Stems>,
        lookup: &'a dyn Fn(&str) -> Result<ParentAudio, ServiceError>,
    ) -> Result<Self, ServiceError> {
        Ok(Self {
            sidecar: stems.sidecar()?,
            stems,
            lookup,
        })
    }

    fn existing(&self, id: i64) -> Result<Decision, ServiceError> {
        self.sidecar
            .decision(id)
            .map_err(|e| sidecar_error(&e))?
            .ok_or_else(|| sidecar_error(&SidecarError::NoDecision(id)))
    }

    /// Asks a Decision, as `decisions.create` does, and announces it.
    pub(crate) fn create(&self, create: CreateDecision) -> Result<Decision, ServiceError> {
        let track_id = match &create.track {
            Some(track) => Some((self.lookup)(track)?.parent().id.clone()),
            None => None,
        };
        if let Some(run) = create.run {
            facts::open_run(self.stems, run)?;
        }
        let candidates: Vec<Candidate> = create.candidates.into_iter().map(Into::into).collect();
        let decision = self
            .sidecar
            .create_decision(&NewDecision {
                question_type: create.question_type,
                topic: create.topic.as_deref(),
                question: &create.question,
                context: &create.context.unwrap_or_else(|| json!({})),
                candidates: &candidates,
                track_id: track_id.as_deref(),
                run: create.run.map(RunId),
            })
            .map_err(|e| sidecar_error(&e))?;
        self.changed(&decision, None);
        Ok(decision)
    }

    fn list(&self, wanted: &ListDecisions) -> Result<DecisionPage, ServiceError> {
        let limit = wanted.limit.unwrap_or(DEFAULT_DECISION_LIMIT);
        let offset = wanted.offset.unwrap_or(0);
        let (decisions, total) = self
            .sidecar
            .decisions(&DecisionQuery {
                state: wanted.state,
                decider: wanted.decider.as_ref().map(facts::source).transpose()?,
                track_id: wanted.track.clone(),
                topic: wanted.topic.clone(),
                limit,
                offset,
            })
            .map_err(|e| sidecar_error(&e))?;
        let end = offset.saturating_add(decisions.len());
        Ok(DecisionPage {
            next_offset: (end < total && !decisions.is_empty()).then_some(end),
            decisions,
            total,
            offset,
            limit,
        })
    }

    /// Answers a Decision as the Decider named, as `decisions.answer`
    /// does: checked, recorded with its Fact if one is asked for, and
    /// announced.
    pub(crate) fn answer(&self, answer: AnswerDecision) -> Result<Answered, ServiceError> {
        let decision = self.existing(answer.id)?;
        let decider = facts::source(&answer.decider)?;
        // The DJ's answer is the DJ's word, as the DJ's Fact is.
        let probability = answer
            .probability
            .or((decider.kind == SourceKind::Dj && answer.probabilities.is_none()).then_some(1.0));
        self.give(&Given {
            decision: &decision,
            decider: &decider,
            choice: &answer.choice,
            probability,
            probabilities: answer.probabilities.as_ref(),
            reason: answer.reason.as_deref(),
            then: &Then {
                fact: answer.fact,
                run: answer.run,
                description: answer.description,
            },
        })
    }

    fn decide(&self, decide: DecideByRule) -> Result<Answered, ServiceError> {
        let decision = self.existing(decide.id)?;
        let ruling = decider::decide(&decide.rule, &decision).map_err(|none| match none {
            NoRuling::BadRule(why) => ServiceError::new(service_codes::BAD_REQUEST, why),
            NoRuling::NotApplicable(why) => ServiceError::new(
                codes::RULE_NOT_APPLICABLE,
                format!("the rule cannot answer: {why}"),
            ),
        })?;
        self.give(&Given {
            decision: &decision,
            decider: &decider::source(),
            choice: &ruling.choice,
            probability: Some(ruling.choice_probability),
            probabilities: None,
            reason: Some(&ruling.reason),
            then: &Then {
                fact: decide.fact,
                run: decide.run,
                description: decide.description,
            },
        })
    }

    /// Records an answer, and its Fact when one is asked for. Everything is
    /// checked before anything is written, and an answer with a Fact is
    /// written with it as one change to the Sidecar.
    fn give(&self, given: &Given) -> Result<Answered, ServiceError> {
        let decision = given.decision;
        let then = given.then;
        let mut new = NewAnswer {
            decision: decision.id,
            decider: given.decider,
            choice: given.choice,
            choice_probability: given.probability,
            probabilities: given.probabilities,
            reason: given.reason,
            fact_id: None,
            run: then.run.map(RunId),
        };
        let choice_probability = self
            .sidecar
            .check_answer(&new)
            .map_err(|e| sidecar_error(&e))?;
        if then.fact.is_none() && then.description.is_some() {
            return Err(ServiceError::new(
                service_codes::BAD_REQUEST,
                "a description names the Run made for the answer's Fact; this answer produces none",
            ));
        }

        let Some(wanted) = &then.fact else {
            let run = then
                .run
                .map(|run| facts::open_run(self.stems, run))
                .transpose()?;
            new.run = run.as_ref().map(|run| run.id);
            let (decision, answer) = self
                .sidecar
                .answer_decision(&new)
                .map_err(|e| sidecar_error(&e))?;
            self.changed(&decision, Some(answer.id));
            return Ok(Answered {
                decision,
                answer,
                fact: None,
                run,
            });
        };

        let track = wanted
            .track
            .clone()
            .or_else(|| decision.track_id.clone())
            .ok_or_else(|| {
                ServiceError::new(
                    codes::FACT_NEEDS_TRACK,
                    format!(
                        "Decision {} is about no track; say which track the Fact is about",
                        decision.id
                    ),
                )
            })?;
        let value = match &wanted.value {
            Some(value) => value.clone(),
            None => self.value_of(&wanted.kind, decision, given.choice)?,
        };
        // Every check `facts.record` makes, with nothing written: the
        // kind's declaration, the value, the track, the audio, the Run.
        let fact = facts::checked(
            self.stems,
            FactToRecord {
                track,
                kind: wanted.kind.clone(),
                value,
                confidence: None,
                source: None,
            },
            given.decider,
            choice_probability,
            self.lookup,
        )?;
        if let Some(run) = then.run {
            facts::open_run(self.stems, run)?;
        }
        let description = then
            .description
            .as_deref()
            .map(str::trim)
            .filter(|description| !description.is_empty())
            .map_or_else(
                || {
                    format!(
                        "Decision {} answered by {}",
                        decision.id, given.decider.name
                    )
                },
                str::to_owned,
            );
        self.stems
            .before_record(&fact.found, fact.fingerprint.as_ref(), then.run.map(RunId))?;
        // One transaction: the answer, the Fact, what the Fact supersedes
        // and the Run made for them. Clients are told only once it holds.
        let (decision, answer, recorded) = self
            .sidecar
            .answer_decision_with_fact(
                &new,
                &FactOfAnswer {
                    track_id: &fact.track_id,
                    fingerprint: fact.fingerprint.as_ref(),
                    kind: fact.kind.kind(),
                    value: fact.value.clone(),
                    run: match then.run {
                        Some(run) => RunOfAnswer::Open(RunId(run)),
                        None => RunOfAnswer::Made {
                            description: &description,
                        },
                    },
                },
            )
            .map_err(|e| sidecar_error(&e))?;
        let run = self
            .sidecar
            .run(recorded.run)
            .map_err(|e| sidecar_error(&e))?;
        let recorded = Fact::from(recorded);
        // The answer stands whatever becomes of the stale-marking, which
        // the next command about the track's audio does again.
        let _ = facts::recorded(self.stems, &recorded, fact.fingerprint.as_ref());
        self.changed(&decision, Some(answer.id));
        Ok(Answered {
            decision,
            answer,
            fact: Some(recorded),
            run,
        })
    }

    /// The value a choice is as a Fact of `kind`, read by the kind's type.
    /// A kind that is not declared is left for the Facts path to refuse.
    fn value_of(
        &self,
        kind: &str,
        decision: &Decision,
        choice: &str,
    ) -> Result<Value, ServiceError> {
        let kinds = Kinds::load(self.sidecar.dir()).map_err(|e| facts::sidecar_error(&e))?;
        let Some(declared) = kinds.get(kind) else {
            return Ok(json!(choice));
        };
        let yes_no = decision.question_type == QuestionType::YesNo;
        match declared.value {
            ValueType::Text => Ok(json!(choice)),
            ValueType::Texts => Ok(json!([choice])),
            ValueType::Boolean if yes_no => Ok(json!(choice == YES)),
            ValueType::Integer | ValueType::Number => {
                serde_json::from_str::<serde_json::Number>(choice)
                    .map(Value::Number)
                    .map_err(|_| unreadable(kind, &declared.takes(), choice))
            }
            ValueType::Boolean | ValueType::Object => {
                Err(unreadable(kind, &declared.takes(), choice))
            }
        }
    }

    fn changed(&self, decision: &Decision, answer: Option<i64>) {
        self.stems.publish(
            stems::events::SIDECAR_CHANGED,
            json!({
                "track_id": decision.track_id.clone().unwrap_or_default(),
                "kind": CHANGED_KIND,
                "fact_id": null,
                "stale": [],
                "decision_id": decision.id,
                "answer_id": answer,
            }),
        );
    }
}

fn unreadable(kind: &str, takes: &str, choice: &str) -> ServiceError {
    ServiceError::new(
        facts::codes::WRONG_VALUE_TYPE,
        format!(
            "{kind} takes {takes}, and the choice \"{choice}\" cannot be read as one; give the Fact's value"
        ),
    )
}

fn sidecar_error(error: &SidecarError) -> ServiceError {
    let code = match error {
        SidecarError::NoDecision(_) => codes::DECISION_NOT_FOUND,
        SidecarError::Decision(_) => codes::INVALID_DECISION,
        SidecarError::Candidates(_) => codes::INVALID_CANDIDATES,
        SidecarError::NotACandidate { .. } => codes::ANSWER_NOT_A_CANDIDATE,
        SidecarError::Probability(_) => codes::INVALID_PROBABILITY,
        _ => return facts::sidecar_error(error),
    };
    ServiceError::new(code, error.to_string())
}
