//! `lookup`: describing a track for an agent's search and checking a
//! candidate the agent found against it (ADR 0005). Wreckordbox fetches
//! nothing: the agent searches with its own tools and hands the candidate
//! over.

use std::fmt::Write as _;

use clap::Subcommand;
use serde_json::{json, Map, Value};
use wreckordbox_service::lookups::{
    commands, CandidateChecked, CheckCandidate, DescribeTrack, TrackDescription,
};
use wreckordbox_service::{Client, ServiceError};

use crate::facts::{bad, source_text};

#[derive(Subcommand)]
pub enum LookupCommands {
    /// Describe a track precisely, to search for: artist, title, mix name,
    /// duration, label, release date and the audio file's own tags. Reads
    /// only.
    Describe {
        /// The ID column of `tracks list`.
        #[arg(value_name = "TRACK_ID")]
        id: String,
    },
    /// Check a candidate found by a lookup against a track: accepted, held
    /// for review, or rejected, with the reason.
    ///
    /// The check is recorded as a Decision (topic `lookup_match`). When
    /// accepted, the candidate's fields become Facts from `web:ADDRESS`,
    /// waiting like any Fact that is not the DJ's; held or rejected, none
    /// do. Artist, title, mix name and duration must agree (durations
    /// within the tolerance); a different mix, a duration outside the
    /// tolerance, another release year or label is held; a different title
    /// or artist is rejected.
    Check {
        #[arg(value_name = "TRACK_ID")]
        id: String,
        /// The candidate's fields as a JSON object: `artist` and `title`,
        /// and as found `mix_name`, `duration` (seconds or "m:ss"), `label`,
        /// `release_date`, and any declared kind of Fact (`genre`, ...).
        #[arg(long, value_name = "JSON")]
        candidate: String,
        /// Where the candidate was found.
        #[arg(long, value_name = "ADDRESS")]
        address: String,
        /// When it was found, YYYY-MM-DD.
        #[arg(long, value_name = "DATE")]
        date: String,
        /// How far apart the durations may be, in seconds (default 3).
        #[arg(long, value_name = "SECONDS")]
        tolerance: Option<f64>,
        /// Record the check in this open Run, started with `runs start`.
        #[arg(long, value_name = "RUN_ID")]
        run: Option<i64>,
        /// Check and say what would be recorded, recording nothing.
        #[arg(long)]
        dry_run: bool,
    },
}

pub fn run(
    client: &mut Client,
    as_json: bool,
    command: &LookupCommands,
) -> Result<String, ServiceError> {
    match command {
        LookupCommands::Describe { id } => {
            let described: TrackDescription = client.call(
                commands::LOOKUP_DESCRIBE,
                json!(DescribeTrack { id: id.clone() }),
            )?;
            Ok(if as_json {
                json!(described).to_string()
            } else {
                description_text(&described)
            })
        }
        LookupCommands::Check {
            id,
            candidate,
            address,
            date,
            tolerance,
            run,
            dry_run,
        } => {
            let candidate: Map<String, Value> = serde_json::from_str(candidate)
                .map_err(|e| bad(format!("--candidate is a JSON object of fields: {e}")))?;
            let checked: CandidateChecked = client.call(
                commands::LOOKUP_CHECK,
                json!(CheckCandidate {
                    id: id.clone(),
                    candidate,
                    address: address.clone(),
                    date: date.clone(),
                    tolerance_seconds: *tolerance,
                    run: *run,
                    dry_run: *dry_run,
                }),
            )?;
            Ok(if as_json {
                json!(checked).to_string()
            } else {
                checked_text(&checked)
            })
        }
    }
}

fn or_none(text: &str) -> &str {
    if text.is_empty() {
        "(none)"
    } else {
        text
    }
}

fn clock(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn description_text(described: &TrackDescription) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Track  {}", described.track);
    let _ = writeln!(out, "Search  {}", described.search);
    let _ = writeln!(out, "Artist  {}", or_none(&described.artist));
    let _ = writeln!(out, "Title  {}", or_none(&described.title));
    let _ = writeln!(out, "Mix name  {}", or_none(&described.mix_name));
    let _ = writeln!(
        out,
        "Duration  {}",
        if described.duration_seconds == 0 {
            "(unknown)".to_owned()
        } else {
            clock(described.duration_seconds)
        }
    );
    let _ = writeln!(out, "Label  {}", or_none(&described.label));
    let _ = writeln!(
        out,
        "Released  {}",
        described
            .release_date
            .clone()
            .or_else(|| described.year.map(|year| year.to_string()))
            .unwrap_or_else(|| "(unknown)".to_owned())
    );
    let _ = writeln!(out, "File  {}", described.file.path);
    match (&described.file.tags, &described.file.problem) {
        (Some(tags), _) => {
            let _ = writeln!(
                out,
                "File tags  title \"{}\", artist \"{}\", album \"{}\", genre \"{}\", label \"{}\", year {}, key \"{}\", {}",
                tags.title,
                tags.artist,
                tags.album,
                tags.genre,
                tags.label,
                tags.year.map_or_else(|| "(none)".to_owned(), |y| y.to_string()),
                tags.key,
                clock(tags.duration_seconds)
            );
        }
        (None, Some(problem)) => {
            let _ = writeln!(out, "File tags  not read: {problem}");
        }
        (None, None) => {}
    }
    out.trim_end().to_owned()
}

fn checked_text(checked: &CandidateChecked) -> String {
    let mut out = String::new();
    let verdict = match checked.verdict {
        wreckordbox_service::lookup_match::Verdict::Accepted => "Accepted",
        wreckordbox_service::lookup_match::Verdict::Held => "Held for review",
        wreckordbox_service::lookup_match::Verdict::Rejected => "Rejected",
    };
    let _ = writeln!(
        out,
        "{verdict}: the candidate from {} (looked up on {}) for track {}.",
        checked.address, checked.date, checked.track
    );
    let _ = writeln!(out, "Reason: {}.", checked.reason);
    if checked.dry_run {
        let _ = writeln!(out, "Dry run: nothing was recorded.");
        for fact in &checked.proposed {
            let _ = writeln!(out, "Would record {} = {}", fact.kind, fact.value);
        }
    }
    if let Some(decision) = &checked.decision {
        let _ = writeln!(
            out,
            "Recorded as Decision {} ({}).",
            decision.id,
            decision.topic.as_deref().unwrap_or("")
        );
    }
    for fact in &checked.facts {
        let _ = writeln!(
            out,
            "Recorded Fact {}: {} = {} from {}",
            fact.id,
            fact.kind,
            fact.value,
            source_text(&fact.source)
        );
    }
    for skipped in &checked.not_recorded {
        let _ = writeln!(
            out,
            "Not recorded: {} ({}): {}",
            skipped.field, skipped.code, skipped.message
        );
    }
    out.trim_end().to_owned()
}
