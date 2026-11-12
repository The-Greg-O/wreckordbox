//! `decisions`: recorded judgements between fixed candidates, kept in the
//! Sidecar, and the Deciders that answer them.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use clap::{Args, Subcommand, ValueEnum};
use serde_json::{json, Value};
use wreckordbox_service::decisions::{
    commands, Answer, AnswerDecision, Answered, CandidateArg, CreateDecision, DecideByRule,
    Decision, DecisionPage, DecisionState, FactFromAnswer, GetDecision, ListDecisions,
    QuestionType, Rule,
};
use wreckordbox_service::facts::SourceArg;
use wreckordbox_service::{Client, ServiceError};

use crate::facts::{bad, parse_source, source_text, table, time_text};

#[derive(Subcommand)]
pub enum DecisionCommands {
    /// Ask a Decision: a question, the context to answer it from and the
    /// candidates to choose between. It is pending until a Decider answers.
    ///
    /// `--type choice` takes two or more `--candidate`s. `--type yes-no`
    /// takes none: the question is a statement, and its candidates are
    /// `yes` and `no`.
    Create {
        /// `choice` between named candidates, or `yes-no`.
        #[arg(long = "type", value_name = "TYPE")]
        question_type: TypeArg,
        /// The question; for `yes-no`, the statement that is true or not.
        #[arg(long, value_name = "TEXT")]
        question: String,
        /// What a Decider answers from: a JSON object.
        #[arg(long, value_name = "JSON")]
        context: Option<String>,
        /// A candidate's name. Repeat for each; `NAME=DESCRIPTION` also
        /// says what it means.
        #[arg(long = "candidate", value_name = "NAME")]
        candidates: Vec<String>,
        /// What kind of question this is, in a word of your own (`genre`,
        /// `lookup_match`): lower-case letters, digits and underscores.
        #[arg(long, value_name = "TOPIC")]
        topic: Option<String>,
        /// The track the question is about: the ID column of `tracks list`.
        #[arg(long, value_name = "TRACK_ID")]
        track: Option<String>,
        /// Ask it in this Run, started with `runs start`.
        #[arg(long, value_name = "RUN_ID")]
        run: Option<i64>,
    },
    /// List the Decisions no Decider has answered, oldest first.
    ///
    /// With `--decider`, those that Decider has not answered, whoever else
    /// has.
    Pending(ListOptions),
    /// List the Decisions that have an answer, oldest first.
    ///
    /// With `--decider`, those that Decider has answered.
    Answered(ListOptions),
    /// List every Decision, oldest first.
    List(ListOptions),
    /// Show one Decision in full: the question, the context, the candidates
    /// and every answer it has been given.
    Show {
        #[arg(value_name = "DECISION_ID")]
        id: i64,
    },
    /// Answer a Decision as a Decider, under your own name.
    ///
    /// The choice must be one of the Decision's candidates, exactly as
    /// named. An earlier answer is kept beside this one.
    ///
    /// Say how sure you are with `--probability`: the probability, 0 to 1,
    /// that your choice is the right one. If you have a probability for
    /// each candidate give them all with `--probabilities` instead.
    Answer {
        #[arg(value_name = "DECISION_ID")]
        id: i64,
        /// The name of the candidate chosen.
        #[arg(value_name = "CHOICE")]
        choice: String,
        /// Who answers: `agent:NAME`, `tool:NAME`, `dj`.
        #[arg(long, value_name = "SOURCE", value_parser = parse_source)]
        decider: SourceArg,
        /// The probability, 0 to 1, that the choice is right. Not needed
        /// with `--probabilities`, or when the Decider is the DJ.
        #[arg(long, value_name = "0..1")]
        probability: Option<f64>,
        /// The probability of each candidate, as a JSON object by name:
        /// '{"Techno": 0.2, "Tech House": 0.7, "House": 0.1}'.
        #[arg(long, value_name = "JSON")]
        probabilities: Option<String>,
        /// Why, in a sentence.
        #[arg(long, value_name = "TEXT")]
        reason: Option<String>,
        #[command(flatten)]
        then: ThenOptions,
    },
    /// Have the rule-based Decider answer a Decision, as `tool:rule-based`.
    ///
    /// `--match FIELD` chooses the candidate named by that field of the
    /// Decision's context: probability 1 when the names are the same,
    /// 0.9 when exactly one candidate is the same once case, punctuation
    /// and spacing are ignored.
    ///
    /// `--threshold FIELD` with `--at-least` and/or `--at-most` answers a
    /// yes-no Decision: `yes` when that number in the context is within
    /// the bounds, `no` otherwise, with probability 1.
    ///
    /// When the rule cannot answer, nothing is recorded and the Decision
    /// stays as it was.
    Decide {
        #[arg(value_name = "DECISION_ID")]
        id: i64,
        /// Choose the candidate named by this field of the context.
        #[arg(
            long = "match",
            value_name = "FIELD",
            conflicts_with = "threshold",
            required_unless_present = "threshold"
        )]
        matching: Option<String>,
        /// Answer yes or no by whether this number in the context is
        /// within bounds.
        #[arg(long, value_name = "FIELD")]
        threshold: Option<String>,
        /// Yes only if the number is this or more.
        #[arg(long, value_name = "NUMBER", requires = "threshold")]
        at_least: Option<f64>,
        /// Yes only if the number is this or less.
        #[arg(long, value_name = "NUMBER", requires = "threshold")]
        at_most: Option<f64>,
        #[command(flatten)]
        then: ThenOptions,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum TypeArg {
    Choice,
    #[value(alias = "yes_no")]
    YesNo,
}

#[derive(Args)]
pub struct ListOptions {
    /// Read "answered" and "pending" as by this Decider: `agent:NAME`,
    /// `tool:NAME`, `dj`.
    #[arg(long, value_name = "SOURCE", value_parser = parse_source)]
    decider: Option<SourceArg>,
    /// Only Decisions about this track.
    #[arg(long, value_name = "TRACK_ID")]
    track: Option<String>,
    /// Only Decisions of this topic.
    #[arg(long, value_name = "TOPIC")]
    topic: Option<String>,
    /// The most Decisions to print.
    #[arg(long, value_name = "ROWS")]
    limit: Option<usize>,
    /// How many to skip first.
    #[arg(long, value_name = "ROWS")]
    offset: Option<usize>,
}

/// What an answer also does.
#[derive(Args)]
pub struct ThenOptions {
    /// Also record the choice as a Fact of this kind, with the Decider as
    /// its source and the probability of the choice as its confidence.
    #[arg(long, value_name = "KIND")]
    fact: Option<String>,
    /// The track the Fact is about. Left out: the Decision's.
    #[arg(long, value_name = "TRACK_ID", requires = "fact")]
    fact_track: Option<String>,
    /// The Fact's value as JSON. Left out: the choice, read as the kind's
    /// type.
    #[arg(long, value_name = "JSON", requires = "fact")]
    fact_value: Option<String>,
    /// Answer in this Run, started with `runs start`.
    #[arg(long, value_name = "RUN_ID", conflicts_with = "description")]
    run: Option<i64>,
    /// What to call the Run made for the Fact.
    #[arg(long, value_name = "TEXT", requires = "fact")]
    description: Option<String>,
}

impl ThenOptions {
    fn fact(&self) -> Result<Option<FactFromAnswer>, ServiceError> {
        let Some(kind) = &self.fact else {
            return Ok(None);
        };
        Ok(Some(FactFromAnswer {
            kind: kind.clone(),
            track: self.fact_track.clone(),
            value: self
                .fact_value
                .as_deref()
                .map(|text| parsed("--fact-value", text))
                .transpose()?,
        }))
    }
}

/// Runs one `decisions` command and answers what to print.
pub fn run(
    client: &mut Client,
    as_json: bool,
    command: &DecisionCommands,
) -> Result<String, ServiceError> {
    match command {
        DecisionCommands::Create {
            question_type,
            question,
            context,
            candidates,
            topic,
            track,
            run,
        } => {
            let context = context
                .as_deref()
                .map(|text| parsed("--context", text))
                .transpose()?;
            let created: Decision = client.call(
                commands::DECISIONS_CREATE,
                json!(CreateDecision {
                    question_type: match question_type {
                        TypeArg::Choice => QuestionType::Choice,
                        TypeArg::YesNo => QuestionType::YesNo,
                    },
                    topic: topic.clone(),
                    question: question.clone(),
                    context,
                    candidates: candidates.iter().map(|text| candidate(text)).collect(),
                    track: track.clone(),
                    run: *run,
                }),
            )?;
            Ok(if as_json {
                json!(created).to_string()
            } else {
                format!(
                    "Asked Decision {} ({}): {}\nCandidates: {}\nPending. Answer it with: wreckordbox decisions answer {} <CHOICE> --decider <SOURCE> --probability <0..1>",
                    created.id,
                    type_text(created.question_type),
                    created.question,
                    candidates_text(&created),
                    created.id
                )
            })
        }
        DecisionCommands::Pending(options) => {
            list(client, as_json, DecisionState::Pending, options)
        }
        DecisionCommands::Answered(options) => {
            list(client, as_json, DecisionState::Answered, options)
        }
        DecisionCommands::List(options) => list(client, as_json, DecisionState::All, options),
        DecisionCommands::Show { id } => {
            let shown: Decision =
                client.call(commands::DECISIONS_GET, json!(GetDecision { id: *id }))?;
            Ok(if as_json {
                json!(shown).to_string()
            } else {
                shown_text(&shown)
            })
        }
        DecisionCommands::Answer {
            id,
            choice,
            decider,
            probability,
            probabilities,
            reason,
            then,
        } => {
            let probabilities: Option<BTreeMap<String, f64>> = probabilities
                .as_deref()
                .map(|text| {
                    serde_json::from_str(text).map_err(|e| {
                        bad(format!(
                            "--probabilities is a JSON object of candidate names and numbers: {e}"
                        ))
                    })
                })
                .transpose()?;
            let answered: Answered = client.call(
                commands::DECISIONS_ANSWER,
                json!(AnswerDecision {
                    id: *id,
                    decider: decider.clone(),
                    choice: choice.clone(),
                    probability: *probability,
                    probabilities,
                    reason: reason.clone(),
                    fact: then.fact()?,
                    run: then.run,
                    description: then.description.clone(),
                }),
            )?;
            Ok(answered_output(&answered, as_json))
        }
        DecisionCommands::Decide {
            id,
            matching,
            threshold,
            at_least,
            at_most,
            then,
        } => {
            let rule = match (matching, threshold) {
                (Some(field), _) => Rule::Match {
                    field: field.clone(),
                },
                (None, Some(field)) => Rule::Threshold {
                    field: field.clone(),
                    at_least: *at_least,
                    at_most: *at_most,
                },
                (None, None) => return Err(bad("give --match or --threshold".to_owned())),
            };
            let answered: Answered = client.call(
                commands::DECISIONS_DECIDE,
                json!(DecideByRule {
                    id: *id,
                    rule,
                    fact: then.fact()?,
                    run: then.run,
                    description: then.description.clone(),
                }),
            )?;
            Ok(answered_output(&answered, as_json))
        }
    }
}

fn list(
    client: &mut Client,
    as_json: bool,
    state: DecisionState,
    options: &ListOptions,
) -> Result<String, ServiceError> {
    let page: DecisionPage = client.call(
        commands::DECISIONS_LIST,
        json!(ListDecisions {
            state,
            decider: options.decider.clone(),
            track: options.track.clone(),
            topic: options.topic.clone(),
            limit: options.limit,
            offset: options.offset,
        }),
    )?;
    if as_json {
        return Ok(json!(page).to_string());
    }
    if page.decisions.is_empty() {
        return Ok(match state {
            DecisionState::Pending => "No pending Decisions.",
            DecisionState::Answered => "No answered Decisions.",
            DecisionState::All => "No Decisions.",
        }
        .to_owned());
    }
    let mut rows = vec![[
        "DECISION",
        "TYPE",
        "TOPIC",
        "TRACK",
        "QUESTION",
        "CANDIDATES",
        "ANSWERS",
    ]
    .map(str::to_owned)];
    rows.extend(page.decisions.iter().map(|decision| {
        [
            decision.id.to_string(),
            type_text(decision.question_type).to_owned(),
            or_dash(decision.topic.as_deref()),
            or_dash(decision.track_id.as_deref()),
            short(&decision.question, 50),
            short(&candidates_text(decision), 50),
            answers_text(decision),
        ]
    }));
    let mut text = table(&rows);
    let first = page.offset + 1;
    let last = page.offset + page.decisions.len();
    if let Some(next) = page.next_offset {
        let _ = write!(
            text,
            "\nDecisions {first} to {last} of {}; next page: --offset {next}",
            page.total
        );
    } else if page.offset > 0 {
        let _ = write!(text, "\nDecisions {first} to {last} of {}.", page.total);
    }
    Ok(text)
}

/// A candidate as written on the command line: its name, or
/// `NAME=DESCRIPTION`.
fn candidate(text: &str) -> CandidateArg {
    match text.split_once('=') {
        Some((name, description)) => {
            CandidateArg::Described(wreckordbox_service::decisions::Candidate {
                name: name.to_owned(),
                description: Some(description.to_owned()),
            })
        }
        None => CandidateArg::Name(text.to_owned()),
    }
}

fn parsed(option: &str, text: &str) -> Result<Value, ServiceError> {
    serde_json::from_str(text).map_err(|e| bad(format!("{option} is not JSON: {e}")))
}

fn type_text(question_type: QuestionType) -> &'static str {
    match question_type {
        QuestionType::Choice => "choice",
        QuestionType::YesNo => "yes-no",
    }
}

fn candidates_text(decision: &Decision) -> String {
    decision
        .candidates
        .iter()
        .map(|one| one.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The answers of a Decision in one cell: `Tech House 0.8 (agent:tagger)`,
/// one after another.
fn answers_text(decision: &Decision) -> String {
    if decision.answers.is_empty() {
        return "pending".to_owned();
    }
    decision
        .answers
        .iter()
        .map(|answer| {
            format!(
                "{} {} ({})",
                answer.choice,
                answer.choice_probability,
                source_text(&answer.decider)
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn shown_text(decision: &Decision) -> String {
    let mut text = format!(
        "Decision {} ({})\nQuestion    {}\nTopic       {}\nTrack       {}\nRun         {}\nAsked       {}\nContext     {}\nCandidates",
        decision.id,
        type_text(decision.question_type),
        decision.question,
        or_dash(decision.topic.as_deref()),
        or_dash(decision.track_id.as_deref()),
        decision
            .run
            .map_or_else(|| "-".to_owned(), |run| run.0.to_string()),
        time_text(decision.created_at),
        decision.context,
    );
    for one in &decision.candidates {
        let _ = write!(text, "\n  {}", one.name);
        if let Some(description) = &one.description {
            let _ = write!(text, ": {description}");
        }
    }
    if decision.answers.is_empty() {
        text.push_str("\nPending: no Decider has answered.");
        return text;
    }
    let mut rows = vec![[
        "ANSWER",
        "DECIDER",
        "CHOICE",
        "PROBABILITY",
        "FACT",
        "RUN",
        "ANSWERED",
        "REASON",
    ]
    .map(str::to_owned)];
    rows.extend(decision.answers.iter().map(answer_row));
    let _ = write!(text, "\n{}", table(&rows));
    for answer in &decision.answers {
        if let Some(all) = &answer.probabilities {
            let each: Vec<String> = decision
                .candidates
                .iter()
                .filter_map(|one| all.get(&one.name).map(|p| format!("{} {p}", one.name)))
                .collect();
            let _ = write!(
                text,
                "\nAnswer {} probabilities: {}",
                answer.id,
                each.join(", ")
            );
        }
    }
    text
}

fn answer_row(answer: &Answer) -> [String; 8] {
    [
        answer.id.to_string(),
        source_text(&answer.decider),
        answer.choice.clone(),
        answer.choice_probability.to_string(),
        answer
            .fact_id
            .map_or_else(|| "-".to_owned(), |fact| fact.to_string()),
        answer
            .run
            .map_or_else(|| "-".to_owned(), |run| run.0.to_string()),
        time_text(answer.answered_at),
        or_dash(answer.reason.as_deref()),
    ]
}

fn answered_output(answered: &Answered, as_json: bool) -> String {
    if as_json {
        return json!(answered).to_string();
    }
    let answer = &answered.answer;
    let mut text = format!(
        "Decision {} answered by {}: {} (probability {}).",
        answered.decision.id,
        source_text(&answer.decider),
        answer.choice,
        answer.choice_probability
    );
    if let Some(reason) = &answer.reason {
        let _ = write!(text, "\nReason: {reason}");
    }
    if let Some(fact) = &answered.fact {
        let _ = write!(
            text,
            "\nRecorded Fact {} in Run {}: track {} {} = {}.",
            fact.id, fact.run, fact.track_id, fact.kind, fact.value
        );
    }
    let others = answered.decision.answers.len() - 1;
    if others > 0 {
        let _ = write!(
            text,
            "\n{others} earlier answer{} kept beside it.",
            if others == 1 { " is" } else { "s are" }
        );
    }
    text
}

fn or_dash(text: Option<&str>) -> String {
    match text {
        Some(text) if !text.is_empty() => text.to_owned(),
        _ => "-".to_owned(),
    }
}

fn short(text: &str, most: usize) -> String {
    if text.chars().count() > most {
        let mut cut: String = text.chars().take(most - 3).collect();
        cut.push_str("...");
        cut
    } else {
        text.to_owned()
    }
}
