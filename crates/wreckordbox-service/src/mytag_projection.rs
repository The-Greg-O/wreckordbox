//! Projection of the Tag schema to native MyTags (wbx-29, ADR 0006): the
//! deck view's categories, with their tags, put on each track from the
//! standing Facts of their dimensions, under the DJ's projection policy.
//!
//! - `projection.mytags.plan` is the dry run: it reads the Library, the
//!   Sidecar, the Tag schema and the policy and writes nothing at all (no
//!   Library edit, no Run, no Sidecar, no mark, no event).
//! - `projection.mytags.apply` computes the same plan with the same code
//!   and makes its assignment changes as ONE recorded edit through the
//!   single edit path (one Undo step, one Ledger `track_tags` entry, one
//!   `library_changed`, the host told), in a Run of its own by
//!   `tool:projection` or the tool or agent the request names: never the
//!   DJ's (`projection::attribution`, `ledger::Keeper::target`). Given
//!   `expect` (a dry run's `plan`), it refuses unless it would do exactly
//!   that. It refuses while the plan holds a change to the MyTags
//!   themselves (a tag the schema has and the Library does not): those are
//!   listed in `definitions` and not yet applied by this service.
//!
//! **What is matched.** A deck category is the Library's MyTag category of
//! the same name; a deck tag is the MyTag of exactly the same text in it
//! (`DeckTag.label`). Nothing is matched by position. A Library category the
//! deck view does not name (`outside_deck`) and a dimension the deck view
//! leaves out are never written; neither is a Library tag the schema does
//! not have.
//!
//! **What is wanted.** For each track and deck dimension, the standing Fact
//! of the dimension's kind (the ordinary precedence, so the DJ's own Fact
//! always stands over an agent's) names terms; in the tree, each brings its
//! ancestors too when `[deck.genre] tag_parents` is true (`implied`: no Fact
//! of its own is needed or made). A track with no standing Fact for a
//! dimension has no wish for its category: its MyTags there are left alone.
//!
//! **Who owns an assignment** (one track and one MyTag) is decided by
//! provenance, never by comparing values or clocks
//! ([`wreckordbox_sidecar::mytag_projection::TagProvenance`]): the owner is
//! the source of the effective latest recorded write to that assignment in
//! the Ledger, following Undo and Redo to the originating edit. A projection
//! may put a tag on a track only where it is off, no recorded write ever
//! touched that assignment (or an Undo took it back to before any), no row
//! of it was ever written that the Ledger cannot explain (a soft-deleted
//! row: the DJ took it off in rekordbox), and no incomplete provenance (an
//! unsettled intent, an unconfirmed write, an unreadable entry) could
//! concern the track. It may change an assignment that is on, or one it
//! took off, only when the latest recorded write is a projection's own AND
//! the assignment's `djmdSongMyTag` rows are exactly the rows that write
//! read back in its own transaction (its [`TagMark`]) AND no barrier came
//! after it. Anything else is the DJ's and is kept, and reported where a
//! Fact disagrees. So an existing Library, which has no Ledger history for
//! its MyTags, is entirely the DJ's: converting it (`schema convert`) makes
//! the DJ's Facts and no native write, so a projection after it wants
//! exactly what is there and changes nothing; what it may do later is add
//! the MyTags new Facts call for where no tag was ever set, and change again
//! only what it itself put on or took off.
//!
//! The apply re-checks under the edit gate, before anything is written: no
//! MyTag entry newer than the plan concerns a planned track, and every
//! planned assignment's rows are still the rows the plan read; then every
//! track's changes are written by `rbl_db`'s checked writer
//! ([`rbl_db::write::Writer::set_my_tags_if_unchanged`]), which compares the
//! rows again and writes the whole batch in one immediate transaction:
//! all of it or none of it. The intent of the write
//! names the planned tracks (`ledger::name_tracks`), so a crash part-way is
//! a barrier bounded to those tracks, never to every track.
//!
//! The policy file is the DJ's, but nothing authenticates it: every plan
//! and every Run names its hash, and the Tag schema's.
#![allow(
    clippy::doc_markdown,
    reason = "MyTag and the like are the glossary's words, not code items"
)]

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use rbl_db::write::checked::{TagChanges, TagRow};
use rbl_db::DbError;
use rbl_library::history::LibraryEdit;
use rbl_library::Touched;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wreckordbox_sidecar::mytag_projection::{TagMark, TagProvenance};
use wreckordbox_sidecar::precedence::Rules;
use wreckordbox_sidecar::projection::{Barrier, FieldState, Owner};
use wreckordbox_sidecar::tag_schema::{DeckView, Dimension};
use wreckordbox_sidecar::{Fact, FactsSnapshot, Source, TagSchema, TAG_SCHEMA_FILE};

use crate::library::OpenLibrary;
use crate::projection::{hash_of, Apply, Wanted, POLICY_FILE};
use crate::protocol::{codes as service_codes, LibraryChanged, Request, ServiceError};
use crate::stems::Stems;

/// The names of MyTag Projection's commands.
pub mod commands {
    /// The dry run: takes a [`Wanted`](crate::projection::Wanted), answers
    /// a [`Report`](super::Report). Writes nothing.
    pub const PLAN: &str = "projection.mytags.plan";
    /// Applies the plan as one Run.
    pub const APPLY: &str = "projection.mytags.apply";
}

/// The error codes of MyTag Projection, beside `projection::codes`.
pub mod codes {
    /// The plan holds changes to the MyTags themselves (a tag to make),
    /// which this service does not apply yet: nothing was written.
    pub const DEFINITIONS_PENDING: &str = "projection_definitions_pending";
}

/// What the window's Undo calls an applied MyTag projection.
pub const UNDO_LABEL: &str = "MyTag Projection";

const CHANGED_MARK: &str = "projection_changed: ";

/// Whether `command` is one of this module's.
pub(crate) fn answers(command: &str) -> bool {
    matches!(command, commands::PLAN | commands::APPLY)
}

/// The policy's `[mytags]` section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub apply: Apply,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyFile {
    // Energy's and Prep colour's sections are `projection`'s to check.
    #[allow(dead_code, reason = "read and checked by the Energy projection")]
    energy: Option<toml::Value>,
    #[allow(dead_code, reason = "read and checked by the Energy projection")]
    prep_colour: Option<toml::Value>,
    mytags: Option<Rule>,
}

fn invalid_policy(dir: &Path, why: &str) -> ServiceError {
    ServiceError::new(
        crate::projection::codes::POLICY_INVALID,
        format!(
            "{} cannot be used, so nothing was projected: {why}",
            dir.join(POLICY_FILE).display()
        ),
    )
}

/// The `[mytags]` rule and the hash of the policy file's bytes. No file or
/// no section: every change waits for the DJ.
fn policy(dir: &Path) -> Result<(Rule, String), ServiceError> {
    let bytes =
        crate::projection::policy_bytes(dir).map_err(|e| invalid_policy(dir, &e.to_string()))?;
    let text = String::from_utf8(bytes.clone()).map_err(|e| invalid_policy(dir, &e.to_string()))?;
    let file: PolicyFile =
        toml::from_str(&text).map_err(|e| invalid_policy(dir, &e.to_string()))?;
    let rule = file.mytags.unwrap_or(Rule {
        apply: Apply::Wait(crate::projection::WaitWord::Wait),
    });
    if let Apply::AtOrAbove(at) = rule.apply {
        if !(0.0..=1.0).contains(&at) {
            return Err(invalid_policy(
                dir,
                &format!("mytags: apply is \"wait\" or a confidence from 0 to 1, not {at}"),
            ));
        }
    }
    Ok((rule, hash_of(&bytes)))
}

/// A deck category and the Library category it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Category {
    pub label: String,
    pub dimension: String,
    /// The Library's category of that name; `null` when there is none.
    pub category_id: Option<String>,
    /// `matched`, `missing` (no Library category has the label: its
    /// assignments are `blocked`) or `ambiguous` (several have it).
    pub state: String,
}

/// A Library category the deck view does not name: never written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outside {
    pub category_id: String,
    pub category: String,
}

/// A change to the MyTags themselves that the schema calls for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Definition {
    /// `create_tag`: the category has no MyTag with this text.
    pub change: String,
    pub category: String,
    pub category_id: String,
    pub label: String,
    pub dimension: String,
    pub term: String,
}

/// One assignment: a change made (or to be made), held, kept or blocked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assignment {
    pub track: String,
    pub title: String,
    pub category: String,
    #[serde(default)]
    pub category_id: Option<String>,
    /// The MyTag; `null` when there is none to put on (`blocked`).
    #[serde(default)]
    pub tag: Option<String>,
    /// The MyTag's text, as the deck shows it.
    pub label: String,
    pub dimension: String,
    pub term: String,
    /// `add` or `remove`.
    pub change: String,
    /// Put on as the parent of a term the Fact names, not by the Fact.
    pub implied: bool,
    /// The standing Fact of the dimension, which wants it.
    pub fact: i64,
    pub source: Source,
    pub confidence: f64,
    /// Why it is held (`waits_for_dj`, `below_confidence`), kept
    /// (`set_by_dj`, `set_by_other`, `no_record`, `unrecorded_write`, `undone`,
    /// `written_since`, `provenance_incomplete`, `unknown`) or blocked
    /// (`no_category`, `no_my_tag`, `ambiguous_my_tag`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
}

/// A Fact that cannot be projected, and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Unusable {
    pub track: String,
    pub title: String,
    pub dimension: String,
    pub fact: i64,
    pub value: Value,
    pub why: String,
}

/// What a MyTag projection did, or in a dry run would do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub dry_run: bool,
    /// Names exactly these changes under this policy and schema: give it
    /// to `projection.mytags.apply` as `expect`.
    pub plan: String,
    pub policy_file: PathBuf,
    pub policy_hash: String,
    pub policy: Rule,
    pub schema_file: PathBuf,
    pub schema_hash: String,
    /// The deck view's categories, in its order.
    pub categories: Vec<Category>,
    pub outside_deck: Vec<Outside>,
    /// Changes to the MyTags themselves the schema calls for.
    pub definitions: Vec<Definition>,
    /// Assignments changed, or in a dry run to be changed, in track order.
    pub changes: Vec<Assignment>,
    /// Held for the DJ.
    pub held: Vec<Assignment>,
    /// Assignments a projection does not own, left as they are.
    pub kept: Vec<Assignment>,
    /// Assignments with nowhere to go: no category, or no MyTag yet.
    pub blocked: Vec<Assignment>,
    pub invalid: Vec<Unusable>,
    /// Assignments that are already as the standing Facts want them.
    pub unchanged: usize,
    pub library_protected: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// A planned change with what the apply checks again under the edit gate.
struct Planned {
    track: String,
    tag: String,
    on: bool,
    /// The assignment's rows when the plan was made.
    rows: Vec<TagRow>,
}

/// The Library's MyTags as the plan needs them.
struct Native {
    /// `(id, name)` in the deck's order.
    categories: Vec<(String, String)>,
    /// By category id: `(tag id, text)` in order.
    tags: HashMap<String, Vec<(String, String)>>,
    /// By `(track, tag)`: every row, live or not.
    rows: HashMap<(String, String), Vec<TagRow>>,
}

fn read_native(library: &OpenLibrary) -> Result<Native, ServiceError> {
    library.with_db(|db| {
        let failed = |e: DbError| {
            ServiceError::new(
                service_codes::SERVICE_FAILED,
                format!("the Library's MyTags could not be read: {e}"),
            )
        };
        let catalogue_rows = rbl_db::export_info::my_tags(db.connection()).map_err(failed)?;
        let catalogue = rbl_library::read::my_tag_catalogue(&catalogue_rows);
        let categories = catalogue
            .categories
            .iter()
            .map(|c| (c.id.clone(), c.name.clone()))
            .collect();
        let mut tags = HashMap::new();
        for category in &catalogue.categories {
            tags.insert(
                category.id.clone(),
                catalogue
                    .tags_of(category)
                    .map(|t| (t.id.clone(), t.name.clone()))
                    .collect(),
            );
        }
        let mut rows: HashMap<(String, String), Vec<TagRow>> = HashMap::new();
        for (track, tag, row) in
            rbl_db::write::checked::my_tag_rows(db.connection()).map_err(failed)?
        {
            rows.entry((track, tag)).or_default().push(row);
        }
        Ok(Native {
            categories,
            tags,
            rows,
        })
    })
}

/// The Facts of each deck dimension's kind and the provenance of
/// assignments, read without creating, migrating or tidying a Sidecar.
fn read_sidecar(
    stems: &Stems,
    kinds: &[String],
) -> Result<(HashMap<String, Vec<Fact>>, TagProvenance), ServiceError> {
    let failed = |why: String| ServiceError::new(crate::stems::codes::SIDECAR_FAILED, why);
    stems
        .with_sidecar_wait(|open| -> Result<_, String> {
            let text = |e: wreckordbox_sidecar::SidecarError| e.to_string();
            let mut facts = HashMap::new();
            match open {
                Some(sidecar) => {
                    for kind in kinds {
                        facts.insert(kind.clone(), sidecar.facts_of_kind(kind).map_err(text)?);
                    }
                    Ok((facts, sidecar.tag_provenance().map_err(text)?))
                }
                None => match FactsSnapshot::open(stems.sidecar_dir()).map_err(text)? {
                    FactsSnapshot::Absent => Ok((facts, TagProvenance::default())),
                    FactsSnapshot::Unknown(why) => Err(why),
                    FactsSnapshot::Open(reader) => {
                        for kind in kinds {
                            facts.insert(kind.clone(), reader.facts_of_kind(kind).map_err(text)?);
                        }
                        Ok((facts, reader.tag_provenance().map_err(text)?))
                    }
                },
            }
        })?
        .map_err(failed)
}

/// The Tag schema and the hash of its bytes, read without creating
/// anything.
fn schema(dir: &Path) -> Result<(TagSchema, String), ServiceError> {
    let path = dir.join(TAG_SCHEMA_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ServiceError::new(
                crate::schema::codes::TAG_SCHEMA_MISSING,
                format!(
                    "there is no Tag schema at {}, so there is nothing to project: `wreckordbox schema convert` makes one from the MyTags",
                    path.display()
                ),
            ))
        }
        Err(e) => {
            return Err(ServiceError::new(
                crate::schema::codes::TAG_SCHEMA_UNREADABLE,
                format!("{} cannot be read: {e}", path.display()),
            ))
        }
    };
    let text = String::from_utf8_lossy(&bytes);
    let schema = TagSchema::parse(&text).map_err(|problems| {
        let first = problems
            .first()
            .map(|p| match p.line {
                Some(line) => format!("line {line}: {}", p.message),
                None => p.message.clone(),
            })
            .unwrap_or_default();
        ServiceError::new(
            crate::schema::codes::TAG_SCHEMA_INVALID,
            format!(
                "{} has {} mistake(s), so nothing was projected; the first: {first}",
                path.display(),
                problems.len()
            ),
        )
    })?;
    Ok((schema, hash_of(&bytes)))
}

/// The terms a Fact's value names: a list of term identifiers, or one.
fn terms_of(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::String(one) => Some(vec![one.clone()]),
        Value::Array(list) => list.iter().map(|t| t.as_str().map(str::to_owned)).collect(),
        _ => None,
    }
}

type Failure = (Option<LibraryChanged>, ServiceError);

/// Answers one of this module's commands, and what to tell every client
/// when it changed the Library (told even when the answer then failed).
pub(crate) fn answer(
    library: &OpenLibrary,
    stems: &Stems,
    request: &Request,
) -> (Option<LibraryChanged>, Result<Value, ServiceError>) {
    match plan_or_apply(library, stems, request) {
        Ok((report, changed)) => (
            changed,
            serde_json::to_value(&report)
                .map_err(|e| ServiceError::new(service_codes::SERVICE_FAILED, e.to_string())),
        ),
        Err((changed, error)) => (changed, Err(error)),
    }
}

fn plan_or_apply(
    library: &OpenLibrary,
    stems: &Stems,
    request: &Request,
) -> Result<(Report, Option<LibraryChanged>), Failure> {
    let early = |error: ServiceError| (None, error);
    let wanted: Wanted = if request.args.is_null() {
        Wanted::default()
    } else {
        crate::facts::arguments(request).map_err(early)?
    };
    let dry_run = request.command == commands::PLAN;
    if dry_run && wanted.expect.is_some() {
        return Err(early(ServiceError::new(
            service_codes::BAD_REQUEST,
            "expect is for projection.mytags.apply: a dry run is what makes a plan",
        )));
    }
    let (report, planned, watermark) = plan(library, stems, &wanted, dry_run).map_err(early)?;
    if dry_run {
        return Ok((report, None));
    }
    if let Some(expected) = &wanted.expect {
        if *expected != report.plan {
            return Err(early(ServiceError::new(
                crate::projection::codes::PLAN_CHANGED,
                format!(
                    "the MyTag projection now is plan {} and not {expected}, the one checked: nothing was written; run the dry run again",
                    report.plan
                ),
            )));
        }
    }
    if !report.definitions.is_empty() {
        return Err(early(ServiceError::new(
            codes::DEFINITIONS_PENDING,
            format!(
                "the Tag schema has {} MyTag(s) the Library does not (listed in the dry run's definitions); this projection does not make MyTags yet, so nothing was written: make them by hand (`wreckordbox mytags create`) and project again",
                report.definitions.len()
            ),
        )));
    }
    if planned.is_empty() {
        return Ok((report, None));
    }
    crate::projection::pause();
    let (changed, warnings) = write(library, stems, &planned, watermark)?;
    let mut report = report;
    report.warnings = warnings;
    Ok((report, Some(changed)))
}

/// One track and deck category being decided.
struct Deciding<'a> {
    track: &'a str,
    title: &'a str,
    view: &'a DeckView,
    dimension: &'a Dimension,
    category: Option<&'a str>,
    category_state: &'a str,
}

/// The plan, computed the same way for a dry run and for applying, with
/// what the apply checks again and the Ledger watermark it was read at.
#[allow(
    clippy::too_many_lines,
    reason = "one pass over the deck view, in order"
)]
fn plan(
    library: &OpenLibrary,
    stems: &Stems,
    wanted: &Wanted,
    dry_run: bool,
) -> Result<(Report, Vec<Planned>, i64), ServiceError> {
    let dir = stems.sidecar_dir();
    let (rule, policy_hash) = policy(dir)?;
    if let Some(expected) = &wanted.policy_hash {
        if *expected != policy_hash {
            return Err(ServiceError::new(
                crate::projection::codes::PLAN_CHANGED,
                format!(
                    "{} changed while the projection was being made (policy {policy_hash}, not {expected}): nothing was written",
                    dir.join(POLICY_FILE).display()
                ),
            ));
        }
    }
    let (schema, schema_hash) = schema(dir)?;
    let rules =
        Rules::load(dir).map_err(|why| ServiceError::new(crate::standing::RULES_INVALID, why))?;
    let index = library.index()?;
    for id in &wanted.tracks {
        if index.row_of(id).is_none() {
            return Err(ServiceError::new(
                service_codes::TRACK_NOT_FOUND,
                format!("there is no track {id} in the Library"),
            ));
        }
    }
    let native = read_native(library)?;
    let views = schema.deck_view();
    let kinds: Vec<String> = views.iter().map(|v| v.dimension.clone()).collect();
    let (facts, provenance) = read_sidecar(stems, &kinds)?;

    let mut report = Report {
        dry_run,
        plan: String::new(),
        policy_file: dir.join(POLICY_FILE),
        policy_hash,
        policy: rule.clone(),
        schema_file: dir.join(TAG_SCHEMA_FILE),
        schema_hash,
        categories: Vec::new(),
        outside_deck: Vec::new(),
        definitions: Vec::new(),
        changes: Vec::new(),
        held: Vec::new(),
        kept: Vec::new(),
        blocked: Vec::new(),
        invalid: Vec::new(),
        unchanged: 0,
        library_protected: library.protection().protect_library,
        warnings: Vec::new(),
    };
    // Each deck category's Library category, by name.
    let mut placed: Vec<(Option<String>, &'static str)> = Vec::new();
    for view in &views {
        let named: Vec<&(String, String)> = native
            .categories
            .iter()
            .filter(|(_, name)| *name == view.label)
            .collect();
        let (id, state) = match named.as_slice() {
            [one] => (Some(one.0.clone()), "matched"),
            [] => (None, "missing"),
            _ => (None, "ambiguous"),
        };
        report.categories.push(Category {
            label: view.label.clone(),
            dimension: view.dimension.clone(),
            category_id: id.clone(),
            state: state.into(),
        });
        placed.push((id, state));
    }
    for (id, name) in &native.categories {
        if !placed
            .iter()
            .any(|(placed, _)| placed.as_deref() == Some(id))
        {
            report.outside_deck.push(Outside {
                category_id: id.clone(),
                category: name.clone(),
            });
        }
    }
    for (view, (category, _)) in views.iter().zip(&placed) {
        let Some(category) = category else { continue };
        let there = native.tags.get(category).cloned().unwrap_or_default();
        for tag in &view.tags {
            if !there.iter().any(|(_, text)| *text == tag.label) {
                report.definitions.push(Definition {
                    change: "create_tag".into(),
                    category: view.label.clone(),
                    category_id: category.clone(),
                    label: tag.label.clone(),
                    dimension: view.dimension.clone(),
                    term: tag.id.clone(),
                });
            }
        }
    }

    // Facts by track, in the Library's order.
    let mut by_track: BTreeMap<u32, String> = BTreeMap::new();
    for list in facts.values() {
        for fact in list {
            if !wanted.tracks.is_empty() && !wanted.tracks.contains(&fact.track_id) {
                continue;
            }
            if let Some(row) = index.row_of(&fact.track_id) {
                by_track.insert(row, fact.track_id.clone());
            }
        }
    }
    let mut grouped: HashMap<(String, String), Vec<Fact>> = HashMap::new();
    for (kind, list) in &facts {
        for fact in list {
            grouped
                .entry((fact.track_id.clone(), kind.clone()))
                .or_default()
                .push(fact.clone());
        }
    }
    let mut planned = Vec::new();
    for (row, track) in &by_track {
        let title = index.title.get(*row as usize).to_owned();
        for (view, (category, state)) in views.iter().zip(&placed) {
            let Some(dimension) = schema.dimension(&view.dimension) else {
                continue;
            };
            let Some(list) = grouped.get(&(track.clone(), view.dimension.clone())) else {
                continue;
            };
            let deciding = Deciding {
                track,
                title: &title,
                view,
                dimension,
                category: category.as_deref(),
                category_state: state,
            };
            decide(
                &mut report,
                &mut planned,
                &deciding,
                list,
                &rules,
                &rule,
                &native,
                &provenance,
                schema.deck.tag_parents,
            );
        }
    }
    report.plan = digest(&report);
    Ok((report, planned, provenance.watermark))
}

#[allow(
    clippy::too_many_arguments,
    reason = "the plan's state, passed down once"
)]
fn decide(
    report: &mut Report,
    planned: &mut Vec<Planned>,
    one: &Deciding<'_>,
    facts: &[Fact],
    rules: &Rules,
    rule: &Rule,
    native: &Native,
    provenance: &TagProvenance,
    tag_parents: bool,
) {
    let (winner, _) = rules.standing(facts);
    let Some(winner) = winner else { return };
    let unusable = |why: String| Unusable {
        track: one.track.to_owned(),
        title: one.title.to_owned(),
        dimension: one.view.dimension.clone(),
        fact: winner.id,
        value: winner.value.clone(),
        why,
    };
    let Some(terms) = terms_of(&winner.value) else {
        report.invalid.push(unusable(format!(
            "a {} Fact is a list of the schema's term identifiers, not {}",
            one.view.dimension, winner.value
        )));
        return;
    };
    if let Some(unknown) = terms.iter().find(|t| one.dimension.term(t).is_none()) {
        report.invalid.push(unusable(format!(
            "{unknown:?} is not a term of {} in the Tag schema",
            one.view.dimension
        )));
        return;
    }
    // What the Fact names, and what it brings with it in the tree.
    let mut implied: Vec<String> = Vec::new();
    if one.dimension.tree && tag_parents {
        for term in &terms {
            for ancestor in one.dimension.ancestors(term) {
                if !terms.iter().any(|t| t == ancestor) && !implied.iter().any(|t| t == ancestor) {
                    implied.push(ancestor.to_owned());
                }
            }
        }
    }
    let wants = |id: &str| terms.iter().any(|t| t == id) || implied.iter().any(|t| t == id);
    let there = one
        .category
        .and_then(|c| native.tags.get(c))
        .cloned()
        .unwrap_or_default();
    for deck_tag in &one.view.tags {
        let want = wants(&deck_tag.id);
        let mut listed = Assignment {
            track: one.track.to_owned(),
            title: one.title.to_owned(),
            category: one.view.label.clone(),
            category_id: one.category.map(str::to_owned),
            tag: None,
            label: deck_tag.label.clone(),
            dimension: one.view.dimension.clone(),
            term: deck_tag.id.clone(),
            change: if want { "add" } else { "remove" }.into(),
            implied: implied.contains(&deck_tag.id),
            fact: winner.id,
            source: winner.source.clone(),
            confidence: winner.confidence,
            reason: None,
            threshold: None,
        };
        let blocked = |mut listed: Assignment, reason: &str| {
            listed.reason = Some(reason.into());
            listed
        };
        if one.category.is_none() {
            if want {
                let why = if one.category_state == "ambiguous" {
                    "ambiguous_category"
                } else {
                    "no_category"
                };
                report.blocked.push(blocked(listed, why));
            }
            continue;
        }
        let matching: Vec<&(String, String)> = there
            .iter()
            .filter(|(_, text)| *text == deck_tag.label)
            .collect();
        let tag = match matching.as_slice() {
            [one] => one.0.clone(),
            [] => {
                if want {
                    report.blocked.push(blocked(listed, "no_my_tag"));
                }
                continue;
            }
            _ => {
                if want {
                    report.blocked.push(blocked(listed, "ambiguous_my_tag"));
                }
                continue;
            }
        };
        listed.tag = Some(tag.clone());
        let key = (one.track.to_owned(), tag.clone());
        let rows = native.rows.get(&key).cloned().unwrap_or_default();
        let on = rows.iter().any(|row| row.live);
        if want == on {
            if want {
                report.unchanged += 1;
            }
            continue;
        }
        if let Some(reason) = not_owned(
            one.track,
            on,
            &rows,
            provenance.assignments.get(&key),
            provenance.marks.get(&key),
            &provenance.barriers,
        ) {
            listed.reason = Some(reason.into());
            report.kept.push(listed);
            continue;
        }
        match rule.apply {
            Apply::Wait(_) => {
                listed.reason = Some("waits_for_dj".into());
                report.held.push(listed);
            }
            Apply::AtOrAbove(at) if winner.confidence < at => {
                listed.reason = Some("below_confidence".into());
                listed.threshold = Some(at);
                report.held.push(listed);
            }
            Apply::AtOrAbove(_) => {
                planned.push(Planned {
                    track: one.track.to_owned(),
                    tag,
                    on: want,
                    rows,
                });
                report.changes.push(listed);
            }
        }
    }
}

/// Why a projection may not change this assignment: `None` when it may.
/// See the module's documentation; no clock is read.
fn not_owned(
    track: &str,
    on: bool,
    rows: &[TagRow],
    state: Option<&FieldState>,
    mark: Option<&TagMark>,
    barriers: &[Barrier],
) -> Option<&'static str> {
    let barred = |after: i64, cleared: bool| {
        barriers.iter().any(|barrier| {
            barrier.seq > after && barrier.concerns(track) && (cleared || barrier.cleared.is_none())
        })
    };
    let fill = || {
        if barred(0, false) {
            Some("provenance_incomplete")
        } else {
            None
        }
    };
    match state.map(|state| &state.owner) {
        // Never recorded: a tag taken off in rekordbox leaves its row.
        None if !on && rows.iter().any(|row| !row.live) => Some("unrecorded_write"),
        // Back to before any record by an Undo, with rows left: what the
        // Undo left cannot be told from a later add and remove in rekordbox
        // (nothing keeps the Undo's own rows), so the absence is the DJ's;
        // the Undo was the DJ's act too (wbx-29 review).
        Some(Owner::Original) if !on && !rows.is_empty() => Some("undone"),
        None | Some(Owner::Original) if !on => fill(),
        None | Some(Owner::Original) => Some("no_record"),
        Some(Owner::Unknown) => Some("unknown"),
        Some(Owner::Dj) => Some("set_by_dj"),
        Some(Owner::Other(_)) => Some("set_by_other"),
        Some(Owner::Projection) => {
            let Some(mark) = mark else {
                return Some("written_since");
            };
            // Evidence only for the confirmed write it was made for: that
            // entry must be the assignment's latest recorded write. A write
            // cut off before its entry left a mark that proves nothing,
            // whatever barrier is cleared (wbx-29 review).
            if state.map(|state| state.entry) != Some(mark.entry) {
                return Some("written_since");
            }
            let same: Vec<_> = rows.iter().map(as_mark).collect();
            if mark.rows != same || mark.on != on {
                Some("written_since")
            } else if barred(mark.seq, true) {
                Some("provenance_incomplete")
            } else {
                None
            }
        }
    }
}

fn as_mark(row: &TagRow) -> wreckordbox_sidecar::mytag_projection::TagRow {
    wreckordbox_sidecar::mytag_projection::TagRow {
        id: row.id.clone(),
        usn: row.usn,
        updated_at: row.updated_at.clone(),
        live: row.live,
    }
}

/// A name for exactly these changes under this policy and schema.
fn digest(report: &Report) -> String {
    let named: Vec<Value> = report
        .changes
        .iter()
        .map(|c| json!([c.track, c.tag, c.change, c.fact]))
        .collect();
    let defined: Vec<Value> = report
        .definitions
        .iter()
        .map(|d| json!([d.change, d.category_id, d.label]))
        .collect();
    hash_of(
        &serde_json::to_vec(&json!([
            report.policy_hash,
            report.schema_hash,
            named,
            defined
        ]))
        .unwrap_or_default(),
    )
}

/// Makes the planned changes as one recorded edit through the single edit
/// path, each track's by the checked writer, after checking under the edit
/// gate that nothing the plan read has changed.
fn write(
    library: &OpenLibrary,
    stems: &Stems,
    planned: &[Planned],
    watermark: i64,
) -> Result<(LibraryChanged, Vec<String>), Failure> {
    let mut warnings = Vec::new();
    // By track, in the plan's order.
    let mut tracks: Vec<String> = Vec::new();
    for p in planned {
        if !tracks.contains(&p.track) {
            tracks.push(p.track.clone());
        }
    }
    // The write's intent names exactly these tracks: a crash part-way is a
    // barrier bounded to them.
    crate::ledger::name_tracks(&tracks);
    let refused = |why: String| DbError::WriteRefused(format!("{CHANGED_MARK}{why}"));
    let result = library.recorded_edit(Touched::Tracks, UNDO_LABEL, |writer| {
        let newer = stems
            .with_sidecar_wait(|open| {
                open.map_or(Ok(false), |sidecar| sidecar.tags_written_since(watermark, &tracks))
            })
            .map_err(|e| DbError::WriteRefused(e.message))?
            .map_err(|e| DbError::WriteRefused(e.to_string()))?;
        if newer {
            return Err(refused(
                "a track's MyTags were edited while the projection was being made".into(),
            ));
        }
        let now: HashMap<(String, String), Vec<TagRow>> =
            rbl_db::write::checked::my_tag_rows(writer.library().connection())?
                .into_iter()
                .fold(HashMap::new(), |mut all, (track, tag, row)| {
                    all.entry((track, tag)).or_default().push(row);
                    all
                });
        for p in planned {
            let key = (p.track.clone(), p.tag.clone());
            if now.get(&key).cloned().unwrap_or_default() != p.rows {
                return Err(refused(format!(
                    "track {}'s MyTags were written while the projection was being made",
                    p.track
                )));
            }
        }
        hold_before_write();
        // Every track's changes in one checked transaction: written whole,
        // or refused with nothing written.
        let batch: Vec<TagChanges> = tracks
            .iter()
            .map(|track| {
                let mine: Vec<&Planned> = planned.iter().filter(|p| p.track == *track).collect();
                TagChanges {
                    content: track.clone(),
                    changes: mine.iter().map(|p| (p.tag.clone(), p.on)).collect(),
                    expected: mine.iter().map(|p| (p.tag.clone(), p.rows.clone())).collect(),
                }
            })
            .collect();
        let (_, written) = writer
            .set_my_tags_if_unchanged(&batch)
            .map_err(|e| match e {
                DbError::Conflict(why) => refused(why),
                other => other,
            })?;
        let mut left: Vec<((String, String), TagMark)> = Vec::new();
        let mut done = Vec::with_capacity(written.len());
        for (track, written) in batch.iter().zip(written) {
            for (tag, on) in &track.changes {
                let rows = written.rows.get(tag).cloned().unwrap_or_default();
                left.push((
                    (track.content.clone(), tag.clone()),
                    TagMark {
                        rows: rows.iter().map(as_mark).collect(),
                        on: *on,
                        seq: 0,
                        entry: 0,
                    },
                ));
            }
            done.push(written.edit);
        }
        if let Err(why) = keep_marks(stems, left) {
            warnings.push(format!(
                "the rows this projection left could not be kept ({why}): the MyTags it wrote count as the DJ's"
            ));
        }
        Ok((
            (),
            LibraryEdit::TrackTags {
                tags: done,
                comments: Vec::new(),
            },
        ))
    });
    match result {
        Ok(((), made)) => Ok((made, warnings)),
        Err(e) if e.message.starts_with(CHANGED_MARK) => Err((
            None,
            ServiceError::new(
                crate::projection::codes::PLAN_CHANGED,
                format!(
                    "{}: nothing was written; run it again",
                    e.message.trim_start_matches(CHANGED_MARK)
                ),
            ),
        )),
        Err(e) => Err((None, e)),
    }
}

/// A folder in which `projection.mytags.apply`, under the edit gate after
/// its last check and before the checked transaction, makes a file
/// `checked` and waits until a file `write` appears (at most 30 seconds),
/// so a test can write from another connection at exactly that point. Read
/// only in test mode.
pub const HOLD_ENV: &str = "WRECKORDBOX_MYTAG_PROJECTION_HOLD";

fn hold_before_write() {
    if !rbl_db::test_mode() {
        return;
    }
    let Some(dir) = std::env::var_os(HOLD_ENV).map(PathBuf::from) else {
        return;
    };
    let _ = std::fs::write(dir.join("checked"), b"");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !dir.join("write").exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Keeps the marks of what was written, at the Ledger's position now.
fn keep_marks(stems: &Stems, mut marks: Vec<((String, String), TagMark)>) -> Result<(), String> {
    if marks.is_empty() {
        return Ok(());
    }
    stems
        .with_sidecar_wait(|open| -> Result<(), String> {
            let sidecar = open.ok_or("the Sidecar is not open")?;
            let seq = sidecar.ledger_position().map_err(|e| e.to_string())?;
            // The entry this write's record will be: its intent's id.
            let entry = sidecar
                .open_intent_of(commands::APPLY)
                .map_err(|e| e.to_string())?
                .ok_or("this write has no intent in the Ledger")?;
            for (_, mark) in &mut marks {
                mark.seq = seq;
                mark.entry = entry;
            }
            sidecar.set_tag_marks(&marks).map_err(|e| e.to_string())
        })
        .map_err(|e| e.message)?
}
