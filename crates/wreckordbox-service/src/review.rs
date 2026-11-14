//! Reviewing waiting Facts (wbx-27): the queue of what Projection held back
//! for the DJ, and the DJ's decision on each.
//!
//! - `review.waiting {tracks?, field?, from?, at_least?}` lists the
//!   standing Facts Projection's policy holds back (`held` in a dry run:
//!   the policy waits, or the Fact is below its confidence), with the
//!   track, the field, the value now and the value proposed, the source and
//!   the confidence. It is the dry run's own code, so it is a read: no
//!   Library edit, no Run, no Sidecar made, no event.
//! - `review.accept {facts | filter | all, expect?, source}` is the DJ's
//!   own hand set of each chosen Fact's value. See [`accept`].
//! - `review.reject {facts | filter | all, source}` is the DJ's rejection
//!   of the SUGGESTIONS the chosen Facts make (track, field, canonical
//!   value, source): one Sidecar Ledger entry in a DJ Run of its own
//!   ([`wreckordbox_sidecar::review`]), no Library write, no event. The
//!   same suggestion recorded again is never offered nor projected.
//! - `review.rejected {filter}` lists the suggestions standing rejected; a
//!   read.
//! - `review.reopen {facts | filter | all | rejection, source}` is the DJ's
//!   reversal of rejections (by the Fact rejected, a filter, everything, or
//!   one earlier `review.reject` as a whole by its Ledger entry): an
//!   append-only Ledger entry; the rejection stays recorded; the reopened
//!   Facts wait again.
//!
//! Every decision is the DJ's alone and declared on every request:
//! `source` must be the DJ, with no default even in the window and none
//! inherited from a joined Run (`review_not_the_djs`).
//!
//! A selection names exactly one of: `facts` (each must be waiting, or
//! nothing is done: `review_not_waiting`), a filter (`tracks`, `field`,
//! `from`, `at_least`: every waiting Fact that matches all given), or
//! `all: true`; for a reopening, also `rejection`.
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use rbl_db::DbError;
use rbl_library::Touched;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wreckordbox_sidecar::review::{Decision, NotRecorded, Offered, Rejected};
use wreckordbox_sidecar::{FactsSnapshot, Source, SourceKind};
use wreckordbox_stick_write::selection::LibrarySource as _;

use crate::facts::SourceArg;
use crate::library::OpenLibrary;
use crate::projection::{native_number, Field, Held};
use crate::protocol::{codes as service_codes, LibraryChanged, Request, ServiceError};
use crate::stems::Stems;

/// The names of the commands.
pub mod commands {
    /// Lists the waiting Facts; writes nothing.
    pub const REVIEW_WAITING: &str = "review.waiting";
    /// The DJ's acceptance: a hand set through the edit path.
    pub const REVIEW_ACCEPT: &str = "review.accept";
    /// The DJ's rejection: a Sidecar entry; no Library write.
    pub const REVIEW_REJECT: &str = "review.reject";
    /// Lists the suggestions standing rejected; writes nothing.
    pub const REVIEW_REJECTED: &str = "review.rejected";
    /// The DJ's reopening of rejections: a Sidecar entry.
    pub const REVIEW_REOPEN: &str = "review.reopen";
}

/// The error codes.
pub mod codes {
    /// The request does not declare the DJ, or its Run is not the DJ's.
    pub const NOT_THE_DJ: &str = "review_not_the_djs";
    /// A Fact named is not waiting (decided already, applied, kept, or
    /// unknown): nothing was done.
    pub const NOT_WAITING: &str = "review_not_waiting";
    /// What the DJ was shown, or a chosen track, changed before the write:
    /// nothing was written.
    pub const CHANGED: &str = "review_changed";
    /// The accept stopped part-way: what it applied is recorded and named,
    /// with what it did not apply.
    pub const PARTIAL: &str = "review_partial";
}

/// What the window's Undo calls an accept.
pub const UNDO_LABEL: &str = "Accept waiting Facts";

/// Test mode only: a folder of hold files. For a point `P` (`selected`,
/// after the selection is checked and before the write; `between`, after
/// the first value is written), `hold-P` makes the accept write `at-P` and
/// wait for `go-P` (at most a minute). A Fact command kept waiting by an
/// accept writes `facts-waiting` there first.
pub const GATE_ENV: &str = "WRECKORDBOX_REVIEW_GATE";
/// Test mode only: the accept's N-th value (from 1) fails as a refused
/// write would.
pub const FAIL_AT_ENV: &str = "WRECKORDBOX_REVIEW_FAIL_AT";

const CHANGED_MARK: &str = "review_changed: ";

/// The arguments of the commands.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Selection {
    /// These Facts, each of which must be waiting (for a reopening: each
    /// rejected).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<i64>,
    /// Filter: only these tracks.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tracks: Vec<String>,
    /// Filter: only this field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<Field>,
    /// Filter: only Facts from this source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<SourceArg>,
    /// Filter: only Facts at or above this confidence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_least: Option<f64>,
    /// Every waiting (or, for a reopening, rejected) Fact.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub all: bool,
    /// For `review.reopen`: every suggestion this rejection's Ledger entry
    /// rejected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejection: Option<i64>,
    /// For `review.accept`: the rows the DJ was shown, exactly as listed.
    /// The selection must resolve to exactly these, unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expect: Option<Vec<Held>>,
    /// Who decides: must be the DJ.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceArg>,
}

impl Selection {
    fn filtered(&self) -> bool {
        !self.tracks.is_empty()
            || self.field.is_some()
            || self.from.is_some()
            || self.at_least.is_some()
    }

    fn ways(&self) -> usize {
        usize::from(!self.facts.is_empty())
            + usize::from(self.filtered())
            + usize::from(self.all)
            + usize::from(self.rejection.is_some())
    }

    fn matches(&self, track: &str, field: Field, source: &Source, confidence: f64) -> bool {
        (self.tracks.is_empty() || self.tracks.iter().any(|t| t == track))
            && self.field.is_none_or(|f| f == field)
            && self
                .from
                .as_ref()
                .is_none_or(|from| from.kind == source.kind && from.name == source.name)
            && self.at_least.is_none_or(|at| confidence >= at)
    }
}

/// The answer of `review.waiting`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Waiting {
    /// In the Library's track order, then field.
    pub waiting: Vec<Held>,
    pub count: usize,
    /// Library Protection is on: accepting would be refused.
    pub library_protected: bool,
}

/// The answer of `review.accept` and `review.reject`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decided {
    /// The Facts accepted (their values now in the Library) or rejected.
    pub decided: Vec<Held>,
    pub count: usize,
    /// For a rejection: its Ledger entry and Run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejection: Option<Decision>,
}

/// The answer of `review.rejected`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectedList {
    pub rejected: Vec<Rejected>,
    pub count: usize,
}

/// The answer of `review.reopen`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reopened {
    pub reopened: Vec<Rejected>,
    pub count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reopening: Option<Decision>,
}

pub(crate) fn answers(command: &str) -> bool {
    matches!(
        command,
        commands::REVIEW_WAITING
            | commands::REVIEW_ACCEPT
            | commands::REVIEW_REJECT
            | commands::REVIEW_REJECTED
            | commands::REVIEW_REOPEN
    )
}

/// The DJ an accept's request declares, for its Run when the request names
/// none; `None` for any other request or declaration.
pub(crate) fn declared(request: &Request) -> Option<Source> {
    if request.command != commands::REVIEW_ACCEPT {
        return None;
    }
    let given: SourceArg = serde_json::from_value(request.args.get("source")?.clone()).ok()?;
    (given.kind == SourceKind::Dj)
        .then(|| crate::facts::source(&given).ok())
        .flatten()
}

/// Keeps Fact commands out while an accept is checked and written. A Fact
/// command kept waiting says so in the test gate folder first.
pub(crate) fn exclude(stems: &Stems) -> std::sync::MutexGuard<'_, ()> {
    let lock = &stems.fact_writes;
    match lock.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => {
            if let Some(gate) = gate() {
                let _ = std::fs::write(gate.join("facts-waiting"), b"");
            }
            lock.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }
    }
}

fn gate() -> Option<PathBuf> {
    rbl_db::test_mode()
        .then(|| std::env::var_os(GATE_ENV).map(PathBuf::from))
        .flatten()
}

/// The test hold at `point` ([`GATE_ENV`]).
fn hold(point: &str) {
    let Some(gate) = gate() else { return };
    if !gate.join(format!("hold-{point}")).exists() {
        return;
    }
    let _ = std::fs::write(gate.join(format!("at-{point}")), b"");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !gate.join(format!("go-{point}")).exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// The test failure of the `n`-th value ([`FAIL_AT_ENV`]).
fn fails_at(n: usize) -> bool {
    rbl_db::test_mode()
        && std::env::var(FAIL_AT_ENV)
            .ok()
            .and_then(|at| at.trim().parse::<usize>().ok())
            == Some(n)
}

/// Answers one of the commands, and what to tell every client when it
/// changed the Library (told even when the answer is a failure part-way).
pub(crate) fn answer(
    library: &OpenLibrary,
    stems: &Stems,
    request: &Request,
) -> (Option<LibraryChanged>, Result<Value, ServiceError>) {
    let asked: Selection = if request.args.is_null() {
        Selection::default()
    } else {
        match crate::facts::arguments(request) {
            Ok(asked) => asked,
            Err(error) => return (None, Err(error)),
        }
    };
    match request.command.as_str() {
        commands::REVIEW_WAITING => (None, waiting(library, stems, &asked)),
        commands::REVIEW_ACCEPT => match accept(library, stems, &asked) {
            Ok((decided, changed)) => (changed, Ok(json!(decided))),
            Err((changed, error)) => (changed, Err(error)),
        },
        commands::REVIEW_REJECT => (None, reject(library, stems, request, &asked)),
        commands::REVIEW_REJECTED => (None, rejected(stems, &asked)),
        _ => (None, reopen(stems, request, &asked)),
    }
}

fn bad_request(why: &str) -> ServiceError {
    ServiceError::new(service_codes::BAD_REQUEST, why)
}

fn not_the_dj(what: &str) -> ServiceError {
    ServiceError::new(
        codes::NOT_THE_DJ,
        format!(
            "{what} is the DJ's own decision: the request must declare the DJ (`--source dj`) every time, and be in no other source's Run; never decide for an agent or a tool"
        ),
    )
}

/// The DJ the request declares, or the refusal.
fn the_dj(asked: &Selection, what: &str) -> Result<Source, ServiceError> {
    match &asked.source {
        Some(given) if given.kind == SourceKind::Dj => crate::facts::source(given),
        _ => Err(not_the_dj(what)),
    }
}

fn waiting(library: &OpenLibrary, stems: &Stems, asked: &Selection) -> Result<Value, ServiceError> {
    if asked.ways() != usize::from(asked.filtered())
        || asked.source.is_some()
        || asked.expect.is_some()
    {
        return Err(bad_request(
            "review.waiting takes only a filter: tracks, field, from, at_least",
        ));
    }
    let (report, _) = crate::projection::dry_run(library, stems, asked.tracks.clone())?;
    let waiting: Vec<Held> = report
        .held
        .into_iter()
        .filter(|h| asked.matches(&h.track, h.field, &h.source, h.confidence))
        .collect();
    Ok(json!(Waiting {
        count: waiting.len(),
        waiting,
        library_protected: report.library_protected,
    }))
}

/// The waiting Facts `asked` chooses, in order, and the Ledger watermark
/// they were read at.
fn chosen(
    library: &OpenLibrary,
    stems: &Stems,
    asked: &Selection,
) -> Result<(Vec<Held>, i64), ServiceError> {
    if asked.ways() != 1 || asked.rejection.is_some() {
        return Err(bad_request(
            "name exactly one of: the Facts (facts), a filter (tracks, field, from, at_least), or all",
        ));
    }
    let (report, watermark) = crate::projection::dry_run(library, stems, asked.tracks.clone())?;
    if asked.facts.is_empty() {
        let chosen = report
            .held
            .into_iter()
            .filter(|h| asked.all || asked.matches(&h.track, h.field, &h.source, h.confidence))
            .collect();
        return Ok((chosen, watermark));
    }
    let named: HashSet<i64> = asked.facts.iter().copied().collect();
    let chosen: Vec<Held> = report
        .held
        .into_iter()
        .filter(|h| named.contains(&h.fact))
        .collect();
    let found: HashSet<i64> = chosen.iter().map(|h| h.fact).collect();
    let mut missing: Vec<i64> = named.difference(&found).copied().collect();
    // A Fact named and shown but no longer waiting is the binding's to
    // name ([`bound`]); one never shown is simply not waiting.
    if let Some(shown) = &asked.expect {
        missing.retain(|f| !shown.iter().any(|h| h.fact == *f));
    }
    if !missing.is_empty() {
        missing.sort_unstable();
        return Err(ServiceError::new(
            codes::NOT_WAITING,
            format!(
                "Fact(s) {missing:?} are not waiting for the DJ (decided already, applied, kept, or not known): nothing was done; list them with `wreckordbox review waiting`"
            ),
        ));
    }
    Ok((chosen, watermark))
}

/// Whether `now` is the row the DJ was shown as `shown`: the same Fact,
/// track, field, values, source and confidence.
fn same(shown: &Held, now: &Held) -> bool {
    shown.fact == now.fact
        && shown.track == now.track
        && shown.field == now.field
        && shown.before == now.before
        && shown.after == now.after
        && shown.source == now.source
        && shown.confidence.to_bits() == now.confidence.to_bits()
}

/// Refuses, naming them, the Facts that are not now exactly as shown.
fn bound(chosen: &[Held], shown: &[Held]) -> Result<(), ServiceError> {
    let now: HashMap<i64, &Held> = chosen.iter().map(|h| (h.fact, h)).collect();
    let was: HashMap<i64, &Held> = shown.iter().map(|h| (h.fact, h)).collect();
    let mut changed: Vec<i64> = shown
        .iter()
        .filter(|s| now.get(&s.fact).is_none_or(|n| !same(s, n)))
        .map(|s| s.fact)
        .chain(
            chosen
                .iter()
                .filter(|c| !was.contains_key(&c.fact))
                .map(|c| c.fact),
        )
        .collect();
    if changed.is_empty() {
        return Ok(());
    }
    changed.sort_unstable();
    changed.dedup();
    Err(ServiceError::new(
        codes::CHANGED,
        format!(
            "Fact(s) {changed:?} are not as the DJ was shown them (replaced, moved to another track, no longer waiting, changed, or new to the selection): nothing was written; list the waiting Facts again"
        ),
    ))
}

/// A track row as the accept expects it: its revision and its values.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    usn: i64,
    updated_at: String,
    rating: i64,
    colour: i64,
}

fn read_row(db: &rbl_db::Library, track: &str) -> Option<Row> {
    db.connection()
        .query_row(
            "SELECT COALESCE(rb_local_usn, 0), COALESCE(updated_at, ''),
                    COALESCE(Rating, 0), COALESCE(CAST(ColorID AS INTEGER), 0)
             FROM djmdContent WHERE ID = ?1 AND COALESCE(rb_local_deleted, 0) = 0",
            [track],
            |r| {
                Ok(Row {
                    usn: r.get(0)?,
                    updated_at: r.get(1)?,
                    rating: r.get(2)?,
                    colour: r.get(3)?,
                })
            },
        )
        .ok()
}

fn value_of(row: &Row, field: Field) -> i64 {
    match field {
        Field::Energy => row.rating,
        Field::PrepColour => row.colour,
    }
}

type Failure = (Option<LibraryChanged>, ServiceError);

/// `review.accept`: the DJ's own hand set of each chosen Fact's value.
///
/// - Declared: the request names the DJ as its source, and its Run (if it
///   joins one) is the DJ's; nothing is inherited or defaulted.
/// - Bound to what the DJ saw: a selection by filter or `all` resolves to
///   its explicit set first and only that set is applied; with `expect`
///   (the rows the DJ was shown) every chosen Fact must be exactly as
///   shown (same Fact, track, field, values, source, confidence), and the
///   set exactly the one shown, or the accept is refused naming them.
/// - Fact commands are kept out from before the check until the write is
///   done ([`exclude`], taken by the caller); the Library's edit gate is
///   held across the selection and the write.
/// - At the write, on the writer's own connection, each row must still be
///   at the revision (USN and `updated_at`) and value read with the
///   selection, so a write from outside the edit gate (a player, rekordbox)
///   is never overwritten: refused, nothing written.
/// - Through the single edit path as ONE asserting edit: one Undo step, one
///   Ledger entry, the DJ's Facts, the host told. Library Protection
///   refuses it.
/// - Should a later value fail, what was written is still recorded (Undo,
///   Ledger, DJ Facts), the index refreshed and the host told, and the
///   answer names what was applied and what was not
///   (`review_partial`). A crash part-way leaves the write's intent with
///   bounded evidence of the chosen tracks (`dj_facts::snapshot`), which
///   recovery reconciles to the DJ's values; its barrier concerns only
///   those tracks.
fn accept(
    library: &OpenLibrary,
    stems: &Stems,
    asked: &Selection,
) -> Result<(Decided, Option<LibraryChanged>), Failure> {
    let early = |error: ServiceError| (None, error);
    the_dj(asked, "accepting a waiting Fact").map_err(early)?;
    if !crate::ledger::by_the_dj() {
        return Err(early(not_the_dj("accepting a waiting Fact")));
    }
    // The list is read and written under one hold of the edit gate, so no
    // edit made through the edit path comes between them.
    let gate_lock = library.edit_gate();
    let _gate = gate_lock.lock();
    let (chosen, watermark) = chosen(library, stems, asked).map_err(early)?;
    if let Some(shown) = &asked.expect {
        bound(&chosen, shown).map_err(early)?;
    }
    if chosen.is_empty() {
        return Ok((
            Decided {
                decided: chosen,
                count: 0,
                rejection: None,
            },
            None,
        ));
    }
    let mut tracks: Vec<String> = chosen.iter().map(|h| h.track.clone()).collect();
    tracks.sort();
    tracks.dedup();
    // Each row as it is with the selection: its revision and its value.
    let rows: HashMap<String, Row> = library
        .with_db(|db| {
            Ok(tracks
                .iter()
                .filter_map(|t| read_row(db, t).map(|row| (t.clone(), row)))
                .collect())
        })
        .map_err(early)?;
    for held in &chosen {
        let fine = rows
            .get(&held.track)
            .is_some_and(|row| value_of(row, held.field) == native_number(&held.before));
        if !fine {
            return Err(early(changed_error(&format!(
                "track {} is not as the DJ was shown it",
                held.track
            ))));
        }
    }
    hold("selected");
    let refused = |why: String| DbError::WriteRefused(format!("{CHANGED_MARK}{why}"));
    let mut applied: Vec<i64> = Vec::new();
    let made = library.asserting_edit(Touched::Metadata(tracks.clone()), UNDO_LABEL, |writer| {
        // Something recorded wrote one of these tracks since the list was
        // read (an assertion writes no row, so the row alone cannot say).
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
                "a track was edited while the accept was being made".into(),
            ));
        }
        // Each row on the writer's own connection: at the revision and
        // value read with the selection, or nothing is written.
        let mut expected = rows.clone();
        for (track, row) in &expected {
            if read_row(writer.library(), track).as_ref() != Some(row) {
                return Err(refused(format!(
                    "track {track} was written while the accept was being made"
                )));
            }
        }
        let mut done = Vec::with_capacity(chosen.len());
        let mut failure: Option<DbError> = None;
        for (at, held) in chosen.iter().enumerate() {
            let expect = expected.get(&held.track).cloned();
            let now = read_row(writer.library(), &held.track);
            let wrote = if now != expect {
                Err(refused(format!(
                    "track {} was written while the accept was being made",
                    held.track
                )))
            } else if fails_at(at + 1) {
                Err(DbError::WriteRefused(
                    "a test failure (review fail-at)".into(),
                ))
            } else {
                let after = native_number(&held.after);
                match held.field {
                    Field::Energy => {
                        writer.set_rating_with_undo(&held.track, u8::try_from(after).unwrap_or(0))
                    }
                    Field::PrepColour => {
                        writer.set_color_with_undo(&held.track, Some(&after.to_string()))
                    }
                }
            };
            match wrote {
                Ok((_, edit)) => {
                    done.push(edit);
                    applied.push(held.fact);
                    // Our own write moved the row on: the next value for it
                    // expects the revision it left.
                    if let Some(row) = read_row(writer.library(), &held.track) {
                        expected.insert(held.track.clone(), row);
                    }
                    if done.len() == 1 {
                        hold("between");
                    }
                }
                Err(e) if done.is_empty() => return Err(e),
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
        }
        Ok((failure, done))
    });
    match made {
        Ok((None, changed)) => Ok((
            Decided {
                count: chosen.len(),
                decided: chosen,
                rejection: None,
            },
            Some(changed),
        )),
        Ok((Some(failure), changed)) => {
            let not: Vec<i64> = chosen
                .iter()
                .map(|h| h.fact)
                .filter(|f| !applied.contains(f))
                .collect();
            Err((
                Some(changed),
                ServiceError::new(
                    codes::PARTIAL,
                    format!(
                        "the accept stopped part-way ({failure}); applied: {applied:?}, recorded as the DJ's and undoable in one step; not applied: {not:?}, still waiting"
                    ),
                ),
            ))
        }
        Err(e) if e.message.starts_with(CHANGED_MARK) => Err(early(changed_error(
            e.message.trim_start_matches(CHANGED_MARK),
        ))),
        Err(e) => Err(early(e)),
    }
}

fn changed_error(why: &str) -> ServiceError {
    ServiceError::new(
        codes::CHANGED,
        format!("{why}: nothing was written; list the waiting Facts again"),
    )
}

fn offered(held: &Held) -> Offered {
    Offered {
        fact: held.fact,
        track: held.track.clone(),
        field: held.field.name().to_owned(),
        value: held.after.clone(),
        canonical: native_number(&held.after),
        source: held.source.clone(),
        confidence: held.confidence,
    }
}

fn failed(why: impl std::fmt::Display) -> ServiceError {
    ServiceError::new(crate::stems::codes::SIDECAR_FAILED, why.to_string())
}

fn reject(
    library: &OpenLibrary,
    stems: &Stems,
    request: &Request,
    asked: &Selection,
) -> Result<Value, ServiceError> {
    let source = the_dj(asked, "rejecting a waiting Fact")?;
    if asked.expect.is_some() {
        return Err(bad_request("expect is for review.accept"));
    }
    let (chosen, _) = chosen(library, stems, asked)?;
    if chosen.is_empty() {
        return Ok(json!(Decided {
            decided: chosen,
            count: 0,
            rejection: None,
        }));
    }
    let rows: Vec<Offered> = chosen.iter().map(offered).collect();
    let command = json!({"name": request.command, "args": request.args});
    // A Fact is waiting only where a Sidecar holds it.
    let sidecar = stems
        .sidecar_if_made()?
        .ok_or_else(|| failed("there is no Sidecar to record the rejection in"))?;
    match sidecar.reject(&rows, &source, &command).map_err(failed)? {
        Ok(rejection) => Ok(json!(Decided {
            count: chosen.len(),
            decided: chosen,
            rejection: Some(rejection),
        })),
        Err(NotRecorded::NotTheDj | NotRecorded::Nothing) => {
            Err(not_the_dj("rejecting a waiting Fact"))
        }
    }
}

/// Every suggestion standing rejected, read without making a Sidecar.
fn standing_rejections(stems: &Stems) -> Result<Vec<Rejected>, ServiceError> {
    stems
        .with_sidecar_wait(|open| -> Result<_, String> {
            let text = |e: wreckordbox_sidecar::SidecarError| e.to_string();
            match open {
                Some(sidecar) => sidecar.rejections().map_err(text),
                None => match FactsSnapshot::open(stems.sidecar_dir()).map_err(text)? {
                    FactsSnapshot::Absent => Ok(Vec::new()),
                    FactsSnapshot::Unknown(why) => Err(why),
                    FactsSnapshot::Open(reader) => reader.rejections().map_err(text),
                },
            }
        })?
        .map_err(failed)
}

fn field_of(name: &str) -> Option<Field> {
    serde_json::from_value(json!(name)).ok()
}

fn rejected_matching(stems: &Stems, asked: &Selection) -> Result<Vec<Rejected>, ServiceError> {
    Ok(standing_rejections(stems)?
        .into_iter()
        .filter(|r| {
            let o = &r.offered;
            field_of(&o.field)
                .is_some_and(|field| asked.matches(&o.track, field, &o.source, o.confidence))
        })
        .collect())
}

fn rejected(stems: &Stems, asked: &Selection) -> Result<Value, ServiceError> {
    if asked.ways() != usize::from(asked.filtered())
        || asked.source.is_some()
        || asked.expect.is_some()
    {
        return Err(bad_request(
            "review.rejected takes only a filter: tracks, field, from, at_least",
        ));
    }
    let rejected = rejected_matching(stems, asked)?;
    Ok(json!(RejectedList {
        count: rejected.len(),
        rejected,
    }))
}

fn reopen(stems: &Stems, request: &Request, asked: &Selection) -> Result<Value, ServiceError> {
    let source = the_dj(asked, "reopening a rejection")?;
    if asked.ways() != 1 || asked.expect.is_some() {
        return Err(bad_request(
            "name exactly one of: the Facts rejected (facts), a filter (tracks, field, from, at_least), all, or one rejection (its Ledger entry)",
        ));
    }
    let standing = standing_rejections(stems)?;
    let chosen: Vec<Rejected> = if !asked.facts.is_empty() {
        let named: HashSet<i64> = asked.facts.iter().copied().collect();
        let chosen: Vec<Rejected> = standing
            .into_iter()
            .filter(|r| named.contains(&r.offered.fact))
            .collect();
        if chosen.len() < named.len() {
            return Err(ServiceError::new(
                codes::NOT_WAITING,
                "a Fact named is not standing rejected: nothing was reopened; list them with `wreckordbox review rejected`",
            ));
        }
        chosen
    } else if let Some(entry) = asked.rejection {
        standing
            .into_iter()
            .filter(|r| r.rejection.entry == entry)
            .collect()
    } else if asked.all {
        standing
    } else {
        rejected_matching(stems, asked)?
    };
    if chosen.is_empty() {
        return Ok(json!(Reopened {
            reopened: chosen,
            count: 0,
            reopening: None,
        }));
    }
    let rows: Vec<Offered> = chosen.iter().map(|r| r.offered.clone()).collect();
    let command = json!({"name": request.command, "args": request.args});
    let sidecar = stems
        .sidecar_if_made()?
        .ok_or_else(|| failed("there is no Sidecar to record the reopening in"))?;
    match sidecar.reopen(&rows, &source, &command).map_err(failed)? {
        Ok(reopening) => Ok(json!(Reopened {
            count: chosen.len(),
            reopened: chosen,
            reopening: Some(reopening),
        })),
        Err(_) => Err(not_the_dj("reopening a rejection")),
    }
}
