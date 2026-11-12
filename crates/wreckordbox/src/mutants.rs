//! `mutants`: a playlist's own edited version of a track, kept in the
//! Sidecar for one playlist entry.

#[path = "mutant_edits.rs"]
mod editing;
use clap::{Args, Subcommand};
use serde_json::json;
use wreckordbox_service::facts::SourceArg;
use wreckordbox_service::mutants::{
    commands, CreateMutant, ListMutants, Mutant, MutantChanged, MutantList, MutantShown,
    RemoveMutant, WhichMutant,
};
use wreckordbox_service::{Client, ServiceError};

use crate::facts::{parse_source, source_text, table, time_text};

#[derive(Subcommand)]
pub enum MutantCommands {
    #[command(flatten)]
    Edit(editing::Commands),
    /// Mutate a playlist entry: from now on that entry has its own version
    /// of its track, a Mutant. Nothing audible changes until the Mutant is
    /// edited, and the Library is not changed: the playlist's entry there
    /// stays the track.
    ///
    /// Name the entry by its identifier (`entry_id` in
    /// `playlists tracks --json`), or by `--playlist` and `--position`.
    /// The same track elsewhere, in this playlist or another, is another
    /// entry and is not touched.
    Create {
        /// The entry's identifier.
        #[arg(
            value_name = "ENTRY_ID",
            required_unless_present = "playlist",
            conflicts_with_all = ["playlist", "position"]
        )]
        entry: Option<String>,
        /// The playlist's identifier: the ID column of `playlists tree`.
        #[arg(long, value_name = "PLAYLIST_ID", requires = "position")]
        playlist: Option<String>,
        /// The entry's place in the playlist now, from 1.
        #[arg(long, value_name = "N", requires = "playlist")]
        position: Option<usize>,
        #[command(flatten)]
        who: Who,
    },
    /// Show one Mutant: its playlist, entry and parent track, its edits,
    /// and everything that has happened to it.
    Show {
        /// Verify audio if its cached size/mtime/inode verdict is missing.
        #[arg(long)]
        check_audio: bool,
        /// The Mutant's identifier.
        #[arg(
            value_name = "MUTANT_ID",
            required_unless_present = "entry",
            conflicts_with = "entry"
        )]
        id: Option<i64>,
        /// Show the Mutant of this playlist entry.
        #[arg(long, value_name = "ENTRY_ID")]
        entry: Option<String>,
    },
    /// List the Mutants in the Library, or only those of one playlist or
    /// of one track.
    List {
        /// Verify audio if its cached size/mtime/inode verdict is missing.
        #[arg(long)]
        check_audio: bool,
        /// Only the Mutants of this playlist.
        #[arg(long, value_name = "PLAYLIST_ID")]
        playlist: Option<String>,
        /// Only the Mutants whose parent is this track.
        #[arg(long, value_name = "TRACK_ID")]
        track: Option<String>,
    },
    /// Remove a Mutant: its entry is the core track again. The Library is
    /// not changed.
    Remove {
        /// The Mutant's identifier.
        #[arg(
            value_name = "MUTANT_ID",
            required_unless_present = "entry",
            conflicts_with = "entry"
        )]
        id: Option<i64>,
        /// Remove the Mutant of this playlist entry.
        #[arg(long, value_name = "ENTRY_ID")]
        entry: Option<String>,
        #[command(flatten)]
        who: Who,
    },
}

#[derive(Args)]
pub struct Who {
    /// Who is making the change: `dj`, `agent:NAME`, `tool:NAME` or
    /// `web:ADDRESS`. Left out with `--run`, the Run's source.
    #[arg(long, value_name = "SOURCE", value_parser = parse_source)]
    source: Option<SourceArg>,
    /// Make the change in this Run, started with `runs start`.
    #[arg(long, value_name = "RUN_ID", conflicts_with = "description")]
    run: Option<i64>,
    /// What to call the Run made for this command.
    #[arg(long, value_name = "TEXT")]
    description: Option<String>,
}

/// Runs one `mutants` command and answers what to print.
pub fn run(
    client: &mut Client,
    as_json: bool,
    command: &MutantCommands,
) -> Result<String, ServiceError> {
    Ok(match command {
        MutantCommands::Edit(command) => editing::run(client, as_json, command)?,
        MutantCommands::Create {
            entry,
            playlist,
            position,
            who,
        } => {
            let created: MutantChanged = client.call(
                commands::MUTANTS_CREATE,
                json!(CreateMutant {
                    entry: entry.clone(),
                    playlist: playlist.clone(),
                    position: *position,
                    run: who.run,
                    source: who.source.clone(),
                    description: who.description.clone(),
                }),
            )?;
            if as_json {
                json!(created).to_string()
            } else {
                format!(
                    "Mutated {}: track {} \"{}\" is now Mutant {} (Run {}).",
                    place_text(&created.mutant),
                    created.mutant.parent.id,
                    created.mutant.parent.title,
                    created.mutant.id,
                    created.run.id.0
                )
            }
        }
        MutantCommands::Show {
            id,
            entry,
            check_audio,
        } => {
            let shown: MutantShown = client.call(
                commands::MUTANTS_GET,
                json!(WhichMutant {
                    check_audio: *check_audio,
                    id: *id,
                    entry: entry.clone(),
                }),
            )?;
            if as_json {
                json!(shown).to_string()
            } else {
                shown_text(&shown)
            }
        }
        MutantCommands::List {
            playlist,
            track,
            check_audio,
        } => {
            let list: MutantList = client.call(
                commands::MUTANTS_LIST,
                json!(ListMutants {
                    check_audio: *check_audio,
                    playlist: playlist.clone(),
                    track: track.clone(),
                }),
            )?;
            if as_json {
                json!(list).to_string()
            } else {
                list_text(&list)
            }
        }
        MutantCommands::Remove { id, entry, who } => {
            let removed: MutantChanged = client.call(
                commands::MUTANTS_REMOVE,
                json!(RemoveMutant {
                    id: *id,
                    entry: entry.clone(),
                    run: who.run,
                    source: who.source.clone(),
                    description: who.description.clone(),
                }),
            )?;
            if as_json {
                json!(removed).to_string()
            } else {
                format!(
                    "Removed Mutant {}: {} is track {} \"{}\" again (Run {}).",
                    removed.mutant.id,
                    place_text(&removed.mutant),
                    removed.mutant.parent.id,
                    removed.mutant.parent.title,
                    removed.run.id.0
                )
            }
        }
    })
}

/// Which entry a Mutant belongs to, for a sentence.
fn place_text(mutant: &Mutant) -> String {
    match mutant.position {
        Some(position) => format!(
            "entry {position} of playlist {} \"{}\"",
            mutant.playlist.id, mutant.playlist.name
        ),
        None => format!(
            "entry {} (no longer in playlist {})",
            mutant.entry_id, mutant.playlist.id
        ),
    }
}

/// What differs from the parent, in a word. The edits are an object whose
/// keys are the aspects edited.
fn edits_text(mutant: &Mutant) -> String {
    mutant
        .derived
        .as_ref()
        .map_or_else(|| "invalid stored edits".into(), |d| d.description.clone())
}

fn shown_text(shown: &MutantShown) -> String {
    let mutant = &shown.mutant;
    let details = [
        [
            "Mutant".to_owned(),
            if mutant.removed {
                format!("{} (removed)", mutant.id)
            } else {
                mutant.id.to_string()
            },
        ],
        [
            "Playlist".to_owned(),
            format!("{} \"{}\"", mutant.playlist.id, mutant.playlist.name),
        ],
        [
            "Entry".to_owned(),
            match mutant.position {
                Some(position) => format!("position {position} ({})", mutant.entry_id),
                None => format!("no longer in the playlist ({})", mutant.entry_id),
            },
        ],
        [
            "Parent".to_owned(),
            format!("{} \"{}\"", mutant.parent.id, mutant.parent.title),
        ],
        [
            "Fingerprint".to_owned(),
            mutant
                .fingerprint
                .as_ref()
                .map_or_else(|| "-".to_owned(), ToString::to_string),
        ],
        ["Edits".to_owned(), edits_text(mutant)],
        [
            "Key".to_owned(),
            mutant
                .derived
                .as_ref()
                .and_then(|d| d.key.clone())
                .unwrap_or_else(|| "unknown".into()),
        ],
        [
            "Tempo".to_owned(),
            mutant
                .derived
                .as_ref()
                .and_then(|d| d.bpm)
                .map_or_else(|| "unknown".into(), |b| format!("{b:.2} BPM")),
        ],
        [
            "Length".to_owned(),
            mutant
                .derived
                .as_ref()
                .and_then(|d| d.duration_seconds)
                .map_or_else(
                    || "unknown".into(),
                    |s| format!("approximately {s:.0} seconds"),
                ),
        ],
        ["Renderable".to_owned(), readiness_text(mutant)],
    ];
    let mut history = vec![["CHANGE", "WHEN", "SOURCE", "RUN", "EDITS"].map(str::to_owned)];
    history.extend(shown.history.iter().map(|change| {
        [
            change.action.clone(),
            time_text(change.recorded_at),
            source_text(&change.source),
            change.run.to_string(),
            match change.edits.as_object() {
                Some(edits) if !edits.is_empty() => change.edits.to_string(),
                _ => "none".to_owned(),
            },
        ]
    }));
    format!("{}\n\n{}", table(&details), table(&history))
}

fn list_text(list: &MutantList) -> String {
    if list.mutants.is_empty() {
        return "No Mutants.".to_owned();
    }
    let mut rows = vec![[
        "MUTANT",
        "PLAYLIST",
        "NAME",
        "POSITION",
        "TRACK",
        "TITLE",
        "EDITS",
        "SOURCE",
        "RENDERABLE",
    ]
    .map(str::to_owned)];
    rows.extend(list.mutants.iter().map(|mutant| {
        [
            mutant.id.to_string(),
            mutant.playlist.id.clone(),
            dash(&mutant.playlist.name),
            mutant
                .position
                .map_or_else(|| "-".to_owned(), |position| position.to_string()),
            mutant.parent.id.clone(),
            dash(&mutant.parent.title),
            edits_text(mutant),
            source_text(&mutant.source),
            readiness_text(mutant),
        ]
    }));
    table(&rows)
}

fn dash(text: &str) -> String {
    if text.is_empty() {
        "-".to_owned()
    } else {
        text.to_owned()
    }
}

fn readiness_text(mutant: &Mutant) -> String {
    mutant.renderability.reason.as_ref().map_or_else(
        || "yes".into(),
        |e| format!("no: {} ({})", e.message, e.code),
    )
}
