//! `wreckordbox project --mytags`: Projection of the Tag schema to native
//! MyTags (wbx-29).
use std::fmt::Write as _;

use serde_json::json;
use wreckordbox_service::mytag_projection::{commands, Assignment, Report};
use wreckordbox_service::projection::Wanted;
use wreckordbox_service::{Client, ServiceError};

use crate::facts::source_text;

pub fn run(
    client: &mut Client,
    as_json: bool,
    dry_run: bool,
    wanted: Wanted,
) -> Result<String, ServiceError> {
    let command = if dry_run {
        commands::PLAN
    } else {
        commands::APPLY
    };
    let report: Report = client.call(command, json!(wanted))?;
    if as_json {
        return Ok(json!(report).to_string());
    }
    Ok(text(&report))
}

fn line(out: &mut String, a: &Assignment) {
    let _ = writeln!(
        out,
        "  {} {:?} {}: {} {:?}{}{} (Fact {}, {}, confidence {})",
        a.track,
        a.title,
        a.category,
        a.change,
        a.label,
        if a.implied {
            ", the parent of a term"
        } else {
            ""
        },
        a.reason
            .as_ref()
            .map(|r| format!("; {r}"))
            .unwrap_or_default(),
        a.fact,
        source_text(&a.source),
        a.confidence
    );
}

fn text(report: &Report) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{} {} MyTag {} (plan {}).",
        if report.dry_run {
            "Would change"
        } else {
            "Changed"
        },
        report.changes.len(),
        if report.changes.len() == 1 {
            "assignment"
        } else {
            "assignments"
        },
        report.plan
    );
    for a in &report.changes {
        line(&mut out, a);
    }
    for c in &report.categories {
        if c.state != "matched" {
            let _ = writeln!(
                out,
                "The deck's category {:?} ({}) is {} in the Library: nothing is written for it.",
                c.label, c.dimension, c.state
            );
        }
    }
    if !report.outside_deck.is_empty() {
        let names: Vec<&str> = report
            .outside_deck
            .iter()
            .map(|o| o.category.as_str())
            .collect();
        let _ = writeln!(out, "Not on the deck, never written: {}.", names.join(", "));
    }
    if !report.definitions.is_empty() {
        let _ = writeln!(
            out,
            "MyTags the schema has and the Library does not: {} (make them by hand first; applying refuses until then).",
            report.definitions.len()
        );
        for d in &report.definitions {
            let _ = writeln!(out, "  {}: {:?} ({})", d.category, d.label, d.term);
        }
    }
    for (name, list) in [
        ("Held for review", &report.held),
        ("The DJ's MyTags kept", &report.kept),
        ("Nowhere to put", &report.blocked),
    ] {
        if !list.is_empty() {
            let _ = writeln!(out, "{name}: {}.", list.len());
            for a in list {
                line(&mut out, a);
            }
        }
    }
    if !report.invalid.is_empty() {
        let _ = writeln!(out, "Not usable: {}.", report.invalid.len());
        for u in &report.invalid {
            let _ = writeln!(
                out,
                "  {} {:?} {}: Fact {}: {}",
                u.track, u.title, u.dimension, u.fact, u.why
            );
        }
    }
    let _ = writeln!(out, "Already in the Library: {}.", report.unchanged);
    if report.library_protected {
        let _ = writeln!(out, "Library Protection is on: applying would be refused.");
    }
    for warning in &report.warnings {
        let _ = writeln!(out, "Warning: {warning}");
    }
    let _ = write!(
        out,
        "Policy: {} (hash {}); Tag schema: {} (hash {})",
        report.policy_file.display(),
        report.policy_hash,
        report.schema_file.display(),
        report.schema_hash
    );
    out
}
