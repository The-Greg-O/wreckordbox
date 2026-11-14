//! Projection's barriers (wbx-xme): `projection.barriers` lists them and
//! `projection.acknowledge` is the DJ's clearing of one.
//!
//! A barrier is a write the Ledger cannot account for (a write begun and
//! never confirmed, an unreadable entry, a write in progress). While one
//! stands, Projection fills no empty field it could concern; a crashed
//! apply, which names no tracks, concerns every track. The Sidecar module
//! [`wreckordbox_sidecar::projection_barriers`] holds the rules.
//!
//! - `projection.barriers` is a read: no Run, no entry, no Sidecar made, no
//!   event.
//! - `projection.acknowledge {barrier, source}` clears the barrier at that
//!   place in the Ledger, as one Ledger entry in a Run of the DJ's own.
//!   Only a request that names the DJ as its source is admitted: no source,
//!   an agent's or a tool's is refused (`barrier_not_the_djs`), and so is a
//!   request sent in a Run (the Keeper refuses a Run for any command that
//!   does not edit the Library). The window, too, must name the DJ: nothing
//!   defaults to the DJ here. It changes nothing in the Library, so it
//!   takes no edit path and tells no host; nothing else ever clears a
//!   barrier, not a dry run, not a read, not an apply.
//!
//! A cleared barrier lets an EMPTY field be filled again. It never makes a
//! value a projection's: a value projected before it, or left with no
//! record, stays the DJ's.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wreckordbox_sidecar::projection_barriers::NotCleared;
pub use wreckordbox_sidecar::projection_barriers::{BarrierKind, BarrierRecord, Clearing};
use wreckordbox_sidecar::FactsSnapshot;

use crate::facts::SourceArg;
use crate::protocol::{Request, ServiceError};
use crate::stems::Stems;

/// The names of the commands.
pub mod commands {
    /// Lists every barrier; writes nothing.
    pub const PROJECTION_BARRIERS: &str = "projection.barriers";
    /// The DJ's clearing of one barrier.
    pub const PROJECTION_ACKNOWLEDGE: &str = "projection.acknowledge";
}

/// The error codes.
pub mod codes {
    /// The request does not name the DJ as its source.
    pub const NOT_THE_DJ: &str = "barrier_not_the_djs";
    /// No barrier is at that place in the Ledger.
    pub const NOT_FOUND: &str = "barrier_not_found";
    /// The barrier is a write still in progress.
    pub const IN_PROGRESS: &str = "barrier_in_progress";
    /// The DJ cleared it already.
    pub const ALREADY_CLEARED: &str = "barrier_already_cleared";
}

/// What a cleared barrier allows, said in every answer.
pub const ALLOWS: &str = "A cleared barrier lets Projection fill an empty field it concerned (one no recorded write touched). It never makes a value a projection's: a value projected before it, or one with no record, stays the DJ's.";

/// The answer of `projection.barriers`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Barriers {
    /// Oldest first, cleared ones included.
    pub barriers: Vec<BarrierRecord>,
    /// How many stand (are not cleared).
    pub standing: usize,
    pub allows: String,
}

/// The arguments of `projection.acknowledge`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Acknowledge {
    /// The barrier's `seq`, as listed.
    pub barrier: i64,
    /// Who asks: must be the DJ.
    #[serde(default)]
    pub source: Option<SourceArg>,
}

/// The answer of `projection.acknowledge`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Acknowledged {
    pub barrier: i64,
    #[serde(flatten)]
    pub clearing: Clearing,
    pub allows: String,
}

pub(crate) fn answers(command: &str) -> bool {
    matches!(
        command,
        commands::PROJECTION_BARRIERS | commands::PROJECTION_ACKNOWLEDGE
    )
}

pub(crate) fn answer(stems: &Stems, request: &Request) -> Result<Value, ServiceError> {
    if request.command == commands::PROJECTION_BARRIERS {
        let barriers = list(stems)?;
        let standing = barriers.iter().filter(|b| b.cleared.is_none()).count();
        return Ok(json!(Barriers {
            barriers,
            standing,
            allows: ALLOWS.to_owned(),
        }));
    }
    acknowledge(stems, request)
}

fn failed(why: impl std::fmt::Display) -> ServiceError {
    ServiceError::new(crate::stems::codes::SIDECAR_FAILED, why.to_string())
}

/// Every barrier, read without creating, migrating or tidying a Sidecar.
fn list(stems: &Stems) -> Result<Vec<BarrierRecord>, ServiceError> {
    stems
        .with_sidecar_wait(|open| -> Result<_, String> {
            let text = |e: wreckordbox_sidecar::SidecarError| e.to_string();
            match open {
                Some(sidecar) => sidecar.barrier_list().map_err(text),
                None => match FactsSnapshot::open(stems.sidecar_dir()).map_err(text)? {
                    FactsSnapshot::Absent => Ok(Vec::new()),
                    FactsSnapshot::Unknown(why) => Err(why),
                    FactsSnapshot::Open(reader) => reader.barrier_list().map_err(text),
                },
            }
        })?
        .map_err(failed)
}

fn acknowledge(stems: &Stems, request: &Request) -> Result<Value, ServiceError> {
    let asked: Acknowledge = crate::facts::arguments(request)?;
    let not_the_dj = || {
        ServiceError::new(
            codes::NOT_THE_DJ,
            "only the DJ clears a Projection barrier: it is the DJ's decision, never a tool's or an agent's",
        )
    };
    let source = match &asked.source {
        Some(given) if given.kind == wreckordbox_sidecar::SourceKind::Dj => {
            crate::facts::source(given)?
        }
        _ => return Err(not_the_dj()),
    };
    let not_found = || {
        ServiceError::new(
            codes::NOT_FOUND,
            format!(
                "there is no barrier {}: list them with `wreckordbox projection barriers`",
                asked.barrier
            ),
        )
    };
    let sidecar = stems.sidecar_if_made()?.ok_or_else(not_found)?;
    let command = json!({"name": request.command, "args": request.args});
    match sidecar
        .clear_barrier(asked.barrier, &source, &command)
        .map_err(failed)?
    {
        Ok(clearing) => Ok(json!(Acknowledged {
            barrier: asked.barrier,
            clearing,
            allows: ALLOWS.to_owned(),
        })),
        Err(NotCleared::NotFound) => Err(not_found()),
        Err(NotCleared::NotTheDj) => Err(not_the_dj()),
        Err(NotCleared::InProgress) => Err(ServiceError::new(
            codes::IN_PROGRESS,
            format!(
                "barrier {} is a write still in progress: it cannot be cleared until it ends, or, if it never does, until the service restarts and records it as unconfirmed",
                asked.barrier
            ),
        )),
        Err(NotCleared::AlreadyCleared(entry)) => Err(ServiceError::new(
            codes::ALREADY_CLEARED,
            format!(
                "barrier {} was cleared already, in Ledger entry {entry}",
                asked.barrier
            ),
        )),
    }
}
