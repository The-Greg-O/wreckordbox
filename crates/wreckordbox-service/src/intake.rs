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

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rbl_index::{Row, SortColumn, TrackFilter, TrackSource, ViewSpec, COLOR_NAMES};
use rbl_library::history::{LibraryEdit, TRACK_EDIT};
use rbl_library::Touched;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wreckordbox_sidecar::{Fact, FactsSnapshot};
pub use wreckordbox_sidecar::{Source, SourceKind};

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
    /// Sets or clears one track's Prep colour. Takes a
    /// [`SetPrepColour`](super::SetPrepColour), answers a
    /// [`PrepColourSet`](super::PrepColourSet). The only command here that
    /// changes the Library.
    pub const TRACKS_SET_PREP_COLOUR: &str = "tracks.set_prep_colour";
}

/// The error codes of the Intake commands, beside the service's own.
pub mod codes {
    /// No colour label in the Library carries the name the DJ uses for
    /// Problem.
    pub const PROBLEM_COLOUR_NOT_FOUND: &str = "problem_colour_not_found";
    /// More than one colour label carries that name.
    pub const PROBLEM_COLOUR_AMBIGUOUS: &str = "problem_colour_ambiguous";
    /// The Prep colour asked for is neither a name the DJ gave a colour
    /// label, nor a colour, nor `none`; or it names two different colours.
    pub const PREP_COLOUR_NOT_FOUND: &str = "prep_colour_not_found";
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

/// The arguments of `tracks.set_prep_colour`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetPrepColour {
    /// The track's identifier.
    pub id: String,
    /// The Prep colour, in any case: the DJ's name for a colour label
    /// (`Clean`), the colour itself (`Green`), or `none` to clear it.
    pub prep_colour: String,
}

/// What `tracks.set_prep_colour` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrepColourSet {
    pub id: String,
    pub title: String,
    /// The track's Prep colour now, read back; `null` for none.
    pub prep_colour: Option<PrepColour>,
    /// What it was before.
    pub previous: Option<PrepColour>,
    /// Whether the track was not already in that Prep colour.
    pub changed: bool,
    /// The Library's generation after the edit, as the change event
    /// carries it.
    pub generation: u32,
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
    /// Why the page's proposed titles and artists could not be read (the
    /// Sidecar is busy, unreadable, or at a schema only the service's own
    /// opening may change); absent when they were. The page is still right
    /// in every other respect; its `suggestions` are then empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestions_unavailable: Option<String>,
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
    /// The waiting Facts that flag the title or the artist; the detail of
    /// [`needs::TITLE`] and [`needs::ARTIST`]. Newest first.
    pub suggestions: Vec<Suggestion>,
}

/// A title or an artist proposed for a track by a waiting Fact.
///
/// A Fact is waiting when it is live (its source has not replaced it), its
/// source is not the DJ, the DJ has recorded no Fact of the same kind for
/// the track since, and its value is not what the Library already holds.
/// Accepting and rejecting Facts do not exist yet; when they do, they
/// decide this instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    /// The field it proposes a value for: `title` or `artist`. Also the
    /// kind of the Fact.
    pub field: String,
    /// The Fact, as `facts.of_track` numbers it.
    pub fact_id: i64,
    /// The proposed text.
    pub value: String,
    pub source: Source,
    pub confidence: f64,
}

/// The stable names of what a queued track can still need.
pub mod needs {
    /// A waiting Fact proposes another title.
    pub const TITLE: &str = "title";
    /// A waiting Fact proposes another artist.
    pub const ARTIST: &str = "artist";
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
        commands::INTAKE_LIST
            | commands::INTAKE_PROBLEMS
            | commands::INTAKE_COUNTS
            | commands::TRACKS_SET_PREP_COLOUR
    )
}

/// Answers one of this module's commands, and what to tell every client
/// when it changed the Library.
pub(crate) fn answer(
    library: &OpenLibrary,
    stems: &Stems,
    request: &Request,
) -> Result<(Value, Option<LibraryChanged>), ServiceError> {
    if request.command == commands::TRACKS_SET_PREP_COLOUR {
        let (set, changed) = set_prep_colour(library, &required_arguments(request)?)?;
        return Ok((to_value(&set)?, Some(changed)));
    }
    let index = library.index()?;
    let list = |rows: &[Row], page: &Page, colour: Option<PrepColour>| {
        let mut queue = queue(rows, page, colour);
        let on_page = rows
            .get(queue.offset..queue.offset + page_len(rows.len(), &queue))
            .unwrap_or(&[]);
        let known = Known::read(library, stems, &index, on_page)?;
        queue.suggestions_unavailable.clone_from(&known.facts_unavailable);
        queue.tracks = on_page
            .iter()
            .map(|&row| known.describe(&index, row))
            .collect::<Result<_, _>>()?;
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
        // Every colour is there to be named; one the Library does not
        // name goes by the colour's own name.
        Ok((1..=8_u8)
            .zip(COLOR_NAMES)
            .map(|(id, colour)| {
                let name = rows
                    .iter()
                    .find(|(row, _)| row.trim().parse::<u8>() == Ok(id))
                    .map_or("", |(_, name)| name.trim());
                PrepColour {
                    id,
                    colour: colour.to_owned(),
                    label: if name.is_empty() { colour } else { name }.to_owned(),
                }
            })
            .collect())
    })
}

/// Sets one track's Prep colour through the Library's single edit path:
/// the write the window's colour command makes (`set_color_with_undo`),
/// recorded in the edit history as the window records it.
fn set_prep_colour(
    library: &OpenLibrary,
    edit: &SetPrepColour,
) -> Result<(PrepColourSet, LibraryChanged), ServiceError> {
    let labels = labels(library)?;
    let wanted = named_colour(&labels, &edit.prep_colour)?;
    let before = library.index()?;
    let Some(row) = before.row_of(&edit.id) else {
        return Err(ServiceError::new(
            service_codes::TRACK_NOT_FOUND,
            format!("there is no track {} in the Library", edit.id),
        ));
    };
    let colour_of = |index: &rbl_index::Library, row: Row| {
        let id = index.color.get(row as usize).copied().unwrap_or(NO_COLOUR);
        labels.iter().find(|colour| colour.id == id).cloned()
    };
    let previous = colour_of(&before, row);
    let id = before
        .ids
        .get(row as usize)
        .map_or_else(|| edit.id.clone(), u64::to_string);
    let title = before.title.get(row as usize).to_owned();
    let colour = wanted.as_ref().map(|colour| colour.id.to_string());
    let (rows, change) =
        library.recorded_edit(Touched::Metadata(vec![id.clone()]), TRACK_EDIT, |writer| {
            let (changed, recorded) = writer.set_color_with_undo(&id, colour.as_deref())?;
            Ok((changed.rows, LibraryEdit::Track(vec![recorded])))
        })?;
    // Read back from the index the edit refreshed, not assumed.
    let after = library.index()?;
    let prep_colour = after
        .row_of(&id)
        .map_or(wanted, |row| colour_of(&after, row));
    Ok((
        PrepColourSet {
            id,
            title,
            prep_colour,
            previous,
            changed: rows > 0,
            generation: change.generation,
        },
        change,
    ))
}

/// The colour label `text` names: by the DJ's name for it or by the colour
/// itself, in any case. `None` for `none`.
fn named_colour(labels: &[PrepColour], text: &str) -> Result<Option<PrepColour>, ServiceError> {
    let wanted = text.trim().to_lowercase();
    if wanted == "none" {
        return Ok(None);
    }
    let named: Vec<&PrepColour> = labels
        .iter()
        .filter(|colour| {
            colour.label.to_lowercase() == wanted || colour.colour.to_lowercase() == wanted
        })
        .collect();
    let choices = || {
        labels
            .iter()
            .map(|colour| {
                if colour.label == colour.colour {
                    colour.colour.clone()
                } else {
                    format!("{} ({})", colour.label, colour.colour)
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    match named.as_slice() {
        [one] => Ok(Some((*one).clone())),
        [] => Err(ServiceError::new(
            codes::PREP_COLOUR_NOT_FOUND,
            format!(
                "{text:?} is not a Prep colour: it is one of {}, or none",
                choices()
            ),
        )),
        several => Err(ServiceError::new(
            codes::PREP_COLOUR_NOT_FOUND,
            format!(
                "{text:?} names {} Prep colours, one by its label and another by its colour ({}); say which by the other name",
                several.len(),
                several
                    .iter()
                    .map(|colour| format!("{} ({})", colour.label, colour.colour))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    }
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
                    "no colour label in the Library is named {wanted:?}, so nothing says which Prep colour means Problem. The colour labels are: {}. Name one {wanted:?} in rekordbox's preferences, or say what you call it with --label or {PROBLEM_LABEL_ENV}.",
                    there.join(", ")
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
fn rows_coloured(index: &Arc<rbl_index::Library>, colour: u8) -> Vec<Row> {
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
        suggestions_unavailable: None,
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
    /// The page's `title` and `artist` Facts, by track and kind, newest
    /// first: where waiting proposals are.
    facts: HashMap<(String, &'static str), Vec<Fact>>,
    /// Why those could not be read, when they could not.
    facts_unavailable: Option<String>,
}

impl Known {
    fn read(
        library: &OpenLibrary,
        stems: &Stems,
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
            let my_tags = rbl_db::export_info::my_tags(db.connection()).map_err(|e| failed(&e))?;
            let locked = locked(db.connection(), &ids).map_err(|e| failed(&e))?;
            Ok((my_tags, locked))
        })?;
        let (facts, facts_unavailable) = match page_facts(stems, &ids) {
            Ok(facts) => (facts, None),
            Err(why) => (HashMap::new(), Some(why)),
        };
        Ok(Self {
            dimensions: dimensions_reaching_the_deck(&my_tags),
            locked,
            facts,
            facts_unavailable,
        })
    }

    fn describe(&self, index: &rbl_index::Library, row: Row) -> Result<QueuedTrack, ServiceError> {
        let track = crate::tracks::record(index, row);
        let mut suggestions = Vec::new();
        for (field, held) in [(needs::TITLE, &track.title), (needs::ARTIST, &track.artist)] {
            if let Some(facts) = self.facts.get(&(track.id.clone(), field)) {
                suggestions.extend(waiting(facts, field, held));
            }
        }
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
        for field in [needs::TITLE, needs::ARTIST] {
            if suggestions
                .iter()
                .any(|suggestion| suggestion.field == field)
            {
                needs.push(field);
            }
        }
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
        Ok(QueuedTrack {
            track,
            needs: needs.into_iter().map(str::to_owned).collect(),
            empty_dimensions,
            grid,
            suggestions,
        })
    }
}

/// The `title` and `artist` Facts of the page's tracks, read without ever
/// being what creates, migrates or tidies a Sidecar (wbx-62 review): through
/// the service's writer when it is already open, else through a read-only
/// snapshot of the file, with the Sidecar slot held throughout so the writer
/// cannot open under the read. `Err` says why they could not be read; the
/// queue is still listed without them.
fn page_facts(
    stems: &Stems,
    ids: &[String],
) -> Result<HashMap<(String, &'static str), Vec<Fact>>, String> {
    let read = |facts: &dyn Fn(&str, &str) -> Result<Vec<Fact>, String>| {
        let mut out = HashMap::new();
        for id in ids {
            for field in [needs::TITLE, needs::ARTIST] {
                let found = facts(id, field)?;
                if !found.is_empty() {
                    out.insert((id.clone(), field), found);
                }
            }
        }
        Ok(out)
    };
    stems
        .with_current_sidecar(|open| match open {
            Some(sidecar) => read(&|id, kind| sidecar.facts(id, kind).map_err(|e| e.to_string())),
            None => match FactsSnapshot::open(stems.sidecar_dir()) {
                Ok(FactsSnapshot::Absent) => Ok(HashMap::new()),
                Ok(FactsSnapshot::Unknown(why)) => Err(why),
                Ok(FactsSnapshot::Open(reader)) => {
                    read(&|id, kind| reader.facts(id, kind).map_err(|e| e.to_string()))
                }
                Err(error) => Err(format!("the Sidecar could not be read: {error}")),
            },
        })
        .unwrap_or_else(|| Err("the Sidecar is being opened; try again".to_owned()))
}

/// The Facts among `facts` (one track's, of the kind `field`, newest
/// first) that are waiting for the DJ and propose something other than
/// `held`, the text the Library has. See [`Suggestion`] for the rule.
fn waiting(facts: &[Fact], field: &str, held: &str) -> Vec<Suggestion> {
    // Facts are numbered in the order they were recorded.
    let decided = facts
        .iter()
        .filter(|fact| fact.is_live() && fact.source.kind == SourceKind::Dj)
        .map(|fact| fact.id)
        .max();
    facts
        .iter()
        .filter(|fact| fact.is_live() && fact.source.kind != SourceKind::Dj)
        .filter(|fact| decided.is_none_or(|decision| fact.id > decision))
        .filter_map(|fact| {
            let value = fact.value.as_str()?.trim();
            (!value.is_empty() && value != held.trim()).then(|| Suggestion {
                field: field.to_owned(),
                fact_id: fact.id,
                value: value.to_owned(),
                source: fact.source.clone(),
                confidence: fact.confidence,
            })
        })
        .collect()
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
fn locked(connection: &rusqlite::Connection, ids: &[String]) -> rusqlite::Result<HashSet<String>> {
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

fn required_arguments<T: serde::de::DeserializeOwned>(
    request: &Request,
) -> Result<T, ServiceError> {
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
