//! `wreckordbox playlists create | create-folder | add | rename | move`
//! (wbx-uia): making playlists and folders and arranging them from the
//! command line. With `--json` each prints the service's answer, whose
//! `playlist` is the identifier the next command takes.

use clap::Subcommand;
use serde_json::json;
use wreckordbox_service::playlist_making::{
    commands, AddTracks, CreateFolder, CreatePlaylist, MovePlaylist, PlaylistMade, PlaylistPlaced,
    RenamePlaylist, TracksAdded,
};
use wreckordbox_service::{Client, ServiceError};

#[derive(Subcommand)]
pub enum PlaylistMakeCommands {
    /// Make a playlist, optionally in a folder and with its first tracks in
    /// the order given. Prints the new playlist's identifier (`playlist`
    /// with `--json`). Undo in the window takes back the tracks, then the
    /// playlist.
    Create {
        /// The playlist's name.
        #[arg(value_name = "NAME")]
        name: String,
        /// Its first tracks, in order: the ID column of `tracks list`.
        #[arg(value_name = "TRACK_ID")]
        tracks: Vec<String>,
        /// The folder to make it in: an ID of `playlists tree`. Left out,
        /// the top level.
        #[arg(long, value_name = "FOLDER_ID")]
        parent: Option<String>,
        /// Its place among the folder's items, from 1. Left out, the last.
        #[arg(long, value_name = "POSITION")]
        at: Option<usize>,
    },
    /// Make a folder, optionally inside another. Prints its identifier.
    CreateFolder {
        #[arg(value_name = "NAME")]
        name: String,
        #[arg(long, value_name = "FOLDER_ID")]
        parent: Option<String>,
        #[arg(long, value_name = "POSITION")]
        at: Option<usize>,
    },
    /// Add tracks to a playlist, at the end or from a position. A track the
    /// playlist already holds is skipped unless `--allow-duplicates` is
    /// given. An intelligent playlist's tracks are its rule's and are
    /// refused.
    Add {
        #[arg(value_name = "PLAYLIST_ID")]
        playlist: String,
        #[arg(value_name = "TRACK_ID", required = true)]
        tracks: Vec<String>,
        /// The position the first of them takes, from 1. Left out, the end.
        #[arg(long, value_name = "POSITION")]
        at: Option<usize>,
        /// Add a track the playlist already holds again.
        #[arg(long)]
        allow_duplicates: bool,
    },
    /// Rename a playlist or folder.
    Rename {
        #[arg(value_name = "PLAYLIST_ID")]
        playlist: String,
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// Move a playlist or folder into a folder, or to the top level, with
    /// everything in it.
    Move {
        #[arg(value_name = "PLAYLIST_ID")]
        playlist: String,
        /// The folder to move it into. Left out, the top level.
        #[arg(long, value_name = "FOLDER_ID")]
        parent: Option<String>,
        /// Its place among the folder's items, from 1. Left out, the last.
        #[arg(long, value_name = "POSITION")]
        at: Option<usize>,
    },
}

/// Runs one of these commands and answers what to print.
pub fn run(
    client: &mut Client,
    as_json: bool,
    command: &PlaylistMakeCommands,
) -> Result<String, ServiceError> {
    match command {
        PlaylistMakeCommands::Create {
            name,
            tracks,
            parent,
            at,
        } => {
            let made: PlaylistMade = client.call(
                commands::PLAYLISTS_CREATE,
                json!(CreatePlaylist {
                    name: name.clone(),
                    parent: parent.clone(),
                    at: *at,
                    tracks: tracks.clone(),
                }),
            )?;
            Ok(made_text(as_json, &made))
        }
        PlaylistMakeCommands::CreateFolder { name, parent, at } => {
            let made: PlaylistMade = client.call(
                commands::PLAYLISTS_CREATE_FOLDER,
                json!(CreateFolder {
                    name: name.clone(),
                    parent: parent.clone(),
                    at: *at,
                }),
            )?;
            Ok(made_text(as_json, &made))
        }
        PlaylistMakeCommands::Add {
            playlist,
            tracks,
            at,
            allow_duplicates,
        } => {
            let added: TracksAdded = client.call(
                commands::PLAYLISTS_ADD,
                json!(AddTracks {
                    playlist: playlist.clone(),
                    tracks: tracks.clone(),
                    at: *at,
                    allow_duplicates: *allow_duplicates,
                }),
            )?;
            if as_json {
                return Ok(json!(added).to_string());
            }
            let mut said = format!(
                "Added {} to playlist {}",
                count(added.added.len(), "track"),
                added.playlist
            );
            said.push_str(&match at {
                Some(at) if !added.added.is_empty() => format!(" from position {at}."),
                _ => ".".to_owned(),
            });
            if !added.skipped.is_empty() {
                said.push_str(&format!(
                    "\nSkipped, already in it: {}. Add them again with --allow-duplicates.",
                    added.skipped.join(", ")
                ));
            }
            Ok(said)
        }
        PlaylistMakeCommands::Rename { playlist, name } => {
            let placed: PlaylistPlaced = client.call(
                commands::PLAYLISTS_RENAME,
                json!(RenamePlaylist {
                    playlist: playlist.clone(),
                    name: name.clone(),
                }),
            )?;
            Ok(placed_text(
                as_json,
                &placed,
                &format!("Renamed {} to \"{}\".", placed.playlist, placed.name),
            ))
        }
        PlaylistMakeCommands::Move {
            playlist,
            parent,
            at,
        } => {
            let placed: PlaylistPlaced = client.call(
                commands::PLAYLISTS_MOVE,
                json!(MovePlaylist {
                    playlist: playlist.clone(),
                    parent: parent.clone(),
                    at: *at,
                }),
            )?;
            let place = if placed.parent == "root" {
                "the top level".to_owned()
            } else {
                format!("folder {}", placed.parent)
            };
            Ok(placed_text(
                as_json,
                &placed,
                &format!(
                    "Moved {} \"{}\" to {place}, position {}.",
                    placed.playlist, placed.name, placed.at
                ),
            ))
        }
    }
}

fn made_text(as_json: bool, made: &PlaylistMade) -> String {
    if as_json {
        return json!(made).to_string();
    }
    let place = if made.parent == "root" {
        String::new()
    } else {
        format!(" in folder {}", made.parent)
    };
    let tracks = if made.kind == "folder" {
        String::new()
    } else {
        format!(", {}", count(made.added.len(), "track"))
    };
    format!(
        "Made {} {} \"{}\"{place}{tracks}.",
        made.kind, made.playlist, made.name
    )
}

fn placed_text(as_json: bool, placed: &PlaylistPlaced, said: &str) -> String {
    if as_json {
        json!(placed).to_string()
    } else {
        said.to_owned()
    }
}

fn count(n: usize, what: &str) -> String {
    if n == 1 {
        format!("1 {what}")
    } else {
        format!("{n} {what}s")
    }
}
