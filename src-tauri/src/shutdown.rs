//! Closing the window while a Stick load runs (wbx-71).
//!
//! One barrier per window ([`Shutdown`], in `AppState`) counts the Stick
//! loads running and the drain of the hosted service. Stopping the service
//! ([`AppState::stop_service`]) is a drain: the service is taken out and
//! stopped (which asks a running load to stop where it safely can and waits
//! for its answer), then the drain waits until every load command has kept
//! its report ([`crate::stick_load_reports`]) and let go; only then is it
//! complete. A second stop while one drains waits for that one. The barrier
//! outlives taking the service out, so nothing that comes later finds an
//! empty slot and goes ahead.
//!
//! The lifecycle on top ([`exit_requested`], [`on_exit`], [`drain_then`]),
//! called from `lib.rs`'s run loop and `switch_library`:
//! - an exit asked for while a load runs (or a drain is under way) is held;
//!   a thread of its own drains and then asks for the exit again. Every
//!   further ordinary exit request is held until the drain completes.
//! - the final `Exit`, and an exit that cannot be held (a restart's), wait
//!   for the drain: [`on_exit`] stops the service, which joins a drain under
//!   way.
//! - a restart (`switch_library`) drains first, then restarts.
//!
//! No timeout: a load is never abandoned part-way (wbx-70).
//!
//! **Closed** (wbx-45.14): each lifecycle step above first latches the
//! window closed, for good ([`AppState::close_for_shutdown`]);
//! [`AppState::stop_service`] alone stays a plain drain. From then on no new core write starts (the Library core refuses
//! every write with [`CLOSING`], checked under the edit gate, so an edit
//! queued behind another is refused too), the window cannot host a service
//! again ([`AppState::host_service`] refuses), and no new service command
//! is handed a caller. Edits already under way finish, with the Ledger
//! still attached, before the service is let go.

use std::sync::Arc;

use parking_lot::{Condvar, Mutex};

use crate::state::AppState;

#[derive(Default)]
pub struct Shutdown {
    inner: Mutex<Inner>,
    changed: Condvar,
}

#[derive(Default)]
struct Inner {
    /// Stick load commands running, from before the service is asked until
    /// their report is kept.
    loads: usize,
    /// A drain is under way.
    draining: bool,
    /// Drains completed, so a waiter knows the one it joined is over.
    drains: u64,
    /// The window is closing: latched by the first stop, never undone.
    closed: bool,
    /// An exit was held and its drain started.
    exiting: bool,
    /// That drain is over: the exit may happen.
    exit_ready: bool,
}

/// One Stick load command running, until dropped.
pub struct LoadRunning<'a>(&'a Shutdown);

impl Drop for LoadRunning<'_> {
    fn drop(&mut self) {
        let mut inner = self.0.inner.lock();
        inner.loads = inner.loads.saturating_sub(1);
        self.0.changed.notify_all();
    }
}

/// Why nothing more is written once the window is closing.
pub const CLOSING: &str = "wreckordbox is closing, so nothing more is changed. Nothing was written.";

impl Shutdown {
    /// Latches the window closed; answers whether this call closed it.
    pub(crate) fn close(&self) -> bool {
        let mut inner = self.inner.lock();
        let was = inner.closed;
        inner.closed = true;
        !was
    }

    /// Whether the window is closing (or closed).
    pub fn is_closed(&self) -> bool {
        self.inner.lock().closed
    }

    /// Counts a load command until its report is kept (drop the guard then).
    pub(crate) fn load_running(&self) -> LoadRunning<'_> {
        self.inner.lock().loads += 1;
        LoadRunning(self)
    }

    /// Drains: `stop` (take the service out and stop it), then waits until
    /// every load command has kept its report. Joins a drain under way.
    pub(crate) fn drain(&self, stop: impl FnOnce()) {
        let mut inner = self.inner.lock();
        if inner.draining {
            let joined = inner.drains;
            while inner.drains == joined {
                self.changed.wait(&mut inner);
            }
            return;
        }
        inner.draining = true;
        drop(inner);
        stop();
        let mut inner = self.inner.lock();
        while inner.loads > 0 {
            self.changed.wait(&mut inner);
        }
        inner.draining = false;
        inner.drains += 1;
        self.changed.notify_all();
    }
}

/// An exit was asked for (`RunEvent::ExitRequested`): answers whether to
/// prevent it. `preventable` is false for an exit that cannot be held (a
/// restart's); `exit` asks for the exit again once the drain is over.
pub fn exit_requested(
    state: &Arc<AppState>,
    preventable: bool,
    exit: impl FnOnce() + Send + 'static,
) -> bool {
    let shutdown = &state.shutdown;
    let mut inner = shutdown.inner.lock();
    if inner.exit_ready {
        return false;
    }
    if inner.exiting {
        // Held until the drain under way is over; it asks for the exit then.
        return preventable;
    }
    if (inner.loads == 0 && !inner.draining) || !preventable {
        // Nothing to wait for, or it cannot be held: the final `Exit`
        // (`on_exit`) drains.
        return false;
    }
    inner.exiting = true;
    drop(inner);
    let state = Arc::clone(state);
    std::thread::spawn(move || {
        state.close_for_shutdown();
        state.stop_service();
        state.shutdown.inner.lock().exit_ready = true;
        exit();
    });
    true
}

/// The app is exiting (`RunEvent::Exit`), which cannot be prevented: the
/// service is stopped, joining a drain under way, and every load's report
/// is kept before this returns.
pub fn on_exit(state: &Arc<AppState>) {
    state.close_for_shutdown();
    state.stop_service();
}

/// Runs `then` (a restart) once the service is drained: any load stopped
/// where it safely can and its report kept.
pub fn drain_then<T>(state: &Arc<AppState>, then: impl FnOnce() -> T) -> T {
    state.close_for_shutdown();
    state.stop_service();
    then()
}
