//! Whether a track has Stems, for a reader that must not write (wbx-67).
//!
//! `stems.show` observes the track's audio before it answers: it marks
//! Stems of other audio stale, re-attaches Facts, and opens (so makes) the
//! Sidecar. A window that asks each time the DJ loads a track must not
//! write anything by asking, so `stems.status` answers the same question
//! by reading only: it fingerprints the track's file, compares it with the
//! recorded sets, and leaves every set as it was. A Sidecar never made
//! stays unmade; the answer is then "none".
//!
//! It also says which track is being separated now, by any client, so a
//! window that did not start the separation (or was reloaded) can show it.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::protocol::{codes as service_codes, Request, ServiceError};
use crate::stems::{fingerprint, Parent, ParentAudio, StemSet, Stems, TrackId};

/// The command's name.
pub const STEMS_STATUS: &str = "stems.status";

/// What a track's Stems are now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StemsState {
    /// No Stems at all, or none whose files are still there.
    None,
    /// Stems made from the audio the track has now.
    Current,
    /// Stems made from audio the track no longer has (marked stale, or
    /// of another fingerprint).
    Stale,
}

/// Why the track's Stems cannot be judged now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unavailable {
    pub code: String,
    pub message: String,
}

/// What `stems.status` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StemsStatus {
    pub track: Parent,
    pub state: StemsState,
    /// When the set the state describes was recorded (ms since the epoch).
    pub made_at: Option<i64>,
    /// The Fact of that set.
    pub fact_id: Option<i64>,
    /// The track being separated now, by any client; `null` when none.
    pub separating: Option<String>,
    /// Why the state could not be read in full: the track's audio is
    /// missing (its Stems cannot be told current or stale), or the Sidecar
    /// is refused (on a protected Stick) or failed.
    pub unavailable: Option<Unavailable>,
}

/// Answers `stems.status`. Writes nothing.
pub(crate) fn answer(
    stems: &Arc<Stems>,
    request: &Request,
    lookup: &dyn Fn(&str) -> Result<ParentAudio, ServiceError>,
) -> Result<Value, ServiceError> {
    let track: TrackId = serde_json::from_value(request.args.clone()).map_err(|e| {
        ServiceError::new(
            service_codes::BAD_REQUEST,
            format!("bad arguments to {}: {e}", request.command),
        )
    })?;
    let found = lookup(&track.id)?;
    serde_json::to_value(status(stems, &found))
        .map_err(|e| ServiceError::new(service_codes::SERVICE_FAILED, e.to_string()))
}

fn status(stems: &Stems, found: &ParentAudio) -> StemsStatus {
    let mut answer = StemsStatus {
        track: found.parent().clone(),
        state: StemsState::None,
        made_at: None,
        fact_id: None,
        separating: stems.separating(),
        unavailable: None,
    };
    let unavailable = |error: ServiceError| Unavailable {
        code: error.code,
        message: error.message,
    };
    let audio = fingerprint(found.parent(), found.path());
    // Read without the Sidecar's writer (the writer's own connection when
    // it is open, else an immutable snapshot): nothing is opened, made or
    // written, so a Sidecar kept on a Stick, protected or not, is read and
    // left as it is (wbx-45.14).
    let dir = stems.sidecar_dir().to_path_buf();
    let sets: Vec<StemSet> =
        match crate::mutant_reads::inspect(stems, |store| store.stem_facts(&found.parent().id)) {
            Ok(facts) => facts
                .into_iter()
                .filter_map(|fact| StemSet::from_fact(&dir, fact))
                .collect(),
            Err(error) => {
                answer.unavailable = Some(unavailable(error));
                return answer;
            }
        };
    let audio = match audio {
        Ok(audio) => audio,
        Err(error) => {
            answer.unavailable = Some(unavailable(error));
            return answer;
        }
    };
    // Staleness as the audio says now, not as last stored (wbx-t3p).
    let sets: Vec<StemSet> = sets
        .into_iter()
        .map(|set| crate::stems_show::as_now(set, Some(&audio)))
        .collect();
    let describe = |answer: &mut StemsStatus, set: &StemSet, state| {
        answer.state = state;
        answer.made_at = Some(set.recorded_at);
        answer.fact_id = Some(set.fact_id);
    };
    // Newest first: the newest current set of this audio with its files.
    if let Some(set) = sets
        .iter()
        .find(|set| set.is_current() && set.fingerprint == audio && set.audio_present())
    {
        describe(&mut answer, set, StemsState::Current);
    } else if let Some(set) = sets
        .iter()
        .find(|set| set.stale || (!set.superseded && set.fingerprint != audio))
    {
        describe(&mut answer, set, StemsState::Stale);
    }
    answer
}
