//! Lyrics (bead wbx-f6v): the words of a track, kept in the Sidecar so an
//! outside agent can read and search them and match tracks on their
//! themes. Themes themselves are the agent's work; it may record them as
//! ordinary Facts (the built-in `themes` kind).
//!
//! The service never fetches anything (ADR 0005). Lyrics arrive two ways:
//! read from the audio file's own tags (`lyrics.import_embedded`, see
//! [`crate::lyrics_tags`]), or handed in by someone (`lyrics.set`), with
//! the web address and date they were taken from when they came from the
//! web, as a lookup's Facts carry their address.
//!
//! Each record is a Fact of the reserved Judgement kind
//! [`LYRICS`](wreckordbox_sidecar::lyrics::LYRICS), its value a
//! [`Lyrics`]: so it follows the music (re-attached by fingerprint after a
//! re-add, never stale), is its own Ledger entry in the caller's Run, and
//! is replaced only by the same source recording again. Embedded lyrics
//! are recorded as `tool:file-tags`, web lyrics as `web:<address>`, other
//! supplied lyrics as the caller's source. Which record stands for a track
//! follows the DJ's source rules, as `facts.standing` does.
//!
//! Writes are refused for a Library hosted on a Stick (`sidecar_on_stick`,
//! both commands are in the Sidecar's recording list, so a dry run is
//! refused there too). Reads (`lyrics.show`, `lyrics.list`,
//! `lyrics.search`) and a dry run write nothing, make no Sidecar and send
//! no event. Nothing here touches the Library's own fields or a Stick.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wreckordbox_sidecar::lyrics::{Lyrics, Origin, SyncedLine, LYRICS, SEARCH_ENGINE};
use wreckordbox_sidecar::precedence::Rules;
use wreckordbox_sidecar::{Fingerprint, NewFact, Run, Source, SourceKind};

use crate::facts::{self, Fact, SourceArg};
use crate::mutant_reads::{inspect, Store};
use crate::protocol::{codes as service_codes, Request, ServiceError};
use crate::stems::{self, Parent, ParentAudio, SidecarChanged, Stems};

/// The commands.
pub mod commands {
    /// The lyrics the Sidecar holds for one track. Takes a
    /// [`ShowLyrics`](super::ShowLyrics), answers a
    /// [`TrackLyrics`](super::TrackLyrics). Reads only.
    pub const LYRICS_SHOW: &str = "lyrics.show";
    /// Records a track's lyrics as given, or that it has none. Takes a
    /// [`SetLyrics`](super::SetLyrics), answers a
    /// [`LyricsRecorded`](super::LyricsRecorded).
    pub const LYRICS_SET: &str = "lyrics.set";
    /// Reads lyrics from the tracks' own audio tags and records those
    /// found. Takes an [`ImportEmbedded`](super::ImportEmbedded), answers
    /// an [`Imported`](super::Imported).
    pub const LYRICS_IMPORT_EMBEDDED: &str = "lyrics.import_embedded";
    /// Which tracks have lyrics, are instrumental, or are unknown. Takes a
    /// [`ListLyrics`](super::ListLyrics), answers a
    /// [`LyricsList`](super::LyricsList). Reads only.
    pub const LYRICS_LIST: &str = "lyrics.list";
    /// Full-text search over the stored lyrics. Takes a
    /// [`SearchLyrics`](super::SearchLyrics), answers a
    /// [`LyricsFound`](super::LyricsFound). Reads only.
    pub const LYRICS_SEARCH: &str = "lyrics.search";
}

/// The error codes beside the service's, the Facts' and the Stems' own.
pub mod codes {
    /// The lyrics given do not hold together: no words and not
    /// instrumental, both, an address without a date, ...
    pub const INVALID_LYRICS: &str = "invalid_lyrics";
}

/// The source of lyrics read from a file's tags.
pub const FILE_TAGS: &str = "file-tags";

/// The arguments of `lyrics.show`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShowLyrics {
    pub id: String,
}

/// What is known of a track's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// The standing record has words.
    Lyrics,
    /// The standing record says the track has none.
    Instrumental,
    /// Nothing is recorded: nobody has looked, or nobody found any.
    Unknown,
}

/// What `lyrics.show` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackLyrics {
    pub track: Parent,
    pub status: Status,
    /// The record that stands, under the DJ's source rules.
    pub standing: Option<Fact>,
    /// Every live record, one per source, newest first; the value of each
    /// is a [`Lyrics`].
    pub records: Vec<Fact>,
}

/// The arguments of `lyrics.set`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetLyrics {
    pub id: String,
    /// The words, lines separated by newlines. Not with `instrumental`;
    /// with `lines`, left out (it is made from them).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Timed lines, for synced lyrics.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<SyncedLine>,
    /// The track has no words.
    #[serde(default)]
    pub instrumental: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// The web address the lyrics were taken from. The record's source is
    /// then `web:<address>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// The date (`YYYY-MM-DD`) they were taken from `address`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// 0 to 1; left out, 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// Who is recording: the Run's source, and the record's when there is
    /// no address. Needed unless `run`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceArg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// What `lyrics.set` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LyricsRecorded {
    pub run: Run,
    pub created_run: bool,
    pub fact: Fact,
}

/// The arguments of `lyrics.import_embedded`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportEmbedded {
    /// The tracks to read, by identifier; none is every track.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracks: Vec<String>,
    /// Read and report only; record nothing.
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceArg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// What reading one track's tags found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Found {
    /// Lyrics in the tags.
    Found,
    /// The tags hold none. Nothing is recorded: no lyrics in the file does
    /// not mean the track has none.
    None,
    /// The file is missing or its tags cannot be read; `problem` says why.
    Failed,
}

/// One track of `lyrics.import_embedded`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedTrack {
    pub track: Parent,
    pub found: Found,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default)]
    pub synced: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    /// The Fact recorded; `null` in a dry run or when nothing was found.
    #[serde(default)]
    pub fact: Option<i64>,
}

/// What `lyrics.import_embedded` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Imported {
    pub dry_run: bool,
    /// The Run the records are in; `null` in a dry run or when nothing was
    /// found.
    pub run: Option<Run>,
    pub found: usize,
    pub none: usize,
    pub failed: usize,
    pub tracks: Vec<ImportedTrack>,
}

/// The arguments of `lyrics.list`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListLyrics {
    /// Only tracks in this state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
}

/// One track of `lyrics.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedTrack {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub status: Status,
    /// Of the standing record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    #[serde(default)]
    pub synced: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

/// How many tracks are in each state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Counts {
    pub lyrics: usize,
    pub instrumental: usize,
    pub unknown: usize,
}

/// What `lyrics.list` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricsList {
    /// Over the whole Library, whatever `status` asked for.
    pub counts: Counts,
    /// How many tracks matched `status`.
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
    pub next_offset: Option<usize>,
    pub tracks: Vec<ListedTrack>,
}

/// The arguments of `lyrics.search`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchLyrics {
    /// Words, every one of which must be in the lyrics, in any case and
    /// any order; punctuation is ignored.
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// One track whose lyrics match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LyricsHit {
    pub track: ListedTrackRef,
    /// The matching record.
    pub fact: Fact,
    /// Whether that record is the one standing for the track.
    pub standing: bool,
    /// The match in context, the matching words in `[` `]`.
    pub snippet: String,
    /// Lower is a better match (FTS5 bm25).
    pub rank: f64,
}

/// A track named in a search hit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedTrackRef {
    pub id: String,
    pub title: String,
    pub artist: String,
}

/// What `lyrics.search` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LyricsFound {
    /// How the search was done: `sqlite-fts5`.
    pub engine: String,
    pub text: String,
    pub hits: Vec<LyricsHit>,
}

const DEFAULT_LIMIT: usize = 100;

/// Whether `command` is one of this module's.
pub(crate) fn answers(command: &str) -> bool {
    matches!(
        command,
        commands::LYRICS_SHOW
            | commands::LYRICS_SET
            | commands::LYRICS_IMPORT_EMBEDDED
            | commands::LYRICS_LIST
            | commands::LYRICS_SEARCH
    )
}

/// The Library as these commands need it: one track, or the whole index.
pub(crate) struct Library<'a> {
    pub(crate) track: &'a dyn Fn(&str) -> Result<ParentAudio, ServiceError>,
    pub(crate) index: &'a dyn Fn() -> Result<Arc<rbl_index::Library>, ServiceError>,
}

/// Answers one of this module's commands. The Library is only read, to
/// find tracks, and let go before any file is read.
pub(crate) fn answer(
    stems: &Arc<Stems>,
    request: &Request,
    library: &Library<'_>,
) -> Result<Value, ServiceError> {
    match request.command.as_str() {
        commands::LYRICS_SHOW => encode(show(stems, &facts::arguments(request)?, library)?),
        commands::LYRICS_SET => encode(set(stems, facts::arguments(request)?, library)?),
        commands::LYRICS_IMPORT_EMBEDDED => {
            encode(import(stems, &facts::arguments(request)?, library)?)
        }
        commands::LYRICS_LIST => encode(list(stems, &facts::arguments(request)?, library)?),
        _ => encode(search(stems, &facts::arguments(request)?, library)?),
    }
}

fn encode(value: impl Serialize) -> Result<Value, ServiceError> {
    serde_json::to_value(value)
        .map_err(|e| ServiceError::new(service_codes::SERVICE_FAILED, e.to_string()))
}

fn rules(stems: &Stems) -> Result<Rules, ServiceError> {
    Rules::load(stems.sidecar_dir())
        .map_err(|why| ServiceError::new(crate::standing::RULES_INVALID, why))
}

/// Every live Lyrics Fact, by track, newest first; read without writing.
fn records(stems: &Stems) -> Result<HashMap<String, Vec<wreckordbox_sidecar::Fact>>, ServiceError> {
    let all = inspect(stems, |store| match store {
        Store::Open(s) => s.lyrics_records(),
        Store::Snapshot(s) => s.lyrics_records(),
        Store::Absent => Ok(Vec::new()),
    })?;
    let mut by_track: HashMap<String, Vec<_>> = HashMap::new();
    for fact in all {
        by_track.entry(fact.track_id.clone()).or_default().push(fact);
    }
    Ok(by_track)
}

fn lyrics_of(fact: &wreckordbox_sidecar::Fact) -> Option<Lyrics> {
    serde_json::from_value(fact.value.clone()).ok()
}

fn status_of(standing: Option<&wreckordbox_sidecar::Fact>) -> Status {
    match standing.and_then(lyrics_of) {
        None => Status::Unknown,
        Some(lyrics) if lyrics.instrumental => Status::Instrumental,
        Some(_) => Status::Lyrics,
    }
}

fn show(
    stems: &Stems,
    wanted: &ShowLyrics,
    library: &Library<'_>,
) -> Result<TrackLyrics, ServiceError> {
    let found = (library.track)(&wanted.id)?;
    let rules = rules(stems)?;
    let id = &found.parent().id;
    let stored = inspect(stems, |store| match store {
        Store::Open(s) => s.facts(id, LYRICS.name),
        Store::Snapshot(s) => s.facts(id, LYRICS.name),
        Store::Absent => Ok(Vec::new()),
    })?;
    let live: Vec<_> = stored.into_iter().filter(|f| f.is_live()).collect();
    let (standing, _) = rules.standing(&live);
    Ok(TrackLyrics {
        track: found.parent().clone(),
        status: status_of(standing),
        standing: standing.cloned().map(Fact::from),
        records: live.into_iter().map(Fact::from).collect(),
    })
}

fn invalid(why: impl Into<String>) -> ServiceError {
    ServiceError::new(codes::INVALID_LYRICS, why)
}

fn set(
    stems: &Arc<Stems>,
    given: SetLyrics,
    library: &Library<'_>,
) -> Result<LyricsRecorded, ServiceError> {
    let synced = !given.lines.is_empty();
    if synced && given.text.is_some() {
        return Err(invalid(
            "give the words as text or as timed lines, not both",
        ));
    }
    let text = if synced {
        Some(
            given
                .lines
                .iter()
                .map(|line| line.text.trim())
                .collect::<Vec<_>>()
                .join("\n"),
        )
    } else {
        given
            .text
            .map(|text| text.replace("\r\n", "\n").trim().to_owned())
            .filter(|text| !text.is_empty())
    };
    let address = given.address.map(|a| a.trim().to_owned());
    let lyrics = Lyrics {
        instrumental: given.instrumental,
        text,
        synced,
        lines: given.lines,
        language: given.language.map(|l| l.trim().to_owned()),
        origin: Origin::Supplied,
        tag: None,
        address: address.clone(),
        retrieved: given.date.map(|d| d.trim().to_owned()),
    };
    lyrics.check().map_err(invalid)?;
    let confidence = given.confidence.unwrap_or(1.0);
    if !(0.0..=1.0).contains(&confidence) {
        return Err(ServiceError::new(
            facts::codes::INVALID_CONFIDENCE,
            format!("a confidence is from 0 to 1, not {confidence}"),
        ));
    }
    let caller = caller(stems, given.source.as_ref(), given.run)?;
    let source = address.map_or_else(
        || caller.source.clone(),
        |address| Source::new(SourceKind::Web, address),
    );
    let found = (library.track)(&given.id)?;
    let fingerprint = fingerprint(&found)?;
    let description = given.description.unwrap_or_else(|| {
        format!("Record lyrics for track {}", found.parent().id)
    });
    let (run, created_run, mut facts) = write(
        stems,
        &caller,
        &description,
        vec![Pending {
            found,
            fingerprint,
            lyrics,
            source,
            confidence,
        }],
    )?;
    Ok(LyricsRecorded {
        run,
        created_run,
        fact: facts.remove(0),
    })
}

fn import(
    stems: &Arc<Stems>,
    wanted: &ImportEmbedded,
    library: &Library<'_>,
) -> Result<Imported, ServiceError> {
    // Who is asking is settled first, so a write that would be refused for
    // want of a source reads no file; a dry run needs no one.
    let caller = if wanted.dry_run {
        None
    } else {
        Some(caller(stems, wanted.source.as_ref(), wanted.run)?)
    };
    let index = (library.index)()?;
    let rows: Vec<rbl_index::Row> = if wanted.tracks.is_empty() {
        crate::tracks::matching(&index, &crate::tracks::TrackSelection::default())?
    } else {
        let mut rows = Vec::new();
        for id in &wanted.tracks {
            let row = index.row_of(id).ok_or_else(|| {
                ServiceError::new(
                    service_codes::TRACK_NOT_FOUND,
                    format!("there is no track {id} in the Library"),
                )
            })?;
            if !rows.contains(&row) {
                rows.push(row);
            }
        }
        rows
    };
    let mut tracks = Vec::with_capacity(rows.len());
    let mut pending = Vec::new();
    let tags = Source::new(SourceKind::Tool, FILE_TAGS);
    for row in rows {
        let found = stems::audio_at(&index, row);
        let mut outcome = ImportedTrack {
            track: found.parent().clone(),
            found: Found::None,
            tag: None,
            synced: false,
            language: None,
            problem: None,
            fact: None,
        };
        let read = if found.path().is_file() {
            crate::lyrics_tags::read(found.path())
        } else {
            Err(format!(
                "the audio file is missing: there is no {}",
                found.path().display()
            ))
        };
        match read {
            Err(problem) => {
                outcome.found = Found::Failed;
                outcome.problem = Some(problem);
            }
            Ok(None) => {}
            Ok(Some(embedded)) => {
                outcome.found = Found::Found;
                outcome.tag = Some(embedded.tag.to_owned());
                outcome.synced = !embedded.lines.is_empty();
                outcome.language.clone_from(&embedded.language);
                if caller.is_some() {
                    let lyrics = Lyrics {
                        instrumental: false,
                        text: Some(embedded.text),
                        synced: !embedded.lines.is_empty(),
                        lines: embedded.lines,
                        language: embedded.language,
                        origin: Origin::Embedded,
                        tag: Some(embedded.tag.to_owned()),
                        address: None,
                        retrieved: None,
                    };
                    match lyrics.check() {
                        Ok(()) => match fingerprint(&found) {
                            Ok(fingerprint) => pending.push((
                                tracks.len(),
                                Pending {
                                    found,
                                    fingerprint,
                                    lyrics,
                                    source: tags.clone(),
                                    confidence: 1.0,
                                },
                            )),
                            Err(error) => {
                                outcome.found = Found::Failed;
                                outcome.problem = Some(error.message);
                            }
                        },
                        Err(why) => {
                            outcome.found = Found::Failed;
                            outcome.problem = Some(why);
                        }
                    }
                }
            }
        }
        tracks.push(outcome);
    }
    let mut run = None;
    if let (Some(caller), false) = (&caller, pending.is_empty()) {
        let description = wanted.description.clone().unwrap_or_else(|| {
            format!(
                "Import lyrics from the audio tags of {} {}",
                pending.len(),
                if pending.len() == 1 { "track" } else { "tracks" }
            )
        });
        let (at, items): (Vec<usize>, Vec<Pending>) = pending.into_iter().unzip();
        let (written_in, _, facts) = write(stems, caller, &description, items)?;
        for (at, fact) in at.into_iter().zip(facts) {
            tracks[at].fact = Some(fact.id);
        }
        run = Some(written_in);
    }
    let count = |found: Found| tracks.iter().filter(|t| t.found == found).count();
    Ok(Imported {
        dry_run: wanted.dry_run,
        run,
        found: count(Found::Found),
        none: count(Found::None),
        failed: count(Found::Failed),
        tracks,
    })
}

fn list(
    stems: &Stems,
    wanted: &ListLyrics,
    library: &Library<'_>,
) -> Result<LyricsList, ServiceError> {
    let rules = rules(stems)?;
    let by_track = records(stems)?;
    let index = (library.index)()?;
    let rows = crate::tracks::matching(&index, &crate::tracks::TrackSelection::default())?;
    let mut counts = Counts::default();
    let mut matched = Vec::new();
    for row in rows {
        let at = row as usize;
        let id = index.ids.get(at).copied().unwrap_or(0).to_string();
        let live = by_track.get(&id).map_or(&[][..], Vec::as_slice);
        let (standing, _) = rules.standing(live);
        let status = status_of(standing);
        match status {
            Status::Lyrics => counts.lyrics += 1,
            Status::Instrumental => counts.instrumental += 1,
            Status::Unknown => counts.unknown += 1,
        }
        if wanted.status.is_some_and(|wanted| wanted != status) {
            continue;
        }
        let lyrics = standing.and_then(lyrics_of);
        matched.push(ListedTrack {
            title: index.title.get(at).to_owned(),
            artist: index.artist_name(row).to_owned(),
            id,
            status,
            source: standing.map(|fact| fact.source.clone()),
            synced: lyrics.as_ref().is_some_and(|l| l.synced),
            language: lyrics.and_then(|l| l.language),
        });
    }
    let total = matched.len();
    let offset = wanted.offset.unwrap_or(0);
    let limit = wanted.limit.unwrap_or(DEFAULT_LIMIT);
    let tracks: Vec<ListedTrack> = matched.into_iter().skip(offset).take(limit).collect();
    let end = offset.saturating_add(tracks.len());
    Ok(LyricsList {
        counts,
        total,
        offset,
        limit,
        next_offset: (end < total && !tracks.is_empty()).then_some(end),
        tracks,
    })
}

fn search(
    stems: &Stems,
    wanted: &SearchLyrics,
    library: &Library<'_>,
) -> Result<LyricsFound, ServiceError> {
    if wreckordbox_sidecar::lyrics::query_of(&wanted.text).is_none() {
        return Err(ServiceError::new(
            service_codes::BAD_REQUEST,
            "give some words to search the lyrics for",
        ));
    }
    let rules = rules(stems)?;
    let limit = wanted.limit.unwrap_or(DEFAULT_LIMIT);
    let found = inspect(stems, |store| match store {
        Store::Open(s) => s.search_lyrics(&wanted.text, limit),
        Store::Snapshot(s) => s.search_lyrics(&wanted.text, limit),
        Store::Absent => Ok(Vec::new()),
    })?;
    let by_track = if found.is_empty() {
        HashMap::new()
    } else {
        records(stems)?
    };
    let index = (library.index)()?;
    let mut hits = Vec::with_capacity(found.len());
    for hit in found {
        // A record about a track no longer in the Library is not a hit.
        let Some(row) = index.row_of(&hit.fact.track_id) else {
            continue;
        };
        let at = row as usize;
        let live = by_track
            .get(&hit.fact.track_id)
            .map_or(&[][..], Vec::as_slice);
        let standing = rules.standing(live).0.map(|f| f.id) == Some(hit.fact.id);
        hits.push(LyricsHit {
            track: ListedTrackRef {
                id: hit.fact.track_id.clone(),
                title: index.title.get(at).to_owned(),
                artist: index.artist_name(row).to_owned(),
            },
            fact: Fact::from(hit.fact),
            standing,
            snippet: hit.snippet,
            rank: hit.rank,
        });
    }
    Ok(LyricsFound {
        engine: SEARCH_ENGINE.to_owned(),
        text: wanted.text.clone(),
        hits,
    })
}

/// Who a write is by: its Run's source, and the open Run it joins.
struct Caller {
    source: Source,
    run: Option<Run>,
}

fn caller(
    stems: &Arc<Stems>,
    source: Option<&SourceArg>,
    run: Option<i64>,
) -> Result<Caller, ServiceError> {
    let source = source.map(facts::source).transpose()?;
    let run = run.map(|id| facts::open_run(stems, id)).transpose()?;
    let source = source
        .or_else(|| run.as_ref().map(|run| run.source.clone()))
        .ok_or_else(|| {
            ServiceError::new(
                facts::codes::INVALID_SOURCE,
                "say who is recording: a source (dj, agent, tool or web, with a name), or a Run to record in",
            )
        })?;
    Ok(Caller { source, run })
}

/// The fingerprint of a track's audio, when its file is there: a
/// Judgement does without it, but it lets the lyrics follow the music.
fn fingerprint(found: &ParentAudio) -> Result<Option<Fingerprint>, ServiceError> {
    if found.path().is_file() {
        stems::fingerprint(found.parent(), found.path()).map(Some)
    } else {
        Ok(None)
    }
}

struct Pending {
    found: ParentAudio,
    fingerprint: Option<Fingerprint>,
    lyrics: Lyrics,
    source: Source,
    confidence: f64,
}

/// Records lyrics, all or none, in the caller's Run or one made for this
/// command, and tells clients.
fn write(
    stems: &Arc<Stems>,
    caller: &Caller,
    description: &str,
    items: Vec<Pending>,
) -> Result<(Run, bool, Vec<Fact>), ServiceError> {
    let sidecar = stems.sidecar()?;
    let (run, created) = match &caller.run {
        Some(run) => (run.clone(), false),
        None => {
            let id = sidecar
                .begin_run(&caller.source, description.trim())
                .map_err(|e| stems::sidecar_error(&e))?;
            let run = sidecar
                .run(id)
                .map_err(|e| stems::sidecar_error(&e))?
                .ok_or_else(|| ServiceError::new(service_codes::SERVICE_FAILED, "the new Run is gone"))?;
            (run, true)
        }
    };
    // The point every Fact producer shares: re-attach by fingerprint, and
    // mark the track's Measurements made from other audio.
    for item in &items {
        stems.before_record(&item.found, item.fingerprint.as_ref(), Some(run.id))?;
    }
    let values: Vec<Value> = items
        .iter()
        .map(|item| json!(item.lyrics))
        .collect();
    let new: Vec<NewFact> = items
        .iter()
        .zip(&values)
        .map(|(item, value)| NewFact {
            track_id: &item.found.parent().id,
            fingerprint: item.fingerprint.as_ref(),
            kind: LYRICS,
            value: value.clone(),
            source: &item.source,
            confidence: item.confidence,
            run: run.id,
        })
        .collect();
    let written = sidecar.record_many(&new);
    let run = if created {
        let outcome = match &written {
            Ok(facts) if facts.len() == 1 => "recorded lyrics for 1 track".to_owned(),
            Ok(facts) => format!("recorded lyrics for {} tracks", facts.len()),
            Err(error) => format!("nothing recorded: {error}"),
        };
        let _ = sidecar.finish_run(run.id, &outcome);
        sidecar
            .run(run.id)
            .map_err(|e| stems::sidecar_error(&e))?
            .unwrap_or(run)
    } else {
        run
    };
    let facts: Vec<Fact> = written
        .map_err(|e| stems::sidecar_error(&e))?
        .into_iter()
        .map(Fact::from)
        .collect();
    for fact in &facts {
        stems.publish(
            stems::events::SIDECAR_CHANGED,
            json!(SidecarChanged {
                track_id: fact.track_id.clone(),
                kind: fact.kind.clone(),
                fact_id: Some(fact.id),
                stale: Vec::new(),
            }),
        );
    }
    Ok((run, created, facts))
}
