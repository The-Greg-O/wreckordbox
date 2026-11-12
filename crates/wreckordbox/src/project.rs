//! `wreckordbox project`: Projection of Energy and Prep colour (wbx-26).
use std::fmt::Write as _;

use clap::Args;
use serde_json::{json, Value};
use wreckordbox_service::projection::{commands, Report, Wanted};
use wreckordbox_service::{Client, ServiceError};

use crate::facts::source_text;

/// What `wreckordbox project` takes.
#[derive(Args, Debug, Clone)]
pub struct ProjectArgs {
    /// List every track and field that would change, with before and
    /// after, and what would be held or kept. Writes nothing.
    #[arg(long, conflicts_with = "expect")]
    dry_run: bool,
    /// The plan a dry run printed: apply only if exactly those changes
    /// would be made, else refuse and write nothing.
    #[arg(long, value_name = "PLAN")]
    expect: Option<String>,
    /// Only this track; repeat for more. Every track with Facts when left
    /// out.
    #[arg(long = "track", value_name = "TRACK_ID")]
    tracks: Vec<String>,
}

impl ProjectArgs {
    pub fn dry_run(&self) -> bool {
        self.dry_run
    }
}

pub fn run(client: &mut Client, as_json: bool, args: &ProjectArgs) -> Result<String, ServiceError> {
    let command = if args.dry_run {
        commands::PROJECTION_PLAN
    } else {
        commands::PROJECTION_APPLY
    };
    let report: Report = client.call(
        command,
        json!(Wanted {
            tracks: args.tracks.clone(),
            expect: args.expect.clone(),
        }),
    )?;
    if as_json {
        return Ok(json!(report).to_string());
    }
    Ok(text(&report))
}

fn shown(value: &Value) -> String {
    match value {
        Value::Null => "none".to_owned(),
        Value::Object(colour) => match (colour.get("label"), colour.get("colour")) {
            (Some(label), Some(name)) if label == name => name.as_str().unwrap_or("").to_owned(),
            (Some(label), Some(name)) => format!(
                "{} ({})",
                label.as_str().unwrap_or(""),
                name.as_str().unwrap_or("")
            ),
            _ => value.to_string(),
        },
        other => other.to_string(),
    }
}

fn text(report: &Report) -> String {
    let field = |f| {
        serde_json::to_value(f)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default()
    };
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{} {} {} (plan {}).",
        if report.dry_run {
            "Would change"
        } else {
            "Changed"
        },
        report.changes.len(),
        if report.changes.len() == 1 {
            "field"
        } else {
            "fields"
        },
        report.plan
    );
    for c in &report.changes {
        let _ = writeln!(
            out,
            "  {} {:?} {}: {} -> {} (Fact {}, {}, confidence {})",
            c.track,
            c.title,
            field(c.field),
            shown(&c.before),
            shown(&c.after),
            c.fact,
            source_text(&c.source),
            c.confidence
        );
    }
    if !report.held.is_empty() {
        let _ = writeln!(out, "Held for review: {}.", report.held.len());
        for h in &report.held {
            let _ = writeln!(
                out,
                "  {} {:?} {}: {} -> {} ({}; Fact {}, {}, confidence {})",
                h.track,
                h.title,
                field(h.field),
                shown(&h.before),
                shown(&h.after),
                h.reason,
                h.fact,
                source_text(&h.source),
                h.confidence
            );
        }
    }
    if !report.kept.is_empty() {
        let _ = writeln!(out, "The DJ's values kept: {}.", report.kept.len());
        for k in &report.kept {
            let _ = writeln!(
                out,
                "  {} {:?} {}: kept {}; Fact {} ({}) says {}",
                k.track,
                k.title,
                field(k.field),
                shown(&k.value),
                k.fact,
                source_text(&k.source),
                shown(&k.fact_value)
            );
        }
    }
    for (name, list) in [
        ("Not usable", &report.invalid),
        ("Stale, never projected", &report.stale),
    ] {
        if !list.is_empty() {
            let _ = writeln!(out, "{name}: {}.", list.len());
            for u in list {
                let _ = writeln!(
                    out,
                    "  {} {:?} {}: Fact {}: {}",
                    u.track,
                    u.title,
                    field(u.field),
                    u.fact,
                    u.why
                );
            }
        }
    }
    let _ = writeln!(out, "Already in the Library: {}.", report.unchanged);
    if report.library_protected {
        let _ = writeln!(out, "Library Protection is on: applying would be refused.");
    }
    let _ = write!(out, "Policy: {}", report.policy_file.display());
    out
}
