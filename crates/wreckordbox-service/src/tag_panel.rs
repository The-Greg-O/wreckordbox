//! The Intake tag panel (wbx-65): what it shows, and the one way it edits.
//!
//! `intake.tag_panel` describes every dimension of the DJ's tags, in the
//! order the panel shows them, which of them reach the deck as MyTag
//! categories, each one's tags, and optionally what one track carries.
//! With a valid Tag schema in the Sidecar's folder the description is the
//! schema's: its deck categories first, in the deck's order, then its other
//! dimensions. Without one (or with one that has a mistake, which is said)
//! it is the Library's MyTag categories, every one of them deck-facing,
//! through the same answer. [`deck_dimensions`] is the same resolution for
//! the Intake queue's "needs tags" line, so the two never disagree.
//!
//! `intake.set_tags` sets one dimension of one track to exactly the tags
//! the DJ chose (the explicit selection: in the tree, the most specific
//! tags; a parent is implied). It is checked against the description as it
//! is NOW: the dimension's route, its tags, and the version the panel was
//! drawn from, so a click made on a stale panel is refused
//! (`tag_panel_stale`) and the panel reads again. Then it is ONE entry of
//! the shared edit history, so Undo takes it back whichever store it
//! touched:
//!
//! - a deck-facing dimension: the track's MyTags, derived from the
//!   selection (with the tree's parents when the schema's `tag_parents`
//!   says the deck carries them), written as one MyTag edit; the DJ's
//!   Fact follows from it (wbx-19's native capture);
//! - any other dimension: a [`LibraryEdit::Judgements`] step, which writes
//!   no Library row; the Ledger records the DJ's Fact, withdraws it on
//!   Undo (reinstating what stood before, with its source) and records it
//!   again on Redo.
//!
//! Values the description does not know (a term removed from the schema, a
//! MyTag of a deck category no term stands for) are reported as `unmapped`
//! and are never dropped by a replacement unless the DJ names them in
//! `drop_unmapped`. A Fact the Ledger could not record is reported in
//! `facts_not_recorded`, never hidden. Reading writes nothing: no Sidecar
//! is made, opened for writing, migrated or tidied.
#![allow(
    clippy::doc_markdown,
    reason = "MyTag is the glossary's word for the thing, not a code item"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};

use rbl_db::export_info::MyTagRow;
use rbl_library::history::{LibraryEdit, TRACK_EDIT};
use rbl_library::judgements::{JudgementChange, Judgements};
use rbl_library::Touched;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wreckordbox_sidecar::tag_schema::{Dimension as SchemaDimension, Term};
use wreckordbox_sidecar::{
    Fact, FactsSnapshot, Kinds, SourceKind, TagSchema, ValueType, TAG_SCHEMA_FILE,
};

use crate::library::OpenLibrary;
use crate::protocol::{codes, LibraryChanged, Request, ServiceError};
use crate::stems::{self, SidecarChanged, Stems};

/// The names of the commands.
pub mod commands {
    /// The panel's description. Takes a [`TagPanelOf`](super::TagPanelOf),
    /// answers a [`TagPanel`](super::TagPanel). Reads only.
    pub const INTAKE_TAG_PANEL: &str = "intake.tag_panel";
    /// Sets one dimension of one track. Takes a [`SetTags`](super::SetTags),
    /// answers a [`TagsSet`](super::TagsSet).
    pub const INTAKE_SET_TAGS: &str = "intake.set_tags";
}
pub use commands::INTAKE_TAG_PANEL;

/// The error codes of `intake.set_tags`, beside the service's own.
pub mod tag_codes {
    /// The panel the request was drawn from is no longer what the service
    /// would show: the Tag schema, the route or the track's tags changed.
    /// Read the panel again and repeat the click.
    pub const TAG_PANEL_STALE: &str = "tag_panel_stale";
    /// A tag the dimension does not have, two for a dimension that takes
    /// one, or an unmapped value that is not there.
    pub const TAG_NOT_IN_DIMENSION: &str = "tag_not_in_dimension";
    /// A deck-facing tag with no MyTag in the Library yet (the Tag schema
    /// has not been projected), or a value no tag stands for that the
    /// change would have to replace.
    pub const TAG_NOT_SETTABLE: &str = "tag_not_settable";
}

const EDIT_MY_TAG: &str = "my_tag";
const EDIT_FACT: &str = "fact";

/// The arguments of `intake.tag_panel`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TagPanelOf {
    /// The track whose tags to report beside the description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// The answer of `intake.tag_panel`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagPanel {
    /// `tag_schema` or `my_tags`: where the description came from.
    pub source: String,
    /// Why the Tag schema could not be used, when there is one with a
    /// mistake: the panel is then the MyTags'.
    pub schema_problem: Option<String>,
    /// Changes whenever the description does (a schema edit, a MyTag
    /// renamed): the window reads the panel again when it does.
    pub schema_version: String,
    /// In the order the panel shows them: deck-facing first.
    pub dimensions: Vec<PanelDimension>,
    /// The track asked about, if one was.
    pub track: Option<TrackTags>,
}

/// One dimension of the panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "the wire's own description: each is a separate fact about the dimension"
)]
pub struct PanelDimension {
    /// The dimension's name in the Tag schema (the kind of its Facts), or
    /// the MyTag category's identifier.
    pub id: String,
    /// What the DJ reads: the deck's category name for a deck-facing one.
    pub label: String,
    /// Whether it reaches the deck as a MyTag category.
    pub deck: bool,
    /// The genre tree, whose tags may have parents; a child implies them.
    pub tree: bool,
    /// For a deck-facing tree: a track of a child also carries its
    /// parents' MyTags on the deck (the schema's `[deck.genre] tag_parents`).
    pub tag_parents: bool,
    /// The dimension takes one tag at most (a single-text kind such as
    /// `vocals` or `version`): choosing another replaces it.
    pub single: bool,
    /// How a tag is set: `my_tag` (the Library's MyTags) or `fact` (the
    /// DJ's Fact of the kind `id`). Both through `intake.set_tags`.
    pub edit: String,
    /// In the order the panel lists them.
    pub tags: Vec<PanelTag>,
    /// The MyTag category it is in the Library, for a deck-facing one.
    #[serde(skip)]
    category: Option<String>,
}

/// One tag of a dimension.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PanelTag {
    /// The term's identifier, or the MyTag's.
    pub id: String,
    pub name: String,
    /// In the tree: the identifier of the tag this is a child of.
    pub parent: Option<String>,
    /// 0 for a top tag, 1 for its child, and so on.
    pub depth: usize,
    /// The Library's MyTag that stands for it, for a deck-facing dimension;
    /// `null` when the Library has none yet.
    pub my_tag: Option<String>,
}

/// A value a track holds that no tag of the dimension stands for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unmapped {
    /// The term identifier the Fact holds, or the MyTag's identifier.
    pub id: String,
    /// What to show: the MyTag's text, or the identifier.
    pub name: String,
}

/// Which tags one track carries, by dimension `id`, as tag `id`s: the
/// explicit selection (in the tree, the most specific; parents implied).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackTags {
    pub id: String,
    pub set: BTreeMap<String, Vec<String>>,
    /// Values no tag stands for, by dimension: shown, never dropped.
    pub unmapped: BTreeMap<String, Vec<Unmapped>>,
    /// What `intake.set_tags` must be given back, by dimension: the
    /// version of the dimension and of the track's value the panel shows.
    pub versions: BTreeMap<String, String>,
    /// Why the DJ's Facts could not be read, when they could not: the
    /// vocabulary dimensions are then shown as unknown, not empty.
    pub facts_unavailable: Option<String>,
}

/// The arguments of `intake.set_tags`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetTags {
    /// The track.
    pub id: String,
    /// The dimension's `id` in the panel.
    pub dimension: String,
    /// The tags to hold, exactly: the explicit selection. In the tree a
    /// tag and its ancestor is the tag alone.
    pub terms: Vec<String>,
    /// The dimension's version in the panel the DJ clicked. A caller that
    /// drew no panel (the CLI) leaves it out and is not checked for one.
    #[serde(default)]
    pub version: Option<String>,
    /// Unmapped values the DJ chose to take off.
    #[serde(default)]
    pub drop_unmapped: Vec<String>,
    /// Add "My Tag" to the "Comments" (#605), for a deck-facing dimension.
    #[serde(default)]
    pub comments: bool,
}

/// The answer of `intake.set_tags`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagsSet {
    /// Whether anything changed (an equal selection changes nothing and is
    /// not an entry of the history).
    pub changed: bool,
    /// The panel as it is now, for the track.
    pub panel: TagPanel,
    /// Why a DJ Fact the change should have recorded was not, each said:
    /// the Ledger's own reasons. Empty when all were recorded.
    pub facts_not_recorded: Vec<String>,
}

pub(crate) fn answers(command: &str) -> bool {
    matches!(
        command,
        commands::INTAKE_TAG_PANEL | commands::INTAKE_SET_TAGS
    )
}

/// Answers one of this module's commands, and what to tell every client
/// when it changed the Library.
pub(crate) fn answer(
    library: &OpenLibrary,
    stems: &Stems,
    request: &Request,
) -> Result<(Value, Option<LibraryChanged>), ServiceError> {
    if request.command == commands::INTAKE_SET_TAGS {
        let set: SetTags = serde_json::from_value(request.args.clone()).map_err(|e| {
            ServiceError::new(
                codes::BAD_REQUEST,
                format!("bad arguments to {}: {e}", request.command),
            )
        })?;
        let (answer, changed) = set_tags(library, stems, &set)?;
        return Ok((to_value(&answer)?, changed));
    }
    let of: TagPanelOf = if request.args.is_null() {
        TagPanelOf::default()
    } else {
        serde_json::from_value(request.args.clone()).map_err(|e| {
            ServiceError::new(
                codes::BAD_REQUEST,
                format!("bad arguments to {}: {e}", request.command),
            )
        })?
    };
    Ok((
        to_value(&tag_panel(library, stems, of.id.as_deref())?)?,
        None,
    ))
}

fn to_value<T: Serialize>(value: &T) -> Result<Value, ServiceError> {
    serde_json::to_value(value).map_err(|e| ServiceError::new(codes::SERVICE_FAILED, e.to_string()))
}

/// The description, resolved once for a read or a write.
struct Resolved {
    source: &'static str,
    schema_problem: Option<String>,
    dimensions: Vec<PanelDimension>,
    my_tags: Vec<MyTagRow>,
}

impl Resolved {
    fn read(library: &OpenLibrary, stems: &Stems) -> Result<Self, ServiceError> {
        let my_tags = library.with_db(|db| {
            rbl_db::export_info::my_tags(db.connection()).map_err(|e| {
                ServiceError::new(
                    codes::SERVICE_FAILED,
                    format!("the Library's MyTags could not be read: {e}"),
                )
            })
        })?;
        Ok(Self::from_my_tags(my_tags, stems.sidecar_dir()))
    }

    fn from_my_tags(my_tags: Vec<MyTagRow>, sidecar_dir: &std::path::Path) -> Self {
        let path = sidecar_dir.join(TAG_SCHEMA_FILE);
        let (schema, schema_problem) = match std::fs::metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (None, None),
            _ => match TagSchema::load(&path) {
                Ok(schema) => (Some(schema), None),
                Err(problems) => (None, Some(describe_problems(&path, &problems))),
            },
        };
        let (source, dimensions) = match &schema {
            Some(schema) => ("tag_schema", from_schema(schema, &my_tags, sidecar_dir)),
            None => ("my_tags", from_my_tags(&my_tags)),
        };
        Self {
            source,
            schema_problem,
            dimensions,
            my_tags,
        }
    }

    fn schema_version(&self) -> String {
        version_of(&(
            self.source,
            &self.schema_problem,
            serde_json::to_string(&self.dimensions).unwrap_or_default(),
        ))
    }

    fn dimension(&self, id: &str) -> Option<&PanelDimension> {
        self.dimensions.iter().find(|dimension| dimension.id == id)
    }
}

/// The deck-facing dimensions, in the deck's order, each with the index
/// keys of the MyTags that stand for its tags: what the Intake queue
/// counts a track as tagged by. One resolution with the panel's.
pub(crate) fn deck_dimensions(
    my_tags: &[MyTagRow],
    sidecar_dir: &std::path::Path,
) -> Vec<(String, String, &'static str, Vec<i32>)> {
    let resolved = Resolved::from_my_tags(my_tags.to_vec(), sidecar_dir);
    let kind = if resolved.source == "tag_schema" {
        "tag_schema_dimension"
    } else {
        "mytag_category"
    };
    resolved
        .dimensions
        .iter()
        .filter(|dimension| dimension.deck)
        .map(|dimension| {
            (
                dimension.id.clone(),
                dimension.label.clone(),
                kind,
                dimension
                    .tags
                    .iter()
                    .filter_map(|tag| tag.my_tag.as_deref())
                    .map(rbl_index::smart::my_tag_key)
                    .collect(),
            )
        })
        .collect()
}

fn tag_panel(
    library: &OpenLibrary,
    stems: &Stems,
    track: Option<&str>,
) -> Result<TagPanel, ServiceError> {
    let resolved = Resolved::read(library, stems)?;
    panel_of(library, stems, &resolved, track)
}

fn panel_of(
    library: &OpenLibrary,
    stems: &Stems,
    resolved: &Resolved,
    track: Option<&str>,
) -> Result<TagPanel, ServiceError> {
    let track = match track {
        Some(id) => Some(track_tags(library, stems, resolved, id)?),
        None => None,
    };
    Ok(TagPanel {
        source: resolved.source.to_owned(),
        schema_problem: resolved.schema_problem.clone(),
        schema_version: resolved.schema_version(),
        dimensions: resolved.dimensions.clone(),
        track,
    })
}

/// What one track holds in each dimension.
fn track_tags(
    library: &OpenLibrary,
    stems: &Stems,
    resolved: &Resolved,
    id: &str,
) -> Result<TrackTags, ServiceError> {
    let index = library.index()?;
    let row = index.row_of(id).ok_or_else(|| {
        ServiceError::new(codes::TRACK_NOT_FOUND, format!("no track has the id {id}"))
    })?;
    let carried = index.my_tag_keys(row as usize);
    let fact_kinds: Vec<&str> = resolved
        .dimensions
        .iter()
        .filter(|dimension| dimension.edit == EDIT_FACT)
        .map(|dimension| dimension.id.as_str())
        .collect();
    let (facts, facts_unavailable) = if fact_kinds.is_empty() {
        (BTreeMap::new(), None)
    } else {
        match djs_facts(stems, id, &fact_kinds) {
            Ok(facts) => (facts, None),
            Err(why) => (BTreeMap::new(), Some(why)),
        }
    };
    let mut out = TrackTags {
        id: id.to_owned(),
        set: BTreeMap::new(),
        unmapped: BTreeMap::new(),
        versions: BTreeMap::new(),
        facts_unavailable,
    };
    for dimension in &resolved.dimensions {
        let (set, unmapped, raw) = if dimension.edit == EDIT_MY_TAG {
            // The category's MyTags the track carries, mapped or not.
            let in_category: Vec<&MyTagRow> = resolved
                .my_tags
                .iter()
                .filter(|row| {
                    row.attribute == 0
                        && Some(&row.parent) == dimension.category.as_ref()
                        && carried.contains(&rbl_index::smart::my_tag_key(&row.id))
                })
                .collect();
            let mapped: Vec<String> = dimension
                .tags
                .iter()
                .filter(|tag| {
                    tag.my_tag
                        .as_ref()
                        .is_some_and(|my_tag| in_category.iter().any(|row| &row.id == my_tag))
                })
                .map(|tag| tag.id.clone())
                .collect();
            let unmapped: Vec<Unmapped> = in_category
                .iter()
                .filter(|row| {
                    !dimension
                        .tags
                        .iter()
                        .any(|tag| tag.my_tag.as_deref() == Some(row.id.as_str()))
                })
                .map(|row| Unmapped {
                    id: row.id.clone(),
                    name: row.name.clone(),
                })
                .collect();
            let mut raw: Vec<&str> = in_category.iter().map(|row| row.id.as_str()).collect();
            raw.sort_unstable();
            (most_specific(dimension, &mapped), unmapped, raw.join(","))
        } else {
            let held = facts.get(&dimension.id);
            let values = held.map(|(_, value)| term_ids(value)).unwrap_or_default();
            let known: Vec<String> = dimension
                .tags
                .iter()
                .filter(|tag| values.contains(&tag.id))
                .map(|tag| tag.id.clone())
                .collect();
            let unmapped = values
                .iter()
                .filter(|value| !dimension.tags.iter().any(|tag| &tag.id == *value))
                .map(|value| Unmapped {
                    id: value.clone(),
                    name: value.clone(),
                })
                .collect();
            let raw = held.map_or_else(String::new, |(fact, value)| format!("{fact}:{value}"));
            (known, unmapped, raw)
        };
        let version = version_of(&(
            serde_json::to_string(dimension).unwrap_or_default(),
            &raw,
            out.facts_unavailable.is_some() && dimension.edit == EDIT_FACT,
        ));
        if !unmapped.is_empty() {
            out.unmapped.insert(dimension.id.clone(), unmapped);
        }
        out.set.insert(dimension.id.clone(), set);
        out.versions.insert(dimension.id.clone(), version);
    }
    Ok(out)
}

/// `selection` without any tag that is an ancestor of another in it, in
/// the dimension's order: a Fact names the most specific tags.
fn most_specific(dimension: &PanelDimension, selection: &[String]) -> Vec<String> {
    let implied: BTreeSet<String> = selection
        .iter()
        .flat_map(|id| ancestors(dimension, id))
        .collect();
    dimension
        .tags
        .iter()
        .filter(|tag| selection.contains(&tag.id) && !implied.contains(&tag.id))
        .map(|tag| tag.id.clone())
        .collect()
}

fn ancestors(dimension: &PanelDimension, id: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut at = dimension
        .tags
        .iter()
        .find(|tag| tag.id == id)
        .and_then(|tag| tag.parent.clone());
    while let Some(parent) = at {
        if found.contains(&parent) {
            break;
        }
        at = dimension
            .tags
            .iter()
            .find(|tag| tag.id == parent)
            .and_then(|tag| tag.parent.clone());
        found.push(parent);
    }
    found
}

fn version_of(value: &impl Hash) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn describe_problems(
    path: &std::path::Path,
    problems: &[wreckordbox_sidecar::tag_schema::Problem],
) -> String {
    let first = problems
        .first()
        .map_or_else(String::new, |problem| match problem.line {
            Some(line) => format!(" (line {line}: {})", problem.message),
            None => format!(" ({})", problem.message),
        });
    format!(
        "{} has {} mistake{}{first}; the panel shows the Library's MyTags until it is fixed",
        path.display(),
        problems.len(),
        if problems.len() == 1 { "" } else { "s" },
    )
}

/// The Library's MyTag categories, in their order, each deck-facing. A
/// category with no tags is left out: there is nothing in it to set.
fn from_my_tags(my_tags: &[MyTagRow]) -> Vec<PanelDimension> {
    let catalogue = rbl_library::read::my_tag_catalogue(my_tags);
    catalogue
        .categories
        .iter()
        .map(|category| PanelDimension {
            id: category.id.clone(),
            label: category.name.clone(),
            deck: true,
            tree: false,
            tag_parents: false,
            single: false,
            edit: EDIT_MY_TAG.to_owned(),
            tags: catalogue
                .tags
                .iter()
                .filter(|tag| tag.parent == category.id)
                .map(|tag| PanelTag {
                    id: tag.id.clone(),
                    name: tag.name.clone(),
                    parent: None,
                    depth: 0,
                    my_tag: Some(tag.id.clone()),
                })
                .collect(),
            category: Some(category.id.clone()),
        })
        .filter(|dimension| !dimension.tags.is_empty())
        .collect()
}

/// The schema's deck categories in the deck's order, then its other
/// dimensions in the schema's order.
fn from_schema(
    schema: &TagSchema,
    my_tags: &[MyTagRow],
    sidecar_dir: &std::path::Path,
) -> Vec<PanelDimension> {
    // Which dimensions take one value: their kinds, as `facts.*` checks them.
    let kinds = Kinds::load_declared(sidecar_dir)
        .ok()
        .and_then(|kinds| kinds.with_schema(schema).ok());
    let single = |name: &str| {
        kinds
            .as_ref()
            .and_then(|kinds| kinds.get(name))
            .is_some_and(|kind| kind.value == ValueType::Text)
    };
    let mut dimensions = Vec::new();
    for view in schema.deck_view() {
        let Some(dimension) = schema.dimension(&view.dimension) else {
            continue;
        };
        let category = my_tags
            .iter()
            .find(|row| row.attribute == 1 && row.name == view.label);
        let my_tag_of = |text: &str| {
            category.and_then(|category| {
                my_tags
                    .iter()
                    .find(|row| row.attribute == 0 && row.parent == category.id && row.name == text)
                    .map(|row| row.id.clone())
            })
        };
        dimensions.push(PanelDimension {
            id: dimension.name.clone(),
            label: view.label.clone(),
            deck: true,
            tree: dimension.tree,
            tag_parents: dimension.tree && schema.deck.tag_parents,
            single: single(&dimension.name),
            edit: EDIT_MY_TAG.to_owned(),
            tags: view
                .tags
                .iter()
                .filter_map(|tag| {
                    let term = dimension.term(&tag.id)?;
                    Some(panel_tag(dimension, term, my_tag_of(&tag.label)))
                })
                .collect(),
            category: category.map(|row| row.id.clone()),
        });
    }
    for dimension in &schema.dimensions {
        if dimensions.iter().any(|shown| shown.id == dimension.name) {
            continue;
        }
        dimensions.push(PanelDimension {
            id: dimension.name.clone(),
            label: readable(&dimension.name),
            deck: false,
            tree: dimension.tree,
            tag_parents: false,
            single: single(&dimension.name),
            edit: EDIT_FACT.to_owned(),
            tags: tree_order(dimension)
                .into_iter()
                .map(|term| panel_tag(dimension, term, None))
                .collect(),
            category: None,
        });
    }
    dimensions
}

fn panel_tag(dimension: &SchemaDimension, term: &Term, my_tag: Option<String>) -> PanelTag {
    PanelTag {
        id: term.id.clone(),
        name: term.name.clone(),
        parent: term.parent.clone(),
        depth: dimension.depth(&term.id),
        my_tag,
    }
}

/// A dimension's terms depth first: each top term in written order, then
/// its children, as the deck lists the tree.
fn tree_order(dimension: &SchemaDimension) -> Vec<&Term> {
    fn under<'a>(dimension: &'a SchemaDimension, parent: Option<&str>, out: &mut Vec<&'a Term>) {
        for term in &dimension.terms {
            if term.parent.as_deref() == parent {
                out.push(term);
                under(dimension, Some(&term.id), out);
            }
        }
    }
    if !dimension.tree {
        return dimension.terms.iter().collect();
    }
    let mut out = Vec::new();
    under(dimension, None, &mut out);
    for term in &dimension.terms {
        if !out.iter().any(|shown| shown.id == term.id) {
            out.push(term);
        }
    }
    out
}

/// `set_role` -> `Set role`.
fn readable(name: &str) -> String {
    let spaced = name.replace('_', " ");
    let mut chars = spaced.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// The DJ's live Fact of each of `kinds` for the track: its id and value.
/// Read without ever being what creates, migrates or tidies a Sidecar:
/// through the service's writer when it is already open, else a read-only
/// snapshot.
fn djs_facts(
    stems: &Stems,
    track: &str,
    kinds: &[&str],
) -> Result<BTreeMap<String, (i64, Value)>, String> {
    let read = |facts: &dyn Fn(&str) -> Result<Vec<Fact>, String>| {
        let mut out = BTreeMap::new();
        for kind in kinds {
            let found = facts(kind)?;
            let newest = found
                .iter()
                .filter(|fact| fact.is_live() && fact.source.kind == SourceKind::Dj)
                .max_by_key(|fact| fact.id);
            if let Some(fact) = newest {
                out.insert((*kind).to_owned(), (fact.id, fact.value.clone()));
            }
        }
        Ok(out)
    };
    stems
        .with_current_sidecar(|open| match open {
            Some(sidecar) => read(&|kind| sidecar.facts(track, kind).map_err(|e| e.to_string())),
            None => match FactsSnapshot::open(stems.sidecar_dir()) {
                Ok(FactsSnapshot::Absent) => Ok(BTreeMap::new()),
                Ok(FactsSnapshot::Unknown(why)) => Err(why),
                Ok(FactsSnapshot::Open(reader)) => {
                    read(&|kind| reader.facts(track, kind).map_err(|e| e.to_string()))
                }
                Err(error) => Err(format!("the Sidecar could not be read: {error}")),
            },
        })
        .unwrap_or_else(|| Err("the Sidecar is being opened; try again".to_owned()))
}

/// A Fact's value as term ids: a list, or one text.
fn term_ids(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Value::String(text) => vec![text.clone()],
        _ => Vec::new(),
    }
}

fn refused(code: &str, message: impl Into<String>) -> ServiceError {
    ServiceError::new(code, message)
}

/// Sets one dimension of one track: see the module's documentation.
fn set_tags(
    library: &OpenLibrary,
    stems: &Stems,
    set: &SetTags,
) -> Result<(TagsSet, Option<LibraryChanged>), ServiceError> {
    let resolved = Resolved::read(library, stems)?;
    let stale = |why: &str| {
        refused(
            tag_codes::TAG_PANEL_STALE,
            format!("The tags changed since they were shown ({why}); they have been read again: click again."),
        )
    };
    let Some(dimension) = resolved.dimension(&set.dimension) else {
        return Err(stale("that dimension is no longer in the panel"));
    };
    let now = track_tags(library, stems, &resolved, &set.id)?;
    if set
        .version
        .as_ref()
        .is_some_and(|version| now.versions.get(&dimension.id) != Some(version))
    {
        return Err(stale(
            "the Tag schema or the track's tags are not what the panel showed",
        ));
    }
    if dimension.edit == EDIT_FACT {
        if let Some(why) = &now.facts_unavailable {
            return Err(refused(
                codes::SERVICE_FAILED,
                format!("The DJ's tags could not be read, so they were not changed: {why}"),
            ));
        }
    }
    // The selection, checked against the dimension as it is now.
    for term in &set.terms {
        if !dimension.tags.iter().any(|tag| &tag.id == term) {
            return Err(refused(
                tag_codes::TAG_NOT_IN_DIMENSION,
                format!("{} has no tag {term}", dimension.label),
            ));
        }
    }
    let selection = most_specific(dimension, &set.terms);
    if dimension.single && selection.len() > 1 {
        return Err(refused(
            tag_codes::TAG_NOT_IN_DIMENSION,
            format!("{} takes one tag", dimension.label),
        ));
    }
    let unmapped: Vec<String> = now
        .unmapped
        .get(&dimension.id)
        .map(|values| values.iter().map(|value| value.id.clone()).collect())
        .unwrap_or_default();
    for dropped in &set.drop_unmapped {
        if !unmapped.contains(dropped) {
            return Err(refused(
                tag_codes::TAG_NOT_IN_DIMENSION,
                format!("{} holds no value {dropped} to take off", dimension.label),
            ));
        }
    }
    let kept_unmapped: Vec<String> = unmapped
        .iter()
        .filter(|value| !set.drop_unmapped.contains(value))
        .cloned()
        .collect();
    let current = now.set.get(&dimension.id).cloned().unwrap_or_default();
    if selection == current && set.drop_unmapped.is_empty() {
        return Ok((
            TagsSet {
                changed: false,
                panel: panel_of(library, stems, &resolved, Some(&set.id))?,
                facts_not_recorded: Vec::new(),
            },
            None,
        ));
    }
    let changed = if dimension.edit == EDIT_MY_TAG {
        set_my_tags(
            library,
            dimension,
            &set.id,
            &selection,
            &set.drop_unmapped,
            set.comments,
        )?
    } else {
        set_fact(
            library,
            stems,
            dimension,
            &set.id,
            &selection,
            &kept_unmapped,
        )?
    };
    // What the Ledger could not record, said; then the panel as it is now.
    let facts_not_recorded = not_recorded(library, stems, &set.id, &dimension.id);
    stems.publish(
        stems::events::SIDECAR_CHANGED,
        json!(SidecarChanged {
            track_id: set.id.clone(),
            kind: dimension.id.clone(),
            fact_id: None,
            stale: Vec::new(),
        }),
    );
    let resolved = Resolved::read(library, stems)?;
    let panel = panel_of(library, stems, &resolved, Some(&set.id))?;
    let mut facts_not_recorded = facts_not_recorded;
    if dimension.edit == EDIT_FACT && facts_not_recorded.is_empty() {
        // The Ledger records only the DJ's own judgements: say so when it
        // recorded nothing (a client that did not say it is the DJ).
        let held = panel.track.as_ref().map(|track| {
            let mut held = track.set.get(&set.dimension).cloned().unwrap_or_default();
            held.extend(
                track
                    .unmapped
                    .get(&set.dimension)
                    .into_iter()
                    .flatten()
                    .map(|value| value.id.clone()),
            );
            held
        });
        let mut wanted = selection.clone();
        wanted.extend(kept_unmapped.iter().cloned());
        if held.as_ref() != Some(&wanted) {
            facts_not_recorded.push(
                "the DJ's Fact was not recorded: only a change made as the DJ is (a client says --source dj)".to_owned(),
            );
        }
    }
    Ok((
        TagsSet {
            changed: true,
            panel,
            facts_not_recorded,
        },
        Some(changed),
    ))
}

/// Writes a dimension that does not reach the deck as the DJ's Fact: one
/// [`LibraryEdit::Judgements`] step, whose Fact the Ledger records.
fn set_fact(
    library: &OpenLibrary,
    stems: &Stems,
    dimension: &PanelDimension,
    track: &str,
    selection: &[String],
    kept_unmapped: &[String],
) -> Result<LibraryChanged, ServiceError> {
    // A single value no tag stands for would be replaced: only when
    // the DJ said so.
    if dimension.single && !kept_unmapped.is_empty() && !selection.is_empty() {
        return Err(refused(
            tag_codes::TAG_NOT_SETTABLE,
            format!(
                "{} holds {}, which no tag stands for; take it off first",
                dimension.label,
                kept_unmapped.join(", ")
            ),
        ));
    }
    let before = djs_facts(stems, track, &[dimension.id.as_str()])
        .map_err(|why| refused(codes::SERVICE_FAILED, why))?
        .remove(&dimension.id)
        .map_or(Value::Null, |(_, value)| value);
    let after = if dimension.single {
        match (selection.first(), kept_unmapped.first()) {
            (Some(term), _) => json!(term),
            (None, Some(kept)) => json!(kept),
            (None, None) => Value::Null,
        }
    } else {
        let mut terms = selection.to_vec();
        terms.extend(kept_unmapped.iter().cloned());
        json!(terms)
    };
    let judgements = Judgements {
        changes: vec![JudgementChange {
            track: track.to_owned(),
            kind: dimension.id.clone(),
            before: before.to_string(),
            after: after.to_string(),
        }],
    };
    let ((), changed) =
        library.recorded_edit(Touched::Metadata(Vec::new()), TRACK_EDIT, move |_writer| {
            Ok(((), LibraryEdit::Judgements(judgements)))
        })?;
    Ok(changed)
}

/// Writes a deck-facing dimension's selection as the track's MyTags: the
/// selection's MyTags (with their parents' when the deck carries them),
/// none of the dimension's other mapped MyTags, its unmapped MyTags kept
/// unless dropped, every other category untouched. One MyTag edit.
fn set_my_tags(
    library: &OpenLibrary,
    dimension: &PanelDimension,
    track: &str,
    selection: &[String],
    drop_unmapped: &[String],
    comments: bool,
) -> Result<LibraryChanged, ServiceError> {
    let mut wanted: BTreeSet<String> = BTreeSet::new();
    for term in selection {
        let mut with: Vec<String> = vec![term.clone()];
        if dimension.tag_parents {
            with.extend(ancestors(dimension, term));
        }
        for id in with {
            let tag = dimension.tags.iter().find(|tag| tag.id == id);
            let Some(my_tag) = tag.and_then(|tag| tag.my_tag.clone()) else {
                return Err(refused(
                    tag_codes::TAG_NOT_SETTABLE,
                    format!(
                        "{} is not on the deck yet: the Library has no MyTag for it until the Tag schema is projected",
                        tag.map_or(id.as_str(), |tag| tag.name.as_str())
                    ),
                ));
            };
            wanted.insert(my_tag);
        }
    }
    let mapped: BTreeSet<String> = dimension
        .tags
        .iter()
        .filter_map(|tag| tag.my_tag.clone())
        .collect();
    let track = track.to_owned();
    let dropped: BTreeSet<String> = drop_unmapped.iter().cloned().collect();
    let ((), changed) = library.recorded_edit(Touched::Tracks, TRACK_EDIT, move |writer| {
        let held: Vec<String> = {
            let mut statement = writer.library().connection().prepare(
                "SELECT MyTagID FROM djmdSongMyTag WHERE ContentID = ?1 AND rb_local_deleted = 0 ORDER BY TrackNo, ID",
            )?;
            let rows = statement
                .query_map([&track], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            rows
        };
        let mut next: Vec<String> = held
            .iter()
            .filter(|id| !dropped.contains(*id))
            .filter(|id| !mapped.contains(*id) || wanted.contains(*id))
            .cloned()
            .collect();
        for id in &wanted {
            if !next.contains(id) {
                next.push(id.clone());
            }
        }
        let (_, edit) = writer.set_my_tags_with_undo(&track, &next)?;
        let tags = vec![edit];
        let comments = if comments {
            rbl_library::my_tags::refresh_comments(writer, std::slice::from_ref(&track), &tags)?
        } else {
            Vec::new()
        };
        Ok(((), LibraryEdit::TrackTags { tags, comments }))
    })?;
    Ok(changed)
}

/// The Ledger's reasons a DJ Fact of `kind` on `track` was not recorded by
/// the latest entry, when it said any.
fn not_recorded(library: &OpenLibrary, stems: &Stems, track: &str, kind: &str) -> Vec<String> {
    let Some(entry) = library.latest_ledger_entry() else {
        return Vec::new();
    };
    stems
        .with_current_sidecar(|open| {
            let Some(sidecar) = open else {
                return Vec::new();
            };
            let Ok(Some(entry)) = sidecar.ledger_entry(entry) else {
                return Vec::new();
            };
            entry
                .payload
                .as_ref()
                .and_then(|payload| payload.get("facts_not_recorded"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|omission| {
                    let on = omission["track"].as_str().unwrap_or_default();
                    let of = omission["kind"].as_str().unwrap_or_default();
                    (on.is_empty() || on == track)
                        && (of == kind || of == "mytags" || of.is_empty())
                })
                .map(|omission| {
                    omission["reason"]
                        .as_str()
                        .unwrap_or("not recorded")
                        .to_owned()
                })
                .collect()
        })
        .unwrap_or_default()
}
