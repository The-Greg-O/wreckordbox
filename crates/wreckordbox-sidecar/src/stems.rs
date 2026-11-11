//! Stems: the first kind of Fact, a Measurement.
//!
//! A track's Stems are one Fact of kind [`STEMS`] whose value lists the four
//! audio files and what they are, and whose files live in the Sidecar under
//! `stems/<track>/run-<run>/<role>.<extension>`. The audio is kept exactly
//! as the tool that made it wrote it (the separation engine writes 16-bit
//! PCM WAV); nothing is converted on the way in, so whatever a Stick needs
//! (Ogg Vorbis for Native stems) is made from the lossless original when a
//! Stick is written. Each file's format is recorded beside it.
//!
//! The Fact's value:
//!
//! ```json
//! {"samples": 13418790, "sample_rate": 44100,
//!  "produced_by": {"engine_version": "0.1.0", "model": "htdemucs_ft.yaml", ...},
//!  "stems": [{"role": "vocals", "file": "stems/10003/run-7/vocals.wav",
//!             "format": "wav", "channels": 2, "bits_per_sample": 16}, ...]}
//! ```
//!
//! `file` is relative to the Sidecar's folder, so the folder can be moved.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::audio::audio_info;
use crate::store::{Fact, Kind, NewFact, RunId, Sidecar, Source};
use crate::{io, Fingerprint, Result, SidecarError};

/// The kind of Fact a track's Stems are recorded as.
pub const STEMS: Kind<'static> = Kind::measurement("stems");

const STEMS_FOLDER: &str = "stems";

/// What a Stem is of its parent. `Other` is the melody: everything that is
/// not vocals, bass or drums.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Vocals,
    Other,
    Bass,
    Drums,
}

impl Role {
    /// The four roles, in the order a Stem set lists them.
    pub const ALL: [Self; 4] = [Self::Vocals, Self::Other, Self::Bass, Self::Drums];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Vocals => "vocals",
            Self::Other => "other",
            Self::Bass => "bass",
            Self::Drums => "drums",
        }
    }

    /// The role with this name, as [`name`](Self::name) spells it.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.name() == name)
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Four Stem files to record against their parent.
#[derive(Debug, Clone)]
pub struct NewStems<'a> {
    /// The parent track's identifier in the Library.
    pub track_id: &'a str,
    /// The fingerprint of the parent's audio that was separated.
    pub fingerprint: &'a Fingerprint,
    /// The parent's length in sample frames, as the tool that separated it
    /// decoded it. Every Stem must be exactly this long.
    pub parent_samples: u64,
    pub sample_rate: u32,
    /// One file per role. Each is moved into the Sidecar.
    pub files: &'a [(Role, PathBuf)],
    pub source: &'a Source,
    /// What produced them: tool version, model, settings. Kept as given.
    pub produced_by: Value,
    pub run: RunId,
}

/// A track's Stems as recorded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StemSet {
    /// The Fact these are.
    pub fact_id: i64,
    pub track_id: String,
    /// The fingerprint of the parent's audio they were separated from.
    pub fingerprint: Fingerprint,
    /// The parent's, and every Stem's, length in sample frames.
    pub samples: u64,
    pub sample_rate: u32,
    /// Vocals, other, bass, drums, in that order.
    pub stems: Vec<Stem>,
    pub source: Source,
    pub confidence: f64,
    pub produced_by: Value,
    pub run: RunId,
    /// Milliseconds since the Unix epoch.
    pub recorded_at: i64,
    /// The parent's audio has changed since: these no longer match it.
    pub stale: bool,
    /// A later separation took their place.
    pub superseded: bool,
}

impl StemSet {
    /// Whether these are the track's Stems now: not replaced and made from
    /// the audio the track still has.
    #[must_use]
    pub fn is_current(&self) -> bool {
        !self.stale && !self.superseded
    }
}

/// One Stem: its role and its audio file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stem {
    pub role: Role,
    /// The file's full path, inside the Sidecar.
    pub path: PathBuf,
    /// The file's format by its extension, lower case: `wav` today.
    pub format: String,
    pub channels: u16,
    pub bits_per_sample: Option<u32>,
}

/// A Stem as the Fact's value keeps it.
#[derive(Serialize, Deserialize)]
struct StoredStem {
    role: Role,
    /// Relative to the Sidecar's folder.
    file: String,
    format: String,
    channels: u16,
    bits_per_sample: Option<u32>,
}

#[derive(Serialize, Deserialize)]
struct StoredSet {
    samples: u64,
    sample_rate: u32,
    #[serde(default)]
    produced_by: Value,
    stems: Vec<StoredStem>,
}

impl Sidecar {
    /// Records a track's four Stems, attached to their parent.
    ///
    /// Refuses, changing nothing and leaving the files where they are, when
    /// the set is not exactly one Stem per role
    /// ([`SidecarError::StemSet`]), when a file cannot be read as audio, or
    /// when any Stem's length in samples or its sample rate is not its
    /// parent's ([`SidecarError::StemLength`]). The length is read from
    /// each file here; what the tool reported is not taken on trust.
    ///
    /// Otherwise the files are moved into the Sidecar and one Fact is
    /// written, which supersedes the track's earlier Stems from the same
    /// source.
    pub fn record_stems(&self, new: &NewStems) -> Result<StemSet> {
        let mut stored = Vec::with_capacity(Role::ALL.len());
        let mut moves = Vec::with_capacity(Role::ALL.len());
        let folder = Path::new(STEMS_FOLDER)
            .join(folder_name(new.track_id))
            .join(format!("run-{}", new.run.0));
        if new.files.len() != Role::ALL.len() {
            return Err(SidecarError::StemSet(format!(
                "{} files were given",
                new.files.len()
            )));
        }
        for role in Role::ALL {
            let Some((_, path)) = new.files.iter().find(|(given, _)| *given == role) else {
                return Err(SidecarError::StemSet(format!("there is no {role} Stem")));
            };
            let info = audio_info(path)?;
            if info.samples != new.parent_samples {
                return Err(SidecarError::StemLength {
                    role,
                    samples: info.samples,
                    parent: new.parent_samples,
                });
            }
            if info.sample_rate != new.sample_rate {
                return Err(SidecarError::StemSet(format!(
                    "the {role} Stem is at {} Hz and its parent at {} Hz",
                    info.sample_rate, new.sample_rate
                )));
            }
            let format = path
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("audio")
                .to_ascii_lowercase();
            let file = folder.join(format!("{}.{format}", role.name()));
            moves.push((path.clone(), self.dir().join(&file)));
            stored.push(StoredStem {
                role,
                file: file.to_string_lossy().into_owned(),
                format,
                channels: info.channels,
                bits_per_sample: info.bits_per_sample,
            });
        }

        let destination = self.dir().join(&folder);
        std::fs::create_dir_all(&destination).map_err(io("could not make", &destination))?;
        let recorded = moves
            .iter()
            .try_for_each(|(from, to)| move_file(from, to))
            .and_then(|()| {
                self.record(&NewFact {
                    track_id: new.track_id,
                    fingerprint: Some(new.fingerprint),
                    kind: STEMS,
                    value: json!(StoredSet {
                        samples: new.parent_samples,
                        sample_rate: new.sample_rate,
                        produced_by: new.produced_by.clone(),
                        stems: stored,
                    }),
                    source: new.source,
                    // A separation is what it is; there is nothing to be
                    // unsure of in which files came out.
                    confidence: 1.0,
                    run: new.run,
                })
            });
        match recorded {
            Ok(fact) => self.stem_set(fact).ok_or_else(|| {
                SidecarError::Invalid("the Stems just recorded do not read back".to_owned())
            }),
            Err(error) => {
                // Nothing was recorded, so nothing may be left looking
                // as if it had been.
                let _ = std::fs::remove_dir_all(&destination);
                Err(error)
            }
        }
    }

    /// Every Stem set ever recorded for the track, newest first: the
    /// current one, if there is one, and those that went stale or were
    /// replaced. See [`StemSet::is_current`].
    pub fn stems(&self, track_id: &str) -> Result<Vec<StemSet>> {
        Ok(self
            .facts(track_id, STEMS.name)?
            .into_iter()
            .filter_map(|fact| self.stem_set(fact))
            .collect())
    }

    /// The track's Stems now, if it has any that match `fingerprint`, the
    /// track's audio as it is: neither replaced nor made from other audio.
    pub fn current_stems(
        &self,
        track_id: &str,
        fingerprint: &Fingerprint,
    ) -> Result<Option<StemSet>> {
        Ok(self
            .stems(track_id)?
            .into_iter()
            .find(|set| set.is_current() && set.fingerprint == *fingerprint))
    }

    /// A Stems Fact as a Stem set; `None` for a Fact that is not one.
    fn stem_set(&self, fact: Fact) -> Option<StemSet> {
        let stale = fact.is_stale();
        let superseded = !fact.is_live();
        let stored: StoredSet = serde_json::from_value(fact.value).ok()?;
        Some(StemSet {
            fact_id: fact.id,
            track_id: fact.track_id,
            fingerprint: fact.fingerprint?,
            samples: stored.samples,
            sample_rate: stored.sample_rate,
            stems: stored
                .stems
                .into_iter()
                .map(|stem| Stem {
                    role: stem.role,
                    path: self.dir().join(stem.file),
                    format: stem.format,
                    channels: stem.channels,
                    bits_per_sample: stem.bits_per_sample,
                })
                .collect(),
            source: fact.source,
            confidence: fact.confidence,
            produced_by: stored.produced_by,
            run: fact.run,
            recorded_at: fact.recorded_at,
            stale,
            superseded,
        })
    }
}

/// A track identifier as a folder name. The Library's identifiers are
/// digits; anything else is made harmless rather than trusted.
fn folder_name(track_id: &str) -> String {
    track_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// A rename when both are on one filesystem, which a workspace inside the
/// Sidecar always is; a copy otherwise.
fn move_file(from: &Path, to: &Path) -> Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to).map_err(io("could not copy a Stem to", to))?;
    let _ = std::fs::remove_file(from);
    Ok(())
}
