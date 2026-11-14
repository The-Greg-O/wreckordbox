//! Projection of Energy and Prep colour (wbx-26, ADR 0006): standing Facts
//! become the native star rating and colour label, under the DJ's
//! projection policy.
//!
//! - `projection.plan` is the dry run: it reads the Library, the Sidecar and
//!   the policy and writes nothing at all (no Library edit, no Run, no
//!   Sidecar, no event).
//! - `projection.apply` computes the same plan with the same code and makes
//!   its changes as ONE recorded edit through the single edit path, in a
//!   Run of its own by `tool:projection` or the agent or tool the request
//!   names: never the DJ's, even when the hosting window asks
//!   (`ledger::Keeper::target`). Given `expect` (a dry run's `plan`), it
//!   refuses unless it would do exactly what that dry run said.
//!
//! **Who owns a native field** is decided by provenance, never by comparing
//! values ([`wreckordbox_sidecar::projection::Provenance`]): the owner is
//! the source of the effective latest recorded write to that field in the
//! Ledger, following Undo and Redo. A projection may replace a value only
//! when that write is a projection's own and the track's row is still at
//! exactly the revision (USN and `updated_at`) that write read back under
//! the edit gate and kept in the Sidecar (a `Mark`), holding the value it
//! wrote. No clock is compared: any later write to the row, recorded or
//! not (a player, an import, rekordbox, another field's edit), leaves
//! another revision, and the value is then the DJ's. An empty field nothing
//! recorded wrote may be filled. Incomplete provenance (an unsettled
//! intent, an unconfirmed write, an unreadable entry) that could concern a
//! track is a barrier: its fields are the DJ's (the DJ may clear one,
//! `projection_barriers`, after which only its empty fields may be filled). Everything that is not the
//! projection's is kept and, when a Fact disagrees, reported. Ownership is
//! checked again under the edit gate: if a track's row or its Ledger
//! provenance changed since the plan, the whole apply is refused and
//! nothing written.
//!
//! The value projected is always the standing Fact of the field's own kind
//! (`energy`, `prep_colour`) under the ordinary precedence, so the DJ's
//! Fact can never be passed over; a DJ's Fact is projected like any other
//! when the policy allows, as a tool's write that makes no new DJ Fact.
//! Both kinds are Judgements; no Measurement is ever projected (turning
//! Intensity into an Energy Judgement is wbx-33's). Prep colour Facts name
//! a colour (`Green`), resolved through the fixed colour table to the
//! label's stable id; the DJ's label names are for display only.
//!
//! The policy file is the DJ's, but nothing authenticates it: an agent that
//! can write the Sidecar folder can lower a threshold. Every plan and every
//! Run names the hash of the policy it was made under.
//!
//! Cost: one query for each kind's Facts, one over the Ledger's `track`
//! entries and one over the Library's track rows, whatever the Library's
//! size; no audio is read.
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
use wreckordbox_sidecar::projection::{Barrier, FieldState, Mark, Owner, Provenance};
use wreckordbox_sidecar::{Fact, FactsSnapshot, Source, SourceKind};

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
    /// `projection-policy.toml` cannot be read or has a mistake.
    pub const POLICY_INVALID: &str = "projection_policy_invalid";
    /// The plan is not what the dry run named (`expect`), or the Library
    /// changed while it was being applied: nothing was written.
    pub const PLAN_CHANGED: &str = "projection_changed";
    /// A Projection was asked to record its edits as the DJ's, or to join
    /// the DJ's Run.
    pub const NOT_THE_DJ: &str = "projection_not_the_djs";
}

/// The DJ's policy file, beside the Tag schema in the Sidecar's folder.
pub const POLICY_FILE: &str = "projection-policy.toml";
/// Who a Projection's edits are by when the request names no one.
pub const SOURCE_NAME: &str = "projection";
/// What the window's Undo calls an applied Projection.
pub const UNDO_LABEL: &str = "Projection";

/// Milliseconds `projection.apply` waits between making its plan and
/// writing it, so a test can make a change in between. Read only in test
/// mode (`RBXPORT_TEST=1`).
pub const PAUSE_ENV: &str = "WRECKORDBOX_PROJECTION_PAUSE_MS";

const CHANGED_MARK: &str = "projection_changed: ";

/// Whether `command` is one of this module's.
pub(crate) fn answers(command: &str) -> bool {
    matches!(
        command,
        commands::PROJECTION_PLAN | commands::PROJECTION_APPLY
    )
}

/// The source a Projection's Run is by when the request names none; `None`
/// for any other command.
pub(crate) fn default_source(command: &str) -> Option<Source> {
    (command == commands::PROJECTION_APPLY).then(|| Source::new(SourceKind::Tool, SOURCE_NAME))
}

/// Refuses a projection attributed to the DJ: by the source it names or by
/// the Run it would join. Any other command passes.
pub(crate) fn attribution(command: &str, source: &Source) -> Result<(), ServiceError> {
    if command == commands::PROJECTION_APPLY && source.kind == SourceKind::Dj {
        return Err(ServiceError::new(
            codes::NOT_THE_DJ,
            "a Projection's edits are never recorded as the DJ's, nor in the DJ's Run: name a tool or an agent, or no source",
        ));
    }
    Ok(())
}

/// What a projection's own Run is called: it names the policy in force.
pub(crate) fn description(request: &Request) -> Option<String> {
    (request.command == commands::PROJECTION_APPLY).then(|| {
        format!(
            "Projection under policy {}",
            request.args["policy_hash"].as_str().unwrap_or("unknown")
        )
    })
}

/// A projection's request with the hash of the policy file as it is now
/// (`policy_hash`), unless the request already names one; `None` for any
/// other request. The plan refuses if the file it then reads differs.
pub(crate) fn stamped(request: &Request, stems: &Stems) -> Option<Request> {
    if !answers(&request.command) || request.args.get("policy_hash").is_some() {
        return None;
    }
    let hash = policy_bytes(stems.sidecar_dir())
        .ok()
        .map(|bytes| hash_of(&bytes))?;
    let mut stamped = request.clone();
    match &mut stamped.args {
        Value::Object(args) => {
            args.insert("policy_hash".into(), json!(hash));
        }
        Value::Null => stamped.args = json!({ "policy_hash": hash }),
        _ => return None,
    }
    Some(stamped)
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
    /// The policy the request is made under; the service fills it in. A
    /// policy file that reads otherwise refuses the command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_hash: Option<String>,
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

    /// The field's name, which is also the kind of Fact projected into it.
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

/// The policy file's bytes; none (an empty file's) when there is no file.
fn policy_bytes(dir: &Path) -> Result<Vec<u8>, std::io::Error> {
    let path = dir.join(POLICY_FILE);
    match std::fs::read(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && path.symlink_metadata().is_err() => {
            Ok(Vec::new())
        }
        other => other,
    }
}

impl Policy {
    fn rule(&self, field: Field) -> &Rule {
        match field {
            Field::Energy => &self.energy,
            Field::PrepColour => &self.prep_colour,
        }
    }

    /// Reads the policy without creating anything, and answers it with the
    /// hash of the bytes read. No file: everything waits for the DJ.
    fn load(dir: &Path) -> Result<(Self, String), ServiceError> {
        let path = dir.join(POLICY_FILE);
        let invalid = |why: String| {
            ServiceError::new(
                codes::POLICY_INVALID,
                format!(
                    "{} cannot be used, so nothing was projected: {why}",
                    path.display()
                ),
            )
        };
        let bytes = policy_bytes(dir).map_err(|e| invalid(e.to_string()))?;
        let text = String::from_utf8(bytes.clone()).map_err(|e| invalid(e.to_string()))?;
        let file: PolicyFile = toml::from_str(&text).map_err(|e| invalid(e.to_string()))?;
        let wait = || Rule {
            apply: Apply::Wait(WaitWord::Wait),
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
        Ok((policy, hash_of(&bytes)))
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
    /// `waits_for_dj` (the policy waits) or `below_confidence`.
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
}

/// A value a projection does not own, left as it is although a Fact
/// disagrees with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Kept {
    pub track: String,
    pub title: String,
    pub field: Field,
    /// The value in the Library, kept.
    pub value: Value,
    /// What the disagreeing Fact says.
    pub fact_value: Value,
    pub fact: i64,
    pub source: Source,
    pub confidence: f64,
    /// Whose the value is: `set_by_dj` (the latest recorded write is the
    /// DJ's), `set_by_other` (another source's, not a projection's),
    /// `no_record` (nothing recorded wrote it: a migrated Library),
    /// `written_since` (the projection's value, but the track's row is not
    /// at the revision its write confirmed: something wrote the track after
    /// it, recorded or not, this field or another), `provenance_incomplete`
    /// (a write the Ledger cannot account for, an unsettled intent, an
    /// unconfirmed write or an unreadable entry, could concern the track),
    /// `unknown` (an Undo the Ledger cannot follow), or `dj_fact` (the DJ's
    /// own standing Fact is what the Library holds).
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
    /// Names exactly this list of changes under this policy: give it to
    /// `projection.apply` as `expect` to apply what was checked, or nothing.
    pub plan: String,
    pub policy_file: PathBuf,
    /// The hash of the policy file's bytes as read (of nothing, with no
    /// file). The Run of an apply names it too.
    pub policy_hash: String,
    pub policy: Policy,
    /// Applied, or in a dry run to be applied, in track order.
    pub changes: Vec<Change>,
    /// Held for review.
    pub held: Vec<Held>,
    /// Values a projection does not own, left alone, where a Fact disagrees.
    pub kept: Vec<Kept>,
    /// Facts whose value cannot be written to the field.
    pub invalid: Vec<Unusable>,
    /// Stale Facts: never projected.
    pub stale: Vec<Unusable>,
    /// Tracks and fields whose standing Fact the Library already holds.
    pub unchanged: usize,
    /// Library Protection is on: applying would be refused.
    pub library_protected: bool,
    /// What went wrong without undoing what was applied.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// A planned change with what is checked again under the edit gate.
struct Planned {
    change: Change,
    /// The track row's `updated_at` and USN when the plan was made.
    row: RowStamp,
    /// The track's mark then, whose other fields a new mark keeps.
    mark: Option<Mark>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RowStamp {
    updated_at: String,
    usn: i64,
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
    let (report, planned, watermark) = plan(library, stems, &wanted, dry_run).map_err(early)?;
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
    if planned.is_empty() {
        return Ok((report, None));
    }
    pause();
    let (changed, warnings) = write(library, stems, &planned, watermark)?;
    let mut report = report;
    report.warnings = warnings;
    Ok((report, Some(changed)))
}

/// A folder in which `projection.apply`, between making its plan and
/// writing it, makes a file `paused` and then waits until a file `go`
/// appears (at most 30 seconds), so a test can change something at exactly
/// that point and then let it go on. Read only in test mode.
pub const GATE_ENV: &str = "WRECKORDBOX_PROJECTION_GATE";

/// The test pause between plan and write ([`GATE_ENV`], [`PAUSE_ENV`]).
fn pause() {
    if !rbl_db::test_mode() {
        return;
    }
    if let Some(gate) = std::env::var_os(GATE_ENV).map(PathBuf::from) {
        let _ = std::fs::write(gate.join("paused"), b"");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !gate.join("go").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    if let Some(ms) = std::env::var(PAUSE_ENV)
        .ok()
        .and_then(|ms| ms.trim().parse::<u64>().ok())
    {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

/// The Facts of each field's kind.
type FactsByField = Vec<(Field, Vec<Fact>)>;

/// The Facts of each field's kind, and the provenance of native fields,
/// read without creating, migrating or tidying a Sidecar.
fn read_sidecar(stems: &Stems) -> Result<(FactsByField, Provenance), ServiceError> {
    let failed = |why: String| ServiceError::new(crate::stems::codes::SIDECAR_FAILED, why);
    let columns: Vec<&str> = Field::ALL.iter().map(|field| field.column()).collect();
    stems
        .with_sidecar_wait(|open| -> Result<_, String> {
            let text = |e: wreckordbox_sidecar::SidecarError| e.to_string();
            match open {
                Some(sidecar) => {
                    let mut facts = Vec::new();
                    for field in Field::ALL {
                        facts.push((field, sidecar.facts_of_kind(field.name()).map_err(text)?));
                    }
                    Ok((facts, sidecar.provenance(&columns).map_err(text)?))
                }
                None => match FactsSnapshot::open(stems.sidecar_dir()).map_err(text)? {
                    FactsSnapshot::Absent => Ok((
                        Field::ALL
                            .iter()
                            .map(|field| (*field, Vec::new()))
                            .collect(),
                        Provenance::default(),
                    )),
                    FactsSnapshot::Unknown(why) => Err(why),
                    FactsSnapshot::Open(reader) => {
                        let mut facts = Vec::new();
                        for field in Field::ALL {
                            facts.push((field, reader.facts_of_kind(field.name()).map_err(text)?));
                        }
                        Ok((facts, reader.provenance(&columns).map_err(text)?))
                    }
                },
            }
        })?
        .map_err(failed)
}

/// Every track row's `updated_at` and USN, in one read.
fn row_stamps(library: &OpenLibrary) -> Result<HashMap<String, RowStamp>, ServiceError> {
    library.with_db(|db| {
        let read = || -> rusqlite::Result<HashMap<String, RowStamp>> {
            db.connection()
                .prepare(
                    "SELECT ID, COALESCE(updated_at, ''), COALESCE(rb_local_usn, 0)
                     FROM djmdContent WHERE COALESCE(rb_local_deleted, 0) = 0",
                )?
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        RowStamp {
                            updated_at: row.get(1)?,
                            usn: row.get(2)?,
                        },
                    ))
                })?
                .collect()
        };
        read().map_err(|e| {
            ServiceError::new(
                service_codes::SERVICE_FAILED,
                format!("the Library's tracks could not be read: {e}"),
            )
        })
    })
}

/// The plan, computed the same way for a dry run and for applying, with
/// what the apply checks again and the Ledger watermark it was read at.
fn plan(
    library: &OpenLibrary,
    stems: &Stems,
    wanted: &Wanted,
    dry_run: bool,
) -> Result<(Report, Vec<Planned>, i64), ServiceError> {
    let dir = stems.sidecar_dir();
    let (policy, policy_hash) = Policy::load(dir)?;
    if let Some(expected) = &wanted.policy_hash {
        if *expected != policy_hash {
            return Err(ServiceError::new(
                codes::PLAN_CHANGED,
                format!(
                    "{} changed while the projection was being made (policy {policy_hash}, not {expected}): nothing was written",
                    dir.join(POLICY_FILE).display()
                ),
            ));
        }
    }
    let rules =
        Rules::load(dir).map_err(|why| ServiceError::new(crate::standing::RULES_INVALID, why))?;
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
    let (read, provenance) = read_sidecar(stems)?;
    // Facts by track, then field; tracks in the Library's order.
    let mut by_track: BTreeMap<u32, (String, FactsByField)> = BTreeMap::new();
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
    let stamps = if by_track.is_empty() {
        HashMap::new()
    } else {
        row_stamps(library)?
    };
    let mut report = Report {
        dry_run,
        plan: String::new(),
        policy_file: dir.join(POLICY_FILE),
        policy_hash,
        policy: policy.clone(),
        changes: Vec::new(),
        held: Vec::new(),
        kept: Vec::new(),
        invalid: Vec::new(),
        stale: Vec::new(),
        unchanged: 0,
        library_protected: library.protection().protect_library,
        warnings: Vec::new(),
    };
    let mut planned = Vec::new();
    for (row, (track, mut fields)) in by_track {
        fields.sort_by_key(|(field, _)| *field);
        let title = index.title.get(row as usize).to_owned();
        let stamp = stamps.get(&track).cloned();
        for (field, facts) in fields {
            let one = One {
                track: &track,
                title: &title,
                field,
                native: native(&index, row, field, &labels),
                state: provenance
                    .fields
                    .get(&(track.clone(), field.column().to_owned())),
                row: stamp.as_ref(),
                mark: provenance.marks.get(&track),
                barriers: &provenance.barriers,
            };
            let before = report.changes.len();
            decide(&mut report, &one, &facts, &rules, &policy, &labels);
            if report.changes.len() > before {
                let change = report.changes[before].clone();
                let Some(stamp) = stamp.clone() else {
                    // No row to check again: not written.
                    report.changes.pop();
                    continue;
                };
                planned.push(Planned {
                    change,
                    row: stamp,
                    mark: provenance.marks.get(&track).cloned(),
                });
            }
        }
    }
    report.plan = digest(&report.policy_hash, &report.changes);
    Ok((report, planned, provenance.watermark))
}

/// One track and field being decided.
struct One<'a> {
    track: &'a str,
    title: &'a str,
    field: Field,
    /// What the Library holds now, in the report's form.
    native: Value,
    /// The effective latest recorded write to the field; `None` when no
    /// recorded write ever touched it.
    state: Option<&'a FieldState>,
    /// The track row's revision now; `None` when it cannot be read.
    row: Option<&'a RowStamp>,
    /// The revision a projection's own write last left on the row.
    mark: Option<&'a Mark>,
    /// What the Ledger cannot account for.
    barriers: &'a [Barrier],
}

/// Why a projection may not write this field: `None` when it may.
///
/// It may fill a field that is empty, that no recorded write touched (or an
/// Undo returned to before any), and that no incomplete provenance could
/// concern. It may replace a value only when the latest recorded write is a
/// projection's own, the row's revision is exactly the one that write
/// confirmed ([`Mark`]: its USN and `updated_at`, read back under the gate),
/// the field still holds the value it wrote, and nothing the Ledger cannot
/// account for came after it. Anything else is the DJ's. No clock is read:
/// a revision is equal or it is not.
fn not_owned(one: &One<'_>) -> Option<&'static str> {
    // Whether a barrier after `after` could concern the track; with
    // `cleared`, counting those the DJ cleared too.
    let barred = |after: i64, cleared: bool| {
        one.barriers.iter().any(|barrier| {
            barrier.seq > after
                && barrier.concerns(one.track)
                && (cleared || barrier.cleared.is_none())
        })
    };
    // An empty field may be filled once the DJ has cleared every barrier
    // that could concern it (wbx-xme); clearing makes nothing else fillable.
    let empty = || {
        if barred(0, false) {
            Some("provenance_incomplete")
        } else {
            None
        }
    };
    match one.state {
        None if one.native.is_null() => empty(),
        None => Some("no_record"),
        Some(state) => match &state.owner {
            Owner::Original if one.native.is_null() => empty(),
            Owner::Original => Some("no_record"),
            Owner::Unknown => Some("unknown"),
            Owner::Dj => Some("set_by_dj"),
            Owner::Other(_) => Some("set_by_other"),
            Owner::Projection => {
                let Some(mark) = one.mark else {
                    return Some("written_since");
                };
                let same_revision = one
                    .row
                    .is_some_and(|row| row.usn == mark.usn && row.updated_at == mark.updated_at);
                let same_value =
                    mark.fields.get(one.field.column()) == Some(&native_number(&one.native));
                if !same_revision || !same_value {
                    Some("written_since")
                } else if barred(mark.seq, true) {
                    // Cleared or not: the cleared write may have been made
                    // after this projection, so the value is the DJ's.
                    Some("provenance_incomplete")
                } else {
                    None
                }
            }
        },
    }
}

/// Decides one track and field from its Facts: a change, held, kept,
/// unusable, or nothing to do.
fn decide(
    report: &mut Report,
    one: &One<'_>,
    facts: &[Fact],
    rules: &Rules,
    policy: &Policy,
    labels: &[PrepColour],
) {
    let field = one.field;
    let native = one.native.clone();
    let unusable = |fact: &Fact, why: String| Unusable {
        track: one.track.to_owned(),
        title: one.title.to_owned(),
        field,
        fact: fact.id,
        value: fact.value.clone(),
        why,
    };
    for fact in facts.iter().filter(|f| f.is_live() && f.is_stale()) {
        report
            .stale
            .push(unusable(fact, "a stale Fact: never projected".into()));
    }
    let (winner, _) = rules.standing(facts);
    let Some(winner) = winner else { return };
    let wanted = match target(field, &winner.value, labels) {
        Ok(wanted) => wanted,
        Err(why) => {
            report.invalid.push(unusable(winner, why));
            return;
        }
    };
    let kept = |value: Value, fact: &Fact, reason: &str| Kept {
        track: one.track.to_owned(),
        title: one.title.to_owned(),
        field,
        value: native.clone(),
        fact_value: value,
        fact: fact.id,
        source: fact.source.clone(),
        confidence: fact.confidence,
        reason: reason.into(),
    };
    if wanted == native {
        report.unchanged += 1;
        // The DJ's value stands; say so where another source disagrees.
        if winner.source.kind == SourceKind::Dj {
            let others: Vec<Fact> = facts
                .iter()
                .filter(|f| f.source.kind != SourceKind::Dj)
                .cloned()
                .collect();
            if let (Some(other), _) = rules.standing(&others) {
                if let Ok(value) = target(field, &other.value, labels) {
                    if value != native {
                        report.kept.push(kept(value, other, "dj_fact"));
                    }
                }
            }
        }
        return;
    }
    if let Some(reason) = not_owned(one) {
        report.kept.push(kept(wanted, winner, reason));
        return;
    }
    let held = |reason: &str, threshold| Held {
        track: one.track.to_owned(),
        title: one.title.to_owned(),
        field,
        before: native.clone(),
        after: wanted.clone(),
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
            track: one.track.to_owned(),
            title: one.title.to_owned(),
            field,
            before: native,
            after: wanted,
            fact: winner.id,
            source: winner.source.clone(),
            confidence: winner.confidence,
        }),
    }
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

/// The value a Fact would give `field`, in the report's form. A Prep colour
/// Fact names the colour, resolved through the fixed colour table to its
/// label's stable id; the DJ's label names play no part.
fn target(field: Field, value: &Value, labels: &[PrepColour]) -> Result<Value, String> {
    match field {
        Field::Energy => value
            .as_u64()
            .filter(|n| (1..=5).contains(n))
            .map(|n| json!(n))
            .ok_or_else(|| format!("Energy is a whole number from 1 to 5, not {value}")),
        Field::PrepColour => {
            let text = value
                .as_str()
                .ok_or_else(|| format!("a Prep colour is a colour's name, not {value}"))?;
            let position = rbl_index::COLOR_NAMES
                .iter()
                .position(|name| name.eq_ignore_ascii_case(text.trim()))
                .ok_or_else(|| {
                    format!(
                        "{text:?} is not a colour: a Prep colour Fact names one of {}",
                        rbl_index::COLOR_NAMES.join(", ")
                    )
                })?;
            let id = u8::try_from(position + 1).unwrap_or(0);
            labels
                .iter()
                .find(|colour| colour.id == id)
                .map(|colour| json!(colour))
                .ok_or_else(|| format!("there is no colour label {id}"))
        }
    }
}

/// The number a report value stands for: stars, or a colour label's id;
/// none is 0.
fn native_number(value: &Value) -> i64 {
    value.as_i64().or_else(|| value["id"].as_i64()).unwrap_or(0)
}

/// A hash the same in every build (FNV-1a), as 16 hex digits.
fn hash_of(bytes: &[u8]) -> String {
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{hash:016x}")
}

/// A name for exactly this list of changes under this policy.
fn digest(policy_hash: &str, changes: &[Change]) -> String {
    let named: Vec<Value> = changes
        .iter()
        .map(|c| json!([c.track, c.field, c.before, c.after, c.fact]))
        .collect();
    hash_of(&serde_json::to_vec(&json!([policy_hash, named])).unwrap_or_default())
}

/// Makes the planned changes as one recorded edit through the single edit
/// path. Under the edit gate, before anything is written, each change is
/// checked again: its track's row has not been written since the plan
/// (`updated_at` and USN), the Library still holds its `before`, and no
/// Ledger entry newer than the plan's names the track (an assertion that
/// writes no row, the DJ setting the value it already has). Any of these
/// refuses the whole apply.
fn write(
    library: &OpenLibrary,
    stems: &Stems,
    planned: &[Planned],
    watermark: i64,
) -> Result<(LibraryChanged, Vec<String>), Failure> {
    let mut warnings = Vec::new();
    let mut ids: Vec<String> = planned.iter().map(|p| p.change.track.clone()).collect();
    ids.dedup();
    let tracks = ids.clone();
    let refused = |why: String| DbError::WriteRefused(format!("{CHANGED_MARK}{why}"));
    let result = library.recorded_edit(Touched::Metadata(ids), UNDO_LABEL, |writer| {
        let newer = stems
            .with_sidecar_wait(|open| {
                open.map_or(Ok(false), |sidecar| {
                    sidecar.written_since(watermark, &tracks)
                })
            })
            .map_err(|e| DbError::WriteRefused(e.message))?
            .map_err(|e| DbError::WriteRefused(e.to_string()))?;
        if newer {
            return Err(refused(
                "a track was edited while the projection was being made".into(),
            ));
        }
        let now = library
            .index()
            .map_err(|e| DbError::WriteRefused(e.message))?;
        for p in planned {
            let change = &p.change;
            let stamp: Option<RowStamp> = writer
                .library()
                .connection()
                .query_row(
                    "SELECT COALESCE(updated_at, ''), COALESCE(rb_local_usn, 0)
                     FROM djmdContent WHERE ID = ?1 AND COALESCE(rb_local_deleted, 0) = 0",
                    [&change.track],
                    |row| {
                        Ok(RowStamp {
                            updated_at: row.get(0)?,
                            usn: row.get(1)?,
                        })
                    },
                )
                .ok();
            let row = now.row_of(&change.track);
            let current = row.map(|row| match change.field {
                Field::Energy => i64::from(now.rating.get(row as usize).copied().unwrap_or(0)),
                Field::PrepColour => i64::from(now.color.get(row as usize).copied().unwrap_or(0)),
            });
            if stamp.as_ref() != Some(&p.row) || current != Some(native_number(&change.before)) {
                return Err(refused(format!(
                    "track {} was written while the projection was being made",
                    change.track
                )));
            }
        }
        let mut done = Vec::new();
        let mut failure = None;
        for p in planned {
            let change = &p.change;
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
        // What the projection's own write left: each written row's revision,
        // read back here under the gate, kept as its mark. A later projection
        // may replace these values only while the row is still at it.
        if let Err(why) = keep_marks(writer, stems, &planned[..done.len()]) {
            warnings.push(format!(
                "the revisions this projection left could not be kept ({why}): the values it wrote count as the DJ's until projected again"
            ));
        }
        Ok((failure, LibraryEdit::Track(done)))
    });
    match result {
        Ok((None, made)) => Ok((made, warnings)),
        Ok((Some(failure), made)) => Err((
            Some(made),
            ServiceError::new(
                service_codes::WRITE_REFUSED,
                format!("the projection stopped part-way; what it wrote is recorded in its Run and can be undone: {failure}"),
            ),
        )),
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

/// Keeps a [`Mark`] for each track `written` changed: the row's revision as
/// read back on the writer's connection, the Ledger position, and the
/// values written, with the fields of the track's earlier mark when the row
/// was still at that mark's revision when the plan was made.
fn keep_marks(
    writer: &rbl_db::write::Writer,
    stems: &Stems,
    written: &[Planned],
) -> Result<(), String> {
    let mut marks: BTreeMap<String, Mark> = BTreeMap::new();
    for p in written {
        let track = &p.change.track;
        if !marks.contains_key(track) {
            let row: RowStamp = writer
                .library()
                .connection()
                .query_row(
                    "SELECT COALESCE(updated_at, ''), COALESCE(rb_local_usn, 0)
                     FROM djmdContent WHERE ID = ?1",
                    [track],
                    |row| {
                        Ok(RowStamp {
                            updated_at: row.get(0)?,
                            usn: row.get(1)?,
                        })
                    },
                )
                .map_err(|e| e.to_string())?;
            let fields = p
                .mark
                .as_ref()
                .filter(|mark| mark.usn == p.row.usn && mark.updated_at == p.row.updated_at)
                .map(|mark| mark.fields.clone())
                .unwrap_or_default();
            marks.insert(
                track.clone(),
                Mark {
                    usn: row.usn,
                    updated_at: row.updated_at,
                    seq: 0,
                    fields,
                },
            );
        }
        if let Some(mark) = marks.get_mut(track) {
            mark.fields.insert(
                p.change.field.column().to_owned(),
                native_number(&p.change.after),
            );
        }
    }
    stems
        .with_sidecar_wait(|open| -> Result<(), String> {
            let sidecar = open.ok_or("the Sidecar is not open")?;
            let seq = sidecar.ledger_position().map_err(|e| e.to_string())?;
            let marks: Vec<(String, Mark)> = marks
                .into_iter()
                .map(|(track, mark)| (track, Mark { seq, ..mark }))
                .collect();
            sidecar
                .set_projection_marks(&marks)
                .map_err(|e| e.to_string())
        })
        .map_err(|e| e.message)?
}

fn to_value<T: Serialize>(value: &T) -> Result<Value, ServiceError> {
    serde_json::to_value(value)
        .map_err(|e| ServiceError::new(service_codes::SERVICE_FAILED, e.to_string()))
}
