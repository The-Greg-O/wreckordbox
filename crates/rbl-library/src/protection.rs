//! Library Protection: the DJ's switch, on until turned off, that refuses
//! every change to the Library, whoever asks for it (the window, a script,
//! the command line, a player over LINK) and whether it is a new edit or an
//! Undo or Redo.
//!
//! The switch is kept in one file, [`FILE`], in the core's recovery folder
//! (the state folder the window and the Library service share). That file
//! is its only source of truth: the window's own preference is at most
//! taken up once, the first time, while no file has ever been written
//! ([`LibraryCore::adopt_protection`]).
//!
//! - No file: protection is **on** ([`Recorded::Never`]).
//! - A file that cannot be read or makes no sense: **on**, until a change
//!   is recorded ([`Recorded::Unreadable`]).
//! - Turning it on where it cannot be recorded: **on** in this process all
//!   the same, until a change is recorded ([`Recorded::Unsaved`]).
//!
//! Every change is made under the core's edit gate and an exclusive lock on
//! [`LOCK_FILE`], so writers in this process and in another one take turns;
//! each writes its own new temporary file and renames it over the old. Each
//! change has a revision higher than any before it, and watchers
//! ([`LibraryCore::watch_protection`]) hear the changes in order.
//!
//! [`LibraryCore::write_then`] checks the switch under the same gate before
//! every write, so an edit already waiting when protection is turned on is
//! refused.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::LibraryCore;

/// The file's name inside the shared state folder.
pub const FILE: &str = "library-protection.json";

/// The file every writer holds an exclusive lock on while it changes
/// [`FILE`].
pub const LOCK_FILE: &str = "library-protection.lock";

/// What the DJ is told when a change is refused: the window's own words
/// (`refusal(true)` in `src/lib/menu.ts`).
pub const MESSAGE: &str =
    "Editing is locked by Library Protection. Turn it off in Preferences to edit.";

/// How the state came about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    /// Nothing has ever been recorded: on.
    Never,
    /// The file cannot be read, or does not say anything sensible: on.
    Unreadable,
    /// The file says so.
    Set,
    /// Turned on in this process, but the file could not be written: on.
    Unsaved,
}

/// Whether Library Protection is on, and which change made it so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtectionState {
    /// Whether every change to the Library is refused.
    pub protect_library: bool,
    /// Higher for every later change; a state with a lower revision than
    /// one already seen is stale.
    pub revision: u64,
    pub recorded: Recorded,
}

/// A change of the switch: what it was, and what it is now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Turned {
    pub previous: bool,
    pub now: ProtectionState,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    protect_library: bool,
    #[serde(default)]
    revision: u64,
}

/// Where the switch is kept, given the shared state folder.
#[must_use]
pub fn file(state_dir: &Path) -> PathBuf {
    state_dir.join(FILE)
}

/// What the file in `state_dir` says, read now.
#[must_use]
pub fn read(state_dir: &Path) -> ProtectionState {
    let on = |recorded| ProtectionState { protect_library: true, revision: 0, recorded };
    match std::fs::read(file(state_dir)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => on(Recorded::Never),
        Err(_) => on(Recorded::Unreadable),
        Ok(text) => match serde_json::from_slice::<Stored>(&text) {
            Ok(stored) => ProtectionState {
                protect_library: stored.protect_library,
                revision: stored.revision,
                recorded: Recorded::Set,
            },
            Err(_) => on(Recorded::Unreadable),
        },
    }
}

/// The refusal of a write while protection is on.
#[must_use]
pub fn refusal() -> rbl_db::DbError {
    rbl_db::DbError::WriteRefused(MESSAGE.to_owned())
}

/// Whether `error` is [`refusal`].
#[must_use]
pub fn refused(error: &rbl_db::DbError) -> bool {
    matches!(error, rbl_db::DbError::WriteRefused(why) if why == MESSAGE)
}

/// Records the switch in `state_dir` with no core to tell: for setting up a
/// test, or a tool with no Library loaded. Takes its turn with every other
/// writer, as [`LibraryCore::set_protection`] does.
pub fn store(state_dir: &Path, protect_library: bool) -> std::io::Result<ProtectionState> {
    let _turn = Turn::take(state_dir)?;
    let revision = read(state_dir).revision + 1;
    write(state_dir, protect_library, revision)
}

/// The switch as a core holds it: the highest revision this process has
/// given out, whether an unrecorded "on" is in force, and who is told.
pub(crate) struct Switch {
    dir: PathBuf,
    held: parking_lot::Mutex<Held>,
    watchers: parking_lot::Mutex<Vec<Watcher>>,
}

#[derive(Default)]
struct Held {
    highest: u64,
    /// Turned on, not recorded: on until a change is.
    unsaved_on: bool,
}

type Watcher = Box<dyn FnMut(ProtectionState) -> bool + Send>;

impl Switch {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self { dir, held: parking_lot::Mutex::new(Held::default()), watchers: parking_lot::Mutex::new(Vec::new()) }
    }

    fn current_with(&self, held: &Held) -> ProtectionState {
        let mut state = read(&self.dir);
        state.revision = state.revision.max(held.highest);
        if held.unsaved_on {
            state.protect_library = true;
            state.recorded = Recorded::Unsaved;
        }
        state
    }

    pub(crate) fn current(&self) -> ProtectionState {
        self.current_with(&self.held.lock())
    }

    fn tell(&self, state: ProtectionState) {
        self.watchers.lock().retain_mut(|watcher| watcher(state));
    }

    /// Records `on`; with `only_if_never`, only when nothing has ever been
    /// recorded. Call under the core's edit gate.
    fn turn(&self, on: bool, only_if_never: bool) -> std::io::Result<Turned> {
        let mut held = self.held.lock();
        let before = self.current_with(&held);
        if only_if_never && before.recorded != Recorded::Never {
            return Ok(Turned { previous: before.protect_library, now: before });
        }
        let revision = before.revision + 1;
        let written = Turn::take(&self.dir).and_then(|_turn| {
            // Another process may have written since `before` was read.
            let revision = revision.max(read(&self.dir).revision + 1);
            write(&self.dir, on, revision)
        });
        match written {
            Ok(now) => {
                held.highest = now.revision;
                held.unsaved_on = false;
                self.tell(now);
                Ok(Turned { previous: before.protect_library, now })
            }
            Err(error) => {
                if on && !before.protect_library {
                    // Asked for on and could not record it: on here anyway.
                    held.highest = revision;
                    held.unsaved_on = true;
                    let now = self.current_with(&held);
                    self.tell(now);
                }
                Err(error)
            }
        }
    }
}

/// An exclusive lock on [`LOCK_FILE`], released when dropped.
struct Turn(std::fs::File);

impl Turn {
    fn take(dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join(LOCK_FILE))?;
        lock.lock()?;
        Ok(Self(lock))
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// Writes the file whole through a temporary file of its own, renamed over
/// the old one, so a reader never sees half of one. Call holding a [`Turn`].
fn write(dir: &Path, protect_library: bool, revision: u64) -> std::io::Result<ProtectionState> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let text = serde_json::to_vec(&Stored { protect_library, revision })?;
    let partial = dir.join(format!("{FILE}.{}.{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let written = std::fs::OpenOptions::new().write(true).create_new(true).open(&partial).and_then(|mut out| {
        out.write_all(&text)?;
        out.sync_all()
    });
    if let Err(error) = written.and_then(|()| std::fs::rename(&partial, file(dir))) {
        let _ = std::fs::remove_file(&partial);
        return Err(error);
    }
    // The rename itself survives a crash once the folder is synced.
    #[cfg(unix)]
    std::fs::File::open(dir)?.sync_all()?;
    Ok(ProtectionState { protect_library, revision, recorded: Recorded::Set })
}

impl LibraryCore {
    /// Library Protection as it stands, read now.
    pub fn protection(&self) -> ProtectionState {
        self.protection.current()
    }

    /// Turns Library Protection on or off and records it, under the edit
    /// gate, so no edit is half-way through while it changes and every edit
    /// after it sees it. Watchers are told before this returns. Turning it
    /// on where it cannot be recorded is an error, and leaves it on in this
    /// process until a change can be recorded.
    ///
    /// Blocking.
    pub fn set_protection(&self, on: bool) -> std::io::Result<Turned> {
        let _gate = self.edit_gate().lock();
        self.protection.turn(on, false)
    }

    /// The window's stored preference, taken up only if nothing has ever
    /// been recorded; otherwise what is recorded stands. Answers the state
    /// either way.
    ///
    /// Blocking.
    pub fn adopt_protection(&self, on: bool) -> std::io::Result<ProtectionState> {
        let _gate = self.edit_gate().lock();
        self.protection.turn(on, true).map(|turned| turned.now)
    }

    /// Tells `watcher` of every change of Library Protection made through
    /// this core, in order, until it answers `false`. It runs under the
    /// edit gate: hand the state to a channel and return.
    pub fn watch_protection(&self, watcher: impl FnMut(ProtectionState) -> bool + Send + 'static) {
        self.protection.watchers.lock().push(Box::new(watcher));
    }
}
