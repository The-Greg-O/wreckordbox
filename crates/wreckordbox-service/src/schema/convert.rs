//! `schema.convert`: a first Tag schema from the MyTags the Library has,
//! and every tag on a track as a Fact from the DJ.
//!
//! The conversion reads the Library and writes only to the Sidecar's
//! folder: the Tag schema file and the Facts. What it works out:
//!
//! - **Which category is the genre tree**: the one most of whose tags are
//!   written as children, with a leading `>`; [`Convert::tree`] says
//!   otherwise. The others become dimensions as they are.
//! - **The tree**, from the list's order: a tag with no `>` starts a group
//!   and is the parent of the `>` tags that follow it. `> >` is a child of
//!   the child before it. Spacing around the `>` is not significant.
//! - **Names**: a term's name is the tag's text without its `>` prefix. A
//!   child whose name ends in a short marker in brackets, `(T)`, loses the
//!   marker when the same name stands under another parent, which is what
//!   such a marker was written for. Whenever the deck view's rule would not
//!   give the tag's exact text back, the term keeps that text as its `deck`.
//! - **Odd entries** (a child before any parent, a tag with no letter or
//!   digit, a divider) stay terms, with a remark in the file, and are
//!   reported.
//! - **Facts**: one per track and dimension, of the dimension's kind, its
//!   value the identifiers of the terms the track's tags are. In the tree a
//!   child implies its parents, so a parent's tag on a track that also
//!   carries one of its children is not recorded; whether the deck shows
//!   such parents is the schema's `tag_parents`, set from what most tracks
//!   have today.
//!
//! Running it again records only what has changed, and a Tag schema that is
//! not the one the conversion would write is left alone unless
//! [`Convert::replace`] is set.

#![allow(
    clippy::doc_markdown,
    reason = "MyTag is rekordbox's word for the thing, written as a word, not code"
)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use rbl_index::smart::my_tag_key;
use rbl_library::read::my_tag_catalogue;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wreckordbox_sidecar::tag_schema::{
    is_dimension_name, is_term_id, Deck, DeckCategory, Dimension, Term, DEFAULT_CHILD_PREFIX, GENRE,
};
use wreckordbox_sidecar::{
    Declaration, Kinds, NewFact, Run, RunId, Sidecar, Source, SourceKind, TagSchema, ValueType,
};

use super::{codes, declared_kinds, file, sidecar_error};
use crate::library::OpenLibrary;
use crate::protocol::{codes as service_codes, ServiceError};
use crate::server::open_error;
use crate::stems::{self, SidecarChanged, Stems};

/// What marks a tag as a child in a flat list.
const CHILD_MARK: char = '>';

/// What the Run of a conversion is called.
pub const RUN_DESCRIPTION: &str = "Convert the Library's MyTags to Facts";

/// The arguments of `schema.convert`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Convert {
    /// The MyTag category that is the genre tree, by identifier or name, or
    /// `none` for no tree. Left out: the category whose tags are written as
    /// children.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<String>,
    /// What a category's dimension is called, by the category's identifier
    /// or name. A dimension's name is its kind of Fact and cannot be
    /// changed later without leaving its Facts behind.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub names: BTreeMap<String, String>,
    /// Write the Tag schema even where there is another one already.
    #[serde(default)]
    pub replace: bool,
    /// Make the Library's MyTags the DJ's word again where a Fact from the
    /// DJ says otherwise: such Facts are superseded. Without it they are
    /// left as they are and reported.
    #[serde(default)]
    pub reimport: bool,
    /// Work everything out and answer it, the file's text included, but
    /// write no file and record no Fact.
    #[serde(default)]
    pub dry_run: bool,
}

/// What became of the Tag schema file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileState {
    /// There was none; it was written.
    Written,
    /// It was already exactly what the conversion writes.
    Unchanged,
    /// Another was there and was replaced, as asked.
    Replaced,
    /// A dry run: nothing was written.
    NotWritten,
}

/// One MyTag category and the dimension it became.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvertedCategory {
    pub category_id: String,
    /// The category's name, which is its label in the deck view.
    pub category: String,
    /// The dimension's name, which is also the kind of its Facts.
    pub dimension: String,
    /// The genre tree.
    pub tree: bool,
    pub terms: usize,
}

/// How the genre tree was found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tree {
    pub category: String,
    /// True when the command named it; false when the tags' prefixes did.
    pub told: bool,
    pub parents: usize,
    pub children: usize,
    /// What stands before a child's name on the deck.
    pub child_prefix: String,
}

/// A child whose hand-written marker was taken off its name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disambiguated {
    /// The term's identifier.
    pub id: String,
    /// Its name now.
    pub name: String,
    /// The identifier of its parent.
    pub parent: String,
    /// The tag's text, which the term keeps as its `deck`.
    pub tag: String,
}

/// A tag that does not look like a term. It is one all the same.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OddEntry {
    pub category: String,
    pub dimension: String,
    /// The term's identifier.
    pub id: String,
    /// The tag's text.
    pub tag: String,
    pub why: String,
    /// How many tracks carry it.
    pub tracks: usize,
}

/// Parents' tags on tracks that carry one of their children.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ImpliedParents {
    /// Times a track carries a child and its parent's tag too. The Fact
    /// names the child; the parent is implied.
    pub with_parent: usize,
    /// Times a track carries a child without its parent's tag.
    pub without_parent: usize,
    /// What the schema's deck view says of it: whether a track of a child
    /// carries its parents' MyTags.
    pub tag_parents: bool,
}

/// A Fact from the DJ that does not say what the Library's MyTags say.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conflict {
    pub track_id: String,
    /// The dimension, which is the Fact's kind.
    pub dimension: String,
    /// What the DJ's Fact says.
    pub fact: Value,
    /// What the track's MyTags say: the terms' identifiers, none when the
    /// track has no tag of the dimension.
    pub library: Value,
}

/// What `schema.convert` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Converted {
    pub file: PathBuf,
    pub file_state: FileState,
    /// In the deck's order.
    pub categories: Vec<ConvertedCategory>,
    /// `null` when no category is a tree.
    pub tree: Option<Tree>,
    pub disambiguated: Vec<Disambiguated>,
    pub odd: Vec<OddEntry>,
    pub implied_parents: ImpliedParents,
    /// How many tracks carry a MyTag.
    pub tracks: usize,
    /// How many tags on tracks there are; each is in a Fact, named or
    /// implied.
    pub track_tags: usize,
    /// Facts recorded by this command (in a dry run: that would be).
    pub facts_recorded: usize,
    /// Facts that were already there with the same value.
    pub facts_unchanged: usize,
    /// Facts from the DJ that say otherwise than the Library's MyTags and
    /// were left as they are: how many, and each. Recorded over only when
    /// the command says `reimport`.
    #[serde(default)]
    pub facts_conflicting: usize,
    #[serde(default)]
    pub conflicts: Vec<Conflict>,
    /// The Run the Facts were recorded in; `null` when none was.
    pub run: Option<Run>,
    /// The file's text, in a dry run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// One MyTag category as the Library has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Category {
    pub id: String,
    pub name: String,
    /// `(identifier, text)` in the category's order.
    pub tags: Vec<(String, String)>,
}

/// The Library's MyTags and which tracks carry which.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct MyTags {
    /// In the deck's order.
    pub categories: Vec<Category>,
    /// `(track identifier, the index keys of its tags)` for every track,
    /// in the collection's order.
    pub tracks: Vec<(String, Vec<i32>)>,
}

/// Reads the Library's MyTags. The only moment the conversion holds the
/// Library.
pub(crate) fn my_tags(library: &OpenLibrary) -> Result<MyTags, ServiceError> {
    let index = library.index()?;
    let rows = library
        .with_db(|db| rbl_db::export_info::my_tags(db.connection()).map_err(|e| open_error(&e)))?;
    let catalogue = my_tag_catalogue(&rows);
    let categories = catalogue
        .categories
        .iter()
        .map(|category| Category {
            id: category.id.clone(),
            name: category.name.clone(),
            tags: catalogue
                .tags_of(category)
                .map(|tag| (tag.id.clone(), tag.name.clone()))
                .collect(),
        })
        .collect();
    let tracks = (0..index.len())
        .map(|row| {
            (
                index.ids.get(row).copied().unwrap_or(0).to_string(),
                index.my_tag_keys(row).to_vec(),
            )
        })
        .collect();
    Ok(MyTags { categories, tracks })
}

/// The schema the Library's MyTags make, and what was found on the way.
struct Plan {
    schema: TagSchema,
    categories: Vec<ConvertedCategory>,
    tree: Option<Tree>,
    disambiguated: Vec<Disambiguated>,
    odd: Vec<OddEntry>,
    /// A tag's index key to its dimension (by position) and term.
    terms: HashMap<i32, (usize, String)>,
}

/// One conversion at a time in this process: what a conversion finds (the
/// schema there is, the Facts there are) is still so when it writes.
static CONVERTING: Mutex<()> = Mutex::new(());

/// Numbers the working files, so no two are ever the same one.
static WORKING_FILES: AtomicU64 = AtomicU64::new(0);

pub(super) fn convert(
    stems: &Stems,
    convert: &Convert,
    read: &dyn Fn() -> Result<MyTags, ServiceError>,
) -> Result<Converted, ServiceError> {
    let _one_at_a_time = CONVERTING.lock().unwrap_or_else(PoisonError::into_inner);
    let declared = declared_kinds(stems)?;
    let library = read()?;
    let mut plan = plan(&library, convert, &declared)?;
    let (wanted, implied, track_tags) = wanted(&plan, &library);
    plan.schema.deck.tag_parents = implied.tag_parents;

    // Everything that could be wrong is found before anything is written,
    // in a dry run as in a real one: the schema, then every Fact.
    let text = checked_text(&plan.schema, &library, &declared)?;
    let kinds = declared.with_schema(&plan.schema).map_err(invalid)?;
    for (_, at, value) in &wanted {
        let kind = kind_of(&kinds, &plan.schema, *at)?;
        kind.check(&json!(value))
            .map_err(|why| ServiceError::new(service_codes::SERVICE_FAILED, why))?;
    }

    let path = file(stems)?;
    let existing = existing(&path)?;
    let file_state = match &existing {
        _ if convert.dry_run => FileState::NotWritten,
        None => FileState::Written,
        Some(existing) if existing == text.as_bytes() => FileState::Unchanged,
        Some(_) if convert.replace => FileState::Replaced,
        Some(_) => {
            return Err(ServiceError::new(
                codes::TAG_SCHEMA_EXISTS,
                format!(
                    "there is already a Tag schema at {}, and it is not the one this conversion writes: it has been edited, or the Library's MyTags have changed. Nothing was written and no Fact was recorded. To write over it, ask again with --replace",
                    path.display()
                ),
            ));
        }
    };

    let sidecar = stems.sidecar()?;
    let sorted = sort_out(
        &sidecar,
        &kinds,
        &plan.schema,
        &library,
        &wanted,
        convert.reimport,
    )?;
    let mut converted = Converted {
        file: path.clone(),
        file_state,
        categories: plan.categories,
        tree: plan.tree,
        disambiguated: plan.disambiguated,
        odd: plan.odd,
        implied_parents: implied,
        tracks: library
            .tracks
            .iter()
            .filter(|(_, keys)| !keys.is_empty())
            .count(),
        track_tags,
        facts_recorded: sorted.to_record.len(),
        facts_unchanged: sorted.unchanged,
        facts_conflicting: sorted.conflicts.len(),
        conflicts: sorted.conflicts,
        run: None,
        text: None,
    };
    if convert.dry_run {
        converted.text = Some(text);
        return Ok(converted);
    }

    let installed = matches!(file_state, FileState::Written | FileState::Replaced);
    if installed {
        write_whole(&path, text.as_bytes())?;
    }
    if sorted.to_record.is_empty() {
        return Ok(converted);
    }
    match record(stems, &sidecar, &kinds, &plan.schema, &sorted.to_record) {
        Ok(run) => {
            converted.run = sidecar.run(run).map_err(|e| sidecar_error(&e))?;
            Ok(converted)
        }
        // The schema and its Facts go together: without the Facts, the
        // file is put back as it was.
        Err(error) if installed => {
            let file = match put_back(&path, existing.as_deref()) {
                Ok(()) if existing.is_some() => "the Tag schema is as it was".to_owned(),
                Ok(()) => "no Tag schema was written".to_owned(),
                Err(e) => format!(
                    "the Tag schema {} could not be put back as it was ({e}): it is the conversion's, without its Facts",
                    path.display()
                ),
            };
            Err(ServiceError::new(
                &error.code,
                format!(
                    "{}. No Fact was recorded, and {file}",
                    error.message.trim_end_matches('.')
                ),
            ))
        }
        Err(error) => Err(error),
    }
}

/// What a track's tags say of one dimension: the track, the dimension by
/// position, the terms' identifiers.
type Wanted<'a> = (&'a str, usize, Vec<String>);

/// What each track's tags say, dimension by dimension: `(track, dimension
/// by position, the terms' identifiers)`. With it, how parents put on by
/// hand stand, and how many tags on tracks there are.
fn wanted<'a>(plan: &Plan, library: &'a MyTags) -> (Vec<Wanted<'a>>, ImpliedParents, usize) {
    let mut implied = ImpliedParents::default();
    let mut track_tags = 0;
    let mut wanted = Vec::new();
    for (track, keys) in &library.tracks {
        let mut by_dimension: Vec<Vec<&str>> = vec![Vec::new(); plan.schema.dimensions.len()];
        for key in keys {
            if let Some((dimension, term)) = plan.terms.get(key) {
                by_dimension[*dimension].push(term);
                track_tags += 1;
            }
        }
        for (at, carried) in by_dimension.into_iter().enumerate() {
            if carried.is_empty() {
                continue;
            }
            let dimension = &plan.schema.dimensions[at];
            let value = most_specific(dimension, &carried, &mut implied);
            wanted.push((track.as_str(), at, value));
        }
    }
    implied.tag_parents = implied.with_parent >= implied.without_parent;
    (wanted, implied, track_tags)
}

/// The kind of the Facts of the schema's dimension at `at`.
fn kind_of<'a>(
    kinds: &'a Kinds,
    schema: &TagSchema,
    at: usize,
) -> Result<&'a Declaration, ServiceError> {
    let name = &schema.dimensions[at].name;
    kinds.get(name).ok_or_else(|| {
        ServiceError::new(
            service_codes::SERVICE_FAILED,
            format!("the dimension {name} has no kind"),
        )
    })
}

/// The Tag schema there is: `None` only when there is no such file. A file
/// that is there and cannot be read is said to be so, and is never taken
/// for an absent one.
fn existing(path: &std::path::Path) -> Result<Option<Vec<u8>>, ServiceError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ServiceError::new(
            codes::TAG_SCHEMA_UNREADABLE,
            format!(
                "there is a Tag schema at {} and it cannot be read: {e}. It was left as it is, nothing was written and no Fact was recorded",
                path.display()
            ),
        )),
    }
}

/// What the conversion's Facts come to beside the Facts the DJ has.
struct Sorted<'a> {
    /// `(track, dimension by position, value)` to record.
    to_record: Vec<(&'a str, usize, Value)>,
    /// How many are there already, saying the same.
    unchanged: usize,
    /// The DJ's Facts that say otherwise and are left alone.
    conflicts: Vec<Conflict>,
}

/// Sorts what the tags say against what the DJ's live Facts say. A Fact
/// from the DJ is never recorded over unless `reimport`: where it says
/// otherwise than the tags (other terms, or terms where the track has no
/// tag), it stands and is a conflict.
fn sort_out<'a>(
    sidecar: &Sidecar,
    kinds: &Kinds,
    schema: &TagSchema,
    library: &'a MyTags,
    wanted: &[Wanted<'a>],
    reimport: bool,
) -> Result<Sorted<'a>, ServiceError> {
    let dj = Source::new(SourceKind::Dj, "dj");
    let said = |track: &str, at: usize| -> Result<Option<Value>, ServiceError> {
        let kind = kind_of(kinds, schema, at)?;
        Ok(sidecar
            .facts(track, &kind.name)
            .map_err(|e| sidecar_error(&e))?
            .into_iter()
            .find(|fact| fact.is_live() && fact.source == dj)
            .map(|fact| kind.normalise(fact.value)))
    };
    let mut sorted = Sorted {
        to_record: Vec::new(),
        unchanged: 0,
        conflicts: Vec::new(),
    };
    let mut by_place: HashMap<(&str, usize), &Vec<String>> = HashMap::new();
    for (track, at, value) in wanted {
        by_place.insert((track, *at), value);
    }
    let none = Vec::new();
    for (track, _) in &library.tracks {
        for at in 0..schema.dimensions.len() {
            let tags = by_place.get(&(track.as_str(), at)).copied();
            let value = json!(tags.unwrap_or(&none));
            match (said(track, at)?, tags) {
                (None, Some(_)) => sorted.to_record.push((track, at, value)),
                (None, None) => {}
                (Some(fact), Some(_)) if fact == value => sorted.unchanged += 1,
                // Nothing said of a track with no tag is nothing to say.
                (Some(fact), None) if fact == value => {}
                (Some(_), _) if reimport => sorted.to_record.push((track, at, value)),
                (Some(fact), _) => sorted.conflicts.push(Conflict {
                    track_id: track.clone(),
                    dimension: schema.dimensions[at].name.clone(),
                    fact,
                    library: value,
                }),
            }
        }
    }
    Ok(sorted)
}

/// Records `to_record` (track, dimension by position, value) as Facts from
/// the DJ in one Run, tells of each, and answers the Run. A Run that was
/// begun is finished whether or not its Facts were recorded.
fn record(
    stems: &Stems,
    sidecar: &Sidecar,
    kinds: &Kinds,
    schema: &TagSchema,
    to_record: &[(&str, usize, Value)],
) -> Result<RunId, ServiceError> {
    let dj = Source::new(SourceKind::Dj, "dj");
    let mut of_kind = Vec::with_capacity(to_record.len());
    for (_, at, _) in to_record {
        of_kind.push(kind_of(kinds, schema, *at)?);
    }
    let run = sidecar
        .begin_run(&dj, RUN_DESCRIPTION)
        .map_err(|e| sidecar_error(&e))?;
    let new: Vec<NewFact> = to_record
        .iter()
        .zip(&of_kind)
        .map(|((track, _, value), kind)| NewFact {
            track_id: track,
            // A Judgement follows the music; the audio is not read, so a
            // whole Library converts without opening one audio file.
            fingerprint: None,
            kind: kind.kind(),
            value: value.clone(),
            source: &dj,
            confidence: 1.0,
            run,
        })
        .collect();
    let written = sidecar.record_many(&new);
    let outcome = match &written {
        Ok(facts) if facts.len() == 1 => "recorded 1 Fact".to_owned(),
        Ok(facts) => format!("recorded {} Facts", facts.len()),
        Err(error) => format!("nothing recorded: {error}"),
    };
    let _ = sidecar.finish_run(run, &outcome);
    for fact in written.map_err(|e| sidecar_error(&e))? {
        stems.publish(
            stems::events::SIDECAR_CHANGED,
            json!(SidecarChanged {
                track_id: fact.track_id,
                kind: fact.kind,
                fact_id: Some(fact.id),
                stale: Vec::new(),
            }),
        );
    }
    Ok(run)
}

/// The terms a Fact names for a track that carries `carried`: in the tree,
/// the most specific ones, since a child implies its parents. In the
/// dimension's own order.
fn most_specific(
    dimension: &Dimension,
    carried: &[&str],
    implied: &mut ImpliedParents,
) -> Vec<String> {
    let mut ancestors: HashSet<&str> = HashSet::new();
    if dimension.tree {
        for id in carried {
            let above = dimension.ancestors(id);
            if let Some(parent) = above.first() {
                if carried.contains(parent) {
                    implied.with_parent += 1;
                } else {
                    implied.without_parent += 1;
                }
            }
            ancestors.extend(above);
        }
    }
    dimension
        .terms
        .iter()
        .filter(|term| carried.contains(&term.id.as_str()) && !ancestors.contains(term.id.as_str()))
        .map(|term| term.id.clone())
        .collect()
}

/// The schema's file text, having made sure the schema is one that may be
/// used, that its text reads back as itself, and that its deck view is the
/// Library's MyTags exactly.
fn checked_text(
    schema: &TagSchema,
    library: &MyTags,
    declared: &Kinds,
) -> Result<String, ServiceError> {
    let problems = schema.problems();
    if !problems.is_empty() {
        return Err(invalid(problems));
    }
    declared.clone().with_schema(schema).map_err(invalid)?;
    let text = schema.to_toml();
    let read = TagSchema::parse(&text).map_err(invalid)?;
    let made: Vec<(String, Vec<String>)> = read
        .deck_view()
        .into_iter()
        .map(|category| {
            (
                category.label,
                category.tags.into_iter().map(|tag| tag.label).collect(),
            )
        })
        .collect();
    let there: Vec<(String, Vec<String>)> = library
        .categories
        .iter()
        .map(|category| {
            (
                category.name.clone(),
                category.tags.iter().map(|(_, text)| text.clone()).collect(),
            )
        })
        .collect();
    if made != there {
        return Err(ServiceError::new(
            service_codes::SERVICE_FAILED,
            "the conversion did not reproduce the Library's MyTags exactly; nothing was written. This is a fault in wreckordbox, not in the Library",
        ));
    }
    Ok(text)
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "it is what `map_err` is given"
)]
fn invalid(problems: Vec<wreckordbox_sidecar::tag_schema::Problem>) -> ServiceError {
    let all: Vec<String> = problems.iter().map(ToString::to_string).collect();
    ServiceError::new(
        codes::TAG_SCHEMA_INVALID,
        format!(
            "the Library's MyTags do not make a Tag schema that can be used: {}",
            all.join("; ")
        ),
    )
}

/// Writes the file whole: a working file of its own beside it first, then
/// in its place.
fn write_whole(path: &std::path::Path, bytes: &[u8]) -> Result<(), ServiceError> {
    use std::io::Write as _;
    let beside = path.with_extension(format!(
        "toml.{}-{}.writing",
        std::process::id(),
        WORKING_FILES.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&beside)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        })
        .and_then(|()| std::fs::rename(&beside, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&beside);
            ServiceError::new(
                stems::codes::SIDECAR_FAILED,
                format!(
                    "the Tag schema {} could not be written: {e}",
                    path.display()
                ),
            )
        })
}

/// Puts the Tag schema back as it was before the conversion wrote it: its
/// earlier bytes, or no file.
fn put_back(path: &std::path::Path, earlier: Option<&[u8]>) -> Result<(), String> {
    match earlier {
        Some(bytes) => write_whole(path, bytes).map_err(|error| error.message),
        None => match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        },
    }
}

/// A tag's text as a child would be written: how many levels down its `>`s
/// put it, and what follows them.
fn levels(text: &str) -> (usize, &str) {
    let mut depth = 0;
    let mut rest = text.trim_start();
    while let Some(after) = rest.strip_prefix(CHILD_MARK) {
        depth += 1;
        rest = after.trim_start();
    }
    (depth, rest.trim_end())
}

fn plan(library: &MyTags, convert: &Convert, declared: &Kinds) -> Result<Plan, ServiceError> {
    let categories = &library.categories;
    let find = |named: &str| -> Result<usize, ServiceError> {
        categories
            .iter()
            .position(|category| category.id == named)
            .or_else(|| {
                categories
                    .iter()
                    .position(|category| category.name == named)
            })
            .ok_or_else(|| {
                let known: Vec<String> = categories
                    .iter()
                    .map(|category| format!("{} \"{}\"", category.id, category.name))
                    .collect();
                ServiceError::new(
                    codes::MY_TAG_CATEGORY_NOT_FOUND,
                    format!(
                        "the Library has no MyTag category {named}. It has: {}",
                        if known.is_empty() {
                            "none".to_owned()
                        } else {
                            known.join(", ")
                        }
                    ),
                )
            })
    };

    let children_of = |category: &Category| {
        category
            .tags
            .iter()
            .filter(|(_, text)| levels(text).0 > 0)
            .count()
    };
    let (tree_at, told) = match convert.tree.as_deref() {
        Some("none") => (None, true),
        Some(named) => (Some(find(named)?), true),
        None => {
            // The category most of whose tags are written as children; the
            // first of them when two tie.
            let mut best: Option<(usize, usize)> = None;
            for (at, category) in categories.iter().enumerate() {
                let children = children_of(category);
                if children > 0 && best.is_none_or(|(_, most)| children > most) {
                    best = Some((at, children));
                }
            }
            (best.map(|(at, _)| at), false)
        }
    };

    let names = dimension_names(categories, convert, tree_at, declared, &find)?;

    let mut plan = Plan {
        schema: TagSchema::default(),
        categories: Vec::new(),
        tree: None,
        disambiguated: Vec::new(),
        odd: Vec::new(),
        terms: HashMap::new(),
    };
    let mut deck = Deck::default();
    // The tree goes first in the schema, as it does in the file.
    let mut order: Vec<usize> = (0..categories.len()).collect();
    order.sort_by_key(|&at| tree_at != Some(at));
    let mut dimension_at = vec![0; categories.len()];
    for (position, &at) in order.iter().enumerate() {
        let category = &categories[at];
        let name = names[at].clone().unwrap_or_default();
        let tree = tree_at == Some(at);
        let made = if tree {
            let prefix = child_prefix(category);
            deck.child_prefix.clone_from(&prefix);
            let made = tree_terms(category, &prefix);
            plan.tree = Some(Tree {
                category: category.name.clone(),
                told,
                parents: made
                    .terms
                    .iter()
                    .filter(|term| term.parent.is_none())
                    .count(),
                children: made
                    .terms
                    .iter()
                    .filter(|term| term.parent.is_some())
                    .count(),
                child_prefix: prefix,
            });
            made
        } else {
            list_terms(category)
        };
        for ((tag, text), term) in category.tags.iter().zip(&made.terms) {
            plan.terms
                .insert(my_tag_key(tag), (position, term.id.clone()));
            if let Some(why) = made.odd.get(&term.id) {
                plan.odd.push(OddEntry {
                    category: category.name.clone(),
                    dimension: name.clone(),
                    id: term.id.clone(),
                    tag: text.clone(),
                    why: why.clone(),
                    tracks: library
                        .tracks
                        .iter()
                        .filter(|(_, keys)| keys.contains(&my_tag_key(tag)))
                        .count(),
                });
            }
        }
        plan.disambiguated.extend(made.disambiguated);
        dimension_at[at] = position;
        plan.schema.dimensions.push(Dimension {
            name,
            tree,
            description: String::new(),
            terms: made.terms,
            sources: BTreeMap::new(),
            line: None,
        });
    }
    for (at, category) in categories.iter().enumerate() {
        let dimension = &plan.schema.dimensions[dimension_at[at]];
        deck.categories.push(DeckCategory {
            dimension: dimension.name.clone(),
            label: category.name.clone(),
            order: Vec::new(),
            line: None,
        });
        plan.categories.push(ConvertedCategory {
            category_id: category.id.clone(),
            category: category.name.clone(),
            dimension: dimension.name.clone(),
            tree: dimension.tree,
            terms: dimension.terms.len(),
        });
    }
    plan.schema.deck = deck;
    Ok(plan)
}

/// What each category's dimension is called: what the command says, else
/// the tree's name for the tree and [`dimension_name`] for the others.
fn dimension_names(
    categories: &[Category],
    convert: &Convert,
    tree_at: Option<usize>,
    declared: &Kinds,
    find: &dyn Fn(&str) -> Result<usize, ServiceError>,
) -> Result<Vec<Option<String>>, ServiceError> {
    let mut names: Vec<Option<String>> = vec![None; categories.len()];
    for (category, name) in &convert.names {
        let at = find(category)?;
        let tree = tree_at == Some(at);
        let fits = if tree {
            name == GENRE
        } else {
            is_dimension_name(name) && name != GENRE
        };
        if !fits {
            return Err(ServiceError::new(
                service_codes::BAD_REQUEST,
                if tree {
                    format!("the genre tree's dimension is called {GENRE}, not \"{name}\"")
                } else {
                    format!("a dimension's name is lower-case letters, digits and underscores, starting with a letter, and {GENRE} is the tree's; \"{name}\" will not do")
                },
            ));
        }
        // The Facts of a category are lists: a track can carry several of
        // its tags.
        if let Some(kind) = declared.get(name).filter(|_| !tree) {
            if !holds_a_category(kind) {
                return Err(ServiceError::new(
                    service_codes::BAD_REQUEST,
                    format!(
                        "{name} is already a kind of Fact, a {} taking {}, and cannot hold a MyTag category: a track can carry several of a category's tags, so its dimension's kind takes a list of texts. Call the dimension something else, such as {name}_tags",
                        kind.nature,
                        kind.takes()
                    ),
                ));
            }
        }
        names[at] = Some(name.clone());
    }
    let mut taken: HashSet<String> = HashSet::from([GENRE.to_owned()]);
    for name in names.iter().flatten() {
        if !taken.insert(name.clone()) && name != GENRE {
            return Err(ServiceError::new(
                service_codes::BAD_REQUEST,
                format!("two categories were given the dimension name {name}"),
            ));
        }
    }
    for (at, category) in categories.iter().enumerate() {
        if names[at].is_some() {
            continue;
        }
        let name = if tree_at == Some(at) {
            GENRE.to_owned()
        } else {
            let name = dimension_name(&category.name, at, declared, &taken);
            taken.insert(name.clone());
            name
        };
        names[at] = Some(name);
    }
    Ok(names)
}

/// What a category's dimension is called when nobody says: its label in
/// lower case with underscores; a built-in kind that takes several terms
/// when the label begins with its name (`Mood/Emotion` is `mood`); never a
/// kind that could not hold a track's several tags.
fn dimension_name(label: &str, at: usize, declared: &Kinds, taken: &HashSet<String>) -> String {
    let slug = slug(label, '_');
    let usable = |name: &str| declared.get(name).is_none_or(holds_a_category);
    let first = slug.split('_').next().unwrap_or_default();
    let built_in = declared
        .get(first)
        .is_some_and(|kind| kind.built_in && usable(first));
    let mut name = if built_in && !taken.contains(first) {
        first.to_owned()
    } else if !is_dimension_name(&slug) {
        format!("category_{}", at + 1)
    } else if usable(&slug) {
        slug
    } else {
        format!("{slug}_tags")
    };
    let base = name.clone();
    let mut n = 2;
    while taken.contains(&name) || !usable(&name) {
        name = format!("{base}_{n}");
        n += 1;
    }
    name
}

/// Whether a kind declared already can be the kind of a MyTag category's
/// dimension: one the Tag schema may govern, taking a list of texts.
fn holds_a_category(kind: &Declaration) -> bool {
    kind.may_be_dimension() && kind.value == ValueType::Texts
}

/// Text as part of an identifier: lower-case letters and digits, anything
/// else a single `joint`.
fn slug(text: &str, joint: char) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with(joint) {
            out.push(joint);
        }
    }
    out.trim_end_matches(joint).to_owned()
}

/// An identifier nothing in `taken` has, from `wanted`.
fn unique_id(wanted: &str, taken: &mut HashSet<String>) -> String {
    let base = if is_term_id(wanted) { wanted } else { "term" };
    let mut id = base.to_owned();
    let mut n = 2;
    while !taken.insert(id.clone()) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

/// The terms a category makes.
struct Made {
    /// One per tag, in the category's order.
    terms: Vec<Term>,
    /// Why a term is odd, by its identifier.
    odd: HashMap<String, String>,
    disambiguated: Vec<Disambiguated>,
}

/// Why a tag's text does not look like a term, when it does not.
fn oddity(name: &str) -> Option<String> {
    const RULES: &str = "-=_*#~.:|+/\\";
    if !name.chars().any(char::is_alphanumeric) {
        return Some("it has no letter or digit, so it looks like a divider, not a tag".to_owned());
    }
    let ruled = |c: Option<char>| c.is_some_and(|c| RULES.contains(c));
    if ruled(name.chars().next()) && ruled(name.chars().last()) {
        return Some("it is set between rules, so it looks like a heading, not a tag".to_owned());
    }
    None
}

/// A category that is not the tree: each tag a term, as it is.
fn list_terms(category: &Category) -> Made {
    let mut made = Made {
        terms: Vec::new(),
        odd: HashMap::new(),
        disambiguated: Vec::new(),
    };
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    for (_, text) in &category.tags {
        let (name, mut odd) = term_name(text.trim(), text, &mut names);
        let mut term = Term::new(unique_id(&slug(&name, '-'), &mut ids), name);
        if term.name != *text {
            term.deck = Some(text.clone());
        }
        odd = odd.or_else(|| oddity(&term.name));
        note(&mut made, &mut term, odd);
        made.terms.push(term);
    }
    made
}

/// A name for a term among its siblings: `wanted`, unless that is empty or
/// a sibling has it.
fn term_name(wanted: &str, text: &str, siblings: &mut HashSet<String>) -> (String, Option<String>) {
    if wanted.is_empty() {
        let mut name = "unnamed".to_owned();
        let mut n = 2;
        while !siblings.insert(name.clone()) {
            name = format!("unnamed {n}");
            n += 1;
        }
        return (name, Some("it has no name".to_owned()));
    }
    if siblings.insert(wanted.to_owned()) {
        return (wanted.to_owned(), None);
    }
    // Two tags that differ only in spacing: the second keeps its own text.
    let mut name = text.trim().to_owned();
    let mut n = 2;
    while !siblings.insert(name.clone()) {
        name = format!("{wanted} ({n})");
        n += 1;
    }
    (
        name,
        Some(format!(
            "it differs from another tag of the same parent, \"{wanted}\", only in its spacing"
        )),
    )
}

fn note(made: &mut Made, term: &mut Term, odd: Option<String>) {
    if let Some(why) = odd {
        term.note = Some(format!("odd: {why}"));
        made.odd.insert(term.id.clone(), why);
    }
}

/// What stands before a child's name in this category: the way most of its
/// children are written.
fn child_prefix(category: &Category) -> String {
    let mut written: Vec<(&str, usize)> = Vec::new();
    for (_, text) in &category.tags {
        let (depth, rest) = levels(text);
        if depth != 1 || rest.is_empty() {
            continue;
        }
        let Some(prefix) = text.trim_end().strip_suffix(rest) else {
            continue;
        };
        match written.iter_mut().find(|(seen, _)| *seen == prefix) {
            Some((_, times)) => *times += 1,
            None => written.push((prefix, 1)),
        }
    }
    // The commonest; the first written of those that tie.
    let mut best: Option<(&str, usize)> = None;
    for (prefix, times) in written {
        if best.is_none_or(|(_, most)| times > most) {
            best = Some((prefix, times));
        }
    }
    best.map_or_else(
        || DEFAULT_CHILD_PREFIX.to_owned(),
        |(prefix, _)| prefix.to_owned(),
    )
}

/// The genre tree a flat list makes.
fn tree_terms(category: &Category, prefix: &str) -> Made {
    let mut made = Made {
        terms: Vec::new(),
        odd: HashMap::new(),
        disambiguated: Vec::new(),
    };
    // Each term's parent, by position; identifiers come last, from the
    // names the terms end up with.
    let mut parents: Vec<Option<usize>> = Vec::new();
    let mut odd: Vec<Option<String>> = Vec::new();
    let mut names: HashMap<Option<usize>, HashSet<String>> = HashMap::new();
    // The term open at each level: `open[0]` the parent of the group.
    let mut open: Vec<usize> = Vec::new();
    for (_, text) in &category.tags {
        let (depth, rest) = levels(text);
        let mut why = None;
        if depth == 0 {
            open.clear();
        } else if depth <= open.len() {
            open.truncate(depth);
        } else if open.is_empty() {
            why = Some("it is written as a child, but no parent comes before it".to_owned());
        } else {
            // Deeper than anything before it allows: a child of the
            // deepest term there is.
            why = Some(format!(
                "it is written {depth} levels down, under a tag that is only {} down",
                open.len() - 1
            ));
        }
        let parent = open.last().copied();
        let wanted = if rest.is_empty() { text.trim() } else { rest };
        let (name, clash) = term_name(wanted, text, names.entry(parent).or_default());
        why = why.or(clash).or_else(|| oddity(&name));
        // A child with no parent stands at the top, and is nobody's parent.
        if depth == 0 || parent.is_some() {
            open.push(made.terms.len());
        }
        parents.push(parent);
        odd.push(why);
        made.terms.push(Term::new(String::new(), name));
    }
    let stripped = strip_markers(&mut made.terms, &parents);
    let mut ids = HashSet::new();
    for at in 0..made.terms.len() {
        let parent = parents[at].map(|parent| made.terms[parent].id.clone());
        let name = slug(&made.terms[at].name, '-');
        let wanted = match &parent {
            Some(parent) if name.is_empty() => parent.clone(),
            Some(parent) => format!("{parent}-{name}"),
            None => name,
        };
        let mut depth = 0;
        let mut above = parents[at];
        while let Some(parent) = above {
            depth += 1;
            above = parents[parent];
        }
        let text = &category.tags[at].1;
        let mut term = Term::new(unique_id(&wanted, &mut ids), made.terms[at].name.clone());
        term.parent = parent;
        // Where the deck view's rule does not give the tag's text, keep it.
        if format!("{}{}", prefix.repeat(depth), term.name) != *text {
            term.deck = Some(text.clone());
        }
        if stripped.contains(&at) {
            made.disambiguated.push(Disambiguated {
                id: term.id.clone(),
                name: term.name.clone(),
                parent: term.parent.clone().unwrap_or_default(),
                tag: text.clone(),
            });
        }
        note(&mut made, &mut term, odd[at].take());
        made.terms[at] = term;
    }
    made
}

/// A name that ends in a short marker in brackets, `Deep (T)`: the name
/// without it.
fn unmarked(name: &str) -> Option<&str> {
    let inside = name.strip_suffix(')')?;
    let (before, marker) = inside.rsplit_once('(')?;
    let short =
        (1..=3).contains(&marker.chars().count()) && marker.chars().all(char::is_alphanumeric);
    let before = before.trim_end();
    (short && !before.is_empty()).then_some(before)
}

/// Takes a child's hand-written marker off its name where the marker was
/// there to tell it from the same name under another parent, and no
/// sibling has the name. Answers which terms, by position.
fn strip_markers(terms: &mut [Term], parents: &[Option<usize>]) -> Vec<usize> {
    let bare: Vec<&str> = terms
        .iter()
        .map(|term| unmarked(&term.name).unwrap_or(&term.name))
        .collect();
    let mut renamed: Vec<(usize, String)> = Vec::new();
    for (at, term) in terms.iter().enumerate() {
        let (Some(name), Some(_)) = (unmarked(&term.name), parents[at]) else {
            continue;
        };
        let others = || (0..terms.len()).filter(move |other| *other != at);
        let elsewhere = others().any(|other| parents[other] != parents[at] && bare[other] == name);
        let beside = others().any(|other| {
            parents[other] == parents[at] && (bare[other] == name || terms[other].name == name)
        });
        if elsewhere && !beside {
            renamed.push((at, name.to_owned()));
        }
    }
    renamed
        .into_iter()
        .map(|(at, name)| {
            terms[at].name = name;
            at
        })
        .collect()
}
