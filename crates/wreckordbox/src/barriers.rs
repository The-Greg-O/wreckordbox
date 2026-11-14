//! `wreckordbox projection barriers` and `wreckordbox projection
//! acknowledge` (wbx-xme): see Projection's barriers, and clear one as the
//! DJ.
use std::fmt::Write as _;

use clap::Subcommand;
use serde_json::json;
use wreckordbox_service::facts::SourceArg;
use wreckordbox_service::projection_barriers::{
    commands, Acknowledged, BarrierKind, Barriers, ALLOWS,
};
use wreckordbox_service::{Config, ServiceError};

use crate::facts::{parse_source, source_text};

#[derive(Subcommand, Debug, Clone)]
pub enum ProjectionCommands {
    /// List the writes the Ledger cannot account for, which stop
    /// Projection filling the fields they could concern: which write, when,
    /// which tracks, and whether the DJ cleared it. Writes nothing.
    Barriers,
    /// Clear one barrier, as the DJ's decision: recorded in the Ledger as
    /// the DJ's. Only `--source dj` is accepted; never run this for an
    /// agent or a tool. A cleared barrier lets Projection fill EMPTY fields
    /// again; it never makes a value a projection's.
    Acknowledge {
        /// The barrier's number, as `projection barriers` lists it.
        #[arg(value_name = "BARRIER")]
        barrier: i64,
        /// Who clears it: must be `dj`.
        #[arg(long, value_name = "SOURCE", value_parser = parse_source)]
        source: Option<SourceArg>,
    },
}

pub fn run(
    config: &Config,
    as_json: bool,
    command: &ProjectionCommands,
) -> Result<String, ServiceError> {
    let client = &mut crate::connect(config)?;
    match command {
        ProjectionCommands::Barriers => {
            let listed: Barriers = client.call(commands::PROJECTION_BARRIERS, json!({}))?;
            if as_json {
                return Ok(json!(listed).to_string());
            }
            Ok(barriers_text(&listed))
        }
        ProjectionCommands::Acknowledge { barrier, source } => {
            let done: Acknowledged = client.call(
                commands::PROJECTION_ACKNOWLEDGE,
                json!({"barrier": barrier, "source": source}),
            )?;
            if as_json {
                return Ok(json!(done).to_string());
            }
            Ok(format!(
                "Barrier {} cleared by the DJ (Ledger entry {}, Run {}).\n{ALLOWS}",
                done.barrier, done.clearing.entry, done.clearing.run
            ))
        }
    }
}

fn barriers_text(listed: &Barriers) -> String {
    let mut out = String::new();
    if listed.barriers.is_empty() {
        return "No barriers: Projection may fill every empty field.".to_owned();
    }
    let _ = writeln!(
        out,
        "{} barrier(s), {} standing.",
        listed.barriers.len(),
        listed.standing
    );
    for b in &listed.barriers {
        let what = match b.what {
            BarrierKind::Unconfirmed => "a write begun and never confirmed",
            BarrierKind::Unreadable => "an entry that cannot be read",
            BarrierKind::InProgress => "a write in progress",
        };
        let tracks = b.tracks.as_ref().map_or_else(
            || "every track".to_owned(),
            |t| format!("tracks {}", t.join(", ")),
        );
        let state = b.cleared.as_ref().map_or_else(
            || "standing".to_owned(),
            |c| format!("cleared by the DJ in entry {}", c.entry),
        );
        let _ = writeln!(
            out,
            "  {}: {what}, by {}{}{} at {} ms; {tracks}; {state}",
            b.seq,
            source_text(&b.source),
            b.command
                .as_ref()
                .map_or_else(String::new, |c| format!(" ({c})")),
            b.run.map_or_else(String::new, |r| format!(" in Run {r}")),
            b.at,
        );
    }
    out.push_str(&listed.allows);
    out
}
