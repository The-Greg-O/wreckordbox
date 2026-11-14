//! What the Review view (wbx-74) needs beside wbx-27's commands, all of it
//! read-only or a notice:
//!
//! - [`of`]: the Runs the waiting Facts were recorded in, each with its
//!   Facts, so a listing can be grouped by Run. `review.waiting` answers
//!   them as `runs`. Read like the listing itself: on the open Sidecar, or
//!   a read-only snapshot, never making one. Not covered by the listing's
//!   token (a Run finishing changes nothing an accept would do).
//! - [`decided`]: the `review_decided` event, `{decision: "rejected" |
//!   "reopened", facts: [ids]}`, sent when the DJ's rejection or reopening
//!   is recorded. Neither touches the Library, so without it nothing tells
//!   a client showing the queue that it changed. An accept needs none: it
//!   is a Library edit and sends `library_changed`.
use serde::{Deserialize, Serialize};
use serde_json::json;
use wreckordbox_sidecar::{FactsSnapshot, Run};

use crate::projection::Held;
use crate::protocol::ServiceError;
use crate::stems::Stems;

/// The name of the event.
pub const REVIEW_DECIDED: &str = "review_decided";

/// A Run some waiting Facts were recorded in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaitingRun {
    #[serde(flatten)]
    pub run: Run,
    /// Its waiting Facts, lowest identifier first.
    pub facts: Vec<i64>,
}

/// The Runs of `waiting`, lowest identifier first. A Fact whose Run cannot
/// be found is in none.
pub(crate) fn of(stems: &Stems, waiting: &[Held]) -> Result<Vec<WaitingRun>, ServiceError> {
    let ids: Vec<i64> = waiting.iter().map(|h| h.fact).collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let pairs = stems
        .with_sidecar_wait(|open| -> Result<_, String> {
            let text = |e: wreckordbox_sidecar::SidecarError| e.to_string();
            match open {
                Some(sidecar) => sidecar.runs_of_facts(&ids).map_err(text),
                None => match FactsSnapshot::open(stems.sidecar_dir()).map_err(text)? {
                    FactsSnapshot::Absent => Ok(Vec::new()),
                    FactsSnapshot::Unknown(why) => Err(why),
                    FactsSnapshot::Open(reader) => reader.runs_of_facts(&ids).map_err(text),
                },
            }
        })?
        .map_err(|why| ServiceError::new(crate::stems::codes::SIDECAR_FAILED, why))?;
    let mut runs: Vec<WaitingRun> = Vec::new();
    for (fact, run) in pairs {
        match runs.iter_mut().find(|r| r.run.id == run.id) {
            Some(known) => known.facts.push(fact),
            None => runs.push(WaitingRun {
                run,
                facts: vec![fact],
            }),
        }
    }
    for run in &mut runs {
        run.facts.sort_unstable();
    }
    runs.sort_by_key(|r| r.run.id.0);
    Ok(runs)
}

/// Tells every client the DJ's `decision` on `facts` is recorded.
pub(crate) fn decided(stems: &Stems, decision: &str, facts: &[i64]) {
    stems.publish(
        REVIEW_DECIDED,
        json!({ "decision": decision, "facts": facts }),
    );
}
