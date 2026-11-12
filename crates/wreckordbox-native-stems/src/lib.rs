//! Native stems: a track's Stems written to a Stick in the form a modded
//! XDJ-AZ plays on its pads.
//!
//! The layout is the published AZ-MODS stems format: a `STEMS` folder at
//! the Stick's root, one folder per track named from the track's file name
//! as the Stick's exported library holds it, and in it `vocals.ogg`,
//! `other.ogg`, `bass.ogg` and `drums.ogg` (Ogg Vorbis, 44100 Hz, stereo,
//! all starting at the track's first sample frame and all the same
//! length), plus `source.json` saying which file the set was made from.
//! Players without the mod ignore the folder, so the Stick stays
//! Rekordbox-readable: nothing under `PIONEER` or `Contents` is touched.
//!
//! This crate is written from that format's specification and holds no
//! AZ-MODS code. It sits beside the export crates and only reads through
//! them (ADR 0003): [`rbl_export::device_library`] tells it what path the
//! Stick's library holds for a track.
//!
//! Safety: [`write`] refuses a Stick that does not carry the Spare stick
//! marker ([`SPARE_STICK_MARKER`]) before it reads or writes anything else
//! there.
//!
//! [`write`] is the whole interface:
//!
//! ```no_run
//! # use std::path::Path;
//! # use wreckordbox_native_stems::{write, Outcome, Request, StemFiles};
//! let outcome = write(&Request {
//!     stick: Path::new("/run/media/dj/SPARE"),
//!     library_id: 0,
//!     track_id: 10_003,
//!     stems: StemFiles {
//!         vocals: Path::new("vocals.wav"),
//!         other: Path::new("other.wav"),
//!         bass: Path::new("bass.wav"),
//!         drums: Path::new("drums.wav"),
//!     },
//!     frames: None,
//! })?;
//! match outcome {
//!     Outcome::Written(written) => println!("{}", written.dir.display()),
//!     Outcome::Skipped(skipped) => println!("skipped: {}", skipped.reason),
//! }
//! # Ok::<(), wreckordbox_native_stems::NativeStemsError>(())
//! ```

use std::path::{Path, PathBuf};

mod encode;
mod identity;
pub mod naming;
mod stick;
mod work;

pub use encode::{container_frames, SAMPLE_RATE, VORBIS_QUALITY};
pub use naming::Unnameable;
pub use stick::{exported, is_spare_stick, Exported, SPARE_STICK_MARKER};

/// The folder at a Stick's root that holds every track's Native stems.
pub const STEMS_FOLDER: &str = "STEMS";

/// The four file names of a set, in the layout's order (the pads' order).
pub const FILE_NAMES: [&str; 4] = ["vocals.ogg", "other.ogg", "bass.ogg", "drums.ogg"];

/// Where the deck mounts a USB, for judging its 300-byte limit on a
/// track's absolute path: the layout's own example.
const DECK_MOUNT: &str = "/media/usb/sda1/";
const DECK_PATH_BYTES: usize = 299;

/// A track's four Stems, as audio files the Sidecar holds.
#[derive(Debug, Clone, Copy)]
pub struct StemFiles<'a> {
    pub vocals: &'a Path,
    /// The melody: everything that is not vocals, bass or drums.
    pub other: &'a Path,
    pub bass: &'a Path,
    pub drums: &'a Path,
}

impl<'a> StemFiles<'a> {
    fn in_order(&self) -> [&'a Path; 4] {
        [self.vocals, self.other, self.bass, self.drums]
    }
}

/// Native stems to write for one track.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    /// The Stick's root: where it is mounted.
    pub stick: &'a Path,
    /// The Library's own identifier (`DBID`), so a Stick exported from
    /// another Library is not mistaken for this one's; 0 when unknown.
    pub library_id: u64,
    /// The track's identifier in the Library.
    pub track_id: u64,
    pub stems: StemFiles<'a>,
    /// The length every Stem must have, in sample frames: the parent
    /// track's, as the Sidecar recorded it. `None` only requires the four
    /// to be equal.
    pub frames: Option<u64>,
}

/// What [`write`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Written(Written),
    /// The track's file name is one the layout cannot represent. Nothing
    /// was written for it; this is a report, not a failure.
    Skipped(Skipped),
}

/// A set of Native stems on the Stick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    /// The path the Stick's library holds for the track, relative to the
    /// Stick's root.
    pub track_path: String,
    /// The track's key: what the deck matches the folder by.
    pub key: String,
    /// The folder's name inside the stems folder.
    pub folder: String,
    /// The folder itself.
    pub dir: PathBuf,
    /// Each Stem's length in sample frames.
    pub frames: u64,
    /// Whether an earlier set for this track was replaced.
    pub replaced: bool,
    /// Things the DJ should know that did not stop the write.
    pub warnings: Vec<String>,
}

/// A track that cannot have Native stems.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// The path the Stick's library holds for the track.
    pub track_path: String,
    /// Its last component: the name the layout cannot represent.
    pub file_name: String,
    pub reason: Unnameable,
}

/// Why Native stems were not written. In every case the Stick holds what
/// it held before, apart from a `STEMS` folder that may have been made.
#[derive(Debug, thiserror::Error)]
pub enum NativeStemsError {
    /// The Stick does not carry the Spare stick marker. Nothing was written.
    #[error(
        "{} is not marked as a Spare stick, so nothing was written to it. A Stick is only written \
         to when it holds an empty file named {SPARE_STICK_MARKER} at its root; put one there if this \
         Stick is safe to overwrite, and never on a Gig stick",
        stick.display()
    )]
    NotSpareStick { stick: PathBuf },
    /// The Stick holds no exported library at all.
    #[error("{} holds no exported library: export the track to it first", stick.display())]
    NoLibrary { stick: PathBuf },
    /// The Stick's exported library could not be read.
    #[error("the exported library on {} cannot be read: {message}", stick.display())]
    LibraryUnreadable { stick: PathBuf, message: String },
    /// The Stick's last export did not finish: nothing is read or written
    /// until the DJ settles it (wbx-45.5), which this never does.
    #[error(
        "the last write to {} did not finish: settle it from the Stick's panel first. Nothing was read or written",
        stick.display()
    )]
    UnfinishedWrite { stick: PathBuf },
    /// The Stick's exported library holds no export of the track.
    #[error("the Stick holds no export of track {track_id}: export the track to it first")]
    TrackNotOnStick { track_id: u64 },
    /// The Stick's library names a file for the track that is not there.
    #[error(
        "the Stick's library holds track {track_id} at {track_path}, and there is no such file on the Stick"
    )]
    TrackFileMissing { track_id: u64, track_path: String },
    /// A Stem's audio could not be read, or is not audio this can write.
    #[error("cannot read the Stem {}: {message}", file.display())]
    Audio { file: PathBuf, message: String },
    /// The Stems are not all the same length, or not the parent's.
    #[error(
        "the Stem {} is {frames} sample frames long where {expected} were expected: Stems must all be exactly as long as their parent",
        file.display()
    )]
    StemLength {
        file: PathBuf,
        frames: u64,
        expected: u64,
    },
    /// Something on the way is a link, or leads off the Stick: following
    /// it would read or write somewhere the Spare stick marker does not
    /// speak for. Nothing was written.
    #[error("{} {why}, so nothing was written", path.display())]
    OffStick { path: PathBuf, why: String },
    /// The folder a second song of the same file name goes in already
    /// holds a set that is not this track's. Nothing was written.
    #[error(
        "the stems folder {folder} already holds a set that is not this track's, and it is the only folder this track's Native stems can go in; it was left as it is"
    )]
    FolderTaken { folder: String },
    /// A replacement failed half-way and the earlier set could not be put
    /// back. It is whole, under a working name, and the next write puts
    /// it back.
    #[error(
        "{}: {source}; the earlier set could not be put back and is kept at {}, from where the next write restores it",
        dir.display(),
        backup.display()
    )]
    Rollback {
        dir: PathBuf,
        backup: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot encode {}: {message}", file.display())]
    Encode { file: PathBuf, message: String },
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl NativeStemsError {
    pub(crate) fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

/// Writes Native stems for one track to a Stick that holds an export of it.
///
/// In order: the Stick must carry the Spare stick marker; its exported
/// library must hold the track; the folder's name is worked out from the
/// path that library holds (a name the layout cannot represent ends here,
/// as [`Outcome::Skipped`]); the four Stems are encoded into a dot-named
/// staging folder, which the deck does not look at; and one rename makes
/// the finished set appear. A set already there for this track is swapped
/// out whole, so the deck never sees half of one, and the deck's own
/// waveform cache for the old audio goes with it.
pub fn write(request: &Request<'_>) -> Result<Outcome, NativeStemsError> {
    let stick = request.stick;
    if !is_spare_stick(stick) {
        return Err(NativeStemsError::NotSpareStick {
            stick: stick.to_path_buf(),
        });
    }
    let exported = stick::exported(stick, request.library_id, request.track_id)?;
    let file_name = naming::file_name(&exported.path);
    let key = naming::track_key(&exported.path);
    let named = naming::check_file_name(file_name).and_then(|()| naming::folder_name(key));
    let skipped = |reason| {
        Ok(Outcome::Skipped(Skipped {
            file_name: file_name.to_owned(),
            track_path: exported.path.clone(),
            reason,
        }))
    };
    let base = match named {
        Ok(base) => base,
        Err(reason) => return skipped(reason),
    };
    let origin = identity::Origin::of(&exported.path, &exported.file).map_err(|_| {
        NativeStemsError::TrackFileMissing {
            track_id: request.track_id,
            track_path: exported.path.clone(),
        }
    })?;

    // Where the set goes is settled, and found to be on the Stick, before
    // anything is made there. A folder of the track's name that holds
    // another song's set stays; this track's goes beside it under the
    // layout's hashed name, unless that one is somebody else's too.
    let found_dir = work::stems_dir(stick)?;
    if let Some(stems_dir) = &found_dir {
        // First, what an interrupted write left: it may put back the very
        // folder this track's name leads to.
        work::recover(stems_dir);
    }
    let mut folder = base;
    let mut existing = match &found_dir {
        Some(stems_dir) => work::entry_named(stems_dir, &folder)?,
        None => None,
    };
    if existing
        .as_ref()
        .is_some_and(|held| identity::held(held, &origin) == identity::Held::AnotherTrack)
    {
        folder = naming::suffixed_folder_name(key, &exported.path);
        existing = match &found_dir {
            Some(stems_dir) => work::entry_named(stems_dir, &folder)?,
            None => None,
        };
        if existing
            .as_ref()
            .is_some_and(|held| identity::held(held, &origin) != identity::Held::ThisTrack)
        {
            return Err(NativeStemsError::FolderTaken { folder });
        }
    }
    if let Err(reason) = naming::check_folder_name(&folder) {
        return skipped(reason);
    }

    let stems_dir = match found_dir {
        Some(stems_dir) => stems_dir,
        None => work::make_stems_dir(stick)?,
    };
    let working = work::Names::of(&stems_dir, &folder);
    let frames = stage(&working.staging, request, &origin).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&working.staging);
    })?;
    let dir = stems_dir.join(&folder);
    work::publish(&stems_dir, &working, &dir, existing.as_deref()).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&working.staging);
    })?;

    let mut warnings = Vec::new();
    if DECK_MOUNT.len() + exported.path.len() > DECK_PATH_BYTES {
        warnings.push(format!(
            "the track's path on the Stick is {} bytes long; the deck keeps 300 bytes of a path, mount point included, so it may not find these stems",
            exported.path.len()
        ));
    }
    Ok(Outcome::Written(Written {
        key: key.to_owned(),
        track_path: exported.path,
        folder,
        dir,
        frames,
        replaced: existing.is_some(),
        warnings,
    }))
}

/// Writes the whole set into `staging` and flushes it to the Stick.
/// Answers the Stems' length in sample frames.
fn stage(
    staging: &Path,
    request: &Request<'_>,
    origin: &identity::Origin,
) -> Result<u64, NativeStemsError> {
    work::remove(staging);
    std::fs::create_dir(staging).map_err(|e| NativeStemsError::io(staging, e))?;
    let mut length = request.frames;
    for (source, name) in request.stems.in_order().into_iter().zip(FILE_NAMES) {
        let frames = encode::encode(source, &staging.join(name))?;
        let expected = *length.get_or_insert(frames);
        if frames != expected {
            return Err(NativeStemsError::StemLength {
                file: source.to_path_buf(),
                frames,
                expected,
            });
        }
    }
    let record = staging.join(identity::FILE_NAME);
    work::write_new(&record, origin.record().as_bytes())?;
    work::flush_dir(staging);
    Ok(length.unwrap_or(0))
}
