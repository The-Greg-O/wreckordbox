//! Reading tracks: one track in full, and the general track query.
//!
//! Both answer from the Library core's index with the machinery the window
//! lists tracks with, so a client and the window agree on what matches:
//!
//! - the free-text `search` is the window's search box;
//! - a playlist is opened as the window opens it from the tree (an ordinary
//!   playlist in its own order, an intelligent one by its rule, a folder as
//!   every track beneath it);
//! - key, Energy, Prep colour, MyTags and a whole-number tempo are the
//!   window's Track Filter;
//! - every other field is a condition of an intelligent playlist rule,
//!   evaluated as the window evaluates one.
//!
//! Only two filters have no equivalent in the window and are answered here:
//! the file's location and whether the track has been analysed.
#![allow(
    clippy::doc_markdown,
    reason = "MyTag is the glossary's word for the thing, not a code item"
)]

use std::sync::Arc;

use rbl_index::smart::{Condition, Date, Group, Item, Logic, Operator, Property, SmartRule};
use rbl_index::{
    BpmFilter, Row, SortColumn, TagFilter, TrackFilter, TrackSource, ViewSpec, COLOR_NAMES,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::library::OpenLibrary;
use crate::protocol::{codes, Request, ServiceError};

/// One track in full: takes a [`TrackId`], answers a [`TrackRecord`].
pub const TRACKS_GET: &str = "tracks.get";
/// Tracks selected by any of their fields: takes a [`TrackSelection`],
/// answers a [`TrackPage`].
pub const TRACKS_QUERY: &str = "tracks.query";

/// No playlist has the identifier or the name asked for.
pub const PLAYLIST_NOT_FOUND: &str = "playlist_not_found";
/// No MyTag has the name asked for.
pub const MYTAG_NOT_FOUND: &str = "mytag_not_found";

/// How many tracks `tracks.query` returns when the client names no limit.
pub const DEFAULT_PAGE_LIMIT: usize = 50;

/// The arguments of `tracks.get`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackId {
    /// The track's identifier, as every track listing gives it.
    pub id: String,
}

/// Everything the Library holds about one track, under stable names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackRecord {
    /// The track's identifier in the Library; what an edit names it by.
    pub id: String,
    pub title: String,
    /// Empty when the track has none; so are the other texts.
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub label: String,
    /// The release year; `null` when unknown.
    pub year: Option<u16>,
    /// Beats per minute; `null` for a track with no tempo.
    pub bpm: Option<f64>,
    /// As the Library spells it, for example `8A`; empty when not analysed.
    pub key: String,
    /// The DJ's 1 to 5 judgement of the track's intensity; `null` when not
    /// set. It is stored as `rating`.
    pub energy: Option<u8>,
    /// Stars, 0 to 5: where Energy is stored.
    pub rating: u8,
    /// Where the track is in preparation, as the name of its colour;
    /// `null` when not set. It is stored as `colour`.
    pub prep_colour: Option<String>,
    /// The colour label's name; `null` for no colour.
    pub colour: Option<String>,
    /// The colour label as the Library numbers it, 1 to 8; 0 for no colour.
    pub colour_id: u8,
    pub comment: String,
    pub duration_seconds: u32,
    /// `YYYY-MM-DD`; `null` when the Library does not say.
    pub date_added: Option<String>,
    pub play_count: u16,
    /// The audio file's full path.
    pub location: String,
    pub file_name: String,
    /// Whether the track has been analysed (tempo, key, waveform).
    pub analysed: bool,
    /// The MyTags the track carries, in the order the Library lists them.
    pub my_tags: Vec<TrackTag>,
    /// The playlists the track is in, intelligent ones included.
    pub playlists: Vec<TrackPlaylist>,
}

/// A MyTag on a track.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackTag {
    pub id: String,
    /// The MyTag category the tag belongs to.
    pub category: String,
    pub tag: String,
}

/// A playlist a track is in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackPlaylist {
    pub id: String,
    pub name: String,
}

/// The arguments of `tracks.query`. Every one may be left out; the ones
/// given must all hold.
///
/// A number or a date is one value (`2021`) or an inclusive range
/// (`2019..2021`, `2019..`, `..2021`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TrackSelection {
    /// Text as typed into the window's search box: every word must appear
    /// somewhere in the track's details.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub search: String,
    /// Text the title contains, whatever its case. The same for the other
    /// texts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub genre: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    /// Text the audio file's path contains, whatever its case.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// The key by its name, exactly as the Library spells it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// The release year, or a range of years.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<String>,
    /// A whole tempo (`128` is every track the window's Track Filter lists
    /// under 128), an exact one (`128.03`) or a range (`124..128.5`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bpm: Option<String>,
    /// Energy, 1 to 5, or a range; 0 is a track with none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy: Option<String>,
    /// The Prep colour by its colour's name, or `none` for tracks without.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prep_colour: Option<String>,
    /// Seconds or `M:SS`, or a range of either.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<String>,
    /// `YYYY-MM-DD`, or a range of dates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_added: Option<String>,
    /// How many times the track has been played, or a range.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub play_count: Option<String>,
    /// Whether the track has been analysed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analysed: Option<bool>,
    /// MyTags by name; the track must carry every one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub my_tags: Vec<String>,
    /// Keys by name, a track in any of them (the Track Filter bar's pick;
    /// see [`crate::track_filter`]).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<String>,
    /// Prep colours by colour name, a track with any of them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub prep_colours: Vec<String>,
    /// MyTags picked within a category, one entry per category.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub my_tag_picks: Vec<crate::track_filter::MyTagPick>,
    /// A playlist, by identifier or by name: only its tracks, in its order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist: Option<String>,
    /// The most tracks to return; [`DEFAULT_PAGE_LIMIT`] when left out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// How many matching tracks to skip: a page's `next_offset` fetches the
    /// page after it.
    pub offset: usize,
}

/// What `tracks.query` answers: one page of the tracks that matched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackPage {
    /// In the collection's own order, or the playlist's when one was named.
    pub tracks: Vec<TrackRecord>,
    /// How many tracks matched, on every page together.
    pub total: usize,
    /// How many matching tracks come before this page.
    pub offset: usize,
    /// The most tracks this page could hold.
    pub limit: usize,
    /// The `offset` that fetches the next page; `null` on the last one.
    pub next_offset: Option<usize>,
}

/// Answers `tracks.get` and `tracks.query`.
pub(crate) fn answer(library: &OpenLibrary, request: &Request) -> Result<Value, ServiceError> {
    let index = library.index()?;
    let decoded =
        |e: serde_json::Error| bad_request(format!("bad arguments to {}: {e}", request.command));
    let answer = if request.command == TRACKS_GET {
        let wanted: TrackId = serde_json::from_value(request.args.clone()).map_err(decoded)?;
        serde_json::to_value(get(&index, &wanted.id)?)
    } else {
        let selection: TrackSelection = if request.args.is_null() {
            TrackSelection::default()
        } else {
            serde_json::from_value(request.args.clone()).map_err(decoded)?
        };
        serde_json::to_value(query(&index, &selection)?)
    };
    answer.map_err(|e| ServiceError::new(codes::SERVICE_FAILED, e.to_string()))
}

fn get(index: &rbl_index::Library, id: &str) -> Result<TrackRecord, ServiceError> {
    index.row_of(id).map_or_else(
        || {
            Err(ServiceError::new(
                codes::TRACK_NOT_FOUND,
                format!("there is no track {id} in the Library"),
            ))
        },
        |row| Ok(record(index, row)),
    )
}

fn query(
    index: &Arc<rbl_index::Library>,
    selection: &TrackSelection,
) -> Result<TrackPage, ServiceError> {
    let rows = matching(index, selection)?;
    let limit = selection.limit.unwrap_or(DEFAULT_PAGE_LIMIT);
    let total = rows.len();
    let offset = selection.offset.min(total);
    let end = offset.saturating_add(limit).min(total);
    let tracks: Vec<TrackRecord> = rows
        .get(offset..end)
        .unwrap_or(&[])
        .iter()
        .map(|&row| record(index, row))
        .collect();
    Ok(TrackPage {
        tracks,
        total,
        offset,
        limit,
        next_offset: (end < total && end > offset).then_some(end),
    })
}

/// Every track a selection matches, in the collection's own order or the
/// playlist's when one was named; its `limit` and `offset` are not applied.
pub(crate) fn matching(
    index: &Arc<rbl_index::Library>,
    selection: &TrackSelection,
) -> Result<Vec<Row>, ServiceError> {
    let source = match &selection.playlist {
        Some(wanted) => playlist_source(index, wanted)?,
        None => TrackSource::Collection,
    };
    let view = index.open_view(&ViewSpec {
        source,
        sort: SortColumn::TrackNo,
        descending: false,
        query: selection.search.clone(),
        filter: track_filter(index, selection)?,
    });
    let mut rows = view.rows;

    let conditions = conditions(selection)?;
    if !conditions.is_empty() {
        let rule = SmartRule {
            root: Group {
                logic: Logic::All,
                items: conditions.into_iter().map(Item::Condition).collect(),
            },
        };
        let mut admitted = vec![false; index.len()];
        for row in rule.evaluate_on(index, &Date::today()) {
            if let Some(slot) = admitted.get_mut(row as usize) {
                *slot = true;
            }
        }
        rows.retain(|&row| admitted.get(row as usize).copied().unwrap_or(false));
    }
    if let Some(wanted) = &selection.location {
        let wanted = wanted.to_lowercase();
        rows.retain(|&row| {
            index
                .folder_path
                .get(row as usize)
                .to_lowercase()
                .contains(&wanted)
        });
    }
    if let Some(wanted) = selection.analysed {
        rows.retain(|&row| analysed(index, row) == wanted);
    }
    Ok(rows)
}

/// The playlist as the window opens it from the tree.
fn playlist_source(index: &rbl_index::Library, wanted: &str) -> Result<TrackSource, ServiceError> {
    // Let go of the tree before the view takes it again.
    let playlists = index.playlists();
    let by_id = (0..playlists.len()).find(|&at| {
        playlists
            .ids
            .get(at)
            .is_some_and(|id| id.to_string() == wanted)
    });
    let found = if let Some(at) = by_id {
        at
    } else {
        let named: Vec<usize> = (0..playlists.len())
            .filter(|&at| playlists.name(at) == wanted)
            .collect();
        match named.as_slice() {
            [] => {
                return Err(ServiceError::new(
                    PLAYLIST_NOT_FOUND,
                    format!("there is no playlist {wanted:?} in the Library"),
                ))
            }
            [one] => *one,
            several => {
                let ids: Vec<String> = several
                    .iter()
                    .filter_map(|&at| playlists.ids.get(at).map(u64::to_string))
                    .collect();
                return Err(bad_request(format!(
                    "{} playlists are named {wanted:?}; name one by its identifier: {}",
                    several.len(),
                    ids.join(", ")
                )));
            }
        }
    };
    Ok(if playlists.is_folder(found) {
        TrackSource::PlaylistFolder(found)
    } else if playlists.is_smart(found) {
        TrackSource::SmartPlaylist(found)
    } else {
        TrackSource::Playlist(found)
    })
}

/// The picks the window's Track Filter would hold for this selection.
fn track_filter(
    index: &rbl_index::Library,
    selection: &TrackSelection,
) -> Result<TrackFilter, ServiceError> {
    let mut filter = TrackFilter::default();
    if let Some(key) = &selection.key {
        filter.keys = Some(index.key_ids_named(std::slice::from_ref(key)));
    }
    if let Some(energy) = &selection.energy {
        // With no lower end, from the least Energy there is: a track with
        // none is not below any.
        let (low, high) = whole_range("energy", energy, 1)?;
        if low > 5 || (high > 5 && high != OPEN_HIGH) {
            return Err(bad_request(format!(
                "{energy:?} is not an Energy: Energy is 1 to 5, or 0 for none"
            )));
        }
        filter.ratings = Some(
            (0..=5_u8)
                .filter(|stars| i64::from(*stars) >= low && i64::from(*stars) <= high)
                .collect(),
        );
    }
    if let Some(colour) = &selection.prep_colour {
        let id = if colour.eq_ignore_ascii_case("none") {
            0
        } else {
            COLOR_NAMES
                .iter()
                .position(|name| name.eq_ignore_ascii_case(colour))
                .and_then(|at| u8::try_from(at + 1).ok())
                .ok_or_else(|| {
                    bad_request(format!(
                        "{colour:?} is not a Prep colour: it is one of {}, or none",
                        COLOR_NAMES.join(", ")
                    ))
                })?
        };
        filter.colors = Some(vec![id]);
    }
    if let Some(bpm) = selection.bpm.as_deref().map(str::trim) {
        // A whole tempo is the Track Filter's pick; anything finer is a
        // rule's condition (see `conditions`).
        if let Ok(whole) = bpm.parse::<u32>() {
            filter.bpm = Some(BpmFilter {
                values: vec![whole],
                tolerance_pct: 0,
                master_bpm_x100: None,
            });
        }
    }
    for wanted in &selection.my_tags {
        let ids: Vec<i32> = index
            .my_tags()
            .iter()
            .flat_map(|category| category.tags.iter().zip(&category.ids))
            .filter(|(name, _)| *name == wanted)
            .map(|(_, &id)| id)
            .collect();
        if ids.is_empty() {
            return Err(ServiceError::new(
                MYTAG_NOT_FOUND,
                format!("there is no MyTag {wanted:?} in the Library"),
            ));
        }
        filter.tags.push(TagFilter {
            picks: vec![ids],
            any: false,
        });
    }
    crate::track_filter::extend(index, selection, &mut filter)?;
    Ok(filter)
}

/// The conditions an intelligent playlist would hold for this selection.
fn conditions(selection: &TrackSelection) -> Result<Vec<Condition>, ServiceError> {
    let mut conditions = Vec::new();
    for (property, wanted) in [
        (Property::Title, &selection.title),
        (Property::Artist, &selection.artist),
        (Property::Album, &selection.album),
        (Property::Genre, &selection.genre),
        (Property::Label, &selection.label),
        (Property::Comment, &selection.comment),
    ] {
        if let Some(wanted) = wanted {
            conditions.push(condition(property, Operator::Contains, wanted, ""));
        }
    }
    // A track with no year is not before any year, so a range with no
    // lower end starts at year 1.
    for (name, property, wanted, least) in [
        ("year", Property::Year, &selection.year, 1),
        ("play_count", Property::PlayCount, &selection.play_count, 0),
    ] {
        if let Some(wanted) = wanted {
            let (low, high) = whole_range(name, wanted, least)?;
            conditions.push(condition(
                property,
                Operator::InRange,
                &low.to_string(),
                &high.to_string(),
            ));
        }
    }
    if let Some(wanted) = &selection.duration {
        let (low, high) = range(wanted);
        let seconds = |text: &str, open: i64| -> Result<i64, ServiceError> {
            if text.is_empty() {
                return Ok(open);
            }
            let parsed = match text.split_once(':') {
                Some((minutes, seconds)) => minutes
                    .parse::<u32>()
                    .ok()
                    .zip(seconds.parse::<u32>().ok())
                    .map(|(minutes, seconds)| i64::from(minutes) * 60 + i64::from(seconds)),
                None => text.parse::<u32>().ok().map(i64::from),
            };
            parsed.ok_or_else(|| {
                bad_request(format!(
                    "{wanted:?} is not a duration: give seconds or M:SS, or a range like 3:00..6:00"
                ))
            })
        };
        conditions.push(condition(
            Property::Duration,
            Operator::InRange,
            &seconds(low, 0)?.to_string(),
            &seconds(high, OPEN_HIGH)?.to_string(),
        ));
    }
    if let Some(wanted) = &selection.date_added {
        let (low, high) = range(wanted);
        let date = |text: &str, open: &str| -> Result<String, ServiceError> {
            if text.is_empty() {
                return Ok(open.to_owned());
            }
            let day = Date::parse(text).filter(|_| text.len() == 10);
            day.map(|_| text.to_owned()).ok_or_else(|| {
                bad_request(format!(
                    "{wanted:?} is not a date: give YYYY-MM-DD, or a range like 2026-01-01..2026-06-30"
                ))
            })
        };
        // The first and last days a rule's date can name.
        conditions.push(condition(
            Property::DateAdded,
            Operator::InRange,
            &date(low, "1970-01-02")?,
            &date(high, "3000-12-31")?,
        ));
    }
    if let Some(wanted) = selection.bpm.as_deref().map(str::trim) {
        if wanted.parse::<u32>().is_err() {
            let (low, high) = range(wanted);
            // A rule holds a tempo in hundredths.
            let hundredths = |text: &str, open: i64| -> Result<i64, ServiceError> {
                if text.is_empty() {
                    return Ok(open);
                }
                text.parse::<f64>()
                    .ok()
                    .filter(|bpm| bpm.is_finite() && *bpm >= 0.0 && *bpm < 100_000.0)
                    .map(|bpm| {
                        #[allow(clippy::cast_possible_truncation, reason = "bounded just above")]
                        let hundredths = (bpm * 100.0).round() as i64;
                        hundredths
                    })
                    .ok_or_else(|| {
                        bad_request(format!(
                            "{wanted:?} is not a tempo: give a BPM, or a range like 120..128"
                        ))
                    })
            };
            // A track with no tempo is not slower than any tempo.
            conditions.push(condition(
                Property::Bpm,
                Operator::InRange,
                &hundredths(low, 1)?.to_string(),
                &hundredths(high, OPEN_HIGH)?.to_string(),
            ));
        }
    }
    Ok(conditions)
}

fn condition(property: Property, operator: Operator, left: &str, right: &str) -> Condition {
    Condition {
        property,
        operator,
        left: left.to_owned(),
        right: right.to_owned(),
        unit: String::new(),
    }
}

/// The far end of a range with no upper bound: the largest number a rule's
/// condition can hold.
const OPEN_HIGH: i64 = i32::MAX as i64;

/// The two ends of `low..high`; a single value is both ends, and an end
/// left out is empty.
fn range(text: &str) -> (&str, &str) {
    let text = text.trim();
    match text.split_once("..") {
        Some((low, high)) => (low.trim(), high.trim()),
        None => (text, text),
    }
}

/// A range of whole numbers; one with no lower end starts at `least`.
fn whole_range(name: &str, text: &str, least: i64) -> Result<(i64, i64), ServiceError> {
    let (low, high) = range(text);
    let end = |text: &str, open: i64| {
        if text.is_empty() {
            return Ok(open);
        }
        text.parse::<u32>().map(i64::from).map_err(|_| {
            bad_request(format!(
                "{text:?} is not a whole number for {name}: give one, or a range like 2..4"
            ))
        })
    };
    Ok((end(low, least)?, end(high, OPEN_HIGH)?))
}

fn bad_request(message: String) -> ServiceError {
    ServiceError::new(codes::BAD_REQUEST, message)
}

fn analysed(index: &rbl_index::Library, row: Row) -> bool {
    index.analysed.get(row as usize).copied().unwrap_or(0) != 0
}

pub(crate) fn record(index: &rbl_index::Library, row: Row) -> TrackRecord {
    let at = row as usize;
    let text = |column: &rbl_index::strings::StrColumn| column.get(at).to_owned();
    let bpm_x100 = index.bpm_x100.get(at).copied().unwrap_or(0);
    let rating = index.rating.get(at).copied().unwrap_or(0);
    let colour_id = index.color.get(at).copied().unwrap_or(0);
    let colour = usize::from(colour_id)
        .checked_sub(1)
        .and_then(|at| COLOR_NAMES.get(at))
        .map(|name| (*name).to_owned());
    let carried = index.my_tag_keys(at);
    let my_tags = index
        .my_tags()
        .iter()
        .flat_map(|category| {
            category
                .tags
                .iter()
                .zip(&category.ids)
                .filter(|(_, id)| carried.contains(id))
                .map(|(tag, id)| TrackTag {
                    id: id.to_string(),
                    category: category.name.clone(),
                    tag: tag.clone(),
                })
        })
        .collect();
    let containing = index.playlists_containing(row);
    let tree = index.playlists();
    let playlists = containing
        .into_iter()
        .map(|at| TrackPlaylist {
            id: tree.ids.get(at).copied().unwrap_or(0).to_string(),
            name: tree.name(at).to_owned(),
        })
        .collect();
    let year = index.year.get(at).copied().unwrap_or(0);
    let date_added = text(&index.date_added);
    TrackRecord {
        id: index.ids.get(at).copied().unwrap_or(0).to_string(),
        title: text(&index.title),
        artist: index.artist_name(row).to_owned(),
        album: index.album_name(row).to_owned(),
        genre: index.genre_name(row).to_owned(),
        label: index.label_name(row).to_owned(),
        year: (year > 0).then_some(year),
        bpm: (bpm_x100 > 0).then(|| f64::from(bpm_x100) / 100.0),
        key: index.key_name(row).to_owned(),
        energy: (rating > 0).then_some(rating),
        rating,
        prep_colour: colour.clone(),
        colour,
        colour_id,
        comment: text(&index.comment),
        duration_seconds: index.length_sec.get(at).copied().unwrap_or(0),
        date_added: (!date_added.is_empty()).then_some(date_added),
        play_count: index.play_count.get(at).copied().unwrap_or(0),
        location: text(&index.folder_path),
        file_name: text(&index.file_name),
        analysed: analysed(index, row),
        my_tags,
        playlists,
    }
}
