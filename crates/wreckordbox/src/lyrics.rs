//! `lyrics show | set | import-embedded | list | search`: the words of a
//! track, kept in the Sidecar so an agent can match tracks on their
//! themes. wreckordbox never fetches lyrics; it reads those already in a
//! file's tags and keeps those handed to it.

use std::fmt::Write as _;
use std::io::Read as _;
use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};
use serde_json::{json, Value};
use wreckordbox_service::facts::SourceArg;
use wreckordbox_service::lyrics::{
    commands, Found, Imported, LyricsFound, LyricsList, LyricsRecorded, Status, TrackLyrics,
};
use wreckordbox_service::{Client, ServiceError};

use crate::facts::{bad, parse_source, source_text};

#[derive(Subcommand)]
pub enum LyricsCommands {
    /// Show the lyrics the Sidecar holds for one track: the record that
    /// stands, and every source's.
    Show {
        /// The track's identifier: the ID column of `tracks list`.
        #[arg(value_name = "TRACK_ID")]
        id: String,
    },
    /// Record a track's lyrics, read from a file or standard input, or
    /// that it has none (`--instrumental`). Lyrics taken from a web page
    /// need `--source-url` and `--date`. Changes the Sidecar only, never
    /// the Library or a Stick.
    Set {
        #[arg(value_name = "TRACK_ID")]
        id: String,
        /// A text file of the words; left out or `-`, standard input.
        #[arg(value_name = "FILE")]
        file: Option<PathBuf>,
        /// The track has no words: nobody needs to look further.
        #[arg(long, conflicts_with_all = ["file", "lrc"])]
        instrumental: bool,
        /// The words are LRC: `[mm:ss.xx]` before each line, kept as
        /// synced lyrics.
        #[arg(long)]
        lrc: bool,
        /// The web address the words were taken from.
        #[arg(long, value_name = "URL", requires = "date")]
        source_url: Option<String>,
        /// The date they were taken from it, `YYYY-MM-DD`.
        #[arg(long, value_name = "YYYY-MM-DD", requires = "source_url")]
        date: Option<String>,
        /// The language, as a code such as `eng` or `en`.
        #[arg(long, value_name = "CODE")]
        language: Option<String>,
        /// How sure the source is, 0 to 1. Left out, 1.
        #[arg(long, value_name = "0..1")]
        confidence: Option<f64>,
        #[command(flatten)]
        who: Who,
    },
    /// Read lyrics from the tracks' own audio tags (ID3 USLT/SYLT, Vorbis
    /// LYRICS/UNSYNCEDLYRICS, MP4 lyrics, AIFF's ID3) and record those
    /// found in the Sidecar. Reports found, none or failed per track. The
    /// audio files are only read.
    ImportEmbedded {
        /// The tracks to read, by identifier; left out, every track.
        #[arg(long, value_name = "TRACK_ID", num_args = 1..)]
        tracks: Vec<String>,
        /// Read and report only; record nothing.
        #[arg(long)]
        dry_run: bool,
        #[command(flatten)]
        who: Who,
    },
    /// List the Library's tracks by what is known of their words: lyrics,
    /// instrumental, or unknown.
    List {
        /// Only tracks in this state.
        #[arg(long, value_enum)]
        status: Option<StatusArg>,
        /// How many tracks to list.
        #[arg(long, value_name = "ROWS")]
        limit: Option<usize>,
        /// How many to skip.
        #[arg(long, value_name = "ROWS")]
        offset: Option<usize>,
    },
    /// Search the stored lyrics: tracks whose words contain every word
    /// given, best match first (SQLite full-text search).
    Search {
        #[arg(value_name = "WORDS", required = true, num_args = 1..)]
        words: Vec<String>,
        /// How many hits to show.
        #[arg(long, value_name = "ROWS")]
        limit: Option<usize>,
    },
}

/// Who is recording, as for `facts record`.
#[derive(clap::Args)]
pub struct Who {
    /// Who is recording: `dj`, `agent:NAME`, `tool:NAME`. Left out with
    /// `--run`, the Run's source.
    #[arg(long, value_name = "SOURCE", value_parser = parse_source)]
    source: Option<SourceArg>,
    /// Record in this Run, started with `runs start`.
    #[arg(long, value_name = "RUN_ID", conflicts_with = "description")]
    run: Option<i64>,
    /// What to call the Run made for this command.
    #[arg(long, value_name = "TEXT")]
    description: Option<String>,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum StatusArg {
    Lyrics,
    Instrumental,
    Unknown,
}

impl Who {
    fn into_args(&self, args: &mut serde_json::Map<String, Value>) {
        if let Some(source) = &self.source {
            args.insert("source".to_owned(), json!(source));
        }
        if let Some(run) = self.run {
            args.insert("run".to_owned(), json!(run));
        }
        if let Some(description) = &self.description {
            args.insert("description".to_owned(), json!(description));
        }
    }
}

/// Runs one `lyrics` command and answers what to print.
pub fn run(
    client: &mut Client,
    as_json: bool,
    command: &LyricsCommands,
) -> Result<String, ServiceError> {
    match command {
        LyricsCommands::Show { id } => {
            let shown: TrackLyrics = client.call(commands::LYRICS_SHOW, json!({"id": id}))?;
            Ok(if as_json {
                json!(shown).to_string()
            } else {
                shown_text(&shown)
            })
        }
        LyricsCommands::Set {
            id,
            file,
            instrumental,
            lrc,
            source_url,
            date,
            language,
            confidence,
            who,
        } => {
            let mut args = serde_json::Map::new();
            args.insert("id".to_owned(), json!(id));
            if *instrumental {
                args.insert("instrumental".to_owned(), json!(true));
            } else {
                let text = read_words(file.as_ref())?;
                if *lrc {
                    args.insert("lines".to_owned(), json!(lrc_lines(&text)?));
                } else {
                    args.insert("text".to_owned(), json!(text));
                }
            }
            for (key, value) in [
                ("address", source_url),
                ("date", date),
                ("language", language),
            ] {
                if let Some(value) = value {
                    args.insert(key.to_owned(), json!(value));
                }
            }
            if let Some(confidence) = confidence {
                args.insert("confidence".to_owned(), json!(confidence));
            }
            who.into_args(&mut args);
            let recorded: LyricsRecorded =
                client.call(commands::LYRICS_SET, Value::Object(args))?;
            Ok(if as_json {
                json!(recorded).to_string()
            } else {
                format!(
                    "Recorded lyrics for track {} as Fact {} (source {}), in Run {}\n",
                    recorded.fact.track_id,
                    recorded.fact.id,
                    source_text(&recorded.fact.source),
                    recorded.run.id.0
                )
            })
        }
        LyricsCommands::ImportEmbedded {
            tracks,
            dry_run,
            who,
        } => {
            let mut args = serde_json::Map::new();
            if !tracks.is_empty() {
                args.insert("tracks".to_owned(), json!(tracks));
            }
            if *dry_run {
                args.insert("dry_run".to_owned(), json!(true));
            }
            who.into_args(&mut args);
            let imported: Imported =
                client.call(commands::LYRICS_IMPORT_EMBEDDED, Value::Object(args))?;
            Ok(if as_json {
                json!(imported).to_string()
            } else {
                imported_text(&imported)
            })
        }
        LyricsCommands::List {
            status,
            limit,
            offset,
        } => {
            let status = status.map(|s| match s {
                StatusArg::Lyrics => Status::Lyrics,
                StatusArg::Instrumental => Status::Instrumental,
                StatusArg::Unknown => Status::Unknown,
            });
            let listed: LyricsList = client.call(
                commands::LYRICS_LIST,
                json!({"status": status, "limit": limit, "offset": offset}),
            )?;
            Ok(if as_json {
                json!(listed).to_string()
            } else {
                list_text(&listed)
            })
        }
        LyricsCommands::Search { words, limit } => {
            let found: LyricsFound = client.call(
                commands::LYRICS_SEARCH,
                json!({"text": words.join(" "), "limit": limit}),
            )?;
            Ok(if as_json {
                json!(found).to_string()
            } else {
                search_text(&found)
            })
        }
    }
}

fn read_words(file: Option<&PathBuf>) -> Result<String, ServiceError> {
    match file {
        Some(path) if path.as_os_str() != "-" => std::fs::read_to_string(path)
            .map_err(|e| bad(format!("cannot read {}: {e}", path.display()))),
        _ => {
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .map_err(|e| bad(format!("cannot read standard input: {e}")))?;
            Ok(text)
        }
    }
}

/// The timed lines of LRC text: `[mm:ss]`, `[mm:ss.xx]` or `[mm:ss.xxx]`
/// before a line's words (several stamps share them). Tag lines such as
/// `[ar:...]` and lines without words are left out.
fn lrc_lines(text: &str) -> Result<Vec<Value>, ServiceError> {
    let mut lines: Vec<(u32, String)> = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut stamps = Vec::new();
        while let Some(after) = rest.strip_prefix('[') {
            let Some((stamp, tail)) = after.split_once(']') else {
                break;
            };
            match lrc_time(stamp) {
                Some(at) => stamps.push(at),
                None => break,
            }
            rest = tail.trim_start();
        }
        let words = rest.trim();
        if stamps.is_empty() || words.is_empty() {
            continue;
        }
        for at in stamps {
            lines.push((at, words.to_owned()));
        }
    }
    if lines.is_empty() {
        return Err(bad(
            "no timed lines in the LRC text: each line starts [mm:ss.xx]".to_owned(),
        ));
    }
    lines.sort_by_key(|(at, _)| *at);
    Ok(lines
        .into_iter()
        .map(|(at, text)| json!({"at_ms": at, "text": text}))
        .collect())
}

fn lrc_time(stamp: &str) -> Option<u32> {
    let (minutes, seconds) = stamp.split_once(':')?;
    let minutes: u32 = minutes.parse().ok()?;
    let (whole, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
    let whole: u32 = whole.parse().ok()?;
    if whole >= 60 || fraction.len() > 3 || !fraction.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let millis = if fraction.is_empty() {
        0
    } else {
        format!("{fraction:0<3}").parse::<u32>().ok()?
    };
    Some(minutes * 60_000 + whole * 1000 + millis)
}

fn status_word(status: Status) -> &'static str {
    match status {
        Status::Lyrics => "lyrics",
        Status::Instrumental => "instrumental",
        Status::Unknown => "unknown",
    }
}

fn shown_text(shown: &TrackLyrics) -> String {
    let mut out = format!(
        "Track {} \"{}\": {}\n",
        shown.track.id,
        shown.track.title,
        status_word(shown.status)
    );
    if let Some(standing) = &shown.standing {
        let _ = writeln!(
            out,
            "Standing: Fact {} from {}",
            standing.id,
            source_text(&standing.source)
        );
        if let Some(text) = standing.value.get("text").and_then(Value::as_str) {
            out.push('\n');
            out.push_str(text);
            out.push('\n');
        }
    }
    if shown.records.len() > 1 {
        let _ = writeln!(out, "\nOther records:");
        for fact in shown
            .records
            .iter()
            .filter(|f| Some(f.id) != shown.standing.as_ref().map(|s| s.id))
        {
            let _ = writeln!(out, "  Fact {} from {}", fact.id, source_text(&fact.source));
        }
    }
    out
}

fn imported_text(imported: &Imported) -> String {
    let mut out = String::new();
    for track in &imported.tracks {
        let what = match track.found {
            Found::Found => format!(
                "found ({}{})",
                track.tag.as_deref().unwrap_or("tags"),
                if track.synced { ", synced" } else { "" }
            ),
            Found::None => "none".to_owned(),
            Found::Failed => format!(
                "failed: {}",
                track.problem.as_deref().unwrap_or("unknown problem")
            ),
        };
        let _ = writeln!(out, "{} \"{}\": {what}", track.track.id, track.track.title);
    }
    let _ = writeln!(
        out,
        "{} found, {} none, {} failed{}",
        imported.found,
        imported.none,
        imported.failed,
        match (&imported.run, imported.dry_run) {
            (_, true) => "; dry run, nothing recorded".to_owned(),
            (Some(run), false) => format!("; recorded in Run {}", run.id.0),
            (None, false) => "; nothing to record".to_owned(),
        }
    );
    out
}

fn list_text(listed: &LyricsList) -> String {
    let mut out = String::new();
    for track in &listed.tracks {
        let _ = writeln!(
            out,
            "{}\t{}\t{} - {}",
            track.id,
            status_word(track.status),
            track.artist,
            track.title
        );
    }
    let _ = writeln!(
        out,
        "{} with lyrics, {} instrumental, {} unknown",
        listed.counts.lyrics, listed.counts.instrumental, listed.counts.unknown
    );
    out
}

fn search_text(found: &LyricsFound) -> String {
    let mut out = String::new();
    for hit in &found.hits {
        let _ = writeln!(
            out,
            "{} {} - {}: {}",
            hit.track.id, hit.track.artist, hit.track.title, hit.snippet
        );
    }
    if found.hits.is_empty() {
        out.push_str("No stored lyrics contain every word.\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_lines_are_timed_sorted_and_skip_tags_and_empty_lines() {
        let lines = lrc_lines(
            "[ar:Someone]\n[00:12.50]Second line\n[00:01]First line\n[00:20.5][01:02.125]Twice\n[00:30.00]\n",
        )
        .unwrap();
        assert_eq!(
            lines,
            [
                json!({"at_ms": 1000, "text": "First line"}),
                json!({"at_ms": 12_500, "text": "Second line"}),
                json!({"at_ms": 20_500, "text": "Twice"}),
                json!({"at_ms": 62_125, "text": "Twice"}),
            ]
        );
        assert!(lrc_lines("no stamps at all").is_err());
    }
}
