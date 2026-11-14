//! Hold-and-release points for controlled race tests (wbx-45.14).
//!
//! A write that a test must interrupt at a known moment calls [`point`]
//! with the moment's name. Unarmed (always, outside test mode) it returns
//! at once. A test arms a name with [`arm`] (only under `RBXPORT_TEST=1`),
//! waits with [`Armed::reached`] until a writer stands there, does what the
//! race needs (lock the Stick, edit the Library, ...) and lets the writer go
//! with [`Armed::release`] (or by dropping the handle). No sleeps: each step
//! waits on the other's signal.
//!
//! Points are scoped by a path (the writer's state folder), so tests running
//! in parallel, each with its own temporary state folder, never hold each
//! other's writers.
//!
//! A point can also be armed to fail ([`arm_failure`]): the writer's
//! [`fails`] answers `true` once, for a test that needs a step to fail
//! after an earlier one has written.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::{Condvar, Mutex};

#[derive(Default)]
struct Points {
    /// Armed points: whether a writer stands there, whether it may go.
    armed: BTreeMap<String, Point>,
    failures: BTreeSet<String>,
}

#[derive(Default, Clone, Copy)]
struct Point {
    reached: bool,
    released: bool,
}

/// A playlist export to a Stick has checked its confirmation against the
/// Library's generation and not yet read what it exports (wbx-45.14).
pub const EXPORT_CONFIRMED: &str = "export.confirmed";

static ANY: AtomicBool = AtomicBool::new(false);
static POINTS: Mutex<Option<Points>> = Mutex::new(None);
static CHANGED: Condvar = Condvar::new();

/// How long a test waits for the other side before it fails (never a
/// writer: a writer held at a point waits for its release, however long).
const TEST_WAIT: Duration = Duration::from_secs(60);

fn test_mode() -> bool {
    std::env::var_os("RBXPORT_TEST").is_some_and(|v| v == "1")
}

fn with<T>(f: impl FnOnce(&mut Points) -> T) -> T {
    let mut points = POINTS.lock();
    f(points.get_or_insert_with(Points::default))
}

fn key(name: &str, scope: &Path) -> String {
    format!("{name}@{}", scope.display())
}

/// A writer whose state folder is `scope` reaches the moment `name`: held
/// there while a test has armed it.
pub fn point(name: &str, scope: &Path) {
    if !ANY.load(Ordering::Acquire) {
        return;
    }
    let name = &key(name, scope);
    let mut points = POINTS.lock();
    let Some(state) = points.as_mut().and_then(|p| p.armed.get_mut(name)) else {
        return;
    };
    state.reached = true;
    CHANGED.notify_all();
    loop {
        match points.as_ref().and_then(|p| p.armed.get(name)) {
            Some(state) if !state.released => CHANGED.wait(&mut points),
            _ => break,
        }
    }
    if let Some(p) = points.as_mut() {
        p.armed.remove(name);
    }
    CHANGED.notify_all();
}

/// Whether the step `name` is to fail, once (armed by [`arm_failure`]).
pub fn fails(name: &str, scope: &Path) -> bool {
    ANY.load(Ordering::Acquire) && with(|p| p.failures.remove(&key(name, scope)))
}

/// A point a test armed: the writer is held there until released.
#[must_use]
pub struct Armed {
    name: String,
}

/// Arms the point `name` for writers whose state folder is `scope`; the next writer to reach it is held there.
///
/// # Panics
///
/// Outside test mode (`RBXPORT_TEST=1`).
pub fn arm(name: &str, scope: &Path) -> Armed {
    assert!(
        test_mode(),
        "test holds are armed only under RBXPORT_TEST=1"
    );
    let name = key(name, scope);
    with(|p| p.armed.insert(name.clone(), Point::default()));
    ANY.store(true, Ordering::Release);
    Armed { name }
}

/// Makes the step `name` fail once, for a writer of `scope`.
///
/// # Panics
///
/// Outside test mode (`RBXPORT_TEST=1`).
pub fn arm_failure(name: &str, scope: &Path) {
    assert!(
        test_mode(),
        "test holds are armed only under RBXPORT_TEST=1"
    );
    with(|p| p.failures.insert(key(name, scope)));
    ANY.store(true, Ordering::Release);
}

impl Armed {
    /// Waits until a writer stands at the point.
    ///
    /// # Panics
    ///
    /// When none arrives within a minute: the race the test meant did not
    /// happen.
    pub fn reached(&self) {
        let mut points = POINTS.lock();
        let deadline = std::time::Instant::now() + TEST_WAIT;
        loop {
            match points.as_ref().and_then(|p| p.armed.get(&self.name)) {
                Some(state) if state.reached => return,
                Some(_) => {
                    assert!(
                        !CHANGED.wait_until(&mut points, deadline).timed_out(),
                        "no writer reached {}",
                        self.name
                    );
                }
                None => panic!("{} was released before it was reached", self.name),
            }
        }
    }

    /// Whether a writer stands at the point now (without waiting).
    pub fn is_reached(&self) -> bool {
        with(|p| p.armed.get(&self.name).is_some_and(|s| s.reached))
    }

    /// Lets the writer at the point go (and disarms it).
    pub fn release(self) {
        drop(self);
    }
}

impl Drop for Armed {
    fn drop(&mut self) {
        with(|p| {
            if let Some(state) = p.armed.get_mut(&self.name) {
                if state.reached {
                    state.released = true;
                } else {
                    p.armed.remove(&self.name);
                }
            }
        });
        CHANGED.notify_all();
    }
}
