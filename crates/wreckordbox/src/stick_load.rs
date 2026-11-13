//! `wreckordbox stick list`, `stick dry-run` and `stick load` (wbx-70):
//! loading a Stick through the Library service.
//!
//! The confirmation the window's hold gives is, here, two explicit steps
//! the DJ takes: unlocking the Stick (`stick unlock`, by name for a Gig
//! stick) and passing the plan id of the dry run to `stick load --plan`.
//! Agents must never unlock a Stick unless the DJ asked for that Stick, by
//! name (AGENTS.md, "Safety").

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use clap::Args;
use serde_json::json;
use wreckordbox_service::stick_load::{
    commands, events, Changes, DryRun, Held, Load, Space, Stick, Sticks,
};
use wreckordbox_service::{Client, ServiceError};

use crate::stick::StickCommands;

#[derive(Args)]
pub struct DryRunArgs {
    /// Where the Stick is mounted (its root), as `stick list` shows it.
    #[arg(value_name = "STICK")]
    pub stick: PathBuf,
    /// The playlists to load, by identifier (`playlists tree`); a folder
    /// stands for everything in it.
    #[arg(value_name = "PLAYLIST_ID", required = true)]
    pub playlists: Vec<String>,
}

#[derive(Args)]
pub struct LoadArgs {
    /// Where the Stick is mounted (its root), as `stick list` shows it.
    #[arg(value_name = "STICK")]
    pub stick: PathBuf,
    /// The playlists to load, as given to `stick dry-run`.
    #[arg(value_name = "PLAYLIST_ID", required = true)]
    pub playlists: Vec<String>,
    /// The plan id `stick dry-run` printed for exactly this load.
    #[arg(long, value_name = "PLAN_ID", required = true)]
    pub plan: String,
}

/// Whether `command` is one of these, which need the service.
pub fn uses_service(command: &StickCommands) -> bool {
    matches!(
        command,
        StickCommands::List | StickCommands::DryRun(_) | StickCommands::Load(_)
    )
}

/// The service does not run where this command was typed.
fn absolute(stick: &Path) -> PathBuf {
    std::path::absolute(stick).unwrap_or_else(|_| stick.to_path_buf())
}

/// Runs one of these commands: what to print, and whether it succeeded.
pub fn run(
    client: &mut Client,
    as_json: bool,
    command: &StickCommands,
) -> Result<(String, bool), ServiceError> {
    match command {
        StickCommands::DryRun(args) => {
            let dry: DryRun = client.call(
                commands::STICKS_DRY_RUN,
                json!({ "stick": absolute(&args.stick), "playlists": args.playlists }),
            )?;
            let text = if as_json {
                json!(dry).to_string()
            } else {
                dry_run_text(&dry)
            };
            Ok((text, true))
        }
        StickCommands::Load(args) => {
            client.subscribe()?;
            let mut shown = String::new();
            let load: Load = client.call_watching(
                commands::STICKS_LOAD,
                json!({
                    "stick": absolute(&args.stick),
                    "playlists": args.playlists,
                    "plan": args.plan,
                }),
                |event| {
                    if event.event != events::STICK_LOAD_PROGRESS {
                        return;
                    }
                    let Some(title) = event.data["track"]["title"].as_str() else {
                        return;
                    };
                    let stage = event.data["stage"].as_str().unwrap_or("");
                    let line = format!("{}: {title}", stage.replace('_', " "));
                    if line != shown {
                        let _ = writeln!(std::io::stderr(), "{line}");
                        shown = line;
                    }
                },
            )?;
            let ok = load.outcome == "loaded";
            let text = if as_json {
                json!(load).to_string()
            } else {
                load_text(&load)
            };
            Ok((text, ok))
        }
        _ => {
            let sticks: Sticks = client.call(commands::STICKS_LIST, json!({}))?;
            let text = if as_json {
                json!(sticks).to_string()
            } else {
                list_text(&sticks.sticks)
            };
            Ok((text, true))
        }
    }
}

fn kind(stick: &Stick) -> &'static str {
    if stick.kind == "spare" {
        "Spare stick"
    } else {
        "Gig stick"
    }
}

fn bytes(value: Option<u64>) -> String {
    value.map_or_else(
        || "unknown".to_owned(),
        |b| {
            #[allow(clippy::cast_precision_loss, reason = "a size shown to one decimal")]
            let mb = b as f64 / 1_000_000.0;
            format!("{mb:.1} MB")
        },
    )
}

fn protection(stick: &Stick) -> &'static str {
    if stick.protected {
        "protected"
    } else {
        "unlocked"
    }
}

fn list_text(sticks: &[Stick]) -> String {
    if sticks.is_empty() {
        return "No Sticks.".to_owned();
    }
    let mut text = String::new();
    for stick in sticks {
        let _ = writeln!(
            text,
            "{}  {}, {}  {}",
            stick.name,
            kind(stick),
            protection(stick),
            stick.path.display()
        );
        let _ = writeln!(
            text,
            "  Last loaded  {}",
            stick.last_loaded.as_deref().unwrap_or("never")
        );
        let _ = writeln!(text, "  Free         {}", bytes(stick.free_bytes));
        if stick.unfinished_write {
            let _ = writeln!(
                text,
                "  A write did not finish: settle it in the window before loading."
            );
        }
    }
    text.trim_end().to_owned()
}

fn changes_text(text: &mut String, changes: &Changes) {
    let c = &changes.counts;
    let _ = writeln!(
        text,
        "Tracks     {} added, {} changed, {} with new audio, {} removed, {} missing",
        c.tracks_added, c.tracks_changed, c.audio_changed, c.tracks_removed, c.tracks_missing
    );
    let _ = writeln!(
        text,
        "Playlists  {} created, {} written again ({} reordered), {} removed",
        c.playlists_created, c.playlists_rewritten, c.playlists_reordered, c.playlists_removed
    );
    let _ = writeln!(text, "Native stems  {}", c.native_stems);
    let line = |t: &wreckordbox_service::stick_load::TrackLine| {
        if t.id.is_empty() {
            format!("\"{}\"", t.title)
        } else {
            format!("{} \"{}\"", t.id, t.title)
        }
    };
    for (heading, tracks) in [
        ("Add track", &changes.tracks.added),
        ("New audio for track", &changes.tracks.audio_changed),
        ("Remove track", &changes.tracks.removed),
        ("Missing audio, not loaded:", &changes.tracks.missing),
    ] {
        for t in tracks {
            let _ = writeln!(text, "  {heading} {}", line(t));
        }
    }
    for t in &changes.tracks.changed {
        let _ = writeln!(
            text,
            "  Change track {} \"{}\": {}",
            t.id,
            t.title,
            t.fields.join(", ")
        );
    }
    for name in &changes.playlists.created {
        let _ = writeln!(text, "  Create playlist {name}");
    }
    for p in &changes.playlists.rewritten {
        let counts = match (p.added, p.removed) {
            (Some(a), Some(r)) => format!(" (+{a} -{r})"),
            _ => String::new(),
        };
        let order = if p.reordered == Some(true) {
            ", reordered"
        } else {
            ""
        };
        let _ = writeln!(text, "  Write playlist {} again{counts}{order}", p.name);
    }
    for name in &changes.playlists.removed {
        let _ = writeln!(text, "  Remove playlist {name}");
    }
    for t in &changes.native_stems {
        let _ = writeln!(text, "  Native stems for track {}", line(t));
    }
}

fn space_text(text: &mut String, space: &Space) {
    let about = if space.approximate { "about " } else { "" };
    let _ = writeln!(
        text,
        "Space      {} free now, {about}{} needed, {about}{} free after",
        bytes(space.free_before),
        bytes(Some(space.needed_bytes)),
        bytes(space.free_after),
    );
    if let Some(short) = space.shortfall_bytes {
        let _ = writeln!(
            text,
            "It does not fit: {} ({short} bytes) too little room. A load would be refused.",
            bytes(Some(short))
        );
    }
}

fn dry_run_text(dry: &DryRun) -> String {
    let names: Vec<&str> = dry.playlists.iter().map(|p| p.name.as_str()).collect();
    let mut text = format!(
        "Dry run: loading {} onto {} ({}, {}) at {}\n",
        names.join(", "),
        dry.stick.name,
        kind(&dry.stick),
        protection(&dry.stick),
        dry.stick.path.display()
    );
    changes_text(&mut text, &dry.changes);
    space_text(&mut text, &dry.space);
    text.push_str("Nothing was written.\n");
    if dry.stick.protected {
        let _ = writeln!(
            text,
            "The Stick is protected: the DJ unlocks it first (wreckordbox stick unlock {}{}).",
            dry.stick.path.display(),
            if dry.stick.kind == "spare" {
                String::new()
            } else {
                format!(" --name {}", dry.stick.name)
            }
        );
    }
    let ids: Vec<&str> = dry.playlists.iter().map(|p| p.id.as_str()).collect();
    match &dry.plan {
        Some(plan) => {
            let _ = write!(
                text,
                "To load exactly this: wreckordbox stick load {} {} --plan {plan}",
                dry.stick.path.display(),
                ids.join(" "),
            );
        }
        None => {
            let _ = write!(
                text,
                "It cannot be loaded: {}.",
                dry.not_loadable
                    .as_deref()
                    .unwrap_or("the plan could not be worked out in full")
            );
        }
    }
    text
}

fn held_text(text: &mut String, heading: &str, held: &Held) {
    if held.tracks.is_empty() && held.playlists.is_empty() {
        return;
    }
    let _ = writeln!(text, "{heading}");
    for t in &held.tracks {
        let _ = writeln!(text, "  Track {} \"{}\"", t.id, t.title);
    }
    for name in &held.playlists {
        let _ = writeln!(text, "  Playlist {name}");
    }
}

fn load_text(load: &Load) -> String {
    let mut text = match load.outcome.as_str() {
        "loaded" => format!(
            "Loaded {} ({}). Everything planned was written.\n",
            load.stick.name,
            kind(&load.stick)
        ),
        "incomplete" => format!(
            "Loaded {} ({}), but not everything planned was written.\n",
            load.stick.name,
            kind(&load.stick)
        ),
        _ => format!(
            "The load to {} failed. The Stick holds what it held before.\n",
            load.stick.name
        ),
    };
    if let Some(error) = &load.error {
        let _ = writeln!(text, "Error: {}", error.message);
    }
    held_text(&mut text, "Written:", &load.written);
    held_text(
        &mut text,
        "Not written (the Stick still has it as before):",
        &load.not_written,
    );
    held_text(
        &mut text,
        "Unchanged (on the Stick already as planned):",
        &load.unchanged,
    );
    for stems in load.native_stems.iter().filter(|s| s.outcome != "written") {
        let _ = writeln!(
            text,
            "Native stems {} for {} \"{}\": {}",
            stems.outcome,
            stems.track.id,
            stems.track.title,
            stems.reason.as_deref().unwrap_or("")
        );
    }
    let _ = write!(text, "Free now: {}", bytes(load.stick.free_bytes));
    text
}
