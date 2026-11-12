//! Native stems: a track's Stems written to a Stick in the form a modded
//! XDJ-AZ plays.
//!
//! The service does this because it owns the Sidecar, where the Stems are.
//! The writing itself is `wreckordbox-native-stems`; this module decides
//! whether there is anything to write and to where:
//!
//! 1. the track must be in the Library;
//! 2. the Stick must be there and carry the Spare stick marker;
//! 3. the track must have current Stems: made from the audio it has now;
//! 4. the Stems must be about as long as the track's own audio;
//! 5. the Stick's exported library must hold an export of the track, on
//!    the Stick, of the very audio the Stems were made from.
//!
//! Each of those is refused with its own code before anything is written
//! to the Stick. A track whose file name the layout cannot represent is not
//! a failure: the command answers that it was skipped, and why.
//!
//! Writing to a Stick is not a Library edit: nothing here goes through the
//! edit path, Library Protection does not apply, and no event is sent
//! (apart from `sidecar_changed` when stale Stems are found out).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use wreckordbox_native_stems::{
    NativeStemsError, Outcome, StemFiles, Unnameable, FILE_NAMES, SAMPLE_RATE, SPARE_STICK_MARKER,
    STEMS_FOLDER,
};
use wreckordbox_sidecar::stems::{Role, StemSet};

use crate::library::OpenLibrary;
use crate::protocol::{codes as service_codes, Request, ServiceError};
use crate::stems::{self, Parent, ParentAudio, Stems};

/// The name of the command.
pub mod commands {
    /// Writes Native stems for one track to a Spare stick that holds an
    /// export of it. Takes a [`WriteStems`](super::WriteStems), answers a
    /// [`NativeStems`](super::NativeStems).
    pub const STEMS_WRITE: &str = "stems.write";
}

/// The error codes of the command, beside the service's own and
/// `track_file_missing` and `sidecar_failed` of the other Stems commands.
pub mod codes {
    /// There is no folder where the Stick was said to be.
    pub const STICK_NOT_FOUND: &str = "stick_not_found";
    /// The Stick does not carry the Spare stick marker. Nothing was written.
    pub const NOT_A_SPARE_STICK: &str = "not_a_spare_stick";
    /// The Stick's exported library holds no export of the track (or the
    /// Stick holds no exported library). Nothing was written.
    pub const TRACK_NOT_ON_STICK: &str = "track_not_on_stick";
    /// The Sidecar holds no Stems made from the audio the track has now:
    /// none at all, or only stale ones. Nothing was written.
    pub const NO_CURRENT_STEMS: &str = "no_current_stems";
    /// The Stick's export of the track is not the audio the Stems were
    /// made from: the track's audio changed since it was exported. Nothing
    /// was written. Exporting the track again puts it right.
    pub const EXPORT_STALE: &str = "export_stale";
    /// Something on the way is a symbolic link or leads off the Stick: the
    /// stems folder, the track's own folder in it, or the path the Stick's
    /// library holds for the track. Nothing was written.
    pub const STICK_UNSAFE: &str = "stick_unsafe";
    /// The only folder the track's Native stems can go in holds a set that
    /// is not this track's. Nothing was written.
    pub const STEMS_FOLDER_TAKEN: &str = "stems_folder_taken";
    /// The Stems could not be encoded or written, or the Stick's exported
    /// library could not be read. No set appeared on the Stick.
    pub const NATIVE_STEMS_FAILED: &str = "native_stems_failed";
    /// The Stick's last export did not finish. Nothing was read or written;
    /// it is settled only by the DJ, from the window (wbx-45.5).
    pub const UNFINISHED_WRITE: &str = "unfinished_write";
}

/// The arguments of `stems.write`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteStems {
    /// The track's identifier, as every track listing gives it.
    pub id: String,
    /// The Stick's root: where it is mounted. An absolute path; the
    /// service does not share the client's working folder.
    pub stick: PathBuf,
}

/// What `stems.write` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeStems {
    pub track: Parent,
    /// `written`, or `skipped` when the track's file name is one the
    /// layout cannot represent (see `skipped`).
    pub outcome: String,
    /// The Stick's root, as given.
    pub stick: PathBuf,
    /// The path the Stick's exported library holds for the track, relative
    /// to the Stick's root: where the folder's name comes from.
    pub track_path: String,
    /// The track's key in the layout: what the deck matches the folder by.
    pub key: Option<String>,
    /// The set's folder, relative to the Stick's root (`STEMS/<name>`).
    pub folder: Option<String>,
    /// The files written into it, in the pads' order.
    pub files: Vec<String>,
    /// Each Stem's length in sample frames: the parent's.
    pub samples: Option<u64>,
    pub sample_rate: Option<u32>,
    /// Whether an earlier set for this track was replaced.
    pub replaced: bool,
    /// Things the DJ should know that did not stop the write.
    pub warnings: Vec<String>,
    /// Why nothing was written, when the outcome is `skipped`.
    pub skipped: Option<SkippedName>,
}

/// A file name the layout cannot represent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedName {
    /// The track's file name on the Stick, as its library holds it.
    pub file_name: String,
    /// `empty`, `starts_with_dot`, `unstorable_character`, `too_long` or
    /// `folder_too_long`.
    pub code: String,
    /// The same in words.
    pub reason: String,
}

/// What the command needs of the Library.
pub(crate) struct Found {
    /// The track and where its audio is.
    audio: ParentAudio,
    /// The Library's own identifier (0 when it has none), which a Stick's
    /// library names for each track it holds.
    library_id: u64,
}

/// Looks the track up in the Library.
pub(crate) fn parent(library: &OpenLibrary, id: &str) -> Result<Found, ServiceError> {
    let audio = stems::parent(library, id)?;
    let library_id = library
        .with_db(|db| Ok(rbl_db::export_info::db_id(db.connection()).unwrap_or(0)))
        .unwrap_or(0);
    Ok(Found { audio, library_id })
}

/// One write at a time: two for the same track would share a staging folder.
static WRITING: Mutex<()> = Mutex::new(());

/// Answers `stems.write`. `lookup` finds the track in the Library and is
/// the only thing here that takes the Library's lock; encoding runs with
/// it free.
pub(crate) fn answer(
    stems: &Arc<Stems>,
    state_dir: &Path,
    request: &Request,
    lookup: &dyn Fn(&str) -> Result<Found, ServiceError>,
) -> Result<Value, ServiceError> {
    let args: WriteStems = serde_json::from_value(request.args.clone()).map_err(|e| {
        ServiceError::new(
            service_codes::BAD_REQUEST,
            format!("bad arguments to {}: {e}", request.command),
        )
    })?;
    let (answer, _) = write_one(
        stems,
        state_dir,
        lookup(&args.id)?,
        &args.stick,
        None,
        &|| {},
    )?;
    serde_json::to_value(answer)
        .map_err(|e| ServiceError::new(service_codes::SERVICE_FAILED, e.to_string()))
}

impl Found {
    /// The track.
    pub(crate) fn track(&self) -> &Parent {
        self.audio.parent()
    }
}

/// The track's complete set of current Stems, as a Stick load plans and
/// binds it (wbx-70): made from the audio the track has now, every role
/// there. Read without opening the Sidecar's writer: the writer's own
/// connection when it is already open, otherwise an immutable read-only
/// snapshot (`mutant_reads::inspect`), so nothing is created, migrated,
/// tidied or marked stale. `None` when there is none, or the Sidecar cannot
/// be read safely now.
pub(crate) fn current_stems_read_only(stems: &Stems, found: &Found) -> Option<CurrentStems> {
    let track = found.audio.parent();
    let fingerprint = stems::fingerprint(track, found.audio.path()).ok()?;
    let dir = stems.sidecar_dir().to_path_buf();
    let facts = crate::mutant_reads::inspect(stems, |store| store.stem_facts(&track.id)).ok()?;
    let set = facts
        .into_iter()
        .filter_map(|fact| StemSet::from_fact(&dir, fact))
        .find(|set| set.is_current() && set.fingerprint == fingerprint && set.audio_present())?;
    if !Role::ALL.iter().all(|role| {
        set.stems
            .iter()
            .any(|s| s.role == *role && s.path.is_file())
    }) {
        return None;
    }
    let files: Vec<String> = set
        .stems
        .iter()
        .map(|stem| {
            let stamp = std::fs::metadata(&stem.path).map_or_else(
                |_| "missing".to_owned(),
                |m| format!("{} bytes, {}", m.len(), modified(&m)),
            );
            format!("{} {stamp}", stem.role.name())
        })
        .collect();
    Some(CurrentStems {
        set: set.clone(),
        identity: format!(
            "Stems Fact {}, {} frames, made from {:?}: {}",
            set.fact_id,
            set.samples,
            set.fingerprint,
            files.join(", ")
        ),
        samples: set.samples,
    })
}

/// A file's modification time, to the nanosecond, for an identity.
fn modified(metadata: &std::fs::Metadata) -> String {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or_else(
            || "unknown time".to_owned(),
            |d| format!("{}ns", d.as_nanos()),
        )
}

/// A track's current Stems, as a Stick load plans them.
pub(crate) struct CurrentStems {
    /// The set itself, as read.
    pub(crate) set: StemSet,
    /// Which set, and each Stem file's size and modification time: another
    /// separation, or a Stem file changed, is another identity.
    pub(crate) identity: String,
    /// The set's length in sample frames.
    pub(crate) samples: u64,
}

/// Writes the Native stems of the track `found` to the Stick at `stick`,
/// with every check `stems.write` makes, Stick Protection first: what
/// `stems.write` does, and a Stick load does for each of its tracks.
///
/// `approved` is the exact Stem set to write (a Stick load's, as its plan
/// approved it); `None` writes the track's current Stems (`stems.write`).
/// Either way the set's files are checked again once staged, before the
/// set is published: a file changed or gone in between publishes nothing.
/// `staged` runs at that moment first (a test seam). Answers what was
/// written and the Stems Fact it was written from.
pub(crate) fn write_one(
    stems: &Stems,
    state_dir: &Path,
    found: Found,
    stick: &Path,
    approved: Option<&StemSet>,
    staged: &dyn Fn(),
) -> Result<(NativeStems, i64), ServiceError> {
    let Found {
        audio: found,
        library_id,
    } = found;
    let track = found.parent().clone();
    if !stick.is_absolute() {
        return Err(ServiceError::new(
            service_codes::BAD_REQUEST,
            format!(
                "{} is not an absolute path: say where the Stick is mounted in full",
                stick.display()
            ),
        ));
    }
    if !stick.is_dir() {
        return Err(ServiceError::new(
            codes::STICK_NOT_FOUND,
            format!("there is no Stick at {}", stick.display()),
        ));
    }
    // Stick Protection first (wbx-45.11), before anything on the Stick is
    // inspected: the Stick holding `stick` (a folder inside it is that
    // Stick), its execution lock taken (shared with the window, the CLI, a
    // lock and an eject) and held to the end of the write, and refused
    // unless the DJ unlocked this very mount.
    let authorized =
        wreckordbox_stick_protection::authorize(state_dir, stick).map_err(|refused| {
            let code = match refused {
                wreckordbox_stick_protection::Refused::Protected { .. } => {
                    service_codes::STICK_PROTECTED
                }
                _ => codes::STICK_NOT_FOUND,
            };
            ServiceError::new(code, refused.to_string())
        })?;
    if !wreckordbox_native_stems::is_spare_stick(stick) {
        return Err(refused(
            &track,
            stick,
            &NativeStemsError::NotSpareStick {
                stick: stick.to_path_buf(),
            },
        ));
    }
    // The exact set approved, when there is one: never the current Stems
    // selected again, which a separation finishing meanwhile can replace.
    let set = match approved {
        Some(approved) => approved.clone(),
        None => current_stems(stems, &found)?,
    };
    let stamps = stem_stamps(&set);
    let file = |role: Role| -> Result<&Path, ServiceError> {
        set.stems
            .iter()
            .find(|stem| stem.role == role)
            .map(|stem| stem.path.as_path())
            .ok_or_else(|| {
                ServiceError::new(
                    codes::NO_CURRENT_STEMS,
                    format!(
                        "the Stems of track {} \"{}\" have no {} Stem",
                        track.id,
                        track.title,
                        role.name()
                    ),
                )
            })
    };
    let files = StemFiles {
        vocals: file(Role::Vocals)?,
        other: file(Role::Other)?,
        bass: file(Role::Bass)?,
        drums: file(Role::Drums)?,
    };
    let track_id = track.id.parse().unwrap_or(0);
    let warnings: Vec<String> = length_checked(&track, found.path(), &set)?
        .into_iter()
        .collect();
    export_checked(&track, stick, library_id, track_id, &set)?;
    let outcome = {
        let _one_at_a_time = WRITING.lock().unwrap_or_else(PoisonError::into_inner);
        // Still this mount, still unlocked, as the write starts.
        authorized.recheck().map_err(|refused| {
            ServiceError::new(service_codes::STICK_PROTECTED, refused.to_string())
        })?;
        wreckordbox_native_stems::write_checked(
            &wreckordbox_native_stems::Request {
                stick,
                library_id,
                track_id,
                stems: files,
                frames: Some(set.samples),
            },
            &|| {
                staged();
                if stem_stamps(&set) == stamps {
                    Ok(())
                } else {
                    Err("a Stem file changed or went missing".to_owned())
                }
            },
        )
        .map_err(|error| refused(&track, stick, &error))?
    };
    let answer = match outcome {
        Outcome::Written(written) => NativeStems {
            track,
            outcome: "written".to_owned(),
            stick: stick.to_path_buf(),
            track_path: written.track_path,
            key: Some(written.key),
            folder: Some(format!("{STEMS_FOLDER}/{}", written.folder)),
            files: FILE_NAMES.iter().map(|&name| name.to_owned()).collect(),
            samples: Some(written.frames),
            sample_rate: Some(SAMPLE_RATE),
            replaced: written.replaced,
            warnings: warnings.into_iter().chain(written.warnings).collect(),
            skipped: None,
        },
        Outcome::Skipped(skipped) => NativeStems {
            track,
            outcome: "skipped".to_owned(),
            stick: stick.to_path_buf(),
            track_path: skipped.track_path,
            key: None,
            folder: None,
            files: Vec::new(),
            samples: None,
            sample_rate: None,
            replaced: false,
            warnings: Vec::new(),
            skipped: Some(SkippedName {
                file_name: skipped.file_name,
                code: match skipped.reason {
                    Unnameable::Empty => "empty",
                    Unnameable::StartsWithDot => "starts_with_dot",
                    Unnameable::Character(_) => "unstorable_character",
                    Unnameable::TooLong => "too_long",
                    Unnameable::FolderTooLong => "folder_too_long",
                }
                .to_owned(),
                reason: skipped.reason.to_string(),
            }),
        },
    };
    Ok((answer, set.fact_id))
}

/// Each Stem file's size and modification time, or that it is gone.
fn stem_stamps(set: &StemSet) -> Vec<String> {
    set.stems
        .iter()
        .map(|stem| {
            std::fs::metadata(&stem.path).map_or_else(
                |_| format!("{} missing", stem.path.display()),
                |m| format!("{} {} {}", stem.path.display(), m.len(), modified(&m)),
            )
        })
        .collect()
}

/// Refuses Stems that are not about as long as the track's own audio.
/// Answers a warning when the track's length cannot be read.
fn length_checked(
    track: &Parent,
    audio: &Path,
    set: &StemSet,
) -> Result<Option<String>, ServiceError> {
    match parent_length(audio) {
        Some(parent) if parent.abs_diff(set.samples) > LENGTH_TOLERANCE => {
            Err(ServiceError::new(
                stems::codes::STEM_LENGTH_MISMATCH,
                format!(
                    "the Stems of track {} \"{}\" are {} sample frames long and the track is {parent} \
                     (at {SAMPLE_RATE} Hz): {} apart, where a decoder accounts for {LENGTH_TOLERANCE} at \
                     most. They would not play in time with the track, so nothing was written.",
                    track.id,
                    track.title,
                    set.samples,
                    parent.abs_diff(set.samples)
                ),
            ))
        }
        Some(_) => Ok(None),
        None => Ok(Some(
            "the track's own length could not be read, so the Stems' length was not checked against it"
                .to_owned(),
        )),
    }
}

/// Refuses a Stick whose export of the track is not on the Stick, or is
/// not the audio the Stems were made from.
fn export_checked(
    track: &Parent,
    stick: &Path,
    library_id: u64,
    track_id: u64,
    set: &StemSet,
) -> Result<(), ServiceError> {
    // The export on the Stick must be the audio the Stems were made from:
    // the record written beside them says they are.
    let exported = wreckordbox_native_stems::exported(stick, library_id, track_id)
        .map_err(|error| refused(track, stick, &error))?;
    if exported.file.is_file() {
        let on_stick = wreckordbox_sidecar::fingerprint_file(&exported.file).map_err(|e| {
            ServiceError::new(
                codes::NATIVE_STEMS_FAILED,
                format!(
                    "no Native stems were written for track {} \"{}\": {e}",
                    track.id, track.title
                ),
            )
        })?;
        if on_stick != set.fingerprint {
            return Err(ServiceError::new(
                codes::EXPORT_STALE,
                format!(
                    "the export of track {} \"{}\" on the Stick at {} is not the audio its Stems \
                     were made from: the track's audio changed after it was exported. Export it \
                     again, then write its Native stems. Nothing was written.",
                    track.id,
                    track.title,
                    stick.display()
                ),
            ));
        }
    }
    Ok(())
}

/// How far a set of Stems may be from its parent's length by the
/// container and still be written, in sample frames at 44100 Hz: a
/// quarter of a second. A decoder may or may not hand back a lossy
/// codec's padding (an MP3's encoder delay and its last frame's fill, an
/// AAC stream's priming: some three thousand frames at the very most), so
/// the two counts need not agree; further apart than this, the Stems were
/// not made from this audio's timeline. This does not show that Stems
/// inside the tolerance are aligned: only a deck can.
const LENGTH_TOLERANCE: u64 = 11_025;

/// The parent's length by its container, in sample frames at 44100 Hz.
fn parent_length(audio: &Path) -> Option<u64> {
    let (frames, rate) = wreckordbox_native_stems::container_frames(audio)?;
    let scaled = u128::from(frames) * u128::from(SAMPLE_RATE) / u128::from(rate);
    u64::try_from(scaled).ok()
}

/// The track's Stems made from the audio it has now, with all four files
/// in the Sidecar. Stems made from other audio are marked stale on the way,
/// as every Stems command does, and are not written.
///
/// Before audio goes to a Stick, every Stem file is hashed again and must
/// be the audio recorded with the set, whatever an earlier look found.
fn current_stems(stems: &Stems, found: &ParentAudio) -> Result<StemSet, ServiceError> {
    let (set, _) = current_stems_verified(stems, found)?;
    if !set.audio_verified() {
        return Err(ServiceError::new(
            codes::NO_CURRENT_STEMS,
            format!(
                "track {} \"{}\" has Stems recorded in the Sidecar whose audio files are not the ones recorded. Run `wreckordbox stems separate {}` first.",
                found.parent().id, found.parent().title, found.parent().id
            ),
        ));
    }
    Ok(set)
}

/// [`current_stems`], with the fingerprint of the audio it was verified
/// against. Also the renderer's boundary (`render_stems`).
pub(crate) fn current_stems_verified(
    stems: &Stems,
    found: &ParentAudio,
) -> Result<(StemSet, wreckordbox_sidecar::Fingerprint), ServiceError> {
    let track = found.parent();
    let audio = found.path();
    let none = |why: &str| {
        ServiceError::new(
            codes::NO_CURRENT_STEMS,
            format!(
                "track {} \"{}\" {why}. Run `wreckordbox stems separate {}` first.",
                track.id, track.title, track.id
            ),
        )
    };
    // A Sidecar nobody has made holds no Stems; asking does not make one.
    let Some(sidecar) = stems.sidecar_if_made()? else {
        return Err(none("has no Stems in the Sidecar"));
    };
    let failed =
        |e: &dyn std::fmt::Display| ServiceError::new(stems::codes::SIDECAR_FAILED, e.to_string());
    let fingerprint = stems::fingerprint(track, audio)?;
    stems.observe_audio(found, Some(&fingerprint))?;
    let current = sidecar
        .current_stems(&track.id, &fingerprint)
        .map_err(|e| failed(&e))?;
    match current {
        Some(set) if set.stems.iter().all(|stem| stem.path.is_file()) => Ok((set, fingerprint)),
        Some(_) => Err(none(
            "has Stems recorded in the Sidecar, and their audio files are gone",
        )),
        None if sidecar.stems(&track.id).map_err(|e| failed(&e))?.is_empty() => {
            Err(none("has no Stems in the Sidecar"))
        }
        None => Err(none(
            "has only stale Stems in the Sidecar: they were made from audio it no longer has",
        )),
    }
}

/// The writer's refusal as the service says it.
fn refused(track: &Parent, stick: &Path, error: &NativeStemsError) -> ServiceError {
    match error {
        NativeStemsError::NotSpareStick { .. } => ServiceError::new(
            codes::NOT_A_SPARE_STICK,
            format!(
                "{} is not marked as a Spare stick, so nothing was written to it. A Stick is only \
                 written to when it holds an empty file named {SPARE_STICK_MARKER} at its root. Put \
                 one there if this Stick is safe to overwrite, and never on a Gig stick.",
                stick.display()
            ),
        ),
        NativeStemsError::NoLibrary { .. } | NativeStemsError::TrackNotOnStick { .. } => {
            ServiceError::new(
                codes::TRACK_NOT_ON_STICK,
                format!(
                    "track {} \"{}\" has no export on the Stick at {}. Export it to the Stick first.",
                    track.id,
                    track.title,
                    stick.display()
                ),
            )
        }
        NativeStemsError::OffStick { .. } => ServiceError::new(
            codes::STICK_UNSAFE,
            format!(
                "no Native stems were written for track {} \"{}\": {error}",
                track.id, track.title
            ),
        ),
        NativeStemsError::UnfinishedWrite { .. } => ServiceError::new(
            codes::UNFINISHED_WRITE,
            format!(
                "no Native stems were written for track {} \"{}\": {error}",
                track.id, track.title
            ),
        ),
        NativeStemsError::FolderTaken { .. } => ServiceError::new(
            codes::STEMS_FOLDER_TAKEN,
            format!(
                "no Native stems were written for track {} \"{}\": {error}",
                track.id, track.title
            ),
        ),
        other => ServiceError::new(
            codes::NATIVE_STEMS_FAILED,
            format!(
                "no Native stems were written for track {} \"{}\": {other}",
                track.id, track.title
            ),
        ),
    }
}
