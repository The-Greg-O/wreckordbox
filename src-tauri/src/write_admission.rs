//! Admission of the window's direct writers while it closes (wbx-45.14,
//! round 1).
//!
//! Every direct write to a Stick or a chosen folder (an export, a sync, a
//! backup, a key tag, a saved file, a settle...) holds a
//! [`crate::stick_guard::StickAccess`], and only `stick_guard` makes one:
//! it is admitted here first. Admission is per state folder (one window's).
//! [`close`] refuses every admission after it, atomically with the window's
//! other latches (`AppState::close_for_shutdown`), and [`wait_idle`] waits
//! for every writer admitted before it to finish, however long that takes:
//! a write is never abandoned part-way.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use parking_lot::{Condvar, Mutex};

use crate::error::{AppError, AppResult, ErrorKind};

#[derive(Default, Clone, Copy)]
struct Admission {
    closed: bool,
    running: usize,
}

static ADMISSIONS: Mutex<BTreeMap<PathBuf, Admission>> = Mutex::new(BTreeMap::new());
static CHANGED: Condvar = Condvar::new();

/// One admitted writer, counted until dropped.
#[derive(Debug)]
pub struct Admitted {
    key: PathBuf,
}

impl Drop for Admitted {
    fn drop(&mut self) {
        let mut all = ADMISSIONS.lock();
        if let Some(admission) = all.get_mut(&self.key) {
            admission.running = admission.running.saturating_sub(1);
        }
        CHANGED.notify_all();
    }
}

/// Admits a writer for the window whose state folder is `state_dir`;
/// refused once that window is closing.
pub fn admit(state_dir: &Path) -> AppResult<Admitted> {
    let mut all = ADMISSIONS.lock();
    let admission = all.entry(state_dir.to_path_buf()).or_default();
    if admission.closed {
        return Err(AppError::new(ErrorKind::ReadOnly, crate::shutdown::CLOSING));
    }
    admission.running += 1;
    Ok(Admitted {
        key: state_dir.to_path_buf(),
    })
}

/// Refuses every admission for `state_dir` from now on.
pub fn close(state_dir: &Path) {
    ADMISSIONS
        .lock()
        .entry(state_dir.to_path_buf())
        .or_default()
        .closed = true;
}

/// How many writers for `state_dir` are running.
pub fn running(state_dir: &Path) -> usize {
    ADMISSIONS.lock().get(state_dir).map_or(0, |a| a.running)
}

/// Waits until every writer admitted for `state_dir` has finished.
pub fn wait_idle(state_dir: &Path) {
    if running(state_dir) > 0 {
        crate::test_holds::point(crate::test_holds::SHUTDOWN_AWAITING_WRITERS, state_dir);
    }
    let mut all = ADMISSIONS.lock();
    while all.get(state_dir).is_some_and(|a| a.running > 0) {
        CHANGED.wait(&mut all);
    }
}
