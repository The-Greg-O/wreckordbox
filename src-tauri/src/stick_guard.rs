//! The one way into a write that may land on a Stick (wbx-45.11, Stick
//! Protection; wbx-45.4).
//!
//! Every command that can write to a Stick first calls [`enter`] (a
//! destination that may or may not be on a Stick: an export folder, a saved
//! file) or [`enter_stick`] (a Stick's own root: an edit of its playlists,
//! its settings, a format, a settle), or [`enter_many`] for a sync. Each:
//!
//! 1. resolves the destination and classifies it natively: on a mounted
//!    Stick (one the device list shows, or a folder standing in for one in
//!    tests) or an ordinary folder;
//! 2. for a Stick, takes the Stick's execution lock (several in a fixed
//!    order, duplicates refused, so two writers can never wait on each
//!    other);
//! 3. refuses, with [`ErrorKind::StickProtected`] and the Stick's name, a
//!    Stick the DJ has not unlocked: nothing is written, nothing settled;
//! 4. answers a [`StickAccess`], which the write helpers that touch a
//!    Stick take as a parameter. Only this module makes one, so a writer
//!    cannot reach those helpers without passing here.
//!
//! An ordinary folder is never refused and needs no approval. The window's
//! per-operation approval (`stick_approval`) is checked by the caller after
//! this, for a Stick only.

use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult, ErrorKind};
use crate::stick_approval::StickLock;

/// Leave to write: to an ordinary folder, or to a Stick the DJ unlocked,
/// held under its lock for as long as this lives.
#[must_use]
pub struct StickAccess {
    stick: Option<PathBuf>,
    _locks: Vec<StickLock>,
}

impl StickAccess {
    /// The Stick written to, or `None` for an ordinary folder.
    pub fn stick(&self) -> Option<&Path> {
        self.stick.as_deref()
    }
}

/// The mounted Sticks' folders, as the device list has them.
fn mounted() -> Vec<PathBuf> {
    rbl_devices::list()
        .into_iter()
        .map(|device| device.mount_point)
        .collect()
}

/// The Stick `destination` is on, or `None` for an ordinary folder.
pub fn stick_of(destination: &Path) -> Option<PathBuf> {
    wreckordbox_stick_protection::stick_root_of(destination, &mounted())
}

fn refused(root: &Path) -> AppError {
    AppError::new(
        ErrorKind::StickProtected,
        wreckordbox_stick_protection::refusal(root),
    )
}

fn check(state_dir: &Path, root: &Path) -> AppResult<()> {
    if wreckordbox_stick_protection::is_unlocked(state_dir, root) {
        Ok(())
    } else {
        Err(refused(root))
    }
}

/// A write to `destination`, which may be on a Stick: see the module.
pub fn enter(state_dir: &Path, destination: &Path) -> AppResult<StickAccess> {
    match stick_of(destination) {
        None => Ok(StickAccess {
            stick: None,
            _locks: Vec::new(),
        }),
        Some(root) => enter_root(state_dir, root),
    }
}

/// A write to the Stick whose root is `root` (its playlists, settings,
/// format, settle): always a Stick, whatever the device list says.
pub fn enter_stick(state_dir: &Path, root: &Path) -> AppResult<StickAccess> {
    if !root.is_dir() {
        return Err(AppError::new(
            ErrorKind::NotFound,
            "That device is no longer connected. It may have been unplugged or renamed.",
        ));
    }
    let root = stick_of(root)
        .unwrap_or_else(|| root.canonicalize().unwrap_or_else(|_| root.to_path_buf()));
    enter_root(state_dir, root)
}

fn enter_root(state_dir: &Path, root: PathBuf) -> AppResult<StickAccess> {
    let lock = crate::stick_approval::lock(&root);
    check(state_dir, &root)?;
    Ok(StickAccess {
        stick: Some(root),
        _locks: vec![lock],
    })
}

/// The destinations of one sync, each classified and resolved: refused
/// when two are the same Stick, then every Stick's lock taken in path
/// order, before any approval is used. Each destination's leave, or why it
/// was refused, in the order asked.
pub fn enter_many(
    state_dir: &Path,
    destinations: &[String],
) -> AppResult<Vec<AppResult<StickAccess>>> {
    // Each destination resolved, and whether it is a Stick or an ordinary folder.
    let places: Vec<Option<(PathBuf, bool)>> = destinations
        .iter()
        .map(|d| {
            let path = Path::new(d);
            if !path.is_dir() {
                return None;
            }
            Some(match stick_of(path) {
                Some(root) => (root, true),
                None => (
                    path.canonicalize().unwrap_or_else(|_| path.to_path_buf()),
                    false,
                ),
            })
        })
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    for (place, _) in places.iter().flatten() {
        if !seen.insert(place.clone()) {
            return Err(AppError::internal(
                "The same USB device was selected more than once. Select it once and retry.",
            ));
        }
    }
    // Locks in path order, whatever order the destinations were asked in.
    let mut order: Vec<usize> = (0..places.len()).filter(|i| places[*i].is_some()).collect();
    order.sort_by(|a, b| {
        places[*a]
            .as_ref()
            .map(|p| &p.0)
            .cmp(&places[*b].as_ref().map(|p| &p.0))
    });
    let mut held: Vec<Option<StickLock>> = (0..places.len()).map(|_| None).collect();
    for i in order {
        if let Some((place, _)) = &places[i] {
            held[i] = Some(crate::stick_approval::lock(place));
        }
    }
    Ok(places
        .into_iter()
        .zip(held)
        .map(|(place, lock)| match (place, lock) {
            (Some((root, true)), Some(lock)) => {
                check(state_dir, &root)?;
                Ok(StickAccess {
                    stick: Some(root),
                    _locks: vec![lock],
                })
            }
            (Some((_, false)), Some(lock)) => Ok(StickAccess {
                stick: None,
                _locks: vec![lock],
            }),
            _ => Err(AppError::new(
                ErrorKind::NotFound,
                "That device is no longer connected. It may have been unplugged or renamed.",
            )),
        })
        .collect())
}

/// Leave to write the audio files at `paths` (their tags): refused when
/// any is on a Stick the DJ has not unlocked. Each Stick's lock is taken
/// once, in path order, and held while the leave lives.
pub fn enter_files<'a>(
    state_dir: &Path,
    paths: impl IntoIterator<Item = &'a Path>,
) -> AppResult<StickAccess> {
    let roots: std::collections::BTreeSet<PathBuf> =
        paths.into_iter().filter_map(stick_of).collect();
    let mut locks = Vec::with_capacity(roots.len());
    for root in &roots {
        locks.push(crate::stick_approval::lock(root));
        check(state_dir, root)?;
    }
    Ok(StickAccess {
        stick: roots.into_iter().next(),
        _locks: locks,
    })
}

/// Whether the Stick at `path` is protected (wbx-45.11).
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri passes command arguments by value"
)]
pub fn stick_protection(
    state: tauri::State<'_, std::sync::Arc<crate::state::AppState>>,
    path: String,
) -> wreckordbox_stick_protection::Status {
    wreckordbox_stick_protection::status(state.backup_dir(), Path::new(&path))
}

/// Unlocks the Stick at `path` until it is ejected, removed or put back,
/// or locked again. A Gig stick needs `name`, typed by the DJ, to be its
/// name; a Spare stick needs only the DJ's confirmation.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri passes command arguments by value"
)]
pub fn stick_unlock(
    state: tauri::State<'_, std::sync::Arc<crate::state::AppState>>,
    path: String,
    name: Option<String>,
) -> AppResult<wreckordbox_stick_protection::Status> {
    use wreckordbox_stick_protection::UnlockError;
    let root = stick_of(Path::new(&path)).unwrap_or_else(|| PathBuf::from(&path));
    wreckordbox_stick_protection::unlock(state.backup_dir(), &root, name.as_deref()).map_err(
        |error| {
            let kind = match error {
                UnlockError::NotFound => ErrorKind::NotFound,
                UnlockError::NameRequired { .. } | UnlockError::NameMismatch { .. } => {
                    ErrorKind::Malformed
                }
                UnlockError::Io(_) => ErrorKind::Internal,
            };
            AppError::new(kind, error.to_string())
        },
    )
}

/// Protects the Stick at `path` again.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri passes command arguments by value"
)]
pub fn stick_lock(
    state: tauri::State<'_, std::sync::Arc<crate::state::AppState>>,
    path: String,
) -> AppResult<wreckordbox_stick_protection::Status> {
    let root = stick_of(Path::new(&path)).unwrap_or_else(|| PathBuf::from(&path));
    wreckordbox_stick_protection::lock(state.backup_dir(), &root).map_err(|e| {
        AppError::internal("The Stick could not be locked.").with_detail(e.to_string())
    })
}

/// Every registered command that can write to a Stick, each entering this
/// guard before it writes (the source guard in `tests/stick_protection.rs`
/// requires every other registered command to be listed there as not
/// writing to a Stick, and finds writers that are neither).
pub const STICK_WRITERS: &[&str] = &[
    // Writes to the Stick itself; each also needs the DJ's per-operation
    // confirmation (`stick_approval::STICK_WRITING_COMMANDS`).
    "export_playlist",
    "export_tracks_to_device",
    "sync_devices",
    "device_playlist_edit",
    "device_playlist_add_files",
    "device_playlist_add_library_tracks",
    "save_device_settings",
    "ensure_device_library",
    "stick_settle",
    "format_device",
    // Files saved where the DJ chooses: a Stick write when that is on one.
    "export_playlist_file",
    "export_xml",
    "export_loop_wav",
    "save_log_text",
    "set_backup_directory",
    "back_up_library",
    "start_backup",
    "delete_backup",
    // Audio tags of tracks whose file is on a Stick.
    "set_track_field",
    "undo_edit",
    "redo_edit",
    "analyse_track",
];

/// For the crate's tests: a window holding `state`, and the Stick at
/// `root` unlocked in it as the DJ would (its name typed).
#[cfg(test)]
#[allow(clippy::expect_used)]
pub(crate) mod testing {
    use std::path::Path;
    use std::sync::Arc;

    use tauri::Manager as _;

    pub(crate) fn app(state: crate::state::AppState) -> tauri::App<tauri::test::MockRuntime> {
        let app = tauri::test::mock_app();
        app.manage(Arc::new(state));
        app
    }

    pub(crate) fn unlock(state: &crate::state::AppState, root: &Path) {
        let name = wreckordbox_stick_protection::name_of(
            &root.canonicalize().unwrap_or_else(|_| root.to_path_buf()),
        );
        wreckordbox_stick_protection::unlock(state.backup_dir(), root, Some(&name))
            .expect("unlocked");
    }

    /// A confirmed token for `op` on `stick`.
    pub(crate) fn token(stick: &Path, op: crate::stick_approval::StickOperation) -> Option<String> {
        crate::stick_approval::propose(stick, op, Some(0))
            .and_then(|plan| crate::stick_approval::approve(&plan))
            .ok()
    }
}
