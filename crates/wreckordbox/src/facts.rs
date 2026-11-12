//! `facts` and `runs`: what is known about a track, kept in the Sidecar
//! with who says so, and the Runs that record it.

use std::fmt::Write as _;
use std::io::Read as _;
use std::path::PathBuf;

use clap::{Args, Subcommand};
use serde_json::{json, Value};
use wreckordbox_service::facts::{
    codes, commands, Declaration, DeclaredKinds, Fact, FactsOfTrack, FinishRun, Recorded, Run,
    Source, SourceArg, SourceKind, StartRun, TrackFacts, ValueType,
};
use wreckordbox_service::{Client, ServiceError};

#[derive(Subcommand)]
pub enum FactCommands {
    /// Record one Fact about a track: a kind, a value, who says so and how
    /// sure they are. The Library is not changed.
    ///
    /// The kind must be declared (`facts kinds` lists them) and the value
    /// must be of the kind's type: text as it is, several values for a
    /// list of texts (`facts record 10003 mood Dark Hypnotic`), anything
    /// else as JSON.
    ///
    /// The Fact belongs to a Run. Give `--run` to record in one started
    /// with `runs start`; otherwise a Run is made for this Fact alone,
    /// called by `--description` if given. The result names the Run.
    ///
    /// A Fact from another source about the same track and kind is left
    /// standing beside this one. The same source recording the same kind
    /// again replaces its own earlier Fact, which is kept.
    Record {
        /// The track's identifier: the ID column of `tracks list`.
        #[arg(value_name = "TRACK_ID")]
        track: String,
        /// The kind of Fact, such as `genre` or `energy`.
        #[arg(value_name = "KIND")]
        kind: String,
        /// The value. More than one only for a kind that takes a list.
        #[arg(value_name = "VALUE", required = true)]
        value: Vec<String>,
        #[command(flatten)]
        how: RecordOptions,
    },
    /// Record many Facts in one Run, read from standard input (or
    /// `--file`): all of them, or none if any is refused.
    ///
    /// One JSON object per line:
    ///
    ///   {"track": "10003", "kind": "genre", "value": "Tech House", "confidence": 0.8}
    ///
    /// `confidence` and `source` (`{"kind": "agent", "name": "tagger"}`)
    /// may be left out of a line when `--confidence` and `--source` (or
    /// the Run's source) say them for all. A JSON array of such objects is
    /// read too.
    #[command(verbatim_doc_comment)]
    RecordMany {
        /// Read the Facts from this file instead of standard input.
        #[arg(long, value_name = "PATH")]
        file: Option<PathBuf>,
        #[command(flatten)]
        how: RecordOptions,
    },
    /// Show the Facts the Sidecar holds for one track, newest first, each
    /// with its source, confidence, Run and time.
    Show {
        /// The track's identifier: the ID column of `tracks list`.
        #[arg(value_name = "TRACK_ID")]
        track: String,
        /// Only Facts of this kind.
        #[arg(long, value_name = "KIND")]
        kind: Option<String>,
        /// Include stale, unverified and replaced Facts in the full history.
        #[arg(long)]
        all: bool,
        /// Retained for compatibility; every read now checks the audio.
        #[arg(long)]
        check_audio: bool,
    },
    /// List the declared kinds of Fact: whether each is a Judgement or a
    /// Measurement, and the type of its value.
    ///
    /// A Judgement is about the music and follows the track when its audio
    /// file is replaced. A Measurement is computed from the audio and is
    /// stale once the audio changes. Other kinds are declared in the kinds
    /// file this command names.
    Kinds,
}

#[derive(Args)]
pub struct RecordOptions {
    /// Who says so: `dj`, `agent:NAME`, `tool:NAME` or `web:ADDRESS`.
    /// Left out with `--run`, the Run's source.
    #[arg(long, value_name = "SOURCE", value_parser = parse_source)]
    source: Option<SourceArg>,
    /// How sure the source is, 0 to 1. Not needed when the source is the
    /// DJ.
    #[arg(long, value_name = "0..1")]
    confidence: Option<f64>,
    /// Record in this Run, started with `runs start`.
    #[arg(long, value_name = "RUN_ID", conflicts_with = "description")]
    run: Option<i64>,
    /// What to call the Run made for this command.
    #[arg(long, value_name = "TEXT")]
    description: Option<String>,
}

#[derive(Subcommand)]
pub enum RunCommands {
    /// Start a Run: one piece of work whose changes belong together. Prints
    /// its identifier, to pass as `--run`.
    Start {
        /// Who the work is by: `dj`, `agent:NAME`, `tool:NAME` or
        /// `web:ADDRESS`.
        #[arg(long, value_name = "SOURCE", value_parser = parse_source)]
        source: SourceArg,
        /// What the work is, for whoever reviews it later.
        #[arg(long, value_name = "TEXT")]
        description: String,
    },
    /// End a Run. Nothing more can be recorded in it.
    Finish {
        #[arg(value_name = "RUN_ID")]
        id: i64,
        /// How it went, in a word or a sentence.
        #[arg(long, value_name = "TEXT")]
        outcome: Option<String>,
    },
}

/// `dj`, `agent:NAME`, `tool:NAME`, `web:ADDRESS`.
fn parse_source(text: &str) -> Result<SourceArg, String> {
    let (kind, name) = text.split_once(':').unwrap_or((text, ""));
    let kind = match kind {
        "dj" => SourceKind::Dj,
        "agent" => SourceKind::Agent,
        "tool" => SourceKind::Tool,
        "web" => SourceKind::Web,
        other => {
            return Err(format!(
                "a source is dj, agent:NAME, tool:NAME or web:ADDRESS, not \"{other}\""
            ))
        }
    };
    Ok(SourceArg {
        kind,
        name: name.to_owned(),
    })
}

/// Runs one `facts` command and answers what to print.
pub fn facts(
    client: &mut Client,
    as_json: bool,
    command: &FactCommands,
) -> Result<String, ServiceError> {
    match command {
        FactCommands::Record {
            track,
            kind,
            value,
            how,
        } => {
            let kinds: DeclaredKinds = client.call(commands::FACTS_KINDS, json!({}))?;
            let declared = kinds.kinds.iter().find(|declared| declared.name == *kind);
            let fact = json!({
                "track": track,
                "kind": kind,
                "value": value_of(declared, kind, value)?,
            });
            record(client, as_json, how, &[fact])
        }
        FactCommands::RecordMany { file, how } => {
            let text = if let Some(path) = file {
                std::fs::read_to_string(path)
                    .map_err(|e| bad(format!("cannot read {}: {e}", path.display())))?
            } else {
                let mut text = String::new();
                std::io::stdin()
                    .read_to_string(&mut text)
                    .map_err(|e| bad(format!("cannot read standard input: {e}")))?;
                text
            };
            record(client, as_json, how, &facts_in(&text)?)
        }
        FactCommands::Show {
            track,
            kind,
            all,
            check_audio,
        } => {
            let shown: TrackFacts = client.call(
                commands::FACTS_OF_TRACK,
                json!(FactsOfTrack {
                    id: track.clone(),
                    kind: kind.clone(),
                    all: *all,
                    check_audio: *check_audio,
                }),
            )?;
            Ok(if as_json {
                json!(shown).to_string()
            } else {
                shown_text(&shown)
            })
        }
        FactCommands::Kinds => {
            let kinds: DeclaredKinds = client.call(commands::FACTS_KINDS, json!({}))?;
            Ok(if as_json {
                json!(kinds).to_string()
            } else {
                kinds_text(&kinds)
            })
        }
    }
}

/// Runs one `runs` command and answers what to print.
pub fn runs(
    client: &mut Client,
    as_json: bool,
    command: &RunCommands,
) -> Result<String, ServiceError> {
    let (run, did): (Run, &str) = match command {
        RunCommands::Start {
            source,
            description,
        } => (
            client.call(
                commands::RUNS_START,
                json!(StartRun {
                    source: source.clone(),
                    description: description.clone(),
                }),
            )?,
            "Started",
        ),
        RunCommands::Finish { id, outcome } => (
            client.call(
                commands::RUNS_FINISH,
                json!(FinishRun {
                    id: *id,
                    outcome: outcome.clone(),
                }),
            )?,
            "Finished",
        ),
    };
    Ok(if as_json {
        json!(run).to_string()
    } else {
        format!(
            "{did} Run {}: {} ({}).",
            run.id.0,
            run.description,
            source_text(&run.source)
        )
    })
}

fn record(
    client: &mut Client,
    as_json: bool,
    how: &RecordOptions,
    facts: &[Value],
) -> Result<String, ServiceError> {
    // Built by hand so a line's own keys reach the service as written and
    // it is the service that says what is wrong with them.
    let mut args = json!({ "facts": facts });
    if let Some(run) = how.run {
        args["run"] = json!(run);
    }
    if let Some(description) = &how.description {
        args["description"] = json!(description);
    }
    if let Some(source) = &how.source {
        args["source"] = json!(source);
    }
    if let Some(confidence) = how.confidence {
        args["confidence"] = json!(confidence);
    }
    let recorded: Recorded = client.call(commands::FACTS_RECORD, args)?;
    Ok(if as_json {
        json!(recorded).to_string()
    } else {
        recorded_text(&recorded)
    })
}

/// The value the words on the command line mean for a kind declared as
/// `declared`: text is taken as written, a list takes every word, and
/// anything else is JSON. For a kind that is not declared the words are
/// sent as text, and the service says the kind is unknown.
fn value_of(
    declared: Option<&Declaration>,
    kind: &str,
    words: &[String],
) -> Result<Value, ServiceError> {
    let value_type = declared.map_or(ValueType::Text, |declared| declared.value);
    if value_type == ValueType::Texts {
        return Ok(json!(words));
    }
    let [word] = words else {
        return Err(ServiceError::new(
            codes::WRONG_VALUE_TYPE,
            format!(
                "{kind} takes one value, not {}; put text with spaces in quotes",
                words.len()
            ),
        ));
    };
    if value_type == ValueType::Text {
        return Ok(json!(word));
    }
    serde_json::from_str(word).map_err(|_| {
        ServiceError::new(
            codes::WRONG_VALUE_TYPE,
            format!(
                "{kind} takes {}, not \"{word}\"",
                declared.map_or_else(|| "JSON".to_owned(), Declaration::takes)
            ),
        )
    })
}

/// The Facts in `text`: one JSON object per line, or one JSON array.
fn facts_in(text: &str) -> Result<Vec<Value>, ServiceError> {
    if text.trim_start().starts_with('[') {
        return serde_json::from_str(text)
            .map_err(|e| bad(format!("the input is not a JSON array of Facts: {e}")));
    }
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(at, line)| {
            serde_json::from_str(line)
                .map_err(|e| bad(format!("line {} is not a JSON object: {e}", at + 1)))
        })
        .collect()
}

fn bad(message: String) -> ServiceError {
    ServiceError::new(wreckordbox_service::codes::BAD_REQUEST, message)
}

fn source_text(source: &Source) -> String {
    let kind = match source.kind {
        SourceKind::Dj => return "dj".to_owned(),
        SourceKind::Agent => "agent",
        SourceKind::Tool => "tool",
        SourceKind::Web => "web",
    };
    format!("{kind}:{}", source.name)
}

fn recorded_text(recorded: &Recorded) -> String {
    let run = recorded.run.id.0;
    let mut text = if let [fact] = recorded.facts.as_slice() {
        format!(
            "Recorded Fact {} in Run {run}: track {} {} = {} ({}, confidence {}).",
            fact.id,
            fact.track_id,
            fact.kind,
            fact.value,
            source_text(&fact.source),
            fact.confidence
        )
    } else {
        let mut tracks: Vec<&str> = recorded
            .facts
            .iter()
            .map(|fact| fact.track_id.as_str())
            .collect();
        tracks.sort_unstable();
        tracks.dedup();
        format!(
            "Recorded {} Facts about {} {} in Run {run}.",
            recorded.facts.len(),
            tracks.len(),
            if tracks.len() == 1 { "track" } else { "tracks" }
        )
    };
    if !recorded.marked_stale.is_empty() {
        let _ = write!(
            text,
            "\n{} earlier Measurement(s) were marked stale: the audio has changed since.",
            recorded.marked_stale.len()
        );
    }
    text
}

fn shown_text(shown: &TrackFacts) -> String {
    let track = &shown.track;
    if shown.facts.is_empty() && shown.stale.is_empty() && shown.unverified.is_empty() {
        return format!("Track {} \"{}\" has no Facts.", track.id, track.title);
    }
    let mut rows = vec![[
        "FACT",
        "KIND",
        "VALUE",
        "SOURCE",
        "CONFIDENCE",
        "RUN",
        "RECORDED",
        "NOTE",
    ]
    .map(str::to_owned)];
    rows.extend(shown.facts.iter().map(fact_row));
    for fact in shown.stale.iter().chain(&shown.unverified) {
        if !shown.facts.iter().any(|standing| standing.id == fact.id) {
            let mut row = fact_row(fact);
            if !fact.stale {
                row[7] = "unverified: audio unavailable".to_owned();
            }
            rows.push(row);
        }
    }
    format!("Track {} \"{}\"\n{}", track.id, track.title, table(&rows))
}

fn fact_row(fact: &Fact) -> [String; 8] {
    let note = match (fact.stale, fact.superseded) {
        (true, true) => "stale, replaced",
        (true, false) => "stale",
        (false, true) => "replaced",
        (false, false) => "",
    };
    [
        fact.id.to_string(),
        fact.kind.clone(),
        value_text(&fact.value),
        source_text(&fact.source),
        fact.confidence.to_string(),
        fact.run.to_string(),
        time_text(fact.recorded_at),
        note.to_owned(),
    ]
}

/// A value on one line: text as it is, a list of texts with commas,
/// anything else as JSON, cut short when long.
fn value_text(value: &Value) -> String {
    let text = match value {
        Value::String(text) => text.clone(),
        Value::Array(items) if items.iter().all(Value::is_string) => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        other => other.to_string(),
    };
    if text.chars().count() > 60 {
        let mut short: String = text.chars().take(57).collect();
        short.push_str("...");
        short
    } else if text.is_empty() {
        "-".to_owned()
    } else {
        text
    }
}

fn kinds_text(kinds: &DeclaredKinds) -> String {
    let mut rows = vec![["KIND", "NATURE", "VALUE", "DECLARED"].map(str::to_owned)];
    rows.extend(kinds.kinds.iter().map(|kind| {
        [
            kind.name.clone(),
            kind.nature.to_string(),
            kind.takes(),
            if kind.built_in {
                "built in"
            } else {
                "kinds file"
            }
            .to_owned(),
        ]
    }));
    format!("{}\nKinds file: {}", table(&rows), kinds.file.display())
}

/// Rows as a table: columns two spaces apart, as wide as their longest
/// cell.
fn table<const N: usize>(rows: &[[String; N]]) -> String {
    let widths: [usize; N] = std::array::from_fn(|column| {
        rows.iter()
            .map(|row| row[column].chars().count())
            .max()
            .unwrap_or(0)
    });
    rows.iter()
        .map(|row| {
            let line: Vec<String> = row
                .iter()
                .zip(widths)
                .map(|(cell, width)| format!("{cell:<width$}"))
                .collect();
            line.join("  ").trim_end().to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Milliseconds since the Unix epoch as `2026-11-11 14:03:22Z`.
fn time_text(millis: i64) -> String {
    let seconds = millis.div_euclid(1000);
    let (days, of_day) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    // Days to a civil date, in the proleptic Gregorian calendar.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}Z",
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60
    )
}
