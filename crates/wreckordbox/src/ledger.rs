//! The Ledger from the command line: the Run options every command that
//! edits the Library takes, the Run such a command reports, and
//! `wreckordbox runs list` and `runs show`.

use std::fmt::Write as _;
use std::sync::{Arc, Mutex, PoisonError};

use clap::Args;
use serde_json::{json, Value};
use wreckordbox_service::facts::{Run, SourceArg};
use wreckordbox_service::ledger::{
    commands, LedgerEntry, ListRuns, RunArg, RunList, RunShown, RunSummary, ShowRun,
};
use wreckordbox_service::{Client, ServiceError};

use crate::facts::{parse_source, source_text, table, time_text};

/// Which Run a command's changes belong to, and who they are by. Given
/// anywhere after `tracks`, `mytags` or `intake`.
#[derive(Args, Debug, Clone, Default)]
pub struct RunOptions {
    /// The open Run these changes belong to, from `runs start`. Without
    /// it the command is a Run of its own.
    #[arg(long, global = true, value_name = "RUN_ID")]
    run: Option<i64>,
    /// Who the change is by: `dj`, `agent:NAME`, `tool:NAME` or
    /// `web:ADDRESS`. Without it, and without `--run`, the change is
    /// recorded as made by `tool:wreckordbox`, never as the DJ's.
    #[arg(long, global = true, value_name = "SOURCE", value_parser = parse_source)]
    source: Option<SourceArg>,
    /// What the work is, for whoever reviews the Run later.
    #[arg(long, global = true, value_name = "TEXT", conflicts_with = "run")]
    description: Option<String>,
}

impl RunOptions {
    /// What to send with the command; nothing when nothing was given.
    fn named(&self) -> Option<RunArg> {
        (self.run.is_some() || self.source.is_some() || self.description.is_some()).then(|| {
            RunArg {
                id: self.run,
                source: self.source.clone(),
                description: self.description.clone(),
            }
        })
    }
}

/// What this one command was given, and the Run the service answered
/// with, for the last line of its output.
struct Invocation {
    options: RunOptions,
    as_json: bool,
    reported: Arc<Mutex<Option<Run>>>,
}

static INVOCATION: Mutex<Option<Invocation>> = Mutex::new(None);

/// Notes the Run options this command was given, for [`attribute`] and
/// [`with_run`].
pub fn begin(options: &RunOptions, as_json: bool) {
    *INVOCATION.lock().unwrap_or_else(PoisonError::into_inner) = Some(Invocation {
        options: options.clone(),
        as_json,
        reported: Arc::new(Mutex::new(None)),
    });
}

/// Sends the command's Run options with every command `client` makes, and
/// keeps the Run the service answers with for [`with_run`].
pub fn attribute(client: &mut Client) {
    if let Some(invocation) = INVOCATION
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
    {
        client.in_run(invocation.options.named());
        client.tell_runs(Arc::clone(&invocation.reported));
    }
}

/// `output` with the Run the command's changes are in: a `run` key in the
/// JSON object; in text a last line saying which Run, also for an edit
/// made by hand, whose Run was made for it. Unchanged when the command
/// changed nothing in the Library.
pub fn with_run(output: &str) -> String {
    let (reported, as_json) = match INVOCATION
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
    {
        Some(invocation) => (
            invocation
                .reported
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            invocation.as_json,
        ),
        _ => (None, false),
    };
    let Some(run) = reported else {
        return output.to_owned();
    };
    if as_json {
        return match serde_json::from_str::<Value>(output) {
            Ok(Value::Object(mut object)) => {
                object.insert("run".to_owned(), json!(run));
                Value::Object(object).to_string()
            }
            _ => output.to_owned(),
        };
    }
    format!(
        "{output}\nRun {}: {} ({}).",
        run.id.0,
        run.description,
        source_text(&run.source)
    )
}

/// `runs list`.
pub fn list(
    client: &mut Client,
    as_json: bool,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<String, ServiceError> {
    let list: RunList = client.call(commands::RUNS_LIST, json!(ListRuns { limit, offset }))?;
    if as_json {
        return Ok(json!(list).to_string());
    }
    if list.total == 0 {
        return Ok("No Runs.".to_owned());
    }
    let mut rows = vec![[
        "RUN",
        "SOURCE",
        "STARTED",
        "ENDED",
        "CHANGES",
        "DESCRIPTION",
    ]
    .map(str::to_owned)];
    rows.extend(list.runs.iter().map(|run| {
        [
            run.run.id.0.to_string(),
            source_text(&run.run.source),
            time_text(run.run.started_at),
            run.run
                .finished_at
                .map_or_else(|| "open".to_owned(), time_text),
            counts_text(run),
            run.run.description.clone(),
        ]
    }));
    let mut text = table(&rows);
    footer(
        &mut text,
        "Runs",
        list.offset,
        list.runs.len(),
        list.total,
        list.next_offset,
    );
    Ok(text)
}

/// `runs show`.
pub fn show(
    client: &mut Client,
    as_json: bool,
    id: i64,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<String, ServiceError> {
    let shown: RunShown = client.call(commands::RUNS_SHOW, json!(ShowRun { id, limit, offset }))?;
    if as_json {
        return Ok(json!(shown).to_string());
    }
    let run = &shown.run.run;
    let mut text = format!(
        "Run {}: {} ({})\nStarted {}; {}\nChanges: {}",
        run.id.0,
        run.description,
        source_text(&run.source),
        time_text(run.started_at),
        match (run.finished_at, &run.outcome) {
            (Some(at), Some(outcome)) => format!("ended {} ({outcome})", time_text(at)),
            (Some(at), None) => format!("ended {}", time_text(at)),
            (None, _) => "still open".to_owned(),
        },
        counts_text(&shown.run),
    );
    if shown.total == 0 {
        text.push_str("\nThis Run changed nothing.");
        return Ok(text);
    }
    let mut rows =
        vec![["ENTRY", "AT", "STORE", "ACTION", "KIND", "SOURCE", "WHAT"].map(str::to_owned)];
    for entry in &shown.entries {
        let mut what = what_lines(entry).into_iter();
        rows.push([
            entry.id.to_string(),
            time_text(entry.at),
            json!(entry.store).as_str().unwrap_or("").to_owned(),
            json!(entry.action).as_str().unwrap_or("").to_owned(),
            entry.kind.clone(),
            source_text(&entry.source),
            what.next().unwrap_or_default(),
        ]);
        // One line for each further value the entry changed.
        rows.extend(what.map(|line| {
            let mut row: [String; 7] = Default::default();
            row[6] = line;
            row
        }));
    }
    text.push('\n');
    text.push_str(&table(&rows));
    footer(
        &mut text,
        "Entries",
        shown.offset,
        shown.entries.len(),
        shown.total,
        shown.next_offset,
    );
    Ok(text)
}

/// What an entry changed, a line for each value; its label when it holds
/// no values.
fn what_lines(entry: &LedgerEntry) -> Vec<String> {
    let mut lines: Vec<String> = entry
        .values
        .iter()
        .map(|value| {
            format!(
                "{} {}: {} -> {}",
                value.subject, value.field, value.before, value.after
            )
        })
        .collect();
    if lines.is_empty() {
        lines.push(if entry.reversible {
            entry.label.clone()
        } else {
            format!("{} (values not kept)", entry.label)
        });
    }
    if let (Some(reverses), Some(first)) = (entry.reverses, lines.first_mut()) {
        let _ = write!(first, " (of entry {reverses})");
    }
    for omitted in &entry.facts_not_recorded {
        lines.push(format!(
            "Fact not recorded: track {} {}: {}",
            omitted.track, omitted.kind, omitted.reason
        ));
    }
    lines
}

fn counts_text(run: &RunSummary) -> String {
    if run.counts.is_empty() {
        return "none".to_owned();
    }
    run.counts
        .iter()
        .map(|count| {
            format!(
                "{} {} {}",
                count.count,
                json!(count.store).as_str().unwrap_or(""),
                count.kind
            )
        })
        .collect::<Vec<String>>()
        .join(", ")
}

fn footer(
    text: &mut String,
    what: &str,
    offset: usize,
    shown: usize,
    total: usize,
    next: Option<usize>,
) {
    if shown == total {
        return;
    }
    let _ = write!(
        text,
        "\n{what} {} to {} of {total}",
        offset + 1,
        offset + shown
    );
    match next {
        Some(next) => {
            let _ = write!(text, "; next page: --offset {next}");
        }
        None => text.push('.'),
    }
}
