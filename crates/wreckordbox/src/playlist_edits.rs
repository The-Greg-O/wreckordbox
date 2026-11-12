//! `wreckordbox playlists remove | delete | reorder | copy` and
//! `wreckordbox tracks remove` (wbx-80): playlist edits from the command
//! line, and what they do to the Mutants of the entries they move.

use clap::{Args, Subcommand};
use serde_json::{json, Value};
use wreckordbox_service::playlist_edits::{
    commands, CollectionRemoval, CopyPlaylist, DeletePlaylist, PlaylistCopied, PlaylistEdited,
    RemoveFromCollection, RemoveTracks, ReorderEntries,
};
use wreckordbox_service::{Client, ServiceError};

#[derive(Subcommand)]
pub enum PlaylistEditCommands {
    /// Take tracks out of a playlist: every entry of each. A Mutant of an
    /// entry taken out is removed with it; Undo in the window brings both
    /// back.
    Remove {
        /// The playlist's identifier: the ID column of `playlists tree`.
        #[arg(value_name = "PLAYLIST_ID")]
        playlist: String,
        /// The tracks' identifiers.
        #[arg(value_name = "TRACK_ID", required = true)]
        tracks: Vec<String>,
    },
    /// Delete a playlist, or a folder and everything in it. Its Mutants are
    /// removed with it; Undo in the window brings them back.
    Delete {
        #[arg(value_name = "PLAYLIST_ID")]
        playlist: String,
    },
    /// Put a playlist's entries in a new order. Every Mutant stays on its
    /// entry.
    Reorder {
        #[arg(value_name = "PLAYLIST_ID")]
        playlist: String,
        /// Every entry of the playlist once, in the new order: the
        /// `entry_id`s of `playlists tracks --json`.
        #[arg(value_name = "ENTRY_ID", required = true)]
        entries: Vec<String>,
    },
    /// Copy a playlist beside it, with its tracks in order and a copy of
    /// each of its Mutants.
    Copy {
        #[arg(value_name = "PLAYLIST_ID")]
        playlist: String,
        /// The copy's name. Left out, the playlist's name and "copy".
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
    },
}

/// `tracks remove`.
#[derive(Args, Debug, Clone)]
pub struct RemoveArgs {
    /// The tracks' identifiers: the ID column of `tracks list`.
    #[arg(value_name = "TRACK_ID", required = true)]
    tracks: Vec<String>,
    /// Remove them. Without it nothing is removed: the command says which
    /// tracks would go and which Mutants would go with them.
    #[arg(long)]
    confirm: bool,
}

/// Runs a playlist edit and answers what to print.
pub fn run(
    client: &mut Client,
    as_json: bool,
    command: &PlaylistEditCommands,
) -> Result<String, ServiceError> {
    match command {
        PlaylistEditCommands::Remove { playlist, tracks } => {
            let edited: PlaylistEdited = client.call(
                commands::PLAYLISTS_REMOVE,
                json!(RemoveTracks {
                    playlist: playlist.clone(),
                    tracks: tracks.clone(),
                }),
            )?;
            Ok(edited_text(
                as_json,
                &edited,
                &format!(
                    "Took {} out of playlist {playlist}.",
                    count(tracks.len(), "track")
                ),
            ))
        }
        PlaylistEditCommands::Delete { playlist } => {
            let edited: PlaylistEdited = client.call(
                commands::PLAYLISTS_DELETE,
                json!(DeletePlaylist {
                    playlist: playlist.clone()
                }),
            )?;
            Ok(edited_text(
                as_json,
                &edited,
                &format!("Deleted playlist {playlist}."),
            ))
        }
        PlaylistEditCommands::Reorder { playlist, entries } => {
            let edited: PlaylistEdited = client.call(
                commands::PLAYLISTS_REORDER,
                json!(ReorderEntries {
                    playlist: playlist.clone(),
                    entries: entries.clone(),
                }),
            )?;
            let said = if edited.changed {
                format!("Put playlist {playlist} in a new order.")
            } else {
                format!("Playlist {playlist} was already in that order.")
            };
            Ok(edited_text(as_json, &edited, &said))
        }
        PlaylistEditCommands::Copy { playlist, name } => {
            let copied: PlaylistCopied = client.call(
                commands::PLAYLISTS_COPY,
                json!(CopyPlaylist {
                    playlist: playlist.clone(),
                    name: name.clone(),
                }),
            )?;
            if as_json {
                return Ok(json!(copied).to_string());
            }
            Ok(format!(
                "Copied playlist {} to {} \"{}\": {}, {}.",
                copied.from,
                copied.playlist,
                copied.name,
                count(copied.tracks, "track"),
                count(copied.mutants.len(), "Mutant")
            ))
        }
    }
}

/// Runs `tracks remove` and answers what to print.
pub fn remove(
    client: &mut Client,
    as_json: bool,
    args: &RemoveArgs,
) -> Result<String, ServiceError> {
    let removal: CollectionRemoval = client.call(
        commands::TRACKS_REMOVE_FROM_COLLECTION,
        json!(RemoveFromCollection {
            tracks: args.tracks.clone(),
            confirm: args.confirm,
        }),
    )?;
    if as_json {
        return Ok(json!(removal).to_string());
    }
    let tracks = removal.tracks.join(", ");
    let mut lines = vec![if removal.removed {
        format!(
            "Removed {} from the collection and every playlist: {tracks}.",
            count(removal.tracks.len(), "track")
        )
    } else {
        format!(
            "Would remove {} from the collection and every playlist: {tracks}.",
            count(removal.tracks.len(), "track")
        )
    }];
    if removal.mutants.is_empty() {
        lines.push("No Mutant is affected.".to_owned());
    } else {
        lines.push(if removal.removed {
            "Mutants removed with them:".to_owned()
        } else {
            "Mutants that would be removed with them:".to_owned()
        });
        lines.extend(removal.mutants.iter().map(mutant_line));
    }
    if !removal.removed {
        lines.push("Nothing was removed. Run it again with --confirm to remove them.".to_owned());
    }
    Ok(lines.join("\n"))
}

fn edited_text(as_json: bool, edited: &PlaylistEdited, said: &str) -> String {
    if as_json {
        return json!(edited).to_string();
    }
    let mut lines = vec![said.to_owned()];
    for mutant in &edited.mutants {
        let state = if mutant["removed"] == true {
            "removed with its entry"
        } else {
            "kept on its entry"
        };
        lines.push(format!("{}: {state}.", mutant_line(mutant).trim_start()));
    }
    lines.join("\n")
}

fn mutant_line(mutant: &Value) -> String {
    let position = mutant["position"]
        .as_u64()
        .map_or_else(|| "an entry".to_owned(), |at| format!("entry {at}"));
    format!(
        "  Mutant {} ({position} of playlist {} \"{}\", track {} \"{}\")",
        mutant["id"],
        text(&mutant["playlist"]["id"]),
        text(&mutant["playlist"]["name"]),
        text(&mutant["parent"]["id"]),
        text(&mutant["parent"]["title"]),
    )
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}

fn count(n: usize, what: &str) -> String {
    if n == 1 {
        format!("1 {what}")
    } else {
        format!("{n} {what}s")
    }
}
