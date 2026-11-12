//! The Tag schema (ADR 0006): the DJ's written vocabulary for describing
//! tracks, and which of it appears on the deck as the four MyTag
//! categories. A plain-text file, [`TAG_SCHEMA_FILE`], that the DJ edits
//! and versions, kept in the Sidecar's folder beside the kinds file.
//!
//! ```toml
//! version = 1
//!
//! # The vocabulary. The genre tree: a term with a parent is a child of
//! # it, and a track of the child is a track of the parent.
//! [genre]
//! terms = [
//!   { id = "house", name = "House" },
//!   { id = "house-deep", name = "Deep", parent = "house" },
//!   { id = "techno", name = "Techno" },
//!   { id = "techno-deep", name = "Deep", parent = "techno", deck = "> Deep (T)" },
//! ]
//!
//! # Any number of other dimensions, each a list of terms.
//! [dimensions.mood]
//! terms = [
//!   { id = "dark", name = "Dark" },
//!   { id = "warm", name = "Warm" },
//! ]
//!
//! # The deck view: which dimensions are MyTag categories, in order.
//! [deck]
//! categories = [
//!   { dimension = "mood", label = "Mood" },
//!   { dimension = "genre", label = "Sub-genre" },
//! ]
//!
//! [deck.genre]
//! child_prefix = "> "
//! tag_parents = true
//! ```
//!
//! **Terms.** One per line, so a change is one line of a diff. `id` is the
//! term's stable identifier: it never changes when the term is renamed or
//! moved, and it is what a Fact holds. `name` is what the DJ reads. `deck`
//! is the exact text of the term's MyTag when the rule below does not give
//! the text wanted. An identifier is unique within its dimension; a name is
//! unique among its siblings, so the same name may sit under two parents.
//!
//! **Dimensions and kinds.** A dimension's name is the name of its kind of
//! Fact: the genre tree's Facts are of kind `genre`, `[dimensions.mood]`'s
//! of kind `mood`. The value of such a Fact is a term's identifier, or a
//! list of them. A dimension whose kind is not declared elsewhere declares
//! it (a Judgement, a list of texts); see [`Kinds::with_schema`]. In the
//! tree a child implies its parents, so a Fact names the most specific
//! terms only.
//!
//! **The deck view.** At most [`MAX_DECK_CATEGORIES`] categories, each a
//! dimension under a label, in the order written. A category lists its
//! dimension's terms in the order they are written, after any named in its
//! own `order`. The tree is flattened depth first: each term, then its
//! children, a child's text being `child_prefix` once per level and then
//! its name. `tag_parents` says whether a track of a child also carries its
//! parents' MyTags on the deck. No two MyTags of one category may have the
//! same text.
//!
//! **Source genre names.** `[genre.sources.<source>]` maps a source's
//! genre names onto the tree (`"Tech House" = "house-tech"`). It is read
//! and its identifiers checked; nothing uses it yet.
//!
//! [`TagSchema::parse`] reads and checks a file and answers every
//! [`Problem`] it finds, each with its line. Nothing else may use a schema
//! that has not passed it.
//!
//! [`Kinds::with_schema`]: crate::Kinds::with_schema
#![allow(
    clippy::doc_markdown,
    reason = "MyTag is rekordbox's word for the thing, written as a word, not code"
)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};
use toml::Spanned;

/// The Tag schema's file in the Sidecar's folder.
pub const TAG_SCHEMA_FILE: &str = "tag-schema.toml";

/// The genre tree's name as a dimension, and the kind of its Facts.
pub const GENRE: &str = "genre";

/// How many MyTag categories rekordbox has.
pub const MAX_DECK_CATEGORIES: usize = 4;

/// How many MyTags rekordbox lets one category hold.
pub const MAX_CATEGORY_TAGS: usize = 50;

/// The prefix of a child's text on the deck when the file names none.
pub const DEFAULT_CHILD_PREFIX: &str = "> ";

/// The only version of the format.
pub const VERSION: i64 = 1;

/// One thing wrong with a Tag schema file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    /// The line of the file it is on, counted from 1, when it has one.
    pub line: Option<usize>,
    pub message: String,
}

impl Problem {
    fn at(line: Option<usize>, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(line) => write!(f, "line {line}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// One term of the vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Term {
    /// Its stable identifier, unique within its dimension.
    pub id: String,
    /// What the DJ reads.
    pub name: String,
    /// In the genre tree: the identifier of the term this is a child of.
    pub parent: Option<String>,
    /// The exact text of its MyTag, when the deck view's rule would give
    /// another.
    pub deck: Option<String>,
    /// A remark written after the term when the file is rendered. Not read
    /// back.
    pub note: Option<String>,
    /// The line of the file it was read from.
    pub line: Option<usize>,
}

impl Term {
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            parent: None,
            deck: None,
            note: None,
            line: None,
        }
    }
}

/// One dimension of the vocabulary: the genre tree or a list of terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dimension {
    /// Its name, which is the name of its kind of Fact.
    pub name: String,
    /// The genre tree, whose terms may have parents.
    pub tree: bool,
    pub description: String,
    /// In the order written.
    pub terms: Vec<Term>,
    /// For the genre tree: each source's genre names and the term each
    /// maps onto. Read and checked, not yet used.
    pub sources: BTreeMap<String, BTreeMap<String, String>>,
    /// The line of its heading in the file.
    pub line: Option<usize>,
}

impl Dimension {
    /// The term with this identifier.
    #[must_use]
    pub fn term(&self, id: &str) -> Option<&Term> {
        self.terms.iter().find(|term| term.id == id)
    }

    /// The identifiers of the terms `id` implies: its parent, its parent's
    /// parent and so on, nearest first. Empty outside the tree.
    #[must_use]
    pub fn ancestors(&self, id: &str) -> Vec<&str> {
        let mut found = Vec::new();
        let mut at = self.term(id).and_then(|term| term.parent.as_deref());
        while let Some(parent) = at {
            if found.contains(&parent) || found.len() > self.terms.len() {
                break;
            }
            found.push(parent);
            at = self.term(parent).and_then(|term| term.parent.as_deref());
        }
        found
    }

    /// How many levels down the tree `id` is: 0 for a term with no parent.
    #[must_use]
    pub fn depth(&self, id: &str) -> usize {
        self.ancestors(id).len()
    }
}

/// One MyTag category of the deck view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeckCategory {
    /// The dimension it shows.
    pub dimension: String,
    /// The category's name on the deck.
    pub label: String,
    /// Identifiers of terms to list first, in this order.
    pub order: Vec<String>,
    pub line: Option<usize>,
}

/// The deck view: what becomes MyTag categories and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deck {
    /// In the order the deck lists them.
    pub categories: Vec<DeckCategory>,
    /// What stands before a child's name, once per level.
    pub child_prefix: String,
    /// Whether a track of a child also carries its parents' MyTags.
    pub tag_parents: bool,
}

impl Default for Deck {
    fn default() -> Self {
        Self {
            categories: Vec::new(),
            child_prefix: DEFAULT_CHILD_PREFIX.to_owned(),
            tag_parents: true,
        }
    }
}

/// One MyTag as the deck view makes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckTag {
    /// The term's identifier.
    pub id: String,
    /// The MyTag's exact text.
    pub label: String,
}

/// One MyTag category as the deck view makes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckView {
    pub label: String,
    pub dimension: String,
    /// In the order the deck lists them.
    pub tags: Vec<DeckTag>,
}

/// A Tag schema. One read with [`TagSchema::parse`] has passed every
/// check; one built in code has not until [`TagSchema::problems`] is empty.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TagSchema {
    /// The genre tree first when there is one, then the others by name.
    pub dimensions: Vec<Dimension>,
    pub deck: Deck,
}

impl TagSchema {
    /// Reads the schema in `path` and checks it.
    pub fn load(path: &Path) -> Result<Self, Vec<Problem>> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| vec![Problem::at(None, format!("cannot be read: {e}"))])?;
        Self::parse(&text)
    }

    /// Reads a schema from the text of its file and checks it: every
    /// problem found, or the schema.
    pub fn parse(text: &str) -> Result<Self, Vec<Problem>> {
        let raw: RawFile = toml::from_str(text).map_err(|e| {
            let line = e.span().map(|span| line_of(text, span.start));
            vec![Problem::at(line, e.message().to_owned())]
        })?;
        let mut problems = Vec::new();
        match &raw.version {
            None => problems.push(Problem::at(
                None,
                format!("the file does not say which version of the format it is; add `version = {VERSION}`"),
            )),
            Some(version) if *version.get_ref() != VERSION => problems.push(Problem::at(
                Some(line_of(text, version.span().start)),
                format!(
                    "version {} is not one this wreckordbox reads; it reads version {VERSION}",
                    version.get_ref()
                ),
            )),
            Some(_) => {}
        }
        let mut dimensions = Vec::new();
        if let Some(genre) = raw.genre {
            dimensions.push(dimension(text, GENRE, true, genre, "[genre]"));
        }
        for (name, written) in raw.dimensions {
            let heading = format!("[dimensions.{name}]");
            if !written.sources.is_empty() {
                problems.push(Problem::at(
                    heading_line(text, &heading),
                    format!("{name}: only the genre tree maps source names (`[genre.sources.…]`)"),
                ));
            }
            dimensions.push(dimension(text, &name, false, written, &heading));
        }
        let mut deck = Deck::default();
        if let Some(written) = raw.deck {
            if let Some(genre) = written.genre {
                if let Some(prefix) = genre.child_prefix {
                    deck.child_prefix = prefix;
                }
                if let Some(tag_parents) = genre.tag_parents {
                    deck.tag_parents = tag_parents;
                }
            }
            deck.categories = written
                .categories
                .into_iter()
                .map(|category| {
                    let line = line_of(text, category.span().start);
                    let category = category.into_inner();
                    DeckCategory {
                        dimension: category.dimension,
                        label: category.label,
                        order: category.order,
                        line: Some(line),
                    }
                })
                .collect();
        }
        let schema = Self { dimensions, deck };
        problems.extend(schema.problems());
        if problems.is_empty() {
            Ok(schema)
        } else {
            problems.sort_by_key(|problem| problem.line.unwrap_or(0));
            Err(problems)
        }
    }

    /// The dimension called `name` (the genre tree is [`GENRE`]).
    #[must_use]
    pub fn dimension(&self, name: &str) -> Option<&Dimension> {
        self.dimensions
            .iter()
            .find(|dimension| dimension.name == name)
    }

    /// The genre tree, when the schema has one.
    #[must_use]
    pub fn genre(&self) -> Option<&Dimension> {
        self.dimensions.iter().find(|dimension| dimension.tree)
    }

    /// The MyTag categories and their MyTags as this schema's deck view
    /// makes them, in order. Meaningful for a schema that has passed its
    /// checks.
    #[must_use]
    pub fn deck_view(&self) -> Vec<DeckView> {
        self.deck
            .categories
            .iter()
            .filter_map(|category| {
                let dimension = self.dimension(&category.dimension)?;
                Some(DeckView {
                    label: category.label.clone(),
                    dimension: dimension.name.clone(),
                    tags: self.tags_of(dimension, &category.order),
                })
            })
            .collect()
    }

    /// The text of `term`'s MyTag.
    #[must_use]
    pub fn deck_label(&self, dimension: &Dimension, term: &Term) -> String {
        if let Some(text) = &term.deck {
            return text.clone();
        }
        let mut label = self.deck.child_prefix.repeat(dimension.depth(&term.id));
        label.push_str(&term.name);
        label
    }

    fn tags_of(&self, dimension: &Dimension, first: &[String]) -> Vec<DeckTag> {
        let listed: Vec<&Term> = if dimension.tree {
            let mut flat = Vec::new();
            flatten(dimension, None, &mut flat);
            flat
        } else {
            let named = first.iter().filter_map(|id| dimension.term(id));
            let rest = dimension
                .terms
                .iter()
                .filter(|term| !first.contains(&term.id));
            named.chain(rest).collect()
        };
        listed
            .into_iter()
            .map(|term| DeckTag {
                id: term.id.clone(),
                label: self.deck_label(dimension, term),
            })
            .collect()
    }

    /// Everything wrong with this schema. Empty for one that may be used.
    #[must_use]
    pub fn problems(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        for dimension in &self.dimensions {
            check_dimension(dimension, &mut problems);
        }
        self.check_deck(&mut problems);
        problems
    }

    fn check_deck(&self, problems: &mut Vec<Problem>) {
        let categories = &self.deck.categories;
        if categories.len() > MAX_DECK_CATEGORIES {
            problems.push(Problem::at(
                categories[MAX_DECK_CATEGORIES].line,
                format!(
                    "the deck view has {} categories; rekordbox has {MAX_DECK_CATEGORIES} MyTag categories, so at most {MAX_DECK_CATEGORIES} dimensions can be projected",
                    categories.len()
                ),
            ));
        }
        let mut dimensions_seen: HashMap<&str, &DeckCategory> = HashMap::new();
        let mut labels_seen: HashMap<&str, &DeckCategory> = HashMap::new();
        for category in categories {
            let label = &category.label;
            if label.trim().is_empty() {
                problems.push(Problem::at(
                    category.line,
                    format!(
                        "the category for {} has no label: its name on the deck",
                        category.dimension
                    ),
                ));
            } else if let Some(earlier) = labels_seen.insert(label, category) {
                problems.push(Problem::at(
                    category.line,
                    format!(
                        "two categories are labelled \"{label}\"{}",
                        also_on(earlier.line)
                    ),
                ));
            }
            if let Some(earlier) = dimensions_seen.insert(&category.dimension, category) {
                problems.push(Problem::at(
                    category.line,
                    format!(
                        "{} is a category twice{}",
                        category.dimension,
                        also_on(earlier.line)
                    ),
                ));
                continue;
            }
            let Some(dimension) = self.dimension(&category.dimension) else {
                let known: Vec<&str> = self
                    .dimensions
                    .iter()
                    .map(|dimension| dimension.name.as_str())
                    .collect();
                problems.push(Problem::at(
                    category.line,
                    format!(
                        "the category \"{label}\" shows the dimension {}, which the schema does not have. It has: {}",
                        category.dimension,
                        if known.is_empty() { "none".to_owned() } else { known.join(", ") }
                    ),
                ));
                continue;
            };
            self.check_category(category, dimension, problems);
        }
    }

    fn check_category(
        &self,
        category: &DeckCategory,
        dimension: &Dimension,
        problems: &mut Vec<Problem>,
    ) {
        let label = &category.label;
        if dimension.tree && !category.order.is_empty() {
            problems.push(Problem::at(
                category.line,
                format!("the category \"{label}\" shows the genre tree, which is listed as it is written; `order` is for the other dimensions"),
            ));
        }
        let mut ordered = HashSet::new();
        for id in &category.order {
            if dimension.term(id).is_none() {
                problems.push(Problem::at(
                    category.line,
                    format!(
                        "the order of the category \"{label}\" names {id}, which is not a term of {}",
                        dimension.name
                    ),
                ));
            } else if !ordered.insert(id) {
                problems.push(Problem::at(
                    category.line,
                    format!("the order of the category \"{label}\" names {id} twice"),
                ));
            }
        }
        if dimension.terms.len() > MAX_CATEGORY_TAGS {
            problems.push(Problem::at(
                category.line,
                format!(
                    "the category \"{label}\" would hold {} MyTags; rekordbox lets a category hold {MAX_CATEGORY_TAGS}",
                    dimension.terms.len()
                ),
            ));
        }
        // What the deck would show twice, rekordbox would refuse to make.
        let mut texts: HashMap<String, &Term> = HashMap::new();
        for term in &dimension.terms {
            let text = self.deck_label(dimension, term);
            if let Some(earlier) = texts.get(&text) {
                problems.push(Problem::at(
                    term.line,
                    format!(
                        "the category \"{label}\" would have two MyTags called \"{text}\": the terms {} and {}{}. Give one of them its own text with `deck = \"…\"`",
                        earlier.id,
                        term.id,
                        also_on(earlier.line)
                    ),
                ));
            } else {
                texts.insert(text, term);
            }
        }
    }

    /// The schema as the text of its file: one term per line, with the
    /// format explained at the top.
    #[must_use]
    pub fn to_toml(&self) -> String {
        let mut out = String::from(HEADING);
        let _ = writeln!(out, "version = {VERSION}");
        for dimension in &self.dimensions {
            out.push('\n');
            if dimension.tree {
                out.push_str("# The genre tree. A term with a parent is a child of it, and a track of the\n# child is a track of the parent.\n[genre]\n");
            } else {
                let _ = writeln!(out, "[dimensions.{}]", dimension.name);
            }
            if !dimension.description.is_empty() {
                let _ = writeln!(out, "description = {}", quoted(&dimension.description));
            }
            out.push_str("terms = [\n");
            for term in &dimension.terms {
                let _ = write!(
                    out,
                    "  {{ id = {}, name = {}",
                    quoted(&term.id),
                    quoted(&term.name)
                );
                if let Some(parent) = &term.parent {
                    let _ = write!(out, ", parent = {}", quoted(parent));
                }
                if let Some(deck) = &term.deck {
                    let _ = write!(out, ", deck = {}", quoted(deck));
                }
                out.push_str(" },");
                if let Some(note) = &term.note {
                    let _ = write!(out, " # {}", note.replace(['\n', '\r'], " "));
                }
                out.push('\n');
            }
            out.push_str("]\n");
            for (source, names) in &dimension.sources {
                let _ = writeln!(out, "\n[genre.sources.{}]", key(source));
                for (name, id) in names {
                    let _ = writeln!(out, "{} = {}", quoted(name), quoted(id));
                }
            }
        }
        out.push_str("\n# The deck view: the dimensions that are MyTag categories, in the deck's order.\n[deck]\ncategories = [\n");
        for category in &self.deck.categories {
            let _ = write!(
                out,
                "  {{ dimension = {}, label = {}",
                quoted(&category.dimension),
                quoted(&category.label)
            );
            if !category.order.is_empty() {
                let order: Vec<String> = category.order.iter().map(|id| quoted(id)).collect();
                let _ = write!(out, ", order = [{}]", order.join(", "));
            }
            out.push_str(" },\n");
        }
        out.push_str("]\n");
        if self.genre().is_some() {
            let _ = write!(
                out,
                "\n# How the genre tree is listed on the deck: what stands before a child's name,\n# and whether a track of a child also carries its parent's MyTag.\n[deck.genre]\nchild_prefix = {}\ntag_parents = {}\n",
                quoted(&self.deck.child_prefix),
                self.deck.tag_parents
            );
        }
        out
    }
}

const HEADING: &str = "\
# The Tag schema: the vocabulary for describing tracks, and which of it the
# deck shows as MyTag categories. Edit it by hand and keep it under version
# control; check it with `wreckordbox schema check`.
#
# A term is one line: its `id` (never change it: Facts hold it, and it is how
# a rename or a move is recognised), its `name`, in the genre tree its
# `parent`, and `deck` when its MyTag's text is not what the deck view makes.
# A dimension's name is the kind of its Facts.
";

/// The names of the dimensions a Tag schema's text has, the genre tree's
/// among them, whether or not the schema is one that can be used. `None`
/// when the text cannot be read even that far.
#[must_use]
pub fn dimension_names(text: &str) -> Option<Vec<String>> {
    let table: toml::Table = toml::from_str(text).ok()?;
    let mut names = Vec::new();
    if table.contains_key(GENRE) {
        names.push(GENRE.to_owned());
    }
    match table.get("dimensions") {
        None => {}
        Some(toml::Value::Table(dimensions)) => names.extend(dimensions.keys().cloned()),
        Some(_) => return None,
    }
    Some(names)
}

/// A dimension's name, or a kind's: lower-case letters, digits and
/// underscores, starting with a letter.
#[must_use]
pub fn is_dimension_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// A term's identifier: lower-case letters, digits, hyphens and
/// underscores, starting with a letter or a digit.
#[must_use]
pub fn is_term_id(id: &str) -> bool {
    id.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

fn check_dimension(dimension: &Dimension, problems: &mut Vec<Problem>) {
    let name = &dimension.name;
    if !dimension.tree {
        if name == GENRE {
            problems.push(Problem::at(
                dimension.line,
                "genre is the genre tree's name; write it as `[genre]`, not `[dimensions.genre]`",
            ));
        } else if !is_dimension_name(name) {
            problems.push(Problem::at(
                dimension.line,
                format!("a dimension's name is lower-case letters, digits and underscores, starting with a letter, not \"{name}\""),
            ));
        }
    }
    let mut ids: HashMap<&str, &Term> = HashMap::new();
    for term in &dimension.terms {
        let id = term.id.as_str();
        if !is_term_id(id) {
            problems.push(Problem::at(
                term.line,
                format!("{name}: a term's id is lower-case letters, digits, hyphens and underscores, not \"{id}\""),
            ));
        }
        if let Some(earlier) = ids.get(id) {
            problems.push(Problem::at(
                term.line,
                format!(
                    "{name}: two terms have the id {id}: \"{}\" and \"{}\"{}",
                    earlier.name,
                    term.name,
                    also_on(earlier.line)
                ),
            ));
        } else {
            ids.insert(id, term);
        }
        if term.name.trim().is_empty() {
            problems.push(Problem::at(
                term.line,
                format!("{name}: the term {id} has no name"),
            ));
        }
        if term.deck.as_ref().is_some_and(String::is_empty) {
            problems.push(Problem::at(
                term.line,
                format!("{name}: the term {id} has an empty `deck` text; leave `deck` out to use the term's name"),
            ));
        }
    }
    let mut siblings: HashMap<(Option<&str>, &str), &Term> = HashMap::new();
    for term in &dimension.terms {
        let id = &term.id;
        let mut parent_known = true;
        if let Some(parent) = &term.parent {
            if !dimension.tree {
                problems.push(Problem::at(
                    term.line,
                    format!("{name}: the term {id} has a parent, but only the genre tree's terms have parents"),
                ));
                continue;
            }
            if parent == id {
                problems.push(Problem::at(
                    term.line,
                    format!("genre: the term {id} is its own parent"),
                ));
                continue;
            }
            if !ids.contains_key(parent.as_str()) {
                parent_known = false;
                problems.push(Problem::at(
                    term.line,
                    format!("genre: the term {id} (\"{}\") is a child with no parent: there is no genre term with the id {parent}", term.name),
                ));
            } else if dimension.ancestors(id).contains(&id.as_str())
                || dimension.ancestors(id).len() > dimension.terms.len()
            {
                problems.push(Problem::at(
                    term.line,
                    format!(
                        "genre: the term {id} is its own ancestor: its parents lead back to it"
                    ),
                ));
                continue;
            }
        }
        if !parent_known || term.name.trim().is_empty() {
            continue;
        }
        if let Some(earlier) = siblings.insert((term.parent.as_deref(), term.name.as_str()), term) {
            let place = match &term.parent {
                Some(parent) => format!("under {parent}"),
                None if dimension.tree => "at the top of the tree".to_owned(),
                None => "in the dimension".to_owned(),
            };
            problems.push(Problem::at(
                term.line,
                format!(
                    "{name}: two terms {place} are called \"{}\": {} and {id}{}",
                    term.name,
                    earlier.id,
                    also_on(earlier.line)
                ),
            ));
        }
    }
    for (source, names) in &dimension.sources {
        for (written, id) in names {
            if !ids.contains_key(id.as_str()) {
                problems.push(Problem::at(
                    dimension.line,
                    format!("genre: the source {source} maps \"{written}\" onto {id}, which is not a genre term"),
                ));
            }
        }
    }
}

/// The tree's terms as the deck lists them: each term, then its children.
fn flatten<'a>(dimension: &'a Dimension, parent: Option<&str>, out: &mut Vec<&'a Term>) {
    for term in &dimension.terms {
        if term.parent.as_deref() == parent && !out.iter().any(|seen| seen.id == term.id) {
            out.push(term);
            flatten(dimension, Some(&term.id), out);
        }
    }
}

fn also_on(line: Option<usize>) -> String {
    line.map_or_else(String::new, |line| {
        format!(" (the other is on line {line})")
    })
}

fn line_of(text: &str, offset: usize) -> usize {
    let end = offset.min(text.len());
    text.get(..end).unwrap_or(text).matches('\n').count() + 1
}

fn heading_line(text: &str, heading: &str) -> Option<usize> {
    text.lines()
        .position(|line| line.trim_start().starts_with(heading))
        .map(|at| at + 1)
}

fn dimension(
    text: &str,
    name: &str,
    tree: bool,
    written: RawDimension,
    heading: &str,
) -> Dimension {
    Dimension {
        name: name.to_owned(),
        tree,
        description: written.description,
        terms: written
            .terms
            .into_iter()
            .map(|term| {
                let line = line_of(text, term.span().start);
                let term = term.into_inner();
                Term {
                    id: term.id,
                    name: term.name,
                    parent: term.parent,
                    deck: term.deck,
                    note: None,
                    line: Some(line),
                }
            })
            .collect(),
        sources: written.sources,
        line: heading_line(text, heading),
    }
}

/// A TOML basic string.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A TOML key: bare when it can be.
fn key(text: &str) -> String {
    let bare = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if bare {
        text.to_owned()
    } else {
        quoted(text)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    version: Option<Spanned<i64>>,
    genre: Option<RawDimension>,
    #[serde(default)]
    dimensions: BTreeMap<String, RawDimension>,
    deck: Option<RawDeck>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDimension {
    #[serde(default)]
    description: String,
    #[serde(default)]
    terms: Vec<Spanned<RawTerm>>,
    #[serde(default)]
    sources: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTerm {
    id: String,
    name: String,
    parent: Option<String>,
    deck: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDeck {
    #[serde(default)]
    categories: Vec<Spanned<RawCategory>>,
    genre: Option<RawGenreDeck>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCategory {
    dimension: String,
    label: String,
    #[serde(default)]
    order: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGenreDeck {
    child_prefix: Option<String>,
    tag_parents: Option<bool>,
}
