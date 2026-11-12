//! Approval tokens for every write to a Stick (wbx-45.4, second review).
//!
//! No command writes to a Stick unless the DJ approved that exact write.
//! `stick_write_plan` is the only place a token is issued: it works out
//! what the write would do, without writing or settling anything, and binds
//! the token to
//!
//! - the operation itself (which playlist, which tracks, which edit),
//! - the Stick: its mount path and volume identity (the mounted directory's
//!   device and inode, so another Stick at the same path is a different
//!   Stick),
//! - the Stick's revision: a fingerprint of the files the plan read (its
//!   databases, manifest, settings, sync record and any unfinished-write
//!   journal), so a Stick changed since the plan is refused,
//! - the Library generation the plan was computed from, for writes whose
//!   contents come from the Library (exports and syncs),
//! - and a lifetime.
//!
//! Each Stick-writing command takes the Stick's execution lock, then
//! [`consume`]s its token, before anything else touches the Stick, recovery
//! of an unfinished write included. A missing, used, expired or mismatched
//! token is refused with a message that asks for a fresh plan, and is gone
//! afterwards either way.
//!
//! Some approved writes are two steps (tracks added to a Stick playlist are
//! exported to the Stick first when it lacks them). Such a token carries its
//! steps; after each step the command calls [`Consumed::finish`], which
//! records the Stick's new revision for the next step, still under the lock.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Condvar, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult, ErrorKind};

/// How long a plan's approval lasts.
pub const APPROVAL_LIFETIME: Duration = Duration::from_secs(10 * 60);

/// One write to a Stick, as the command that performs it describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum StickOperation {
    /// Export Playlist (or Export Folder).
    ExportPlaylist {
        playlist: String,
        delete_unlisted_music: bool,
    },
    /// Export Track: tracks in no playlist (the Stick's playlists from this
    /// Library are written again with them).
    ExportTracks { tracks: Vec<String> },
    /// SYNC of the Sync Manager, for one Stick.
    Sync {
        playlists: Vec<String>,
        delete_unlisted_music: bool,
        automatic: bool,
    },
    /// A change to one library's playlists on the Stick, as sent.
    PlaylistEdit {
        format: String,
        edit: serde_json::Value,
    },
    /// Loose files copied to the Stick and added to one of its playlists.
    PlaylistAddFiles {
        format: String,
        playlist: String,
        files: Vec<String>,
    },
    /// The Stick's own settings (DJ System, written to the Stick).
    SaveSettings { settings: serde_json::Value },
    /// Defaults written to a Stick that has none.
    WriteDefaults,
    /// An empty library put on a blank Stick.
    EnsureLibrary,
    /// The Stick erased and formatted.
    Format { layout: String },
}

/// What one step of an approval allows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Grant {
    /// Exactly this operation.
    Exact(StickOperation),
    /// Export Track of some of these tracks, or none (optional).
    ExportSome(BTreeSet<String>),
    /// An `add` edit of at most `max` entries to this playlist, with this
    /// duplicates choice.
    AddTo {
        format: String,
        playlist: String,
        max: usize,
        duplicates: bool,
    },
}

impl Grant {
    fn allows(&self, op: &StickOperation) -> bool {
        match (self, op) {
            (Self::Exact(want), got) => want == got,
            (Self::ExportSome(allowed), StickOperation::ExportTracks { tracks }) => {
                !tracks.is_empty() && tracks.iter().all(|t| allowed.contains(t))
            }
            (
                Self::AddTo {
                    format,
                    playlist,
                    max,
                    duplicates,
                },
                StickOperation::PlaylistEdit { format: f, edit },
            ) => {
                f == format
                    && edit.get("kind").and_then(|k| k.as_str()) == Some("add")
                    && edit.get("playlist").and_then(|p| p.as_str()) == Some(playlist.as_str())
                    && edit
                        .get("duplicates")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
                        == *duplicates
                    && edit
                        .get("tracks")
                        .and_then(|t| t.as_array())
                        .is_some_and(|t| !t.is_empty() && t.len() <= *max)
            }
            _ => false,
        }
    }

    const fn optional(&self) -> bool {
        matches!(self, Self::ExportSome(_))
    }
}

/// What a token was issued for.
#[derive(Debug, Clone)]
struct Approval {
    stick: PathBuf,
    volume: String,
    revision: String,
    /// The Library generation the plan read; `None` when the write does not
    /// take its contents from the Library.
    generation: Option<u64>,
    issued: Instant,
    steps: VecDeque<Grant>,
}

static APPROVALS: LazyLock<Mutex<HashMap<String, Approval>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
/// The Sticks being written to now, and the signal that one is free again.
static LOCKS: LazyLock<(Mutex<HashSet<PathBuf>>, Condvar)> =
    LazyLock::new(|| (Mutex::new(HashSet::new()), Condvar::new()));

fn approvals() -> MutexGuard<'static, HashMap<String, Approval>> {
    APPROVALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The Stick's execution lock: every approved write holds it from checking
/// its token to the end of the write, so nothing this app does to the Stick
/// can come between the check and the write. Released when dropped.
pub struct StickLock {
    stick: PathBuf,
}

impl Drop for StickLock {
    fn drop(&mut self) {
        let (held, freed) = &*LOCKS;
        held.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.stick);
        freed.notify_all();
    }
}

/// Takes the execution lock of the Stick at `stick`, waiting for a write
/// already holding it.
pub fn lock(stick: &Path) -> StickLock {
    let (held, freed) = &*LOCKS;
    let mut busy = held
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    while busy.contains(stick) {
        busy = freed
            .wait(busy)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }
    busy.insert(stick.to_path_buf());
    StickLock {
        stick: stick.to_path_buf(),
    }
}

/// The mounted directory's identity: another volume at the same path differs.
pub fn volume_identity(stick: &Path) -> AppResult<String> {
    let meta = std::fs::metadata(stick).map_err(|_| gone())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(format!("dev {} ino {}", meta.dev(), meta.ino()))
    }
    #[cfg(not(unix))]
    {
        Ok(format!("created {:?}", meta.created().ok()))
    }
}

/// A fingerprint of what a plan reads on the Stick, by metadata alone (no
/// file is opened): the journal of an unfinished write and the marker at
/// the root, and every file of the export's `rekordbox` folder and the
/// settings beside it.
pub fn stick_revision(stick: &Path) -> String {
    let mut seen: Vec<String> = Vec::new();
    let mut note = |path: &Path| {
        let rel = path
            .strip_prefix(stick)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();
        match std::fs::symlink_metadata(path) {
            Ok(meta) => {
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                seen.push(format!("{rel}:{}:{modified}", meta.len()));
            }
            Err(_) => seen.push(format!("{rel}:-")),
        }
    };
    for name in [".rbxport-publication", ".rbxport-disposable"] {
        note(&stick.join(name));
    }
    if let Ok(entries) = std::fs::read_dir(stick) {
        let mut roots: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        roots.sort();
        for root in roots {
            let mut files = Vec::new();
            for name in [
                "DEVSETTING.DAT",
                "MYSETTING.DAT",
                "MYSETTING2.DAT",
                "DJMMYSETTING.DAT",
                "rbxport/manifest.json",
            ] {
                files.push(root.join(name));
            }
            if let Ok(inner) = std::fs::read_dir(root.join("rekordbox")) {
                files.extend(inner.flatten().map(|e| e.path()));
            }
            files.sort();
            for file in files {
                if file.exists() {
                    note(&file);
                }
            }
        }
    }
    seen.join("\n")
}

fn gone() -> AppError {
    AppError::new(
        ErrorKind::NotFound,
        "That device is no longer connected. It may have been unplugged or renamed.",
    )
}

fn refused(why: &str) -> AppError {
    AppError::new(
        ErrorKind::Conflict,
        format!("{why} Nothing was written to the Stick. Look at what the write would do again and confirm it."),
    )
}

/// Issues a token for `steps` on the Stick at `stick`, bound to what is
/// there now. Called by `stick_write_plan` alone.
pub(crate) fn issue(stick: &Path, steps: Vec<Grant>, generation: Option<u64>) -> AppResult<String> {
    let approval = Approval {
        stick: stick.to_path_buf(),
        volume: volume_identity(stick)?,
        revision: stick_revision(stick),
        generation,
        issued: Instant::now(),
        steps: steps.into(),
    };
    let token = uuid::Uuid::new_v4().simple().to_string();
    let mut all = approvals();
    all.retain(|_, a| a.issued.elapsed() < APPROVAL_LIFETIME);
    all.insert(token.clone(), approval);
    Ok(token)
}

/// A token's step, taken. Call [`Consumed::finish`] once the write is done
/// when the token has further steps; dropped without it, they are gone.
#[must_use]
pub struct Consumed {
    token: String,
    rest: Option<Approval>,
}

impl Consumed {
    /// The write this step allowed is done: the next step, if any, is bound
    /// to the Stick as it is now.
    pub fn finish(mut self, stick: &Path) {
        if let Some(mut next) = self.rest.take() {
            next.revision = stick_revision(stick);
            next.issued = Instant::now();
            approvals().insert(self.token.clone(), next);
        }
    }
}

/// Checks and uses up the approval for `op` on the Stick at `stick`. Call it
/// holding [`lock`], before any recovery or write. `generation` is the
/// Library generation now, for writes whose contents come from the Library.
pub fn consume(
    token: Option<&str>,
    op: &StickOperation,
    stick: &Path,
    generation: u64,
) -> AppResult<Consumed> {
    if !stick.is_dir() {
        return Err(gone());
    }
    let Some(token) = token.filter(|t| !t.is_empty()) else {
        return Err(refused("This write to the Stick was not confirmed."));
    };
    // Taken out whatever happens next: a token is good for one try.
    let Some(mut approval) = approvals().remove(token) else {
        return Err(refused(
            "This confirmation was already used, or is unknown.",
        ));
    };
    if approval.issued.elapsed() >= APPROVAL_LIFETIME {
        return Err(refused("This confirmation has expired."));
    }
    if approval.stick != stick {
        return Err(refused("This confirmation was for another Stick."));
    }
    if volume_identity(stick)? != approval.volume {
        return Err(refused(
            "The Stick at this place is not the one the write was confirmed for.",
        ));
    }
    if stick_revision(stick) != approval.revision {
        return Err(refused("The Stick changed since the write was confirmed."));
    }
    // Only a write whose contents come from the Library is bound to it.
    let from_library = matches!(
        op,
        StickOperation::ExportPlaylist { .. }
            | StickOperation::ExportTracks { .. }
            | StickOperation::Sync { .. }
    );
    if from_library && approval.generation.is_some_and(|g| g != generation) {
        return Err(refused(
            "The Library changed since the write was confirmed.",
        ));
    }
    while let Some(step) = approval.steps.pop_front() {
        if step.allows(op) {
            let rest = if approval.steps.is_empty() {
                None
            } else {
                Some(approval)
            };
            return Ok(Consumed {
                token: token.to_owned(),
                rest,
            });
        }
        if !step.optional() {
            break;
        }
    }
    Err(refused("This confirmation was for a different write."))
}

/// The commands that write to a Stick. Each takes an approval token and
/// consumes it under the Stick's lock before anything else; the source
/// guard in the tests holds this list to the code.
pub const STICK_WRITING_COMMANDS: &[&str] = &[
    "export_playlist",
    "export_tracks_to_device",
    "sync_devices",
    "device_playlist_edit",
    "device_playlist_add_files",
    "save_device_settings",
    "write_device_defaults",
    "ensure_device_library",
    "format_device",
];

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn stick() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("PIONEER/rekordbox")).unwrap();
        std::fs::write(dir.path().join("PIONEER/rekordbox/export.pdb"), b"one").unwrap();
        dir
    }
    fn export(tracks: &[&str]) -> StickOperation {
        StickOperation::ExportTracks {
            tracks: tracks.iter().map(|t| (*t).to_owned()).collect(),
        }
    }

    #[test]
    fn a_write_needs_a_token_and_a_token_is_good_once() {
        let dir = stick();
        let op = export(&["1"]);
        assert!(consume(None, &op, dir.path(), 0).is_err());
        assert!(consume(Some("nope"), &op, dir.path(), 0).is_err());
        let token = issue(dir.path(), vec![Grant::Exact(op.clone())], Some(0)).unwrap();
        let _ = consume(Some(&token), &op, dir.path(), 0).unwrap();
        let again = consume(Some(&token), &op, dir.path(), 0).err().unwrap();
        assert!(again.to_string().contains("already used"), "{again}");
    }

    #[test]
    fn a_token_is_for_its_own_write_stick_library_and_stick_contents() {
        let dir = stick();
        let op = export(&["1"]);
        let other = stick();
        let token = issue(dir.path(), vec![Grant::Exact(op.clone())], Some(0)).unwrap();
        assert!(
            consume(Some(&token), &export(&["2"]), dir.path(), 0).is_err(),
            "another write"
        );
        let token = issue(dir.path(), vec![Grant::Exact(op.clone())], Some(0)).unwrap();
        assert!(
            consume(Some(&token), &op, other.path(), 0).is_err(),
            "another Stick"
        );
        let token = issue(dir.path(), vec![Grant::Exact(op.clone())], Some(0)).unwrap();
        assert!(
            consume(Some(&token), &op, dir.path(), 1).is_err(),
            "the Library changed"
        );
        let token = issue(dir.path(), vec![Grant::Exact(op.clone())], Some(0)).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(dir.path().join("PIONEER/rekordbox/export.pdb"), b"changed").unwrap();
        assert!(
            consume(Some(&token), &op, dir.path(), 0).is_err(),
            "the Stick changed"
        );
        let token = issue(dir.path(), vec![Grant::Exact(op.clone())], Some(0)).unwrap();
        std::fs::write(dir.path().join(".rbxport-publication"), b"{}").unwrap();
        assert!(
            consume(Some(&token), &op, dir.path(), 0).is_err(),
            "an unfinished write appeared"
        );
    }

    #[test]
    fn another_volume_at_the_same_place_is_refused() {
        let parent = tempfile::tempdir().unwrap();
        let mount = parent.path().join("STICK");
        std::fs::create_dir_all(mount.join("PIONEER/rekordbox")).unwrap();
        let op = export(&["1"]);
        let token = issue(&mount, vec![Grant::Exact(op.clone())], None).unwrap();
        // Replaced: same path, another directory (another inode).
        std::fs::remove_dir_all(&mount).unwrap();
        std::fs::create_dir_all(mount.join("PIONEER/rekordbox")).unwrap();
        let refused = consume(Some(&token), &op, &mount, 0).err().unwrap();
        assert!(refused.to_string().contains("not the one"), "{refused}");
    }

    #[test]
    fn a_two_step_approval_allows_its_export_then_its_add_bound_to_the_stick_after_the_export() {
        let dir = stick();
        let add = StickOperation::PlaylistEdit {
            format: "deviceLibrary".into(),
            edit: serde_json::json!({ "kind": "add", "playlist": "7", "tracks": ["101", "102"], "duplicates": false }),
        };
        let steps = vec![
            Grant::ExportSome(["1".to_owned(), "2".to_owned()].into()),
            Grant::AddTo {
                format: "deviceLibrary".into(),
                playlist: "7".into(),
                max: 2,
                duplicates: false,
            },
        ];
        let token = issue(dir.path(), steps.clone(), Some(0)).unwrap();
        assert!(
            consume(Some(&token), &export(&["3"]), dir.path(), 0).is_err(),
            "not an approved track"
        );
        let token = issue(dir.path(), steps.clone(), Some(0)).unwrap();
        let first = consume(Some(&token), &export(&["1"]), dir.path(), 0).unwrap();
        std::fs::write(
            dir.path().join("PIONEER/rekordbox/export.pdb"),
            b"after the export",
        )
        .unwrap();
        first.finish(dir.path());
        let _ = consume(Some(&token), &add, dir.path(), 0).unwrap();
        // Without anything to export, the add alone.
        let token = issue(dir.path(), steps, Some(0)).unwrap();
        let _ = consume(Some(&token), &add, dir.path(), 0).unwrap();
    }

    #[test]
    fn an_expired_token_is_refused() {
        let dir = stick();
        let op = StickOperation::WriteDefaults;
        let token = issue(dir.path(), vec![Grant::Exact(op.clone())], None).unwrap();
        approvals().get_mut(&token).unwrap().issued =
            Instant::now().checked_sub(APPROVAL_LIFETIME).unwrap();
        let expired = consume(Some(&token), &op, dir.path(), 0).err().unwrap();
        assert!(expired.to_string().contains("expired"), "{expired}");
    }
}
