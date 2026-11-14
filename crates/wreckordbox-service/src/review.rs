//! Reviewing waiting Facts (wbx-27): the queue of what Projection held back
//! for the DJ, and the DJ's decision on each.
//!
//! - `review.waiting {tracks?, field?, from?, at_least?}` lists the
//!   standing Facts Projection's policy holds back (`held` in a dry run:
//!   the policy waits, or the Fact is below its confidence), with the
//!   track, the field, the value now and the value proposed, the source and
//!   the confidence. It is the dry run's own code, so it is a read: no
//!   Library edit, no Run, no Sidecar made, no event.
//! - `review.accept {facts | filter | all}` is the DJ's own hand set of
//!   each chosen Fact's value: through the single edit path, as an
//!   asserting edit (the DJ's explicit act, kept as the DJ's even if the
//!   value turned out unchanged), ONE step of Undo, recorded in the Ledger
//!   in the DJ's Run with the DJ's Facts it makes. Only the DJ accepts: the
//!   window, or a request whose Run is the DJ's (`--source dj`); any other
//!   source is refused (`review_not_the_djs`) with nothing written. Library
//!   Protection refuses it like any edit. The selection is made and written
//!   under the edit gate; a track written meanwhile (its row, its value, or
//!   a Ledger entry naming it since the list was read) refuses the whole
//!   accept (`review_changed`).
//! - `review.reject {facts | filter | all, source}` is the DJ's rejection:
//!   one Sidecar Ledger entry in a DJ Run of its own
//!   ([`wreckordbox_sidecar::review`]). It writes nothing to the Library.
//!   A rejected Fact is never offered again and never projected; another
//!   source's answer for the same field may still be offered.
//!
//! A selection names exactly one of: `facts` (each must be waiting, or
//! nothing is done: `review_not_waiting`), a filter (`tracks`, `field`,
//! `from`, `at_least`: every waiting Fact that matches all given), or
//! `all: true`.
use std::collections::HashSet;

use rbl_db::DbError;
use rbl_library::Touched;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wreckordbox_stick_write::selection::LibrarySource as _;

use crate::facts::SourceArg;
use crate::library::OpenLibrary;
use crate::projection::{Field, Held};
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
}

/// The error codes.
pub mod codes {
    /// Accepting or rejecting is not asked for by the DJ.
    pub const NOT_THE_DJ: &str = "review_not_the_djs";
    /// A Fact named is not waiting (decided already, applied, kept, or
    /// unknown): nothing was done.
    pub const NOT_WAITING: &str = "review_not_waiting";
    /// A chosen track was written while the accept was being made: nothing
    /// was written.
    pub const CHANGED: &str = "review_changed";
}

/// What the window's Undo calls an accept.
pub const UNDO_LABEL: &str = "Accept waiting Facts";

const CHANGED_MARK: &str = "review_changed: ";

/// The arguments of all three commands.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Selection {
    /// These Facts, each of which must be waiting.
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
    /// Every waiting Fact.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub all: bool,
    /// For `review.reject`: who rejects, which must be the DJ.
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

    fn matches(&self, held: &Held) -> bool {
        (self.tracks.is_empty() || self.tracks.contains(&held.track))
            && self.field.is_none_or(|field| field == held.field)
            && self
                .from
                .as_ref()
                .is_none_or(|from| from.kind == held.source.kind && from.name == held.source.name)
            && self.at_least.is_none_or(|at| held.confidence >= at)
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
    pub rejection: Option<wreckordbox_sidecar::review::Rejection>,
}

pub(crate) fn answers(command: &str) -> bool {
    matches!(
        command,
        commands::REVIEW_WAITING | commands::REVIEW_ACCEPT | commands::REVIEW_REJECT
    )
}

/// Answers one of the commands, and what to tell every client when it
/// changed the Library.
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
            Err(error) => (None, Err(error)),
        },
        _ => (None, reject(library, stems, request, &asked)),
    }
}

fn bad_request(why: &str) -> ServiceError {
    ServiceError::new(service_codes::BAD_REQUEST, why)
}

fn waiting(library: &OpenLibrary, stems: &Stems, asked: &Selection) -> Result<Value, ServiceError> {
    if !asked.facts.is_empty() || asked.all || asked.source.is_some() {
        return Err(bad_request(
            "review.waiting takes only a filter: tracks, field, from, at_least",
        ));
    }
    let (report, _) = crate::projection::dry_run(library, stems, asked.tracks.clone())?;
    let waiting: Vec<Held> = report
        .held
        .into_iter()
        .filter(|held| asked.matches(held))
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
    let ways = usize::from(!asked.facts.is_empty())
        + usize::from(asked.filtered())
        + usize::from(asked.all);
    if ways != 1 {
        return Err(bad_request(
            "name exactly one of: the Facts (facts), a filter (tracks, field, from, at_least), or all",
        ));
    }
    let (report, watermark) = crate::projection::dry_run(library, stems, asked.tracks.clone())?;
    if asked.facts.is_empty() {
        let chosen = report
            .held
            .into_iter()
            .filter(|held| asked.all || asked.matches(held))
            .collect();
        return Ok((chosen, watermark));
    }
    let named: HashSet<i64> = asked.facts.iter().copied().collect();
    let chosen: Vec<Held> = report
        .held
        .into_iter()
        .filter(|held| named.contains(&held.fact))
        .collect();
    let found: HashSet<i64> = chosen.iter().map(|held| held.fact).collect();
    let mut missing: Vec<i64> = named.difference(&found).copied().collect();
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

fn number(value: &Value) -> i64 {
    value.as_i64().or_else(|| value["id"].as_i64()).unwrap_or(0)
}

fn accept(
    library: &OpenLibrary,
    stems: &Stems,
    asked: &Selection,
) -> Result<(Decided, Option<LibraryChanged>), ServiceError> {
    if asked.source.is_some() {
        return Err(bad_request(
            "review.accept takes who accepts from its Run (`--source dj`), not from source",
        ));
    }
    if !crate::ledger::by_the_dj() {
        return Err(ServiceError::new(
            codes::NOT_THE_DJ,
            "accepting a waiting Fact is the DJ's own act: only the window, or a request in the DJ's Run (`--source dj`), accepts; never accept for an agent or a tool",
        ));
    }
    // The list is read and written under one hold of the edit gate, so no
    // edit made through the edit path comes between them.
    let gate = library.edit_gate();
    let _gate = gate.lock();
    let (chosen, watermark) = chosen(library, stems, asked)?;
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
    let mut tracks: Vec<String> = chosen.iter().map(|held| held.track.clone()).collect();
    tracks.dedup();
    let refused = |why: String| DbError::WriteRefused(format!("{CHANGED_MARK}{why}"));
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
        let now = library
            .index()
            .map_err(|e| DbError::WriteRefused(e.message))?;
        for held in &chosen {
            let current = now.row_of(&held.track).map(|row| match held.field {
                Field::Energy => i64::from(now.rating.get(row as usize).copied().unwrap_or(0)),
                Field::PrepColour => i64::from(now.color.get(row as usize).copied().unwrap_or(0)),
            });
            if current != Some(number(&held.before)) {
                return Err(refused(format!(
                    "track {} was written while the accept was being made",
                    held.track
                )));
            }
        }
        let mut edits = Vec::with_capacity(chosen.len());
        for held in &chosen {
            let after = number(&held.after);
            let (_, edit) = match held.field {
                Field::Energy => {
                    writer.set_rating_with_undo(&held.track, u8::try_from(after).unwrap_or(0))?
                }
                Field::PrepColour => {
                    writer.set_color_with_undo(&held.track, Some(&after.to_string()))?
                }
            };
            edits.push(edit);
        }
        Ok(((), edits))
    });
    match made {
        Ok(((), changed)) => Ok((
            Decided {
                count: chosen.len(),
                decided: chosen,
                rejection: None,
            },
            Some(changed),
        )),
        Err(e) if e.message.starts_with(CHANGED_MARK) => Err(ServiceError::new(
            codes::CHANGED,
            format!(
                "{}: nothing was written; list the waiting Facts again",
                e.message.trim_start_matches(CHANGED_MARK)
            ),
        )),
        Err(e) => Err(e),
    }
}

fn reject(
    library: &OpenLibrary,
    stems: &Stems,
    request: &Request,
    asked: &Selection,
) -> Result<Value, ServiceError> {
    let source = match &asked.source {
        Some(given) if given.kind == wreckordbox_sidecar::SourceKind::Dj => {
            crate::facts::source(given)?
        }
        _ => {
            return Err(ServiceError::new(
                codes::NOT_THE_DJ,
                "rejecting a waiting Fact is the DJ's decision: it must name the DJ as its source (`--source dj`); never reject for an agent or a tool",
            ))
        }
    };
    let (chosen, _) = chosen(library, stems, asked)?;
    if chosen.is_empty() {
        return Ok(json!(Decided {
            decided: chosen,
            count: 0,
            rejection: None,
        }));
    }
    let offered: Vec<Value> = chosen
        .iter()
        .map(|held| {
            json!({
                "fact": held.fact,
                "track": held.track,
                "field": held.field,
                "value": held.after,
                "source": held.source,
                "confidence": held.confidence,
            })
        })
        .collect();
    let command = json!({"name": request.command, "args": request.args});
    let failed = |why: String| ServiceError::new(crate::stems::codes::SIDECAR_FAILED, why);
    // A Fact is waiting only where a Sidecar holds it.
    let sidecar = stems
        .sidecar_if_made()?
        .ok_or_else(|| failed("there is no Sidecar to record the rejection in".into()))?;
    match sidecar
        .reject_facts(&offered, &source, &command)
        .map_err(|e| failed(e.to_string()))?
    {
        Ok(rejection) => Ok(json!(Decided {
            count: chosen.len(),
            decided: chosen,
            rejection: Some(rejection),
        })),
        Err(_) => Err(ServiceError::new(
            codes::NOT_THE_DJ,
            "rejecting a waiting Fact is the DJ's decision",
        )),
    }
}
