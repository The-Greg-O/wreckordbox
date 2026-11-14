//! `wreckordbox review` (wbx-27): the Facts waiting for the DJ, and the
//! DJ's decision on them.
use std::fmt::Write as _;

use clap::{Args, Subcommand};
use serde_json::json;
use wreckordbox_service::facts::SourceArg;
use wreckordbox_service::projection::{Field, Held};
use wreckordbox_service::review::{commands, Decided, RejectedList, Reopened, Selection, Waiting};
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
    /// the DJ's Run. `--source dj` is required every time, also with
    /// `--run`; never run this for an agent or a tool. The Facts are read
    /// first and the accept is bound to them: anything changed since is
    /// refused by name.
    Accept {
        /// These Facts, by id; each must be waiting.
        #[arg(value_name = "FACT")]
        facts: Vec<i64>,
        #[command(flatten)]
        filter: Filter,
        /// Every waiting Fact.
        #[arg(long)]
        all: bool,
        /// The token `review waiting` printed with the listing the DJ
        /// decided from. Required: the accept is refused, showing what
        /// changed, if anything in that listing changed since.
        #[arg(long, value_name = "TOKEN", required = true)]
        token: String,
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
    /// List the suggestions the DJ rejected (track, field, value, source)
    /// and the rejection each is in. Writes nothing.
    Rejected {
        #[command(flatten)]
        filter: Filter,
    },
    /// Reopen rejected suggestions, as the DJ's decision: they wait again.
    /// Recorded in the Ledger; the rejection stays recorded. Only
    /// `--source dj` is accepted.
    Reopen {
        /// The Facts rejected, by id.
        #[arg(value_name = "FACT")]
        facts: Vec<i64>,
        #[command(flatten)]
        filter: Filter,
        /// Every rejected suggestion.
        #[arg(long)]
        all: bool,
        /// Every suggestion one earlier `review reject` rejected: its
        /// Ledger entry.
        #[arg(long, value_name = "ENTRY")]
        rejection: Option<i64>,
        /// Who reopens: must be `dj`.
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
        ..Selection::default()
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
            facts,
            filter,
            all,
            token,
            run,
        } => {
            // Bound to the listing the DJ saw: never listed again here.
            let mut asked = selection(facts, filter, *all, run.source());
            asked.token = Some(token.clone());
            let done: Decided = client.call(commands::REVIEW_ACCEPT, json!(asked))?;
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
        ReviewCommands::Rejected { filter } => {
            let listed: RejectedList = client.call(
                commands::REVIEW_REJECTED,
                json!(selection(&[], filter, false, None)),
            )?;
            if as_json {
                return Ok(json!(listed).to_string());
            }
            if listed.rejected.is_empty() {
                return Ok("No suggestion stands rejected.".to_owned());
            }
            let mut out = format!("{} suggestion(s) rejected by the DJ:\n", listed.count);
            for r in &listed.rejected {
                let o = &r.offered;
                let _ = writeln!(
                    out,
                    "  Fact {}: track {}, {} {}, from {}, rejected in Ledger entry {}",
                    o.fact,
                    o.track,
                    o.field,
                    crate::project::shown(&o.value),
                    source_text(&o.source),
                    r.rejection.entry
                );
            }
            Ok(out)
        }
        ReviewCommands::Reopen {
            facts,
            filter,
            all,
            rejection,
            source,
        } => {
            let mut asked = selection(facts, filter, *all, source.as_ref());
            asked.rejection = *rejection;
            let done: Reopened = client.call(commands::REVIEW_REOPEN, json!(asked))?;
            if as_json {
                return Ok(json!(done).to_string());
            }
            Ok(match &done.reopening {
                None => "Nothing rejected matched: nothing was reopened.".to_owned(),
                Some(r) => format!(
                    "Reopened {} suggestion(s); they wait again. Recorded as the DJ's in Ledger entry {} (Run {}).",
                    done.count, r.entry, r.run
                ),
            })
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
    let _ = writeln!(
        out,
        "To accept from this listing: wreckordbox review accept ... --token {} --source dj",
        listed.token
    );
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
