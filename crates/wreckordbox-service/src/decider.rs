//! The rule-based Decider: answers a Decision by a fixed rule over its
//! context, with no model. It exists so Decisions work, and can be tested
//! and compared against, without one.
//!
//! It is deliberately small and has no judgement of its own. A rule names
//! one field of the Decision's context (a top-level key) and either:
//!
//! - [`Rule::Match`]: chooses the candidate whose name is the field's text.
//!   An exact match is chosen with probability
//!   [`EXACT_MATCH_PROBABILITY`]. Otherwise the text and the names are
//!   compared after [`normalise`] (case folded, anything that is not a
//!   letter or a digit read as a space, spaces collapsed), and if exactly
//!   one candidate is the same it is chosen with probability
//!   [`NORMALISED_MATCH_PROBABILITY`]. For any question type.
//! - [`Rule::Threshold`]: for a yes-or-no question, answers `yes` when the
//!   field, a number, is at least `at_least` and at most `at_most`
//!   (whichever are given, both inclusive) and `no` otherwise, with
//!   probability [`THRESHOLD_PROBABILITY`].
//!
//! When the rule cannot answer (no such field, text that matches no
//! candidate or more than one, a field that is not a number, a threshold
//! asked of a choice) it says so and the Decision is left as it was: the
//! rule never guesses.
//!
//! The probabilities are fixed numbers, not measurements: a rule that
//! applies is taken at its word. Whether they deserve it is what comparing
//! a Decider with the DJ's corrections is for.
//!
//! The same rule over the same Decision always gives the same answer.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use wreckordbox_sidecar::decisions::{Decision, QuestionType, NO, YES};
use wreckordbox_sidecar::{Source, SourceKind};

/// The rule-based Decider's name as a source: the tool `rule-based`.
pub const NAME: &str = "rule-based";

/// The probability given to a candidate whose name is exactly the field's
/// text.
pub const EXACT_MATCH_PROBABILITY: f64 = 1.0;
/// The probability given to the one candidate whose name is the field's
/// text only after normalising both.
pub const NORMALISED_MATCH_PROBABILITY: f64 = 0.9;
/// The probability given to the answer of a threshold.
pub const THRESHOLD_PROBABILITY: f64 = 1.0;

/// The rule-based Decider as a source.
#[must_use]
pub fn source() -> Source {
    Source::new(SourceKind::Tool, NAME)
}

/// A rule to decide by.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case", deny_unknown_fields)]
pub enum Rule {
    /// Choose the candidate named by a field of the context.
    Match {
        /// A top-level key of the context holding text (or a number, read
        /// as its text).
        field: String,
    },
    /// Answer a yes-or-no question by whether a number in the context is
    /// within bounds.
    Threshold {
        /// A top-level key of the context holding a number.
        field: String,
        /// Yes only if the number is this or more.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at_least: Option<f64>,
        /// Yes only if the number is this or less.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at_most: Option<f64>,
    },
}

/// What a rule answers.
#[derive(Debug, Clone, PartialEq)]
pub struct Ruling {
    /// The name of the candidate chosen.
    pub choice: String,
    /// The probability the rule gives to its choice being right.
    pub choice_probability: f64,
    /// Which rule answered and what it saw.
    pub reason: String,
}

/// Why a rule gave no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoRuling {
    /// The rule itself is not one: a threshold with no bound.
    BadRule(String),
    /// The rule cannot answer this Decision.
    NotApplicable(String),
}

/// Text as the match rule compares it: case folded, every run of
/// characters that are not letters or digits read as one space, and no
/// space at either end. `Tech-House`, `tech house` and ` TECH  HOUSE ` are
/// the same.
#[must_use]
pub fn normalise(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Answers `decision` by `rule`, or says why the rule cannot.
pub fn decide(rule: &Rule, decision: &Decision) -> Result<Ruling, NoRuling> {
    match rule {
        Rule::Match { field } => by_match(field, decision),
        Rule::Threshold {
            field,
            at_least,
            at_most,
        } => by_threshold(field, *at_least, *at_most, decision),
    }
}

fn field_of<'a>(field: &str, decision: &'a Decision) -> Result<&'a Value, NoRuling> {
    decision.context.get(field).ok_or_else(|| {
        NoRuling::NotApplicable(format!(
            "the context of Decision {} has no field \"{field}\"",
            decision.id
        ))
    })
}

fn by_match(field: &str, decision: &Decision) -> Result<Ruling, NoRuling> {
    let text = match field_of(field, decision)? {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        _ => {
            return Err(NoRuling::NotApplicable(format!(
                "the field \"{field}\" is not text, so it names no candidate"
            )))
        }
    };
    if let Some(exact) = decision.candidates.iter().find(|one| one.name == text) {
        return Ok(Ruling {
            choice: exact.name.clone(),
            choice_probability: EXACT_MATCH_PROBABILITY,
            reason: format!("match: the field \"{field}\" is exactly \"{text}\""),
        });
    }
    let wanted = normalise(&text);
    let same: Vec<&str> = decision
        .candidates
        .iter()
        .filter(|one| !wanted.is_empty() && normalise(&one.name) == wanted)
        .map(|one| one.name.as_str())
        .collect();
    match same.as_slice() {
        [one] => Ok(Ruling {
            choice: (*one).to_owned(),
            choice_probability: NORMALISED_MATCH_PROBABILITY,
            reason: format!(
                "match: the field \"{field}\" is \"{text}\", which is \"{one}\" once both are normalised"
            ),
        }),
        [] => Err(NoRuling::NotApplicable(format!(
            "the field \"{field}\" is \"{text}\", which matches no candidate"
        ))),
        several => Err(NoRuling::NotApplicable(format!(
            "the field \"{field}\" is \"{text}\", which matches more than one candidate once normalised: {}",
            several.join(", ")
        ))),
    }
}

fn by_threshold(
    field: &str,
    at_least: Option<f64>,
    at_most: Option<f64>,
    decision: &Decision,
) -> Result<Ruling, NoRuling> {
    if at_least.is_none() && at_most.is_none() {
        return Err(NoRuling::BadRule(
            "a threshold needs a bound: at least, at most, or both".to_owned(),
        ));
    }
    if decision.question_type != QuestionType::YesNo {
        return Err(NoRuling::NotApplicable(format!(
            "a threshold answers a yes-or-no question, and Decision {} is a choice",
            decision.id
        )));
    }
    let Some(number) = field_of(field, decision)?.as_f64() else {
        return Err(NoRuling::NotApplicable(format!(
            "the field \"{field}\" is not a number"
        )));
    };
    let within = at_least.is_none_or(|least| number >= least)
        && at_most.is_none_or(|most| number <= most);
    let bounds = match (at_least, at_most) {
        (Some(least), Some(most)) => format!("from {least} to {most}"),
        (Some(least), None) => format!("at least {least}"),
        (None, Some(most)) => format!("at most {most}"),
        (None, None) => String::new(),
    };
    Ok(Ruling {
        choice: if within { YES } else { NO }.to_owned(),
        choice_probability: THRESHOLD_PROBABILITY,
        reason: format!(
            "threshold: the field \"{field}\" is {number}, which {} {bounds}",
            if within { "is" } else { "is not" }
        ),
    })
}
