//! Projection of Energy and Prep colour (wbx-26, ADR 0006): standing Facts
//! become the native star rating and colour label, under the DJ's
//! projection policy.
//!
//! - `projection.plan` is the dry run: it reads the Library, the Sidecar and
//!   the policy and writes nothing at all (no Library edit, no Run, no
//!   Sidecar, no event).
//! - `projection.apply` computes the same plan with the same code and makes
//!   its changes as ONE recorded edit through the single edit path, in one
//!   Run by `tool:projection` (never the DJ). Before writing, under the edit
//!   gate, every change's `before` is checked against the Library; if
//!   anything moved since the plan was made the whole apply is refused and
//!   nothing is written. Given `expect` (a dry run's `plan`), it refuses
//!   unless it would do exactly what that dry run said.
//!
//! The DJ's values are never overwritten. A native value counts as set by
//! the DJ when it is set and is not the value a previous Projection wrote
//! (read from the Ledger: the latest `projection.apply` entry for that track
//! and field). Such a value is kept and, when a Fact disagrees, reported.
//! A standing Fact by the DJ is never written by a tool's Run: if it differs
//! from the Library it is held for the DJ.
//!
//! Cost: one query for each kind's Facts and one over the Ledger's
//! Projection entries, whatever the Library's size; no audio is read unless
//! the policy projects a Measurement kind, whose fingerprints must then be
//! checked (stale Measurements are never projected).
#![allow(
    clippy::doc_markdown,
    reason = "MyTag and the like are the glossary's words, not code items"
)]

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use rbl_db::DbError;
use rbl_library::history::LibraryEdit;
use rbl_library::Touched;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wreckordbox_sidecar::precedence::Rules;
use wreckordbox_sidecar::projection::Projected;
use wreckordbox_sidecar::{Fact, FactsSnapshot, Kinds, Nature, Source, SourceKind, ValueType};

use crate::intake::PrepColour;
use crate::library::OpenLibrary;
use crate::protocol::{codes as service_codes, LibraryChanged, Request, ServiceError};
use crate::stems::Stems;

/// The names of the Projection commands.
pub mod commands {
    /// The dry run: takes a [`Wanted`](super::Wanted), answers a
    /// [`Report`](super::Report). Writes nothing.
    pub const PROJECTION_PLAN: &str = "projection.plan";
    /// Applies the plan as one Run: takes a [`Wanted`](super::Wanted),
    /// answers a [`Report`](super::Report) of what it did.
    pub const PROJECTION_APPLY: &str = "projection.apply";
}

/// The error codes of the Projection commands.
pub mod codes {
    /// `projection.toml` cannot be read or has a mistake.
    pub const POLICY_INVALID: &str = "projection_policy_invalid";
    /// The plan is not what the dry run named (`expect`), or the Library
    /// changed while it was being applied: nothing was written.
    pub const PLAN_CHANGED: &str = "projection_changed";
    /// A Projection was asked to record its edits as the DJ's.
    pub const NOT_THE_DJ: &str = "projection_not_the_djs";
}

/// The DJ's policy file, beside the Tag schema in the Sidecar's folder.
pub const POLICY_FILE: &str = "projection.toml";
/// Who a Projection's edits are by when the request names no one.
pub const SOURCE_NAME: &str = "projection";
/// What the window's Undo calls an applied Projection.
pub const UNDO_LABEL: &str = "Projection";

const CHANGED_MARK: &str = "projection_changed: ";

/// Whether `command` is one of this module's.
pub(crate) fn answers(command: &str) -> bool {
    matches!(
        command,
        commands::PROJECTION_PLAN | commands::PROJECTION_APPLY
    )
}

/// The source a Projection's Run is by when the request names none.
pub(crate) fn default_source(command: &str) -> Option<Source> {
    (command == commands::PROJECTION_APPLY).then(|| Source::new(SourceKind::Tool, SOURCE_NAME))
}

/// The arguments of both commands.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Wanted {
    /// Only these tracks; every track with Facts when empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tracks: Vec<String>,
    /// For `projection.apply`: the `plan` of the dry run the DJ checked.
    /// The apply is refused unless it would make exactly those changes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expect: Option<String>,
}

/// A native field a Projection writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    /// Energy, in the star rating.
    Energy,
    /// Prep colour, the colour label.
    PrepColour,
}

impl Field {
    const ALL: [Self; 2] = [Self::Energy, Self::PrepColour];

    fn name(self) -> &'static str {
        match self {
            Self::Energy => "energy",
            Self::PrepColour => "prep_colour",
        }
    }

    fn column(self) -> &'static str {
        match self {
            Self::Energy => "Rating",
            Self::PrepColour => "ColorID",
        }
    }
}

/// What the policy says of one kind of change.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Apply {
    /// `"wait"`: every change of this kind is held for the DJ.
    Wait(WaitWord),
    /// A confidence from 0 to 1: standing Facts at or above it apply.
    AtOrAbove(f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitWord {
    Wait,
}

/// One kind of change in the policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub apply: Apply,
    /// The kind of Fact projected; the field's own kind when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyFile {
    energy: Option<Rule>,
    prep_colour: Option<Rule>,
}

/// The policy in force, per field; a field the file does not name waits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    pub energy: Rule,
    pub prep_colour: Rule,
}

impl Policy {
    fn rule(&self, field: Field) -> &Rule {
        match field {
            Field::Energy => &self.energy,
            Field::PrepColour => &self.prep_colour,
        }
    }

    /// Reads the policy without creating anything. No file: everything
    /// waits for the DJ.
    fn load(dir: &Path) -> Result<Self, ServiceError> {
        let path = dir.join(POLICY_FILE);
        let invalid = |why: String| {
            ServiceError::new(
                codes::POLICY_INVALID,
                format!("{} cannot be used, so nothing was projected: {why}", path.display()),
            )
        };
        let file: PolicyFile = match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|e| invalid(e.to_string()))?,
            Err(e)
                if e.kind() == std::io::ErrorKind::NotFound && path.symlink_metadata().is_err() =>
            {
                PolicyFile::default()
            }
            Err(e) => return Err(invalid(e.to_string())),
        };
        let wait = || Rule {
            apply: Apply::Wait(WaitWord::Wait),
            from: None,
        };
        let policy = Self {
            energy: file.energy.unwrap_or_else(wait),
            prep_colour: file.prep_colour.unwrap_or_else(wait),
        };
        for field in Field::ALL {
            if let Apply::AtOrAbove(at) = policy.rule(field).apply {
                if !(0.0..=1.0).contains(&at) {
                    return Err(invalid(format!(
                        "{}: apply is \"wait\" or a confidence from 0 to 1, not {at}",
                        field.name()
                    )));
                }
            }
        }
        Ok(policy)
    }
}

/// A change Projection makes (or, in a dry run, would make).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub track: String,
    pub title: String,
    pub field: Field,
    /// Energy 1 to 5, or the Prep colour as `{id, colour, label}`; `null`
    /// for none.
    pub before: Value,
    pub after: Value,
    /// The standing Fact projected.
    pub fact: i64,
    pub source: Source,
    pub confidence: f64,
}

/// A standing Fact held for the DJ instead of applied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Held {
    pub track: String,
    pub title: String,
    pub field: Field,
    /// What the Library holds now.
    pub before: Value,
    /// What the Fact would make it.
    pub after: Value,
    pub fact: i64,
    pub source: Source,
    pub confidence: f64,
    /// `waits_for_dj` (the policy waits), `below_confidence`, or `dj_fact`
    /// (the DJ's own Fact differs from the Library: a tool's Run does not
    /// write it).
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
}

/// A value the DJ set, left as it is although a Fact disagrees with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Kept {
    pub track: String,
    pub title: String,
    pub field: Field,
    /// The DJ's value, in the Library, kept.
    pub value: Value,
    /// What the disagreeing Fact says.
    pub fact_value: Value,
    pub fact: i64,
    pub source: Source,
    pub confidence: f64,
    /// `set_by_dj` (the Library's value is not one a Projection wrote) or
    /// `dj_fact` (the DJ's own Fact stands).
    pub reason: String,
}

/// A Fact that cannot be projected, and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Unusable {
    pub track: String,
    pub title: String,
    pub field: Field,
    pub fact: i64,
    pub value: Value,
    pub why: String,
}

/// What a projection did, or in a dry run would do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub dry_run: bool,
    /// Names exactly this list of changes: give it to `projection.apply`
    /// as `expect` to apply what was checked, or nothing.
    pub plan: String,
    pub policy_file: PathBuf,
    pub policy: Policy,
    /// Applied, or in a dry run to be applied, in track order.
    pub changes: Vec<Change>,
    /// Held for review.
    pub held: Vec<Held>,
    /// The DJ's values left alone, where a Fact disagrees.
    pub kept: Vec<Kept>,
    /// Facts whose value cannot be written to the field.
    pub invalid: Vec<Unusable>,
    /// Stale Measurements: never projected.
    pub stale: Vec<Unusable>,
    /// Tracks and fields whose standing Fact the Library already holds.
    pub unchanged: usize,
    /// Library Protection is on: applying would be refused.
    pub library_protected: bool,
}

/// Answers one of this module's commands, and what to tell every client
/// when it changed the Library (told even when the answer is a failure
/// part-way through).
pub(crate) fn answer(
    library: &OpenLibrary,
    stems: &Stems,
    request: &Request,
) -> (Option<LibraryChanged>, Result<Value, ServiceError>) {
    match plan_or_apply(library, stems, request) {
        Ok((report, changed)) => (changed, to_value(&report)),
        Err((changed, error)) => (changed, Err(error)),
    }
}

type Failure = (Option<LibraryChanged>, ServiceError);

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
    let dry_run = request.command == commands::PROJECTION_PLAN;
    if dry_run && wanted.expect.is_some() {
        return Err(early(ServiceError::new(
            service_codes::BAD_REQUEST,
            "expect is for projection.apply: a dry run is what makes a plan",
        )));
    }
    if !dry_run {
        refuse_the_djs(stems, request).map_err(early)?;
    }
    let report = plan(library, stems, &wanted, dry_run).map_err(early)?;
    if dry_run {
        return Ok((report, None));
    }
    if let Some(expected) = &wanted.expect {
        if *expected != report.plan {
            return Err(early(ServiceError::new(
                codes::PLAN_CHANGED,
                format!(
                    "the projection now is plan {} and not {expected}, the one checked: nothing was written; run the dry run again",
                    report.plan
                ),
            )));
        }
    }
    if report.changes.is_empty() {
        return Ok((report, None));
    }
    let changed = write(library, &report.changes)?;
    Ok((report, Some(changed)))
}

/// A Projection's edits are a tool's or an agent's, never the DJ's.
fn refuse_the_djs(stems: &Stems, request: &Request) -> Result<(), ServiceError> {
    let refused = || {
        ServiceError::new(
            codes::NOT_THE_DJ,
            "a Projection's edits are never recorded as the DJ's: name a tool or an agent, or no source",
        )
    };
    let Some(run) = &request.run else {
        return Ok(());
    };
    if run.source.as_ref().is_some_and(|s| s.kind == SourceKind::Dj) {
        return Err(refused());
    }
    if let (Some(id), None) = (run.id, &run.source) {
        let by_dj = stems
            .sidecar_if_made()?
            .and_then(|sidecar| sidecar.run(wreckordbox_sidecar::RunId(id)).ok().flatten())
            .is_some_and(|run| run.source.kind == SourceKind::Dj);
        if by_dj {
            return Err(refused());
        }
    }
    Ok(())
}

/// The Facts of each field's kind, and what earlier Projections wrote,
/// read without creating, migrating or tidying a Sidecar.
fn read_sidecar(
    stems: &Stems,
    kinds: &[(Field, String)],
) -> Result<(Vec<(Field, Vec<Fact>)>, Projected), ServiceError> {
    let failed = |why: String| ServiceError::new(crate::stems::codes::SIDECAR_FAILED, why);
    stems
        .with_sidecar_wait(|open| -> Result<_, String> {
            match open {
                Some(sidecar) => {
                    let mut facts = Vec::new();
                    for (field, kind) in kinds {
                        facts.push((*field, sidecar.facts_of_kind(kind).map_err(|e| e.to_string())?));
                    }
                    let projected = sidecar
                        .projected_by(commands::PROJECTION_APPLY)
                        .map_err(|e| e.to_string())?;
                    Ok((facts, projected))
                }
                None => match FactsSnapshot::open(stems.sidecar_dir()).map_err(|e| e.to_string())? {
                    FactsSnapshot::Absent => Ok((
                        kinds.iter().map(|(field, _)| (*field, Vec::new())).collect(),
                        Projected::new(),
                    )),
                    FactsSnapshot::Unknown(why) => Err(why),
                    FactsSnapshot::Open(reader) => {
                        let mut facts = Vec::new();
                        for (field, kind) in kinds {
                            facts.push((*field, reader.facts_of_kind(kind).map_err(|e| e.to_string())?));
                        }
                        let projected = reader
                            .projected_by(commands::PROJECTION_APPLY)
                            .map_err(|e| e.to_string())?;
                        Ok((facts, projected))
                    }
                },
            }
        })?
        .map_err(failed)
}

/// The kind each field projects from, checked against the declared kinds.
fn source_kinds(policy: &Policy, kinds: &Kinds, dir: &Path) -> Result<Vec<(Field, String)>, ServiceError> {
    let mut chosen = Vec::new();
    for field in Field::ALL {
        let name = policy
            .rule(field)
            .from
            .clone()
            .unwrap_or_else(|| field.name().to_owned());
        let invalid = |why: String| {
            ServiceError::new(
                codes::POLICY_INVALID,
                format!(
                    "{} cannot be used, so nothing was projected: {}: {why}",
                    dir.join(POLICY_FILE).display(),
                    field.name()
                ),
            )
        };
        let Some(declared) = kinds.get(&name) else {
            return Err(invalid(format!("{name} is not a declared kind of Fact")));
        };
        let fits = match field {
            Field::Energy => matches!(declared.value, ValueType::Integer | ValueType::Number),
            Field::PrepColour => declared.value == ValueType::Text,
        };
        if !fits {
            return Err(invalid(format!(
                "{name} takes {}, which cannot be written to {}",
                declared.value.name(),
                field.name()
            )));
        }
        chosen.push((field, name));
    }
    Ok(chosen)
}

/// The plan, computed the same way for a dry run and for applying.
fn plan(
    library: &OpenLibrary,
    stems: &Stems,
    wanted: &Wanted,
    dry_run: bool,
) -> Result<Report, ServiceError> {
    let dir = stems.sidecar_dir();
    let policy = Policy::load(dir)?;
    let kinds = Kinds::load(dir).map_err(|e| crate::facts::sidecar_error(&e))?;
    let rules = Rules::load(dir)
        .map_err(|why| ServiceError::new(crate::standing::RULES_INVALID, why))?;
    let chosen = source_kinds(&policy, &kinds, dir)?;
    let labels = crate::intake::labels(library)?;
    let index = library.index()?;
    for id in &wanted.tracks {
        if index.row_of(id).is_none() {
            return Err(ServiceError::new(
                service_codes::TRACK_NOT_FOUND,
                format!("there is no track {id} in the Library"),
            ));
        }
    }
    let (read, projected) = read_sidecar(stems, &chosen)?;
    // Facts by track, then field; tracks in the Library's order.
    let mut by_track: BTreeMap<u32, (String, Vec<(Field, Vec<Fact>)>)> = BTreeMap::new();
    for (field, facts) in read {
        let mut grouped: HashMap<String, Vec<Fact>> = HashMap::new();
        for fact in facts {
            grouped.entry(fact.track_id.clone()).or_default().push(fact);
        }
        for (track, facts) in grouped {
            if !wanted.tracks.is_empty() && !wanted.tracks.contains(&track) {
                continue;
            }
            // Facts about a track no longer in the Library project nowhere.
            let Some(row) = index.row_of(&track) else {
                continue;
            };
            by_track
                .entry(row)
                .or_insert_with(|| (track, Vec::new()))
                .1
                .push((field, facts));
        }
    }
    let mut report = Report {
        dry_run,
        plan: String::new(),
        policy_file: dir.join(POLICY_FILE),
        policy: policy.clone(),
        changes: Vec::new(),
        held: Vec::new(),
        kept: Vec::new(),
        invalid: Vec::new(),
        stale: Vec::new(),
        unchanged: 0,
        library_protected: library.protection().protect_library,
    };
    for (row, (track, mut fields)) in by_track {
        fields.sort_by_key(|(field, _)| *field);
        let title = index.title.get(row as usize).to_owned();
        let mut audio_checked = None;
        for (field, mut facts) in fields {
            let unusable = |fact: &Fact, why: String| Unusable {
                track: track.clone(),
                title: title.clone(),
                field,
                fact: fact.id,
                value: fact.value.clone(),
                why,
            };
            // A Measurement is projected only from the audio it measured.
            if facts
                .iter()
                .any(|fact| fact.nature == Nature::Measurement && fact.is_live())
            {
                let current = audio_checked.get_or_insert_with(|| {
                    let found = crate::stems::audio_at(&index, row);
                    crate::stems::fingerprint(found.parent(), found.path()).map_err(|e| e.message)
                });
                for fact in &mut facts {
                    if fact.nature != Nature::Measurement || !fact.is_live() {
                        continue;
                    }
                    match current {
                        Ok(print) => {
                            fact.stale_at = (fact.fingerprint.as_ref() != Some(&*print)).then_some(0);
                        }
                        Err(why) => {
                            report.invalid.push(unusable(
                                fact,
                                format!("its audio could not be checked: {why}"),
                            ));
                            // Not eligible: unverified is not current.
                            fact.stale_at = Some(0);
                            fact.superseded_at = Some(0);
                        }
                    }
                }
            }
            for fact in facts.iter().filter(|f| f.is_live() && f.is_stale()) {
                report.stale.push(unusable(
                    fact,
                    "a Measurement of other audio than the track's: never projected".into(),
                ));
            }
            let native = native(&index, row, field, &labels);
            let (winner, _) = rules.standing(&facts);
            let Some(winner) = winner else { continue };
            let target = match target(field, &winner.value, &labels) {
                Ok(target) => target,
                Err(why) => {
                    report.invalid.push(unusable(winner, why));
                    continue;
                }
            };
            if winner.source.kind == SourceKind::Dj {
                if target != native {
                    report.held.push(Held {
                        track: track.clone(),
                        title: title.clone(),
                        field,
                        before: native,
                        after: target,
                        fact: winner.id,
                        source: winner.source.clone(),
                        confidence: winner.confidence,
                        reason: "dj_fact".into(),
                        threshold: None,
                    });
                    continue;
                }
                report.unchanged += 1;
                // The DJ's value stands; say so where another source disagrees.
                let others: Vec<Fact> = facts
                    .iter()
                    .filter(|f| f.source.kind != SourceKind::Dj)
                    .cloned()
                    .collect();
                if let (Some(other), _) = rules.standing(&others) {
                    if let Ok(value) = crate::projection::target(field, &other.value, &labels) {
                        if value != native {
                            report.kept.push(Kept {
                                track: track.clone(),
                                title: title.clone(),
                                field,
                                value: native.clone(),
                                fact_value: value,
                                fact: other.id,
                                source: other.source.clone(),
                                confidence: other.confidence,
                                reason: "dj_fact".into(),
                            });
                        }
                    }
                }
                continue;
            }
            if target == native {
                report.unchanged += 1;
                continue;
            }
            let stored = projected.get(&(track.clone(), field.column().to_owned()));
            if !native.is_null() && stored.map(stored_number) != Some(native_number(&native)) {
                report.kept.push(Kept {
                    track: track.clone(),
                    title: title.clone(),
                    field,
                    value: native,
                    fact_value: target,
                    fact: winner.id,
                    source: winner.source.clone(),
                    confidence: winner.confidence,
                    reason: "set_by_dj".into(),
                });
                continue;
            }
            let held = |reason: &str, threshold| Held {
                track: track.clone(),
                title: title.clone(),
                field,
                before: native.clone(),
                after: target.clone(),
                fact: winner.id,
                source: winner.source.clone(),
                confidence: winner.confidence,
                reason: reason.into(),
                threshold,
            };
            match policy.rule(field).apply {
                Apply::Wait(_) => report.held.push(held("waits_for_dj", None)),
                Apply::AtOrAbove(at) if winner.confidence < at => {
                    report.held.push(held("below_confidence", Some(at)));
                }
                Apply::AtOrAbove(_) => report.changes.push(Change {
                    track: track.clone(),
                    title: title.clone(),
                    field,
                    before: native,
                    after: target,
                    fact: winner.id,
                    source: winner.source.clone(),
                    confidence: winner.confidence,
                }),
            }
        }
    }
    report.plan = digest(&report.changes);
    Ok(report)
}

/// What the Library holds now for `field`, in the report's form.
fn native(index: &rbl_index::Library, row: u32, field: Field, labels: &[PrepColour]) -> Value {
    let at = row as usize;
    match field {
        Field::Energy => match index.rating.get(at).copied().unwrap_or(0) {
            0 => Value::Null,
            stars => json!(stars),
        },
        Field::PrepColour => {
            let id = index.color.get(at).copied().unwrap_or(0);
            labels
                .iter()
                .find(|colour| colour.id == id)
                .map_or(Value::Null, |colour| json!(colour))
        }
    }
}

/// The value a Fact would give `field`, in the report's form.
pub(crate) fn target(field: Field, value: &Value, labels: &[PrepColour]) -> Result<Value, String> {
    match field {
        Field::Energy => value
            .as_f64()
            .filter(|n| n.fract() == 0.0 && (1.0..=5.0).contains(n))
            .map(|n| json!(n as u8))
            .ok_or_else(|| format!("Energy is a whole number from 1 to 5, not {value}")),
        Field::PrepColour => {
            let text = value
                .as_str()
                .ok_or_else(|| format!("a Prep colour is a name, not {value}"))?;
            match crate::intake::named_colour(labels, text) {
                Ok(Some(colour)) => Ok(json!(colour)),
                Ok(None) => Err("a Prep colour Fact of none is not projected".into()),
                Err(e) => Err(e.message),
            }
        }
    }
}

/// The number a report value stands for: stars, or a colour label's id.
fn native_number(value: &Value) -> i64 {
    value
        .as_i64()
        .or_else(|| value["id"].as_i64())
        .unwrap_or(0)
}

/// The number a Ledger value stands for (`Rating` is stored as a number,
/// `ColorID` as text); none is 0.
fn stored_number(value: &Value) -> i64 {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
        .unwrap_or(0)
}

/// A name for exactly this list of changes, the same in every build.
fn digest(changes: &[Change]) -> String {
    let named: Vec<Value> = changes
        .iter()
        .map(|c| json!([c.track, c.field, c.before, c.after, c.fact]))
        .collect();
    let bytes = serde_json::to_vec(&named).unwrap_or_default();
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{hash:016x}")
}

/// Makes `changes` as one recorded edit through the single edit path.
/// Every change's `before` is checked first, under the edit gate, against
/// the Library as it is: a mismatch refuses the whole apply before anything
/// is written.
fn write(library: &OpenLibrary, changes: &[Change]) -> Result<LibraryChanged, Failure> {
    let mut ids: Vec<String> = changes.iter().map(|c| c.track.clone()).collect();
    ids.dedup();
    let result = library.recorded_edit(Touched::Metadata(ids), UNDO_LABEL, |writer| {
        let now = library
            .index()
            .map_err(|e| DbError::WriteRefused(e.message))?;
        for change in changes {
            let row = now.row_of(&change.track).ok_or_else(|| {
                DbError::WriteRefused(format!("{CHANGED_MARK}track {} is gone", change.track))
            })?;
            let current = match change.field {
                Field::Energy => i64::from(now.rating.get(row as usize).copied().unwrap_or(0)),
                Field::PrepColour => i64::from(now.color.get(row as usize).copied().unwrap_or(0)),
            };
            if current != native_number(&change.before) {
                return Err(DbError::WriteRefused(format!(
                    "{CHANGED_MARK}the {} of track {} changed while the projection was being made",
                    change.field.name(),
                    change.track
                )));
            }
        }
        let mut done = Vec::new();
        let mut failure = None;
        for change in changes {
            let wrote = match change.field {
                Field::Energy => writer.set_rating_with_undo(
                    &change.track,
                    u8::try_from(native_number(&change.after)).unwrap_or(0),
                ),
                Field::PrepColour => writer.set_color_with_undo(
                    &change.track,
                    Some(&native_number(&change.after).to_string()),
                ),
            };
            match wrote {
                Ok((_, edit)) => done.push(edit),
                Err(e) if done.is_empty() => return Err(e),
                // What was written stays recorded, so it can be undone and
                // unwound; the failure is reported, not hidden.
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
        }
        Ok((failure, LibraryEdit::Track(done)))
    });
    match result {
        Ok((None, changed)) => Ok(changed),
        Ok((Some(failure), changed)) => Err((Some(changed), ServiceError::new(
            service_codes::WRITE_REFUSED,
            format!("the projection stopped part-way; what it wrote is recorded in its Run and can be undone: {failure}"),
        ))),
        Err(e) if e.message.starts_with(CHANGED_MARK) => Err((
            None,
            ServiceError::new(
                codes::PLAN_CHANGED,
                format!(
                    "{}: nothing was written; run it again",
                    e.message.trim_start_matches(CHANGED_MARK)
                ),
            ),
        )),
        Err(e) => Err((None, e)),
    }
}

fn to_value<T: Serialize>(value: &T) -> Result<Value, ServiceError> {
    serde_json::to_value(value)
        .map_err(|e| ServiceError::new(service_codes::SERVICE_FAILED, e.to_string()))
}
