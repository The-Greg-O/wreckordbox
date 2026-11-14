//! Where the Sidecar's Stems meet the renderer (wbx-82): the only way a
//! Stem set is handed to `wreckordbox_render`, and only when it is verified
//! current for the track's audio as it is now.
//!
//! The renderer itself has no Sidecar and cannot tell Stems of other audio
//! of the same length from the right ones; it checks length and rate
//! against the parent file. Currency is checked here, by the same machinery
//! every Stems command uses (`stems.show`, `stems.write`): the parent's file
//! is fingerprinted, currency is computed without writing or events,
//! and only a set made
//! from this fingerprint, with all four files present, is handed over.

use std::path::{Path, PathBuf};

use wreckordbox_sidecar::stems::Role;
use wreckordbox_sidecar::Fingerprint;

use crate::protocol::ServiceError;
use crate::stems::{self, Parent, ParentAudio, Stems};

/// A track's Stem set, verified current for its parent's audio when it was
/// handed over, in the renderer's terms.
#[derive(Debug, Clone)]
pub struct StemsForRender {
    pub track_id: String,
    pub title: String,
    /// The parent's audio file, as the Library has it.
    pub parent: PathBuf,
    /// The fingerprint of that audio the Stems were verified against.
    pub fingerprint: Fingerprint,
    /// The Sidecar Fact the Stems are: with `fingerprint`, a render's
    /// identity for a cache key.
    pub fact_id: i64,
    pub stems: wreckordbox_render::Stems,
}

impl StemsForRender {
    /// Whether the parent's audio is still the audio the Stems were verified
    /// against: fingerprints it again. Ask after a render, before using the
    /// file: audio replaced while it rendered makes the render one of
    /// another track's Stems.
    pub fn audio_unchanged(&self) -> Result<bool, ServiceError> {
        let parent = Parent {
            id: self.track_id.clone(),
            title: self.title.clone(),
        };
        Ok(stems::fingerprint(&parent, &self.parent)? == self.fingerprint)
    }
}

/// The track's current Stems for a render, or `no_current_stems` (none,
/// only stale ones, or their files gone), `track_file_missing`, or
/// `sidecar_failed`.
pub(crate) fn current(stems: &Stems, found: &ParentAudio) -> Result<StemsForRender, ServiceError> {
    let (set, fingerprint) = crate::native_stems::current_stems_verified(stems, found)?;
    let path = |role: Role| -> PathBuf {
        set.stems
            .iter()
            .find(|stem| stem.role == role)
            .map(|stem| stem.path.clone())
            .unwrap_or_default()
    };
    let parent = found.parent();
    Ok(StemsForRender {
        track_id: parent.id.clone(),
        title: parent.title.clone(),
        parent: Path::to_path_buf(found.path()),
        fingerprint,
        fact_id: set.fact_id,
        stems: wreckordbox_render::Stems {
            frames: set.samples,
            vocals: path(Role::Vocals),
            other: path(Role::Other),
            bass: path(Role::Bass),
            drums: path(Role::Drums),
        },
    })
}
