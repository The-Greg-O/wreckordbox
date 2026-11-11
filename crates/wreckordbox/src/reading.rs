//! The subcommands that read the DJ's vocabulary and what depends on it:
//! `mytags`, `playlists` and `history`.
//!
//! Each asks the Library service and gives back what to print: a table, or
//! with `--json` the service's result object as it came.

use clap::Subcommand;
use serde_json::json;
use wreckordbox_service::reading::{
    commands, HistoryList, HistoryTracks, ListQuery, ListedTrack, MyTag, MyTagCategories, OfTrack,
    PlaylistKind, PlaylistTracks, PlaylistTree, Rule, RuleCondition, RuleItem, RuleLogic,
    TaggedTracks, TrackMyTags,
};
use wreckordbox_service::{Client, ServiceError, Track, DEFAULT_TRACK_LIMIT};

#[derive(Subcommand)]
pub enum MyTagCommands {
    /// List the My Tag categories and their tags in stored order, with each
    /// tag's identifier and how many tracks carry it.
    List,
    /// Show the My Tags one track carries.
    Show {
        /// The track's identifier: the ID column of `tracks list`.
        #[arg(value_name = "TRACK_ID")]
        track: String,
    },
    /// List the tracks that carry a My Tag, in the collection's order.
    Tracks {
        /// The tag's identifier: the TAG ID column of `mytags list`.
        #[arg(value_name = "TAG_ID")]
        tag: String,
        /// The most rows to print. The JSON result's `total` says how many
        /// tracks carry the tag.
        #[arg(long, value_name = "ROWS", default_value_t = DEFAULT_TRACK_LIMIT)]
        limit: usize,
    },
}

#[derive(Subcommand)]
pub enum PlaylistCommands {
    /// Show the playlist tree: folders, playlists and intelligent
    /// playlists, each intelligent playlist with its rule.
    Tree,
    /// List a playlist's tracks in its order. An intelligent playlist lists
    /// what its rule admits now; a folder, the tracks of every playlist in
    /// it.
    Tracks {
        /// The playlist's identifier: the ID column of `playlists tree`.
        #[arg(value_name = "ID")]
        id: String,
        /// The most rows to print. The JSON result's `total` says how many
        /// tracks the playlist holds.
        #[arg(long, value_name = "ROWS", default_value_t = DEFAULT_TRACK_LIMIT)]
        limit: usize,
    },
}

#[derive(Subcommand)]
pub enum HistoryCommands {
    /// List the occasions play history was recorded on, oldest first.
    List,
    /// List what was played on one occasion, in the order it was played.
    Tracks {
        /// The occasion's identifier: the ID column of `history list`.
        #[arg(value_name = "ID")]
        id: String,
        /// The most rows to print. The JSON result's `total` says how many
        /// plays the occasion holds.
        #[arg(long, value_name = "ROWS", default_value_t = DEFAULT_TRACK_LIMIT)]
        limit: usize,
    },
}

/// Runs a `mytags` subcommand and answers what to print.
pub fn my_tags(
    client: &mut Client,
    as_json: bool,
    command: &MyTagCommands,
) -> Result<String, ServiceError> {
    match command {
        MyTagCommands::List => {
            let found: MyTagCategories = client.call(commands::MYTAGS_LIST, json!({}))?;
            Ok(shown(as_json, &json!(found), my_tags_text(&found)))
        }
        MyTagCommands::Show { track } => {
            let found: TrackMyTags = client.call(
                commands::MYTAGS_OF_TRACK,
                json!(OfTrack { id: track.clone() }),
            )?;
            Ok(shown(as_json, &json!(found), track_my_tags_text(&found)))
        }
        MyTagCommands::Tracks { tag, limit } => {
            let found: TaggedTracks = client.call(commands::MYTAGS_TRACKS, query(tag, *limit))?;
            Ok(shown(as_json, &json!(found), tagged_text(&found)))
        }
    }
}

/// Runs a `playlists` subcommand and answers what to print.
pub fn playlists(
    client: &mut Client,
    as_json: bool,
    command: &PlaylistCommands,
) -> Result<String, ServiceError> {
    match command {
        PlaylistCommands::Tree => {
            let found: PlaylistTree = client.call(commands::PLAYLISTS_TREE, json!({}))?;
            Ok(shown(as_json, &json!(found), playlist_tree_text(&found)))
        }
        PlaylistCommands::Tracks { id, limit } => {
            let found: PlaylistTracks =
                client.call(commands::PLAYLISTS_TRACKS, query(id, *limit))?;
            Ok(shown(as_json, &json!(found), playlist_tracks_text(&found)))
        }
    }
}

/// Runs a `history` subcommand and answers what to print.
pub fn history(
    client: &mut Client,
    as_json: bool,
    command: &HistoryCommands,
) -> Result<String, ServiceError> {
    match command {
        HistoryCommands::List => {
            let found: HistoryList = client.call(commands::HISTORY_LIST, json!({}))?;
            Ok(shown(as_json, &json!(found), history_text(&found)))
        }
        HistoryCommands::Tracks { id, limit } => {
            let found: HistoryTracks = client.call(commands::HISTORY_TRACKS, query(id, *limit))?;
            Ok(shown(as_json, &json!(found), history_tracks_text(&found)))
        }
    }
}

fn query(id: &str, limit: usize) -> serde_json::Value {
    json!(ListQuery {
        id: id.to_owned(),
        limit: Some(limit),
    })
}

fn shown(as_json: bool, result: &serde_json::Value, text: String) -> String {
    if as_json {
        result.to_string()
    } else {
        text
    }
}

fn my_tags_text(found: &MyTagCategories) -> String {
    if found.categories.is_empty() {
        return "No MyTags.".to_owned();
    }
    let mut rows = Vec::new();
    for category in &found.categories {
        // A category with no tags is still one of rekordbox's four.
        if category.tags.is_empty() {
            rows.push(vec![
                or_dash(&category.name),
                category.id.clone(),
                "-".to_owned(),
                "-".to_owned(),
                "-".to_owned(),
            ]);
        }
        rows.extend(category.tags.iter().map(|tag| {
            vec![
                or_dash(&category.name),
                category.id.clone(),
                or_dash(&tag.name),
                tag.id.clone(),
                tag.tracks.to_string(),
            ]
        }));
    }
    table(
        &["CATEGORY", "CATEGORY ID", "TAG", "TAG ID", "TRACKS"],
        rows,
    )
}

fn track_my_tags_text(found: &TrackMyTags) -> String {
    if found.my_tags.is_empty() {
        return format!("Track {} \"{}\" has no MyTags.", found.id, found.title);
    }
    let rows = found
        .my_tags
        .iter()
        .map(|tag| vec![or_dash(&tag.category), or_dash(&tag.name), tag.id.clone()])
        .collect();
    format!(
        "Track {} \"{}\"\n{}",
        found.id,
        found.title,
        table(&["CATEGORY", "TAG", "TAG ID"], rows)
    )
}

fn tagged_text(found: &TaggedTracks) -> String {
    let MyTag {
        id, name, category, ..
    } = &found.my_tag;
    let heading = format!(
        "MyTag {id} \"{name}\" in {category}: {}",
        counted(found.total, "track")
    );
    if found.total == 0 {
        return heading;
    }
    let rows = found.tracks.iter().map(track_cells).collect();
    format!(
        "{heading}\n{}{}",
        table(&TRACK_COLUMNS, rows),
        cut_short(found.tracks.len(), found.total, "tracks")
    )
}

fn playlist_tree_text(found: &PlaylistTree) -> String {
    if found.playlists.is_empty() {
        return "No playlists.".to_owned();
    }
    let rows = found
        .playlists
        .iter()
        .map(|node| {
            let indent = "  ".repeat(node.depth.saturating_sub(1) as usize);
            vec![
                node.id.clone(),
                kind_text(node.kind).to_owned(),
                node.tracks
                    .map_or_else(|| "-".to_owned(), |tracks| tracks.to_string()),
                format!("{indent}{}", or_dash(&node.name)),
                match (&node.rule, node.kind) {
                    (Some(rule), _) => rule_text(rule),
                    (None, PlaylistKind::Intelligent) => "(unreadable rule)".to_owned(),
                    (None, _) => String::new(),
                },
            ]
        })
        .collect();
    table(&["ID", "KIND", "TRACKS", "NAME", "RULE"], rows)
}

fn playlist_tracks_text(found: &PlaylistTracks) -> String {
    let heading = format!(
        "{} {} \"{}\": {}",
        match found.playlist.kind {
            PlaylistKind::Folder => "Folder",
            PlaylistKind::Playlist => "Playlist",
            PlaylistKind::Intelligent => "Intelligent playlist",
        },
        found.playlist.id,
        found.playlist.name,
        counted(found.total, "track")
    );
    numbered_text(heading, &found.tracks, found.total, "tracks")
}

fn history_text(found: &HistoryList) -> String {
    if found.occasions.is_empty() {
        return "No play history.".to_owned();
    }
    let rows = found
        .occasions
        .iter()
        .map(|occasion| {
            vec![
                occasion.id.clone(),
                or_dash(&occasion.name),
                or_dash(&occasion.folder),
                occasion.tracks.to_string(),
            ]
        })
        .collect();
    table(&["ID", "OCCASION", "FILED UNDER", "PLAYS"], rows)
}

fn history_tracks_text(found: &HistoryTracks) -> String {
    let heading = format!(
        "History {} \"{}\": {}",
        found.occasion.id,
        found.occasion.name,
        counted(found.total, "play")
    );
    numbered_text(heading, &found.tracks, found.total, "plays")
}

/// A heading over an ordered list of tracks, each with its place.
fn numbered_text(heading: String, tracks: &[ListedTrack], total: usize, noun: &str) -> String {
    if total == 0 {
        return heading;
    }
    let mut header = vec!["#"];
    header.extend(TRACK_COLUMNS);
    let rows = tracks
        .iter()
        .map(|listed| {
            let mut cells = vec![listed.position.to_string()];
            cells.extend(track_cells(&listed.track));
            cells
        })
        .collect();
    format!(
        "{heading}\n{}{}",
        table(&header, rows),
        cut_short(tracks.len(), total, noun)
    )
}

const TRACK_COLUMNS: [&str; 6] = ["ID", "TITLE", "ARTIST", "BPM", "KEY", "RATING"];

/// A track as a row of `tracks list` shows it.
fn track_cells(track: &Track) -> Vec<String> {
    vec![
        track.id.clone(),
        or_dash(&track.title),
        or_dash(&track.artist),
        track
            .bpm
            .map_or_else(|| "-".to_owned(), |bpm| format!("{bpm:.2}")),
        or_dash(&track.key),
        track.rating.to_string(),
    ]
}

fn kind_text(kind: PlaylistKind) -> &'static str {
    match kind {
        PlaylistKind::Folder => "folder",
        PlaylistKind::Playlist => "playlist",
        PlaylistKind::Intelligent => "intelligent",
    }
}

/// A rule on one line: `all of: my_tag contains Peak [700002]; artist
/// contains Ada`, a nested group in brackets.
fn rule_text(rule: &Rule) -> String {
    let conditions: Vec<String> = rule
        .conditions
        .iter()
        .map(|item| match item {
            RuleItem::Condition(condition) => condition_text(condition),
            RuleItem::Group(nested) => format!("({})", rule_text(nested)),
        })
        .collect();
    format!(
        "{} of: {}",
        match rule.logic {
            RuleLogic::All => "all",
            RuleLogic::Any => "any",
        },
        conditions.join("; ")
    )
}

fn condition_text(condition: &RuleCondition) -> String {
    // The tag by name, with the identifier the rule stores.
    let value = match &condition.my_tag {
        Some(tag) => format!("{} [{}]", tag.name, tag.id),
        None => condition.value.clone(),
    };
    let mut words = vec![
        condition.property.clone(),
        condition.operator.clone(),
        value,
    ];
    if !condition.value_to.is_empty() {
        words.push(format!("to {}", condition.value_to));
    }
    if !condition.unit.is_empty() {
        words.push(condition.unit.clone());
    }
    words.join(" ")
}

/// `1 track`, `2 tracks`.
fn counted(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// The last line of a list the limit cut short; nothing otherwise.
fn cut_short(shown: usize, total: usize, noun: &str) -> String {
    if shown < total {
        format!("\nShowing {shown} of {total} {noun}; raise --limit to see more.")
    } else {
        String::new()
    }
}

/// One line each under a header, columns two spaces apart and as wide as
/// their longest cell.
fn table(header: &[&str], rows: Vec<Vec<String>>) -> String {
    let mut lines: Vec<Vec<String>> = vec![header.iter().map(|&cell| cell.to_owned()).collect()];
    lines.extend(rows);
    let widths: Vec<usize> = (0..header.len())
        .map(|column| {
            lines
                .iter()
                .filter_map(|row| row.get(column))
                .map(|cell| cell.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let lines: Vec<String> = lines
        .iter()
        .map(|row| {
            let line: Vec<String> = row
                .iter()
                .zip(&widths)
                .map(|(cell, &width)| format!("{cell:<width$}"))
                .collect();
            line.join("  ").trim_end().to_owned()
        })
        .collect();
    lines.join("\n")
}

/// A cell with nothing in it would shift the ones after it when read back.
fn or_dash(text: &str) -> String {
    if text.is_empty() {
        "-".to_owned()
    } else {
        text.to_owned()
    }
}
