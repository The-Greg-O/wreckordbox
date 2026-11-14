//! The `mytags` subcommands that change the vocabulary or what tracks
//! carry.
//!
//! Each asks the Library service to make one edit and gives back what to
//! print: one line saying what was done, or with `--json` the service's
//! result object as it came.

use clap::Subcommand;
use serde_json::json;
use wreckordbox_service::mytag_edits::{
    commands, Assign, Assigned, Create, CreateCategory, Created, Delete, Deleted, Kind, Merge,
    Merged, Move, Rename, Renamed, Reorder, Reordered,
};
use wreckordbox_service::{Client, ServiceError};

#[derive(Subcommand)]
pub enum EditCommands {
    /// Add a My Tag to the end of a category.
    Create {
        /// The category's identifier: the CATEGORY ID column of `mytags
        /// list`.
        #[arg(value_name = "CATEGORY_ID")]
        category: String,
        /// What to call the tag. No other tag of the category may have the
        /// name.
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// Refused: rekordbox fixes the My Tag categories at four. Give one of
    /// them another name with `mytags rename`.
    CreateCategory {
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// Give a My Tag, or a category, another name. The tracks that carry
    /// the tag keep it.
    Rename {
        /// The identifier of the tag or the category: the TAG ID or
        /// CATEGORY ID column of `mytags list`.
        #[arg(value_name = "ID")]
        id: String,
        #[arg(value_name = "NAME")]
        name: String,
        /// Also rewrite the comments of the tracks concerned with their
        /// My Tags: pass it while rekordbox's Add "My Tag" to the
        /// "Comments" preference is on.
        #[arg(long)]
        comments: bool,
    },
    /// Move a My Tag to another place in its category, or a category to
    /// another place among the categories.
    Move {
        #[arg(value_name = "ID")]
        id: String,
        /// The place to give it, from 1. A place past the end is the end.
        #[arg(value_name = "POSITION")]
        position: usize,
    },
    /// Put every My Tag of one category, or every category, in the order
    /// given.
    Reorder {
        /// The identifiers of every tag of one category, or of every
        /// category, each once.
        #[arg(value_name = "ID", required = true)]
        order: Vec<String>,
    },
    /// Merge one My Tag into another: every track carrying the first is
    /// given the second, then the first is deleted.
    Merge {
        /// The tag to remove.
        #[arg(value_name = "FROM_TAG_ID")]
        from: String,
        /// The tag that takes its tracks; it may be in another category.
        #[arg(value_name = "INTO_TAG_ID")]
        into: String,
        /// Also rewrite the comments of the tracks concerned with their
        /// My Tags: pass it while rekordbox's Add "My Tag" to the
        /// "Comments" preference is on.
        #[arg(long)]
        comments: bool,
    },
    /// Delete a My Tag, taking it off every track that carries it.
    Delete {
        #[arg(value_name = "TAG_ID")]
        id: String,
        /// Also rewrite the comments of the tracks concerned with their
        /// My Tags: pass it while rekordbox's Add "My Tag" to the
        /// "Comments" preference is on.
        #[arg(long)]
        comments: bool,
    },
    /// Put a My Tag on one or many tracks. A track that already carries it
    /// is left as it is.
    Assign {
        #[arg(value_name = "TAG_ID")]
        tag: String,
        /// The tracks' identifiers: the ID column of `tracks list`.
        #[arg(value_name = "TRACK_ID", required = true)]
        tracks: Vec<String>,
        /// Also rewrite the comments of the tracks concerned with their
        /// My Tags: pass it while rekordbox's Add "My Tag" to the
        /// "Comments" preference is on.
        #[arg(long)]
        comments: bool,
    },
    /// Take a My Tag off one or many tracks. The tag itself stays.
    Unassign {
        #[arg(value_name = "TAG_ID")]
        tag: String,
        /// The tracks' identifiers: the ID column of `tracks list`.
        #[arg(value_name = "TRACK_ID", required = true)]
        tracks: Vec<String>,
        /// Also rewrite the comments of the tracks concerned with their
        /// My Tags: pass it while rekordbox's Add "My Tag" to the
        /// "Comments" preference is on.
        #[arg(long)]
        comments: bool,
    },
}

/// Runs one of [`EditCommands`] and answers what to print.
pub fn run(
    client: &mut Client,
    as_json: bool,
    command: &EditCommands,
) -> Result<String, ServiceError> {
    match command {
        EditCommands::Create { category, name } => {
            let done: Created = client.call(
                commands::MYTAGS_CREATE,
                json!(Create {
                    category: category.clone(),
                    name: name.clone(),
                }),
            )?;
            Ok(shown(
                as_json,
                &json!(done),
                format!(
                    "Created MyTag {} \"{}\" in {}.",
                    done.my_tag.id, done.my_tag.name, done.my_tag.category
                ),
            ))
        }
        EditCommands::CreateCategory { name } => {
            client.call::<serde_json::Value>(
                commands::MYTAGS_CREATE_CATEGORY,
                json!(CreateCategory { name: name.clone() }),
            )?;
            Ok(String::new())
        }
        EditCommands::Rename { id, name, comments } => {
            let done: Renamed = client.call(
                commands::MYTAGS_RENAME,
                json!(Rename {
                    id: id.clone(),
                    name: name.clone(),
                    comments: *comments,
                }),
            )?;
            let what = match done.kind {
                Kind::Tag => "MyTag",
                Kind::Category => "MyTag category",
            };
            Ok(shown(
                as_json,
                &json!(done),
                format!(
                    "Renamed {what} {} from \"{}\" to \"{}\".",
                    done.id, done.previous_name, done.name
                ),
            ))
        }
        EditCommands::Move { id, position } => {
            let done: Reordered = client.call(
                commands::MYTAGS_MOVE,
                json!(Move {
                    id: id.clone(),
                    position: *position,
                }),
            )?;
            Ok(shown(as_json, &json!(done), reordered_text(&done)))
        }
        EditCommands::Reorder { order } => {
            let done: Reordered = client.call(
                commands::MYTAGS_REORDER,
                json!(Reorder {
                    order: order.clone(),
                }),
            )?;
            Ok(shown(as_json, &json!(done), reordered_text(&done)))
        }
        EditCommands::Merge {
            from,
            into,
            comments,
        } => {
            let done: Merged = client.call(
                commands::MYTAGS_MERGE,
                json!(Merge {
                    from: from.clone(),
                    into: into.clone(),
                    comments: *comments,
                }),
            )?;
            Ok(shown(
                as_json,
                &json!(done),
                format!(
                    "Merged MyTag \"{}\" ({}) into \"{}\" ({}): {} moved.",
                    done.from.name,
                    done.from.category,
                    done.into.name,
                    done.into.category,
                    tracks(done.tracks)
                ),
            ))
        }
        EditCommands::Delete { id, comments } => {
            let done: Deleted = client.call(
                commands::MYTAGS_DELETE,
                json!(Delete {
                    id: id.clone(),
                    comments: *comments,
                }),
            )?;
            let carried = if done.tracks == 0 {
                "no track carried it".to_owned()
            } else {
                format!("taken off {}", tracks(done.tracks))
            };
            Ok(shown(
                as_json,
                &json!(done),
                format!(
                    "Deleted MyTag {} \"{}\" from {}; {carried}.",
                    done.my_tag.id, done.my_tag.name, done.my_tag.category
                ),
            ))
        }
        EditCommands::Assign {
            tag,
            tracks,
            comments,
        } => assign(client, as_json, tag, tracks, true, *comments),
        EditCommands::Unassign {
            tag,
            tracks,
            comments,
        } => assign(client, as_json, tag, tracks, false, *comments),
    }
}

fn assign(
    client: &mut Client,
    as_json: bool,
    tag: &str,
    track_ids: &[String],
    on: bool,
    comments: bool,
) -> Result<String, ServiceError> {
    let command = if on {
        commands::MYTAGS_ASSIGN
    } else {
        commands::MYTAGS_UNASSIGN
    };
    let done: Assigned = client.call(
        command,
        json!(Assign {
            id: tag.to_owned(),
            tracks: track_ids.to_vec(),
            comments,
            // Typed at the command line: the DJ's deliberate set, when the
            // command is the DJ's (wbx-29).
            assert: true,
        }),
    )?;
    let did = if on { "Put" } else { "Took" };
    let side = if on { "on" } else { "off" };
    Ok(shown(
        as_json,
        &json!(done),
        format!(
            "{did} MyTag \"{}\" {side} {} of {}.",
            done.my_tag.name,
            done.changed,
            tracks(done.tracks.len())
        ),
    ))
}

/// `1 track`, `2 tracks`.
fn tracks(count: usize) -> String {
    if count == 1 {
        "1 track".to_owned()
    } else {
        format!("{count} tracks")
    }
}

fn reordered_text(done: &Reordered) -> String {
    let names: Vec<&str> = done
        .order
        .iter()
        .map(|placed| placed.name.as_str())
        .collect();
    match &done.category {
        Some(category) => format!("{category} is now in the order: {}.", names.join(", ")),
        None => format!(
            "The MyTag categories are now in the order: {}.",
            names.join(", ")
        ),
    }
}

fn shown(as_json: bool, result: &serde_json::Value, text: String) -> String {
    if as_json {
        result.to_string()
    } else {
        text
    }
}
