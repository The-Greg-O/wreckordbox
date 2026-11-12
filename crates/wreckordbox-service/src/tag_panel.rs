//! The Intake tag panel's description (wbx-65): every dimension of the DJ's
//! tags, in the order the panel shows them, which of them reach the deck as
//! MyTag categories, each one's tags, and optionally which of them one
//! track carries.
//!
//! With a Tag schema in the Sidecar's folder the description is the
//! schema's: its deck categories first, in the deck's order, then its other
//! dimensions. Without one (or with one that has a mistake, which is said)
//! it is the Library's MyTag categories, every one of them deck-facing,
//! through the same answer, so the window draws both alike.
//!
//! A deck-facing dimension is edited with the existing MyTag edits: each tag
//! names the MyTag it is in the Library (`my_tag`), matched by category name
//! and the MyTag's exact text, and `null` when the Library has no such MyTag
//! yet (the schema has not been projected). Any other dimension is the DJ's
//! Fact of that kind. Reading the description writes nothing: no Sidecar is
//! made, opened for writing, migrated or tidied.
#![allow(
    clippy::doc_markdown,
    reason = "MyTag is the glossary's word for the thing, not a code item"
)]

use std::collections::BTreeMap;

use rbl_db::export_info::MyTagRow;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wreckordbox_sidecar::tag_schema::Dimension as SchemaDimension;
use wreckordbox_sidecar::{Fact, FactsSnapshot, SourceKind, TagSchema, TAG_SCHEMA_FILE};

use crate::library::OpenLibrary;
use crate::protocol::{codes, Request, ServiceError};
use crate::stems::Stems;

/// The name of the command.
pub const INTAKE_TAG_PANEL: &str = "intake.tag_panel";

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
    /// In the order the panel shows them: deck-facing first.
    pub dimensions: Vec<PanelDimension>,
    /// The track asked about, if one was.
    pub track: Option<TrackTags>,
}

/// One dimension of the panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// How a tag is set: `my_tag` (the Library's MyTag edits) or `fact`
    /// (the DJ's Fact of the kind `id`).
    pub edit: String,
    /// In the order the panel lists them.
    pub tags: Vec<PanelTag>,
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

/// Which tags one track carries, by dimension `id`, as tag `id`s. In the
/// tree only what was set is listed; its parents are implied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackTags {
    pub id: String,
    pub set: BTreeMap<String, Vec<String>>,
    /// Why the DJ's Facts could not be read, when they could not: the
    /// vocabulary dimensions are then shown as unknown, not empty.
    pub facts_unavailable: Option<String>,
}

pub(crate) fn answers(command: &str) -> bool {
    command == INTAKE_TAG_PANEL
}

/// Answers `intake.tag_panel`.
pub(crate) fn answer(
    library: &OpenLibrary,
    stems: &Stems,
    request: &Request,
) -> Result<Value, ServiceError> {
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
    let panel = tag_panel(library, stems, &of)?;
    serde_json::to_value(&panel).map_err(|e| ServiceError::new(codes::SERVICE_FAILED, e.to_string()))
}

fn tag_panel(
    library: &OpenLibrary,
    stems: &Stems,
    of: &TagPanelOf,
) -> Result<TagPanel, ServiceError> {
    let index = library.index()?;
    let row = match &of.id {
        Some(id) => Some(index.row_of(id).ok_or_else(|| {
            ServiceError::new(codes::TRACK_NOT_FOUND, format!("no track has the id {id}"))
        })?),
        None => None,
    };
    let my_tags = library.with_db(|db| {
        rbl_db::export_info::my_tags(db.connection()).map_err(|e| {
            ServiceError::new(
                codes::SERVICE_FAILED,
                format!("the Library's MyTags could not be read: {e}"),
            )
        })
    })?;
    let path = stems.sidecar_dir().join(TAG_SCHEMA_FILE);
    let (schema, schema_problem) = match std::fs::metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (None, None),
        _ => match TagSchema::load(&path) {
            Ok(schema) => (Some(schema), None),
            Err(problems) => (None, Some(describe_problems(&path, &problems))),
        },
    };
    let (source, dimensions) = match &schema {
        Some(schema) => ("tag_schema", from_schema(schema, &my_tags)),
        None => ("my_tags", from_my_tags(&my_tags)),
    };
    let track = match (&of.id, row) {
        (Some(id), Some(row)) => {
            let carried = index.my_tag_keys(row as usize);
            let fact_kinds: Vec<&str> = dimensions
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
            let set = dimensions
                .iter()
                .map(|dimension| {
                    let set = if dimension.edit == EDIT_MY_TAG {
                        dimension
                            .tags
                            .iter()
                            .filter(|tag| {
                                tag.my_tag.as_deref().is_some_and(|my_tag| {
                                    carried.contains(&rbl_index::smart::my_tag_key(my_tag))
                                })
                            })
                            .map(|tag| tag.id.clone())
                            .collect()
                    } else {
                        // In the panel's order, and only tags it knows.
                        let held = facts.get(&dimension.id).cloned().unwrap_or_default();
                        dimension
                            .tags
                            .iter()
                            .filter(|tag| held.contains(&tag.id))
                            .map(|tag| tag.id.clone())
                            .collect()
                    };
                    (dimension.id.clone(), set)
                })
                .collect();
            Some(TrackTags {
                id: id.clone(),
                set,
                facts_unavailable,
            })
        }
        _ => None,
    };
    Ok(TagPanel {
        source: source.to_owned(),
        schema_problem,
        dimensions,
        track,
    })
}

const EDIT_MY_TAG: &str = "my_tag";
const EDIT_FACT: &str = "fact";

fn describe_problems(
    path: &std::path::Path,
    problems: &[wreckordbox_sidecar::tag_schema::Problem],
) -> String {
    let first = problems.first().map_or_else(String::new, |problem| match problem.line {
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
        })
        .filter(|dimension| !dimension.tags.is_empty())
        .collect()
}

/// The schema's deck categories in the deck's order, then its other
/// dimensions in the schema's order.
fn from_schema(schema: &TagSchema, my_tags: &[MyTagRow]) -> Vec<PanelDimension> {
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
            edit: EDIT_MY_TAG.to_owned(),
            tags: view
                .tags
                .iter()
                .filter_map(|tag| {
                    let term = dimension.term(&tag.id)?;
                    Some(panel_tag(dimension, term, my_tag_of(&tag.label)))
                })
                .collect(),
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
            edit: EDIT_FACT.to_owned(),
            tags: tree_order(dimension)
                .into_iter()
                .map(|term| panel_tag(dimension, term, None))
                .collect(),
        });
    }
    dimensions
}

fn panel_tag(
    dimension: &SchemaDimension,
    term: &wreckordbox_sidecar::tag_schema::Term,
    my_tag: Option<String>,
) -> PanelTag {
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
fn tree_order(dimension: &SchemaDimension) -> Vec<&wreckordbox_sidecar::tag_schema::Term> {
    fn under<'a>(
        dimension: &'a SchemaDimension,
        parent: Option<&str>,
        out: &mut Vec<&'a wreckordbox_sidecar::tag_schema::Term>,
    ) {
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
    // A term whose parent is missing would be refused by the check; keep
    // anything left over rather than lose it.
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

/// The DJ's live Fact of each of `kinds` for the track, as term ids. Read
/// without ever being what creates, migrates or tidies a Sidecar: through
/// the service's writer when it is already open, else a read-only snapshot.
fn djs_facts(
    stems: &Stems,
    track: &str,
    kinds: &[&str],
) -> Result<BTreeMap<String, Vec<String>>, String> {
    let read = |facts: &dyn Fn(&str) -> Result<Vec<Fact>, String>| {
        let mut out = BTreeMap::new();
        for kind in kinds {
            let found = facts(kind)?;
            let newest = found
                .iter()
                .filter(|fact| fact.is_live() && fact.source.kind == SourceKind::Dj)
                .max_by_key(|fact| fact.id);
            if let Some(fact) = newest {
                out.insert((*kind).to_owned(), term_ids(&fact.value));
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

/// A Fact's value as term ids: a list, or one text (a legacy single value).
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
