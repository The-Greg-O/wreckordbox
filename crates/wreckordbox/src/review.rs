//! `wreckordbox review` (wbx-27): the Facts waiting for the DJ, and the
//! DJ's decision on them.
use std::fmt::Write as _;

use clap::{Args, Subcommand};
use serde_json::json;
use wreckordbox_service::facts::SourceArg;
use wreckordbox_service::projection::{Field, Held};
use wreckordbox_service::review::{commands, Decided, Selection, Waiting};
use wreckordbox_service::{Config, ServiceError};

use crate::facts::{parse_source, source_text};

/// Which waiting Facts.
#[derive(Args, Debug, Clone, Default)]
pub struct Filter {
    /// Only this track; repeat for more.
    #[arg(long = "track", value_name = "TRACK_ID")]
    tracks: Vec<String>,
    /// Only this field: `energy` or `prep_colour`.
    #[arg(long, value_name = "FIELD", value_parser = parse_field)]
    field: Option<Field>,
    /// Only Facts from this source (`agent:NAME`, `tool:NAME`, ...).
    #[arg(long, value_name = "SOURCE", value_parser = parse_source)]
    from: Option<SourceArg>,
    /// Only Facts at or above this confidence (0 to 1).
    #[arg(long, value_name = "CONFIDENCE")]
    at_least: Option<f64>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ReviewCommands {
    /// List the Facts Projection held back for the DJ: track, field, the
    /// value now and the value proposed, source, confidence and why it
    /// waits. Writes nothing.
    Waiting {
        #[command(flatten)]
        filter: Filter,
    },
    /// Accept waiting Facts, as the DJ's own hand set: their values are
    /// written to the Library through the edit path, as one Undo step, in
    /// the DJ's Run. Only `--source dj` (or `--run` of the DJ's) is
    /// accepted; never run this for an agent or a tool.
    Accept {
        /// These Facts, by id; each must be waiting.
        #[arg(value_name = "FACT")]
        facts: Vec<i64>,
        #[command(flatten)]
        filter: Filter,
        /// Every waiting Fact.
        #[arg(long)]
        all: bool,
        #[command(flatten)]
        run: crate::ledger::RunOptions,
    },
    /// Reject waiting Facts, as the DJ's decision: recorded in the Ledger
    /// as the DJ's, and never offered or projected again. Writes nothing to
    /// the Library. Only `--source dj` is accepted.
    Reject {
        /// These Facts, by id; each must be waiting.
        #[arg(value_name = "FACT")]
        facts: Vec<i64>,
        #[command(flatten)]
        filter: Filter,
        /// Every waiting Fact.
        #[arg(long)]
        all: bool,
        /// Who rejects: must be `dj`.
        #[arg(long, value_name = "SOURCE", value_parser = parse_source)]
        source: Option<SourceArg>,
    },
}

fn parse_field(text: &str) -> Result<Field, String> {
    serde_json::from_value(json!(text))
        .map_err(|_| format!("{text:?} is not a field: energy or prep_colour"))
}

fn selection(facts: &[i64], filter: &Filter, all: bool, source: Option<&SourceArg>) -> Selection {
    Selection {
        facts: facts.to_vec(),
        tracks: filter.tracks.clone(),
        field: filter.field,
        from: filter.from.clone(),
        at_least: filter.at_least,
        all,
        source: source.cloned(),
    }
}

pub fn run(
    config: &Config,
    as_json: bool,
    command: &ReviewCommands,
) -> Result<String, ServiceError> {
    // Only an accept edits the Library, so only it takes a Run; a
    // rejection names its own source.
    if let ReviewCommands::Accept { run, .. } = command {
        crate::ledger::begin(run, as_json);
    }
    let client = &mut crate::connect(config)?;
    match command {
        ReviewCommands::Waiting { filter } => {
            let listed: Waiting = client.call(
                commands::REVIEW_WAITING,
                json!(selection(&[], filter, false, None)),
            )?;
            if as_json {
                return Ok(json!(listed).to_string());
            }
            Ok(waiting_text(&listed))
        }
        ReviewCommands::Accept {
            facts, filter, all, ..
        } => {
            let done: Decided = client.call(
                commands::REVIEW_ACCEPT,
                json!(selection(facts, filter, *all, None)),
            )?;
            if as_json {
                return Ok(json!(done).to_string());
            }
            Ok(decided_text("Accepted", &done))
        }
        ReviewCommands::Reject {
            facts,
            filter,
            all,
            source,
        } => {
            let done: Decided = client.call(
                commands::REVIEW_REJECT,
                json!(selection(facts, filter, *all, source.as_ref())),
            )?;
            if as_json {
                return Ok(json!(done).to_string());
            }
            let mut out = decided_text("Rejected", &done);
            if let Some(r) = &done.rejection {
                let _ = write!(
                    out,
                    "Recorded as the DJ's in Ledger entry {} (Run {}); the Library is unchanged.",
                    r.entry, r.run
                );
            }
            Ok(out)
        }
    }
}

fn field_name(field: Field) -> &'static str {
    match field {
        Field::Energy => "energy",
        Field::PrepColour => "prep_colour",
    }
}

fn line(out: &mut String, h: &Held) {
    let _ = writeln!(
        out,
        "  Fact {}: track {} {:?}, {} {} -> {}, from {} at {}, {}",
        h.fact,
        h.track,
        h.title,
        field_name(h.field),
        crate::project::shown(&h.before),
        crate::project::shown(&h.after),
        source_text(&h.source),
        h.confidence,
        match (h.reason.as_str(), h.threshold) {
            ("below_confidence", Some(at)) => format!("below the policy's {at}"),
            ("waits_for_dj", _) => "the policy waits for the DJ".to_owned(),
            (other, _) => other.to_owned(),
        }
    );
}

fn waiting_text(listed: &Waiting) -> String {
    if listed.waiting.is_empty() {
        return "Nothing is waiting for the DJ.".to_owned();
    }
    let mut out = format!("{} Fact(s) waiting for the DJ:\n", listed.count);
    for h in &listed.waiting {
        line(&mut out, h);
    }
    if listed.library_protected {
        out.push_str("Library Protection is on: accepting would be refused.\n");
    }
    out
}

fn decided_text(what: &str, done: &Decided) -> String {
    if done.decided.is_empty() {
        return "Nothing waiting matched: nothing was done.\n".to_owned();
    }
    let mut out = format!("{what} {} Fact(s):\n", done.count);
    for h in &done.decided {
        line(&mut out, h);
    }
    out
}
