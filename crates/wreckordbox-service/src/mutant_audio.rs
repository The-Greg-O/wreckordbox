//! Cheap, read-only readiness; full verification is explicit and never observes
//! stale Facts or writes the Sidecar. Render/Stick operations verify again.
use super::{inspect, Mutants};
use crate::mutant_edits::{codes, Edits, Renderability, Stem};
use crate::{protocol::ServiceError, tracks::TrackRecord};
use std::path::Path;
use wreckordbox_sidecar::stems::{file_stamp, Role, StemSet};

fn unverified() -> ServiceError {
    ServiceError::new("mutant_audio_unverified", "audio verification is not cached for these files; use --check-audio (render and Stick load verify again)")
}

impl Mutants<'_> {
    pub(super) fn readiness(
        &self,
        edits: &Edits,
        parent: Option<&TrackRecord>,
        check_audio: bool,
    ) -> Result<Renderability, ServiceError> {
        let source = super::editing::source_readiness(parent);
        let Some(parent) = parent.filter(|_| source.renderable) else {
            return Ok(source);
        };
        let set = if edits.stems.is_some() {
            match self.check_stems(parent, check_audio) {
                Ok(set) => Some(set),
                Err(error)
                    if matches!(error.code.as_str(), "sidecar_failed" | "sidecar_timeout") =>
                {
                    return Err(error)
                }
                Err(error) => return Ok(Renderability::blocked(error)),
            }
        } else {
            None
        };
        Ok(preflight(parent, edits, set.as_ref()))
    }

    pub(super) fn check_stems(
        &self,
        parent: &TrackRecord,
        check_audio: bool,
    ) -> Result<StemSet, ServiceError> {
        let sets: Vec<_> = inspect(self.stems, |store| store.stem_facts(&parent.id))?
            .into_iter()
            .filter(wreckordbox_sidecar::Fact::is_live)
            .filter_map(|f| StemSet::from_fact(self.stems.sidecar_dir(), f))
            .collect();
        if sets.is_empty() {
            return Err(ServiceError::new(
                codes::STEMS_MISSING,
                "separation is needed: the parent has no Stems",
            ));
        }
        let path = Path::new(&parent.location);
        let fingerprint = if check_audio {
            wreckordbox_sidecar::fingerprint_cached(path)
                .map_err(|e| ServiceError::new("track_file_missing", e.to_string()))?
        } else {
            wreckordbox_sidecar::known_fingerprint(path).ok_or_else(unverified)?
        };
        let matching: Vec<_> = sets
            .into_iter()
            .filter(|s| s.fingerprint == fingerprint)
            .collect();
        if matching.is_empty() {
            return Err(ServiceError::new(
                codes::STEMS_STALE,
                "separation is needed: the Stems belong to different parent audio",
            ));
        }
        let mut unknown = false;
        for set in matching {
            let present = if check_audio {
                Some(set.audio_present())
            } else {
                set.audio_cached()
            };
            match present {
                Some(true) => return Ok(set),
                None => unknown = true,
                Some(false) => {}
            }
        }
        Err(if unknown {
            unverified()
        } else {
            ServiceError::new(
                codes::STEMS_MISSING,
                "separation is needed: Stem files are missing or damaged",
            )
        })
    }
}

// Cache only header/capability verdicts. Hash currency is checked separately on
// every request using the Sidecar's shared size/mtime/inode cache above.
fn preflight(parent: &TrackRecord, edits: &Edits, set: Option<&StemSet>) -> Renderability {
    #[derive(PartialEq, Eq, Hash)]
    struct CheckKey {
        parent: (
            std::path::PathBuf,
            Option<wreckordbox_sidecar::stems::FileStamp>,
        ),
        edits: String,
        frames: Option<u64>,
        stems: Vec<(
            std::path::PathBuf,
            Option<wreckordbox_sidecar::stems::FileStamp>,
        )>,
    }
    type Cache = std::collections::HashMap<CheckKey, Renderability>;
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Cache>> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let path = Path::new(&parent.location);
    let stamps = || CheckKey {
        parent: (path.to_path_buf(), file_stamp(path)),
        edits: format!("{edits:?}"),
        frames: set.map(|s| s.samples),
        stems: set
            .map(|s| {
                s.stems
                    .iter()
                    .map(|stem| (stem.path.clone(), file_stamp(&stem.path)))
                    .collect()
            })
            .unwrap_or_default(),
    };
    let before = stamps();
    if let Some(known) = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&before)
    {
        return known.clone();
    }
    let kept = edits
        .stems
        .as_ref()
        .map_or(wreckordbox_render::Kept::ALL, |kept| {
            wreckordbox_render::Kept {
                vocals: kept.contains(&Stem::Vocals),
                other: kept.contains(&Stem::Melody),
                bass: kept.contains(&Stem::Bass),
                drums: kept.contains(&Stem::Drums),
            }
        });
    let stems = set.map(|set| {
        let path = |role| {
            set.stems
                .iter()
                .find(|stem| stem.role == role)
                .map(|stem| stem.path.clone())
                .unwrap_or_default()
        };
        wreckordbox_render::Stems {
            frames: set.samples,
            vocals: path(Role::Vocals),
            other: path(Role::Other),
            bass: path(Role::Bass),
            drums: path(Role::Drums),
        }
    });
    let Ok(key_shift) = i8::try_from(edits.key_shift.unwrap_or(0)) else {
        return Renderability::blocked(ServiceError::new(
            codes::KEY_SHIFT_LIMIT,
            "key shift is out of range",
        ));
    };
    let verdict = match wreckordbox_render::preflight(
        path,
        kept,
        stems.as_ref(),
        wreckordbox_render::Edits {
            key_shift,
            tempo: edits.tempo.unwrap_or(1.0),
        },
    ) {
        Ok(()) => Renderability::ready(),
        Err(error) => {
            use wreckordbox_render::RenderError;
            let code = match error {
                RenderError::StemAlignmentUnknown { .. } => codes::STEM_ALIGNMENT_UNKNOWN,
                RenderError::StemsDoNotFitParent { .. } => "stems_do_not_fit_parent",
                RenderError::KeyShiftUnavailable => "key_shift_unavailable",
                RenderError::TempoUnavailable => "tempo_unavailable",
                _ => "mutant_render_unavailable",
            };
            let message = if code == codes::STEM_ALIGNMENT_UNKNOWN {
                format!("{error}; awaiting wbx-49.1")
            } else {
                error.to_string()
            };
            Renderability::blocked(ServiceError::new(code, message))
        }
    };
    if before == stamps() {
        let mut cache = cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if cache.len() >= 4096 {
            cache.clear();
        }
        cache.insert(before, verdict.clone());
        verdict
    } else {
        Renderability::blocked(unverified())
    }
}
