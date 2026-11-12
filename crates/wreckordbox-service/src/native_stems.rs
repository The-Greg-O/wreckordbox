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
//! 4. the Stick's exported library must hold an export of the track.
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
use serde_json::{json, Value};
use wreckordbox_native_stems::{
    NativeStemsError, Outcome, StemFiles, Unnameable, FILE_NAMES, SAMPLE_RATE, SPARE_STICK_MARKER,
    STEMS_FOLDER,
};
use wreckordbox_sidecar::stems::{Role, StemSet};

use crate::library::OpenLibrary;
use crate::protocol::{codes as service_codes, Request, ServiceError};
use crate::stems::{self, Parent, ParentAudio, SidecarChanged, Stems};

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
    /// The Stems could not be encoded or written, or the Stick's exported
    /// library could not be read. No set appeared on the Stick.
    pub const NATIVE_STEMS_FAILED: &str = "native_stems_failed";
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
    /// `empty`, `starts_with_dot`, `unstorable_character` or `too_long`.
    pub code: String,
    /// The same in words.
    pub reason: String,
}

/// The track, where its audio is, and the Library's own identifier (0 when
/// it has none), which a Stick's library names for each track it holds.
pub(crate) fn parent(library: &OpenLibrary, id: &str) -> Result<(ParentAudio, u64), ServiceError> {
    let found = stems::parent(library, id)?;
    let library_id = library
        .with_db(|db| Ok(rbl_db::export_info::db_id(db.connection()).unwrap_or(0)))
        .unwrap_or(0);
    Ok((found, library_id))
}

/// One write at a time: two for the same track would share a staging folder.
static WRITING: Mutex<()> = Mutex::new(());

/// Answers `stems.write`. `lookup` finds the track in the Library and is
/// the only thing here that takes the Library's lock; encoding runs with
/// it free.
pub(crate) fn answer(
    stems: &Arc<Stems>,
    request: &Request,
    lookup: &dyn Fn(&str) -> Result<(ParentAudio, u64), ServiceError>,
) -> Result<Value, ServiceError> {
    let args: WriteStems = serde_json::from_value(request.args.clone()).map_err(|e| {
        ServiceError::new(
            service_codes::BAD_REQUEST,
            format!("bad arguments to {}: {e}", request.command),
        )
    })?;
    let (found, library_id) = lookup(&args.id)?;
    let track = found.parent().clone();
    let stick = args.stick.as_path();
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
    if !wreckordbox_native_stems::is_spare_stick(stick) {
        return Err(refused(
            &track,
            stick,
            &NativeStemsError::NotSpareStick {
                stick: stick.to_path_buf(),
            },
        ));
    }
    let set = current_stems(stems, &track, found.path())?;
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
    let outcome = {
        let _one_at_a_time = WRITING.lock().unwrap_or_else(PoisonError::into_inner);
        wreckordbox_native_stems::write(&wreckordbox_native_stems::Request {
            stick,
            library_id,
            track_id: track.id.parse().unwrap_or(0),
            stems: files,
            frames: Some(set.samples),
        })
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
            warnings: written.warnings,
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
                }
                .to_owned(),
                reason: skipped.reason.to_string(),
            }),
        },
    };
    serde_json::to_value(answer)
        .map_err(|e| ServiceError::new(service_codes::SERVICE_FAILED, e.to_string()))
}

/// The track's Stems made from the audio it has now, with all four files
/// in the Sidecar. Stems made from other audio are marked stale on the way,
/// as every Stems command does, and are not written.
fn current_stems(stems: &Stems, track: &Parent, audio: &Path) -> Result<StemSet, ServiceError> {
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
    let marked_stale = sidecar
        .mark_stale(&track.id, &fingerprint)
        .map_err(|e| failed(&e))?;
    if !marked_stale.is_empty() {
        stems.publish(
            stems::events::SIDECAR_CHANGED,
            json!(SidecarChanged {
                track_id: track.id.clone(),
                kind: wreckordbox_sidecar::stems::STEMS.name.to_owned(),
                fact_id: None,
                stale: marked_stale,
            }),
        );
    }
    let current = sidecar
        .current_stems(&track.id, &fingerprint)
        .map_err(|e| failed(&e))?;
    match current {
        Some(set) if set.stems.iter().all(|stem| stem.path.is_file()) => Ok(set),
        Some(_) => Err(none(
            "has Stems recorded in the Sidecar, and their audio files are gone",
        )),
        None if sidecar
            .stems(&track.id)
            .map_err(|e| failed(&e))?
            .is_empty() =>
        {
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
        other => ServiceError::new(
            codes::NATIVE_STEMS_FAILED,
            format!(
                "no Native stems were written for track {} \"{}\": {other}",
                track.id, track.title
            ),
        ),
    }
}
