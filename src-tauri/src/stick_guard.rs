//! The one way into a write that may land on a Stick (wbx-45.11, Stick
//! Protection; wbx-45.4).
//!
//! Every command that can write to a Stick first calls [`enter`] (a
//! destination that may or may not be on a Stick: an export folder, a saved
//! file), [`enter_stick`] (a Stick's own root: its playlists, its settings,
//! a format, a settle), [`enter_many`] (a sync) or [`enter_files`] (audio
//! tags). Each classifies the destination from its volume
//! ([`wreckordbox_stick_protection::classify`], never the device list the
//! interface was shown), and for a Stick takes its execution lock (a lock
//! file in the state folder, shared with the Library service, the CLI, a
//! DJ's lock and an eject; several in path order) and refuses, with
//! [`ErrorKind::StickProtected`], a Stick the DJ has not unlocked for this
//! very mount.
//!
//! What it answers, a [`StickAccess`], is the capability the write helpers
//! that touch a Stick take as a parameter: only this module makes one, so a
//! write cannot be made without holding the Stick's lock and its unlock. A
//! long write calls [`StickAccess::recheck`] before each step and stops when
//! the Stick was removed or replaced.
//!
//! **Lock order, everywhere:** the Library edit gate first, then the
//! Sticks' execution locks, then the analysis lock. A writer that reads the
//! Library takes the edit gate before it enters here, captures what it
//! needs, and lets the gate go before the long write; nothing holding a
//! Stick's lock takes the edit gate afterwards.

use std::path::{Path, PathBuf};

use wreckordbox_stick_protection::{Authorization, Refused};

use crate::error::{AppError, AppResult, ErrorKind};

/// Leave to write: to an ordinary folder, or to Sticks the DJ unlocked,
/// held under their execution locks for as long as this lives.
#[must_use]
#[derive(Debug)]
pub struct StickAccess {
    held: Vec<Authorization>,
    /// Counted as a running writer of its window until dropped, so closing
    /// the window waits for it (wbx-45.14); none for a test's ordinary leave.
    _admitted: Option<crate::write_admission::Admitted>,
}

impl StickAccess {
    /// The Stick written to (the first, for several), or `None` for an
    /// ordinary folder.
    pub fn stick(&self) -> Option<&Path> {
        self.held.iter().find_map(Authorization::stick)
    }

    /// Ends the unlock of every Stick held (an eject at the end of a write),
    /// under the locks this already holds.
    pub fn relock(&self) -> std::io::Result<()> {
        self.held.iter().try_for_each(Authorization::relock)
    }

    /// Whether the write may go on: every Stick held is the same mount and
    /// still unlocked. Call before each step of a long write.
    pub fn recheck(&self) -> AppResult<()> {
        self.held
            .iter()
            .try_for_each(|held| held.recheck().map_err(|e| refused(&e)))
    }

    /// [`Self::recheck`] for the one Stick holding `path` (none for a path
    /// on an ordinary folder): a write to that file may go on (wbx-45.14).
    pub fn recheck_for(&self, path: &Path) -> AppResult<()> {
        self.held
            .iter()
            .filter(|held| held.stick().is_some_and(|root| path.starts_with(root)))
            .try_for_each(|held| held.recheck().map_err(|e| refused(&e)))
    }
}

/// The Stick `destination` is on, or `None` for an ordinary folder.
pub fn stick_of(destination: &Path) -> Option<PathBuf> {
    wreckordbox_stick_protection::stick_root(destination)
}

pub(crate) fn refused(why: &Refused) -> AppError {
    let kind = match why {
        Refused::Protected { .. } => ErrorKind::StickProtected,
        Refused::Replaced { .. } => ErrorKind::NotFound,
        Refused::Io { .. } => ErrorKind::Internal,
    };
    AppError::new(kind, why.to_string())
}

fn gone() -> AppError {
    AppError::new(
        ErrorKind::NotFound,
        "That device is no longer connected. It may have been unplugged or renamed.",
    )
}

/// A write to `destination`, which may be on a Stick: see the module.
pub fn enter(state_dir: &Path, destination: &Path) -> AppResult<StickAccess> {
    let admitted = crate::write_admission::admit(state_dir)?;
    let held =
        wreckordbox_stick_protection::authorize(state_dir, destination).map_err(|e| refused(&e))?;
    Ok(StickAccess { held: vec![held], _admitted: Some(admitted) })
}

/// A write to the Stick whose root is `root` (its playlists, settings,
/// format, settle): always a Stick, whatever its volume says.
pub fn enter_stick(state_dir: &Path, root: &Path) -> AppResult<StickAccess> {
    let admitted = crate::write_admission::admit(state_dir)?;
    if !root.is_dir() {
        return Err(gone());
    }
    let held =
        wreckordbox_stick_protection::authorize_stick(state_dir, root).map_err(|e| refused(&e))?;
    Ok(StickAccess { held: vec![held], _admitted: Some(admitted) })
}

/// Leave to write, or recover, the Library at `location` (wbx-45.11): a
/// Library kept on a Stick is written only while that Stick is unlocked,
/// as the Library core's own edits are (`rbl_library::stick`).
pub fn enter_library(
    state_dir: &Path,
    location: &rbl_db::LibraryLocation,
) -> AppResult<StickAccess> {
    enter(
        state_dir,
        location.master_db.parent().unwrap_or(&location.master_db),
    )
}

/// The destinations of one sync: each resolved and classified, refused when
/// two are the same place, then every Stick entered in path order, before
/// any approval is used. Each destination's leave, or why it was refused,
/// in the order asked.
pub fn enter_many(
    state_dir: &Path,
    destinations: &[String],
) -> AppResult<Vec<AppResult<StickAccess>>> {
    let places: Vec<Option<PathBuf>> = destinations
        .iter()
        .map(|d| {
            let path = Path::new(d);
            path.is_dir().then(|| {
                stick_of(path)
                    .unwrap_or_else(|| path.canonicalize().unwrap_or_else(|_| path.to_path_buf()))
            })
        })
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    for place in places.iter().flatten() {
        if !seen.insert(place.clone()) {
            return Err(AppError::internal(
                "The same USB device was selected more than once. Select it once and retry.",
            ));
        }
    }
    // Entered in path order, whatever order the destinations were asked in.
    let mut order: Vec<usize> = (0..places.len()).filter(|i| places[*i].is_some()).collect();
    order.sort_by(|a, b| places[*a].cmp(&places[*b]));
    let mut entered: Vec<Option<AppResult<StickAccess>>> =
        (0..places.len()).map(|_| None).collect();
    for i in order {
        if let Some(place) = &places[i] {
            entered[i] = Some(enter(state_dir, place));
        }
    }
    Ok(entered
        .into_iter()
        .map(|e| e.unwrap_or_else(|| Err(gone())))
        .collect())
}

/// Leave to write the audio files at `paths` (their tags): refused when any
/// is on a Stick the DJ has not unlocked. Each Stick entered once, in path
/// order, and held while the leave lives.
pub fn enter_files<'a>(
    state_dir: &Path,
    paths: impl IntoIterator<Item = &'a Path>,
) -> AppResult<StickAccess> {
    let admitted = crate::write_admission::admit(state_dir)?;
    let roots: std::collections::BTreeSet<PathBuf> =
        paths.into_iter().filter_map(stick_of).collect();
    let mut held = Vec::with_capacity(roots.len());
    for root in &roots {
        held.push(
            wreckordbox_stick_protection::authorize_stick(state_dir, root)
                .map_err(|e| refused(&e))?,
        );
    }
    Ok(StickAccess { held, _admitted: Some(admitted) })
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
    wreckordbox_stick_protection::unlock(state.backup_dir(), Path::new(&path), name.as_deref())
        .map_err(|error| {
            let kind = match error {
                UnlockError::NotAStick => ErrorKind::NotFound,
                UnlockError::NameRequired { .. } | UnlockError::NameMismatch { .. } => {
                    ErrorKind::Malformed
                }
                UnlockError::NoIdentity => ErrorKind::StickProtected,
                UnlockError::Io(_) => ErrorKind::Internal,
            };
            AppError::new(kind, error.to_string())
        })
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
    // Waits for any write to the Stick to finish before it answers.
    wreckordbox_stick_protection::lock(state.backup_dir(), Path::new(&path)).map_err(|e| {
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
    // The Stick view's load through the hosted service (wbx-71).
    "stick_view_load",
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

    /// Leave to write to an ordinary folder, for helpers tested directly.
    pub(crate) fn ordinary() -> super::StickAccess {
        super::StickAccess { held: Vec::new(), _admitted: None }
    }

    pub(crate) fn app(state: crate::state::AppState) -> tauri::App<tauri::test::MockRuntime> {
        let app = tauri::test::mock_app();
        app.manage(Arc::new(state));
        app
    }

    pub(crate) fn unlock(state: &crate::state::AppState, root: &Path) {
        // A temporary folder standing in for a Stick (the tests run with RBXPORT_TEST=1).
        std::fs::write(
            root.join(wreckordbox_stick_protection::STAND_IN_MARKER),
            b"",
        )
        .expect("marked");
        let name = wreckordbox_stick_protection::name_of(
            &root.canonicalize().unwrap_or_else(|_| root.to_path_buf()),
        );
        wreckordbox_stick_protection::unlock(state.backup_dir(), root, Some(&name))
            .expect("unlocked");
    }

    /// A confirmed token for `op` on `stick`.
    pub(crate) fn token(
        state: &crate::state::AppState,
        stick: &Path,
        op: crate::stick_approval::StickOperation,
    ) -> Option<String> {
        crate::stick_approval::propose(stick, op, Some(0))
            .and_then(|plan| crate::stick_approval::approve(state.backup_dir(), &plan))
            .ok()
    }
}
