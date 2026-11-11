//! `tracks show` and `tracks query`: one track in full, and tracks selected
//! by any of their fields.
#![allow(
    clippy::doc_markdown,
    reason = "these comments are the command's help, where MyTag is a word, not code"
)]

use std::fmt::Write as _;

use clap::{Args, ValueEnum};
use serde_json::json;
use wreckordbox_service::tracks::{
    TrackId, TrackPage, TrackRecord, TrackSelection, DEFAULT_PAGE_LIMIT, TRACKS_GET, TRACKS_QUERY,
};
use wreckordbox_service::{Client, ServiceError};

#[derive(Args)]
pub struct ShowArgs {
    /// The track's identifier: the ID column of `tracks list` and
    /// `tracks query`.
    #[arg(value_name = "ID")]
    id: String,
}

/// A number or a date is one value (`2021`) or an inclusive range
/// (`2019..2021`, `2019..`, `..2021`).
#[derive(Args)]
pub struct QueryArgs {
    /// Only tracks matching this text, as the window's search box would:
    /// every word must appear somewhere in the track's details.
    #[arg(long, value_name = "TEXT")]
    search: Option<String>,
    /// Only tracks whose title contains this text, in any case.
    #[arg(long, value_name = "TEXT")]
    title: Option<String>,
    /// Only tracks whose artist contains this text.
    #[arg(long, value_name = "TEXT")]
    artist: Option<String>,
    /// Only tracks whose album contains this text.
    #[arg(long, value_name = "TEXT")]
    album: Option<String>,
    /// Only tracks whose genre contains this text.
    #[arg(long, value_name = "TEXT")]
    genre: Option<String>,
    /// Only tracks whose label contains this text.
    #[arg(long, value_name = "TEXT")]
    label: Option<String>,
    /// Only tracks whose comment contains this text.
    #[arg(long, value_name = "TEXT")]
    comment: Option<String>,
    /// Only tracks whose audio file's path contains this text.
    #[arg(long, value_name = "TEXT")]
    location: Option<String>,
    /// Only tracks released in this year, or range of years.
    #[arg(long, value_name = "YEAR")]
    year: Option<String>,
    /// Only tracks at this tempo: `128` is every track the Track Filter
    /// lists under 128, `128.03` is exact, `124..128.5` is a range.
    #[arg(long, value_name = "BPM")]
    bpm: Option<String>,
    /// Only tracks in this key, spelled as the Library spells it.
    #[arg(long, value_name = "KEY")]
    key: Option<String>,
    /// Only tracks with this Energy (the star rating), 1 to 5, or a range;
    /// 0 is a track with none.
    #[arg(long, visible_alias = "rating", value_name = "ENERGY")]
    energy: Option<String>,
    /// Only tracks with this Prep colour (the colour label): Pink, Red,
    /// Orange, Yellow, Green, Aqua, Blue or Purple, or `none`.
    #[arg(
        long,
        visible_alias = "colour",
        alias = "prep-color",
        alias = "color",
        value_name = "COLOUR"
    )]
    prep_colour: Option<String>,
    /// Only tracks this long: seconds or M:SS, or a range of either.
    #[arg(long, value_name = "TIME")]
    duration: Option<String>,
    /// Only tracks added on this day (YYYY-MM-DD), or in a range of days.
    #[arg(long, value_name = "DATE")]
    date_added: Option<String>,
    /// Only tracks played this many times, or a range.
    #[arg(long, value_name = "COUNT")]
    play_count: Option<String>,
    /// Only tracks that have, or have not, been analysed.
    #[arg(long, alias = "analyzed", value_name = "yes|no")]
    analysed: Option<YesNo>,
    /// Only tracks carrying this MyTag, by name. Give it more than once
    /// for tracks carrying every one.
    #[arg(long, value_name = "TAG")]
    mytag: Vec<String>,
    /// Only the tracks of this playlist, by name or identifier, in its
    /// order. A folder is every track beneath it.
    #[arg(long, value_name = "PLAYLIST")]
    playlist: Option<String>,
    /// The most rows in a page.
    #[arg(long, value_name = "ROWS", default_value_t = DEFAULT_PAGE_LIMIT)]
    limit: usize,
    /// How many matching tracks to skip. Each page says which offset
    /// fetches the next one.
    #[arg(long, value_name = "ROWS", default_value_t = 0)]
    offset: usize,
}

#[derive(Clone, Copy, ValueEnum)]
enum YesNo {
    Yes,
    No,
}

/// What `tracks show` prints.
pub fn show(client: &mut Client, args: &ShowArgs, as_json: bool) -> Result<String, ServiceError> {
    let track: TrackRecord = client.call(
        TRACKS_GET,
        json!(TrackId {
            id: args.id.clone()
        }),
    )?;
    Ok(if as_json {
        json!(track).to_string()
    } else {
        record_text(&track)
    })
}

/// What `tracks query` prints.
pub fn query(client: &mut Client, args: &QueryArgs, as_json: bool) -> Result<String, ServiceError> {
    let selection = TrackSelection {
        search: args.search.clone().unwrap_or_default(),
        title: args.title.clone(),
        artist: args.artist.clone(),
        album: args.album.clone(),
        genre: args.genre.clone(),
        label: args.label.clone(),
        comment: args.comment.clone(),
        location: args.location.clone(),
        key: args.key.clone(),
        year: args.year.clone(),
        bpm: args.bpm.clone(),
        energy: args.energy.clone(),
        prep_colour: args.prep_colour.clone(),
        duration: args.duration.clone(),
        date_added: args.date_added.clone(),
        play_count: args.play_count.clone(),
        analysed: args.analysed.map(|answer| matches!(answer, YesNo::Yes)),
        my_tags: args.mytag.clone(),
        playlist: args.playlist.clone(),
        limit: Some(args.limit),
        offset: args.offset,
    };
    let page: TrackPage = client.call(TRACKS_QUERY, json!(selection))?;
    Ok(if as_json {
        json!(page).to_string()
    } else {
        page_text(&page)
    })
}

/// One track, a field to a line.
fn record_text(track: &TrackRecord) -> String {
    let stars = match track.rating {
        1 => "1 star".to_owned(),
        count => format!("{count} stars"),
    };
    let rows = [
        ("ID", track.id.clone()),
        ("Title", or_dash(&track.title)),
        ("Artist", or_dash(&track.artist)),
        ("Album", or_dash(&track.album)),
        ("Genre", or_dash(&track.genre)),
        ("Label", or_dash(&track.label)),
        ("Year", dash_or(track.year)),
        ("BPM", bpm_text(track.bpm)),
        ("Key", or_dash(&track.key)),
        ("Energy", dash_or(track.energy)),
        ("Rating", stars),
        ("Prep colour", dash_or(track.prep_colour.as_ref())),
        (
            "Colour",
            track.colour.as_ref().map_or_else(
                || "-".to_owned(),
                |name| format!("{name} ({})", track.colour_id),
            ),
        ),
        ("Comment", or_dash(&track.comment)),
        ("Duration", duration_text(track.duration_seconds)),
        ("Date added", dash_or(track.date_added.as_ref())),
        ("Play count", track.play_count.to_string()),
        ("Location", or_dash(&track.location)),
        (
            "Analysed",
            if track.analysed { "yes" } else { "no" }.to_owned(),
        ),
        (
            "MyTags",
            or_dash(
                &track
                    .my_tags
                    .iter()
                    .map(|tag| format!("{}: {}", tag.category, tag.tag))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        ),
        (
            "Playlists",
            or_dash(
                &track
                    .playlists
                    .iter()
                    .map(|playlist| playlist.name.clone())
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        ),
    ];
    let width = rows
        .iter()
        .map(|(name, _)| name.chars().count())
        .max()
        .unwrap_or(0);
    rows.iter()
        .map(|(name, value)| format!("{name:<width$}  {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A page of tracks as a table, and which tracks of how many it holds.
fn page_text(page: &TrackPage) -> String {
    if page.total == 0 {
        return "No tracks.".to_owned();
    }
    if page.tracks.is_empty() {
        return format!(
            "No tracks from offset {}: {} matched.",
            page.offset, page.total
        );
    }
    let mut rows = vec![[
        "ID",
        "TITLE",
        "ARTIST",
        "GENRE",
        "BPM",
        "KEY",
        "ENERGY",
        "PREP COLOUR",
        "TIME",
    ]
    .map(str::to_owned)];
    rows.extend(page.tracks.iter().map(|track| {
        [
            track.id.clone(),
            or_dash(&track.title),
            or_dash(&track.artist),
            or_dash(&track.genre),
            bpm_text(track.bpm),
            or_dash(&track.key),
            dash_or(track.energy),
            dash_or(track.prep_colour.as_ref()),
            duration_text(track.duration_seconds),
        ]
    }));
    let mut text = table(&rows);
    let shown = page.tracks.len();
    if shown < page.total {
        let _ = write!(
            text,
            "\nTracks {} to {} of {}",
            page.offset + 1,
            page.offset + shown,
            page.total
        );
        match page.next_offset {
            Some(next) => {
                let _ = write!(text, "; next page: --offset {next}");
            }
            None => text.push('.'),
        }
    }
    text
}

/// Rows as lines, columns two spaces apart and as wide as their longest
/// cell.
fn table<const N: usize>(rows: &[[String; N]]) -> String {
    let widths: [usize; N] = std::array::from_fn(|column| {
        rows.iter()
            .map(|row| row[column].chars().count())
            .max()
            .unwrap_or(0)
    });
    rows.iter()
        .map(|row| {
            let line: Vec<String> = row
                .iter()
                .zip(widths)
                .map(|(cell, width)| format!("{cell:<width$}"))
                .collect();
            line.join("  ").trim_end().to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn bpm_text(bpm: Option<f64>) -> String {
    bpm.map_or_else(|| "-".to_owned(), |bpm| format!("{bpm:.2}"))
}

fn duration_text(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn dash_or<T: ToString>(value: Option<T>) -> String {
    value.map_or_else(|| "-".to_owned(), |value| value.to_string())
}

/// A cell with nothing in it would shift the ones after it when read back.
fn or_dash(text: &str) -> String {
    if text.is_empty() {
        "-".to_owned()
    } else {
        text.to_owned()
    }
}
