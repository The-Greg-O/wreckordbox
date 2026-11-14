//! `stems.show`, which only reads (wbx-t3p).
//!
//! It used to observe the track's audio before answering: that marked
//! Measurements of other audio stale (or current again), re-attached Facts
//! and opened, so made, the Sidecar. A question must not write, so now:
//!
//! - The Stems are read without the Sidecar's writer (its own connection
//!   when it is open, else an immutable snapshot, `mutant_reads::inspect`):
//!   a Sidecar never made stays unmade, and one on a Stick, protected or
//!   not, is read and left as it is.
//! - Staleness is worked out for the answer from the track's audio as it
//!   is now: a set is stale exactly when its fingerprint differs from the
//!   audio's. The stored mark is used only when the audio cannot be read.
//!   Nothing is stored; the stored mark is still written by the commands
//!   that write (`stems.separate`, every Fact producer, `stems.write`).

use crate::protocol::ServiceError;
use crate::stems::{fingerprint, Fingerprint, ParentAudio, StemSet, Stems, TrackStems};

/// Answers `stems.show` for a track found in the Library. Writes nothing.
pub(crate) fn show(stems: &Stems, found: &ParentAudio) -> Result<TrackStems, ServiceError> {
    // Missing audio: the sets cannot be told current, so none is.
    let audio = fingerprint(found.parent(), found.path()).ok();
    let dir = stems.sidecar_dir().to_path_buf();
    let facts = crate::mutant_reads::inspect(stems, |store| store.stem_facts(&found.parent().id))?;
    let (current, previous): (Vec<StemSet>, Vec<StemSet>) = facts
        .into_iter()
        .filter_map(|fact| StemSet::from_fact(&dir, fact))
        .map(|set| as_now(set, audio.as_ref()))
        .partition(|set| set.is_current() && audio.as_ref() == Some(&set.fingerprint));
    Ok(TrackStems {
        track: found.parent().clone(),
        // Stems whose audio files are missing or are not the ones recorded
        // are not Stems: the track reads as needing separation.
        stems: current.into_iter().find(StemSet::audio_present),
        previous,
    })
}

/// The set with `stale` as the audio says now, when the audio is known:
/// every reader of Stems judges currency this way, not by the stored mark.
pub(crate) fn as_now(mut set: StemSet, audio: Option<&Fingerprint>) -> StemSet {
    if let Some(audio) = audio {
        set.stale = set.fingerprint != *audio;
    }
    set
}
