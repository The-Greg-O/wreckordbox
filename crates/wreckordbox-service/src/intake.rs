//! Intake and Problems: the two queues the DJ prepares tracks from.
//!
//! Neither is a playlist. A track is waiting for Intake while it has no
//! Prep colour, and it is in Problems while its Prep colour is the one the
//! DJ uses for Problem. Both are answered from the Library core's index, in
//! the collection's order, a page at a time like `tracks.query`.
#![allow(
    clippy::doc_markdown,
    reason = "MyTag is the glossary's word for the thing, not a code item"
)]

use std::collections::HashSet;

use rbl_index::{Row, SortColumn, TrackFilter, TrackSource, ViewSpec, COLOR_NAMES};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::library::OpenLibrary;
use crate::protocol::{codes as service_codes, LibraryChanged, Request, ServiceError};
use crate::stems::Stems;
use crate::tracks::{TrackRecord, DEFAULT_PAGE_LIMIT};

/// The names of the Intake commands.
pub mod commands {
    /// The tracks with no Prep colour. Takes a [`Page`](super::Page),
    /// answers a [`Queue`](super::Queue).
    pub const INTAKE_LIST: &str = "intake.list";
    /// The tracks whose Prep colour is the one the DJ uses for Problem.
    /// Takes a [`ProblemsPage`](super::ProblemsPage), answers a
    /// [`Queue`](super::Queue).
    pub const INTAKE_PROBLEMS: &str = "intake.problems";
    /// How many tracks wait in each queue, without listing them. Takes a
    /// [`CountsOf`](super::CountsOf), answers a [`Counts`](super::Counts).
    pub const INTAKE_COUNTS: &str = "intake.counts";
}

/// The error codes of the Intake commands, beside the service's own.
pub mod codes {
    /// No colour label in the Library carries the name the DJ uses for
    /// Problem.
    pub const PROBLEM_COLOUR_NOT_FOUND: &str = "problem_colour_not_found";
    /// More than one colour label carries that name.
    pub const PROBLEM_COLOUR_AMBIGUOUS: &str = "problem_colour_ambiguous";
}

/// The name the DJ gives the colour label that marks a Problem, when
/// neither the command nor [`PROBLEM_LABEL_ENV`] says otherwise.
pub const DEFAULT_PROBLEM_LABEL: &str = "Problem";
/// Names the colour label that marks a Problem, for a DJ who calls it
/// something else. Read by the service on each command.
pub const PROBLEM_LABEL_ENV: &str = "WRECKORDBOX_PROBLEM_LABEL";

/// Which part of a queue to answer. Both may be left out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Page {
    /// The most tracks to return; [`DEFAULT_PAGE_LIMIT`] when left out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// How many of the queue's tracks to skip: a page's `next_offset`
    /// fetches the page after it.
    pub offset: usize,
}

/// The arguments of `intake.problems`: a [`Page`], and optionally the name
/// of the colour label that marks a Problem.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProblemsPage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    pub offset: usize,
    /// The colour label's name as the DJ wrote it in rekordbox, in any
    /// case. Left out, it is [`PROBLEM_LABEL_ENV`]'s, else
    /// [`DEFAULT_PROBLEM_LABEL`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// The arguments of `intake.counts`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CountsOf {
    /// The name of the colour label that marks a Problem, as for
    /// `intake.problems`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// How many tracks wait in each queue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Counts {
    /// Tracks with no Prep colour.
    pub intake: usize,
    /// Tracks marked Problem; `null` when no colour label says which Prep
    /// colour that is, and `problems_unavailable` then says why.
    pub problems: Option<usize>,
    /// The DJ's Problem colour; `null` with `problems`.
    pub problem_colour: Option<PrepColour>,
    /// Why Problems cannot be counted: the error `intake.problems` would
    /// answer. Left out when it can.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problems_unavailable: Option<ServiceError>,
}

/// A colour label of the Library: one of rekordbox's eight colours, and
/// the name the DJ gave it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrepColour {
    /// The colour label as the Library numbers it, 1 to 8.
    pub id: u8,
    /// The colour itself, as `tracks.get` names it: `Red`.
    pub colour: String,
    /// What the DJ calls it: `Problem`. The colour's own name when the DJ
    /// has not renamed it.
    pub label: String,
}

/// One page of a queue, paged as `tracks.query` pages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Queue {
    /// In the collection's own order.
    pub tracks: Vec<QueuedTrack>,
    /// How many tracks are in the queue, on every page together.
    pub total: usize,
    /// How many of the queue's tracks come before this page.
    pub offset: usize,
    /// The most tracks this page could hold.
    pub limit: usize,
    /// The `offset` that fetches the next page; `null` on the last one.
    pub next_offset: Option<usize>,
    /// The Prep colour every track of the queue has: `null` for Intake,
    /// the DJ's Problem colour for Problems.
    pub prep_colour: Option<PrepColour>,
}

/// A track in a queue: its full record, as `tracks.get` gives it, and
/// what it still needs before it is prepared.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueuedTrack {
    #[serde(flatten)]
    pub track: TrackRecord,
    /// What is still empty, by the stable names in [`needs`], always in
    /// that module's order. It informs; nothing is refused because of it.
    pub needs: Vec<String>,
    /// The dimensions reaching the deck in which the track has no tag at
    /// all; the detail of [`needs::TAGS`].
    pub empty_dimensions: Vec<Dimension>,
    /// Where the beat grid stands; the detail of [`needs::GRID`].
    pub grid: GridState,
}

/// The stable names of what a queued track can still need.
pub mod needs {
    /// No Energy: the track has no star rating.
    pub const ENERGY: &str = "energy";
    /// No tag in at least one dimension that reaches the deck; which ones
    /// is the track's `empty_dimensions`.
    pub const TAGS: &str = "tags";
    /// No hot cue.
    pub const HOT_CUES: &str = "hot_cues";
    /// The beat grid is not confirmed; the track's `grid` says how far it
    /// got.
    pub const GRID: &str = "grid";
}

/// A dimension of the DJ's tags that reaches the deck. Today that is a
/// MyTag category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dimension {
    /// The MyTag category's identifier.
    pub id: String,
    pub name: String,
    /// What the dimension is in the Library: `mytag_category`.
    pub kind: String,
    /// The tags of the dimension, as a track's MyTags are keyed.
    #[serde(skip)]
    tags: Vec<i32>,
}

/// Where a track's beat grid stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GridState {
    /// The track has not been analysed, so it has no grid.
    NotAnalysed,
    /// Analysed, and nothing in the Library says the DJ has checked the
    /// grid.
    NotConfirmed,
    /// The DJ has locked the track's analysis (rekordbox's Analysis Lock),
    /// which is how the Library records that a grid is to be left as it is.
    Confirmed,
}

pub(crate) fn answers(command: &str) -> bool {
    matches!(
        command,
        commands::INTAKE_LIST | commands::INTAKE_PROBLEMS | commands::INTAKE_COUNTS
    )
}

/// Answers one of this module's commands, and what to tell every client
/// when it changed the Library.
pub(crate) fn answer(
    library: &OpenLibrary,
    _stems: &Stems,
    request: &Request,
) -> Result<(Value, Option<LibraryChanged>), ServiceError> {
    let index = library.index()?;
    let list = |rows: &[Row], page: &Page, colour: Option<PrepColour>| {
        let mut queue = queue(rows, page, colour);
        let on_page = rows
            .get(queue.offset..queue.offset + page_len(rows.len(), &queue))
            .unwrap_or(&[]);
        let known = Known::read(library, &index, on_page)?;
        queue.tracks = on_page
            .iter()
            .map(|&row| known.describe(&index, row))
            .collect();
        to_value(&queue)
    };
    let answer = match request.command.as_str() {
        commands::INTAKE_LIST => {
            let page: Page = arguments(request)?;
            list(&rows_coloured(&index, NO_COLOUR), &page, None)?
        }
        commands::INTAKE_COUNTS => {
            let wanted: CountsOf = arguments(request)?;
            let intake = rows_coloured(&index, NO_COLOUR).len();
            // The tab for Intake still needs its number when nothing says
            // which colour means Problem.
            let counts = match problem_colour(&labels(library)?, wanted.label.as_deref()) {
                Ok(problem) => Counts {
                    intake,
                    problems: Some(rows_coloured(&index, problem.id).len()),
                    problem_colour: Some(problem),
                    problems_unavailable: None,
                },
                Err(why) => Counts {
                    intake,
                    problems: None,
                    problem_colour: None,
                    problems_unavailable: Some(why),
                },
            };
            to_value(&counts)?
        }
        _ => {
            let wanted: ProblemsPage = arguments(request)?;
            let problem = problem_colour(&labels(library)?, wanted.label.as_deref())?;
            let page = Page {
                limit: wanted.limit,
                offset: wanted.offset,
            };
            list(&rows_coloured(&index, problem.id), &page, Some(problem))?
        }
    };
    Ok((answer, None))
}

/// The Library's colour labels with the names the DJ gave them
/// (`djmdColor`, whose `Commnt` is the name rekordbox shows and lets the DJ
/// change). The window does not read these: it names the eight colours
/// itself ([`COLOR_NAMES`]).
fn labels(library: &OpenLibrary) -> Result<Vec<PrepColour>, ServiceError> {
    library.with_db(|db| {
        let read = || -> rusqlite::Result<Vec<(String, String)>> {
            db.connection()
                .prepare(
                    "SELECT ID, COALESCE(Commnt, '') FROM djmdColor
                     WHERE COALESCE(rb_local_deleted, 0) = 0 ORDER BY SortKey, ID",
                )?
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        };
        let rows = read().map_err(|e| {
            ServiceError::new(
                service_codes::SERVICE_FAILED,
                format!("the Library's colour labels could not be read: {e}"),
            )
        })?;
        Ok(rows
            .into_iter()
            .filter_map(|(id, name)| {
                let id = id.trim().parse::<u8>().ok()?;
                let colour = *COLOR_NAMES.get(usize::from(id).checked_sub(1)?)?;
                let name = name.trim();
                Some(PrepColour {
                    id,
                    colour: colour.to_owned(),
                    label: if name.is_empty() { colour } else { name }.to_owned(),
                })
            })
            .collect())
    })
}

/// The name the DJ uses for Problem: the command's, else the
/// environment's, else [`DEFAULT_PROBLEM_LABEL`].
fn problem_label(asked: Option<&str>) -> String {
    asked
        .map(str::to_owned)
        .or_else(|| std::env::var(PROBLEM_LABEL_ENV).ok())
        .map(|label| label.trim().to_owned())
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| DEFAULT_PROBLEM_LABEL.to_owned())
}

/// The colour label the DJ uses for Problem: the one whose name is the
/// Problem label, whatever its case. Never a particular colour.
fn problem_colour(labels: &[PrepColour], asked: Option<&str>) -> Result<PrepColour, ServiceError> {
    let wanted = problem_label(asked);
    let named: Vec<&PrepColour> = labels
        .iter()
        .filter(|colour| colour.label.to_lowercase() == wanted.to_lowercase())
        .collect();
    match named.as_slice() {
        [one] => Ok((*one).clone()),
        [] => {
            let there: Vec<String> = labels
                .iter()
                .map(|colour| format!("{} ({})", colour.label, colour.colour))
                .collect();
            Err(ServiceError::new(
                codes::PROBLEM_COLOUR_NOT_FOUND,
                format!(
                    "no colour label in the Library is named {wanted:?}, so nothing says which Prep colour means Problem. {} Name one {wanted:?} in rekordbox's preferences, or say what you call it with --label or {PROBLEM_LABEL_ENV}.",
                    if there.is_empty() {
                        "The Library names no colour labels.".to_owned()
                    } else {
                        format!("The colour labels are: {}.", there.join(", "))
                    }
                ),
            ))
        }
        several => Err(ServiceError::new(
            codes::PROBLEM_COLOUR_AMBIGUOUS,
            format!(
                "{} colour labels are named {wanted:?} ({}); only one can mean Problem",
                several.len(),
                several
                    .iter()
                    .map(|colour| colour.colour.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    }
}

/// The colour label of a track with no Prep colour.
const NO_COLOUR: u8 = 0;

/// The collection's tracks carrying the colour label `colour`, as the
/// window's Track Filter picks them.
fn rows_coloured(index: &std::sync::Arc<rbl_index::Library>, colour: u8) -> Vec<Row> {
    index
        .open_view(&ViewSpec {
            source: TrackSource::Collection,
            sort: SortColumn::TrackNo,
            descending: false,
            query: String::new(),
            filter: TrackFilter {
                colors: Some(vec![colour]),
                ..TrackFilter::default()
            },
        })
        .rows
}

/// The page of `rows` asked for, with its tracks still to be filled in.
fn queue(rows: &[Row], page: &Page, prep_colour: Option<PrepColour>) -> Queue {
    let limit = page.limit.unwrap_or(DEFAULT_PAGE_LIMIT);
    let total = rows.len();
    let offset = page.offset.min(total);
    let end = offset.saturating_add(limit).min(total);
    Queue {
        tracks: Vec::new(),
        total,
        offset,
        limit,
        next_offset: (end < total && end > offset).then_some(end),
        prep_colour,
    }
}

/// How many tracks the page holds.
fn page_len(total: usize, queue: &Queue) -> usize {
    queue.limit.min(total.saturating_sub(queue.offset))
}

/// What is read once for a page, beside the index, to say what each of its
/// tracks still needs.
struct Known {
    dimensions: Vec<Dimension>,
    /// The identifiers of the page's tracks whose analysis is locked.
    locked: HashSet<String>,
}

impl Known {
    fn read(
        library: &OpenLibrary,
        index: &rbl_index::Library,
        on_page: &[Row],
    ) -> Result<Self, ServiceError> {
        let ids: Vec<String> = on_page
            .iter()
            .filter_map(|&row| index.ids.get(row as usize).map(u64::to_string))
            .collect();
        let (my_tags, locked) = library.with_db(|db| {
            let failed = |e: &dyn std::fmt::Display| {
                ServiceError::new(
                    service_codes::SERVICE_FAILED,
                    format!("the Library could not be read: {e}"),
                )
            };
            let my_tags =
                rbl_db::export_info::my_tags(db.connection()).map_err(|e| failed(&e))?;
            let locked = locked(db.connection(), &ids).map_err(|e| failed(&e))?;
            Ok((my_tags, locked))
        })?;
        Ok(Self {
            dimensions: dimensions_reaching_the_deck(&my_tags),
            locked,
        })
    }

    fn describe(&self, index: &rbl_index::Library, row: Row) -> QueuedTrack {
        let track = crate::tracks::record(index, row);
        let carried = index.my_tag_keys(row as usize);
        let empty_dimensions: Vec<Dimension> = self
            .dimensions
            .iter()
            .filter(|dimension| !dimension.tags.iter().any(|tag| carried.contains(tag)))
            .cloned()
            .collect();
        let grid = if !track.analysed {
            GridState::NotAnalysed
        } else if self.locked.contains(&track.id) {
            GridState::Confirmed
        } else {
            GridState::NotConfirmed
        };
        let mut needs = Vec::new();
        if track.energy.is_none() {
            needs.push(needs::ENERGY);
        }
        if !empty_dimensions.is_empty() {
            needs.push(needs::TAGS);
        }
        if index.cues().hot_letters_of(row).is_empty() {
            needs.push(needs::HOT_CUES);
        }
        if grid != GridState::Confirmed {
            needs.push(needs::GRID);
        }
        QueuedTrack {
            track,
            needs: needs.into_iter().map(str::to_owned).collect(),
            empty_dimensions,
            grid,
        }
    }
}

/// The dimensions of the DJ's tags that reach the deck, in the order the
/// deck lists them. The one place that decides it.
///
/// Today a dimension reaches the deck exactly when it is a MyTag category
/// (the Track Filter reads MyTags and nothing else of the kind), so these
/// are the Library's MyTag categories. A category holding no tags is left
/// out: there is nothing in it a track could be given. When the Tag schema
/// says which of its dimensions are projected to the deck, this is what
/// changes.
fn dimensions_reaching_the_deck(my_tags: &[rbl_db::export_info::MyTagRow]) -> Vec<Dimension> {
    let catalogue = rbl_library::read::my_tag_catalogue(my_tags);
    catalogue
        .categories
        .iter()
        .map(|category| Dimension {
            id: category.id.clone(),
            name: category.name.clone(),
            kind: "mytag_category".to_owned(),
            tags: catalogue
                .tags
                .iter()
                .filter(|tag| tag.parent == category.id)
                .map(|tag| rbl_index::smart::my_tag_key(&tag.id))
                .collect(),
        })
        .filter(|dimension| !dimension.tags.is_empty())
        .collect()
}

/// Which of `ids` have their analysis locked: rekordbox's Analysis Lock,
/// bit 7 of `djmdContent.Analysed`, the bit the window's grid lock sets.
/// The index keeps only whether a track is analysed at all.
fn locked(
    connection: &rusqlite::Connection,
    ids: &[String],
) -> rusqlite::Result<HashSet<String>> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let marks = vec!["?"; ids.len()].join(", ");
    connection
        .prepare(&format!(
            "SELECT ID FROM djmdContent
             WHERE ID IN ({marks}) AND (COALESCE(Analysed, 0) & 128) != 0"
        ))?
        .query_map(rusqlite::params_from_iter(ids), |row| row.get(0))?
        .collect()
}

fn arguments<T: serde::de::DeserializeOwned + Default>(
    request: &Request,
) -> Result<T, ServiceError> {
    if request.args.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(request.args.clone()).map_err(|e| {
        ServiceError::new(
            service_codes::BAD_REQUEST,
            format!("bad arguments to {}: {e}", request.command),
        )
    })
}

fn to_value<T: Serialize>(value: &T) -> Result<Value, ServiceError> {
    serde_json::to_value(value)
        .map_err(|e| ServiceError::new(service_codes::SERVICE_FAILED, e.to_string()))
}
