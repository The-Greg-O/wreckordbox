//! Loading a Stick through the service (wbx-70), so the window and the CLI
//! share it.
//!
//! - `sticks.list`: the mounted Sticks, each with its kind (Gig or Spare),
//!   Stick Protection, last load and free space.
//! - `sticks.dry_run`: what loading chosen playlists would do, worked out by
//!   the window's own planner (`wreckordbox_stick_write`), writing nothing
//!   to the Stick or the Library; answers a **plan id**.
//! - `sticks.load`: the load. A Stick write like any other: refused on a
//!   protected Stick, under the Stick's execution lock, and only with the
//!   plan id of a dry run of exactly this load, unexpired, on the Stick and
//!   Library as they were planned. The plan id is the confirmation the
//!   window's hold gives; there is no hold here. Refused before anything is
//!   written when it would not fit, saying by how much. The export is the
//!   window's (`rbl_export`, unchanged), with progress events naming each
//!   track; afterwards the Stick is read back and the answer says what it
//!   holds of the plan and what it does not.
//!
//! A load is a sync of the chosen playlists: the Stick ends up holding them
//! and no other playlist of this Library (tracks put on it on their own by
//! Export Track stay, as the Sync Manager keeps them). On a Spare stick the
//! tracks' Native stems are written after the export.
//!
//! Not a Library edit: the Library is only read, so nothing here goes
//! through the edit path or sends `library_changed`.
//!
//! wbx-83 (Mutants rendered at load time) adds a step between the plan and
//! the export: see `load::plan_load`'s note.

mod load;
pub mod sticks;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use load::{TEST_FAIL_AFTER_FILE, TEST_PAUSED_FILE, TEST_PAUSE_AT_FILE};

/// In the state folder, in test mode only: how long, in milliseconds,
/// stopping the service waits for a running load before it says so in
/// the log (it goes on waiting).
pub const TEST_SHUTDOWN_PATIENCE_FILE: &str = "test-stick-load-shutdown-patience-ms";

/// How long stopping the service is patient with a running load.
pub(crate) fn shutdown_patience(state_dir: &std::path::Path) -> std::time::Duration {
    sticks::testing()
        .then(|| {
            std::fs::read_to_string(state_dir.join(TEST_SHUTDOWN_PATIENCE_FILE))
                .ok()?
                .trim()
                .parse()
                .ok()
        })
        .flatten()
        .map_or(
            std::time::Duration::from_secs(180),
            std::time::Duration::from_millis,
        )
}
pub use sticks::{Stick, Sticks};
pub use sticks::{TEST_CLUSTER_BYTES_FILE, TEST_FREE_BYTES_FILE};

/// The names of the commands.
pub mod commands {
    /// No arguments; answers [`Sticks`](super::Sticks).
    pub const STICKS_LIST: &str = "sticks.list";
    /// Takes a [`DryRunArgs`](super::DryRunArgs); answers a
    /// [`DryRun`](super::DryRun). Writes nothing.
    pub const STICKS_DRY_RUN: &str = "sticks.dry_run";
    /// Takes a [`LoadArgs`](super::LoadArgs); answers a
    /// [`Load`](super::Load), with [`STICK_LOAD_PROGRESS`](super::events::STICK_LOAD_PROGRESS)
    /// events on the way.
    pub const STICKS_LOAD: &str = "sticks.load";
}

/// The events a load sends.
pub mod events {
    /// `{"stick", "stage", "done", "total", "track": {"id", "title"} | null}`:
    /// the track being written, while a load runs. `stage` is the export's
    /// (`checking`, `copying`, `database`, `verifying`, `publishing`), or
    /// `native_stems`, `done`, `failed`.
    pub const STICK_LOAD_PROGRESS: &str = "stick_load_progress";
}

/// The error codes, beside the service's own (`stick_protected`,
/// `bad_request`), `stick_not_found` and `playlist_not_found`. Each is
/// answered before anything is written to the Stick.
pub mod codes {
    /// The path is not a Stick (an ordinary folder or disk), or is a folder
    /// inside one rather than its root.
    pub const NOT_A_STICK: &str = "not_a_stick";
    /// The load has no plan of its own: the plan id is unknown, used,
    /// expired, for other playlists or another Stick, or the Stick or the
    /// Library changed since the dry run. Make a new dry run.
    pub const STICK_LOAD_UNCONFIRMED: &str = "stick_load_unconfirmed";
    /// The load would not fit on the Stick; the message says by how much.
    pub const STICK_TOO_SMALL: &str = "stick_too_small";
    /// An earlier write to the Stick did not finish: settle it in the
    /// window ("Finish or roll back the unfinished write") first.
    pub const STICK_UNFINISHED_WRITE: &str = "stick_unfinished_write";
    /// The load could not be worked out (the Stick could not be read).
    pub const STICK_LOAD_FAILED: &str = "stick_load_failed";
    /// What the load would write is not what its dry run reported: a
    /// track's audio, a Stem set, a playlist or the changes themselves
    /// differ now. The message says what changed. Make a new dry run.
    pub const STICK_LOAD_CHANGED: &str = "stick_load_changed";
    /// The plan could not be worked out in full (the Stick's database is in
    /// WAL mode or has SQLite companion files beside it, so what the export
    /// would copy cannot be read without writing to it): no load is issued
    /// or performed.
    pub const STICK_LOAD_INCOMPLETE: &str = "stick_load_incomplete";
}

/// The arguments of `sticks.dry_run`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DryRunArgs {
    /// Where the Stick is mounted: its root, absolute.
    pub stick: PathBuf,
    /// The playlists to put on it, by identifier (`playlists.tree`); a
    /// folder stands for everything in it.
    pub playlists: Vec<String>,
}

/// The arguments of `sticks.load`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadArgs {
    pub stick: PathBuf,
    pub playlists: Vec<String>,
    /// The plan id the dry run of exactly this load answered.
    pub plan: String,
}

/// A track named in a plan or a report.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrackLine {
    /// Its Library identifier; `""` for a track that is not the Library's.
    pub id: String,
    pub title: String,
}

/// A track on the Stick whose data the load changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedTrack {
    pub id: String,
    pub title: String,
    /// The fields that differ (`Title`, `Artist`, `Album`, `Genre`,
    /// `Label`, `Comment`, `BPM`, `Key`, `Energy`, `Colour`, `MyTags`).
    pub fields: Vec<String>,
}

/// A playlist on the Stick written again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RewrittenPlaylist {
    pub name: String,
    /// Entries it gains and loses; `null` when the Stick does not record
    /// what it held (an older export).
    pub added: Option<usize>,
    pub removed: Option<usize>,
    /// The entries it keeps change order; `null` as above.
    pub reordered: Option<bool>,
}

/// How many of each kind of change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub tracks_added: usize,
    /// Tracks on the Stick whose data changes (`tracks.changed`).
    pub tracks_changed: usize,
    /// Tracks on the Stick whose audio is copied again (changed at the source).
    pub audio_changed: usize,
    pub tracks_removed: usize,
    pub tracks_missing: usize,
    /// Tracks whose audio on the Stick is kept as it is.
    pub audio_reused: usize,
    pub playlists_created: usize,
    pub playlists_rewritten: usize,
    pub playlists_reordered: usize,
    pub playlists_removed: usize,
    pub native_stems: usize,
}

/// The tracks a load changes, by kind.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackChanges {
    pub added: Vec<TrackLine>,
    pub changed: Vec<ChangedTrack>,
    pub audio_changed: Vec<TrackLine>,
    pub removed: Vec<TrackLine>,
    /// Chosen tracks whose audio file is missing: not loaded.
    pub missing: Vec<TrackLine>,
}

/// The playlists a load changes, by kind (folders are not listed).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistChanges {
    /// New to the Stick, in tree order.
    pub created: Vec<String>,
    /// On the Stick already, written again.
    pub rewritten: Vec<RewrittenPlaylist>,
    /// The names of those rewritten whose kept entries change order.
    pub reordered: Vec<String>,
    /// This Library's playlists on the Stick that the load takes off it.
    pub removed: Vec<String>,
}

/// What a load does to a Stick: counts and full lists.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Changes {
    pub counts: Counts,
    pub tracks: TrackChanges,
    pub playlists: PlaylistChanges,
    /// The tracks whose Native stems are written (a Spare stick only).
    pub native_stems: Vec<TrackLine>,
    /// How many tracks' cues and analysis are written from the Library
    /// (every track loaded; not itemised).
    pub analysis_written: usize,
    /// Whether the audio lists were worked out: not on a Stick whose
    /// library could not be inspected without writing to it.
    pub audio_estimated: bool,
}

/// The room a load needs and leaves.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Space {
    /// Free space on the Stick now; `null` when it cannot be read.
    pub free_before: Option<u64>,
    /// The most room the load takes at once: the export keeps the old
    /// library until the new one is in place, plus the Native stems.
    pub needed_bytes: u64,
    /// Free space after the load, as the plan works it out.
    pub free_after: Option<u64>,
    /// By how much the load would not fit; `null` when it fits or the free
    /// space is not known.
    pub shortfall_bytes: Option<u64>,
    /// Of `needed_bytes`, the most the Native stems take at once once the
    /// export is in place: each set's files and folder, in whole clusters,
    /// its identity record and its staging, with the set it replaces still
    /// there until the new one is in.
    pub native_stems_bytes: u64,
    /// The sizes are estimates.
    pub approximate: bool,
}

/// A playlist chosen for the load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chosen {
    pub id: String,
    pub name: String,
}

/// What `sticks.dry_run` answers: what `sticks.load` with the same
/// playlists would do now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DryRun {
    pub stick: Stick,
    pub playlists: Vec<Chosen>,
    pub changes: Changes,
    pub space: Space,
    /// The plan id `sticks.load` must carry: good once, for exactly what
    /// this dry run reports, on this Stick and Library as they are now, for
    /// ten minutes. `null` when the plan could not be worked out in full
    /// (`not_loadable` says why): such a load is never performed.
    pub plan: Option<String>,
    /// Why there is no plan id.
    pub not_loadable: Option<String>,
}

/// Of the changes a load planned, those the Stick shows (or does not show)
/// when it is read back after it, by comparing it with the plan again:
/// fields, membership and order, audio.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Held {
    pub tracks: Vec<TrackLine>,
    pub playlists: Vec<String>,
}

/// A track's Native stems in a load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeStemsLine {
    pub track: TrackLine,
    /// `written`, `skipped` (a name the layout cannot hold) or `failed`.
    pub outcome: String,
    /// Why, when not written.
    pub reason: Option<String>,
    /// The Stems Fact of the set written, when one was: the set the dry
    /// run approved.
    pub stems_fact: Option<i64>,
}

/// A load's error, once it had started writing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadError {
    pub code: String,
    pub message: String,
}

/// What `sticks.load` answers once it has started writing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Load {
    /// The Stick as it is after the load.
    pub stick: Stick,
    /// `loaded`: everything planned was written. `incomplete`: the export
    /// was written but not everything planned is on the Stick (a track's
    /// audio unreadable, Native stems refused). `failed`: the export failed;
    /// the Stick holds what it held before, as `written` says.
    pub outcome: String,
    pub error: Option<LoadError>,
    /// What the load set out to do: its dry run's `changes`.
    pub planned: Changes,
    /// Of the tracks and playlists the load was to put on the Stick, those
    /// it holds now, read back from it.
    pub written: Held,
    /// And those it does not: the change was not made (an old value, the old
    /// order or the old audio is still there).
    pub not_written: Held,
    /// What the load was to put on the Stick that needed no change: on the
    /// Stick already as planned, neither written nor failed.
    pub unchanged: Held,
    pub native_stems: Vec<NativeStemsLine>,
}

/// Whether `command` is one of these.
pub(crate) fn answers(command: &str) -> bool {
    matches!(
        command,
        commands::STICKS_LIST | commands::STICKS_DRY_RUN | commands::STICKS_LOAD
    )
}

/// The loads running in one service, so that stopping it (idle, a `stop`,
/// a window taking the Library over) waits for each to end and its caller
/// to have its answer, after asking it to stop where it safely can: a
/// load stopped before it publishes is a failure that says what is on the
/// Stick, never a disconnect.
#[derive(Default)]
pub(crate) struct Loads {
    active: std::sync::Mutex<usize>,
    idle: std::sync::Condvar,
    stopping: std::sync::atomic::AtomicBool,
}

/// One load's hold on its service, from its request to its answer sent.
pub(crate) struct LoadHold<'a>(&'a Loads);

impl Drop for LoadHold<'_> {
    fn drop(&mut self) {
        let mut active = self.0.lock();
        *active = active.saturating_sub(1);
        self.0.idle.notify_all();
    }
}

impl Loads {
    fn lock(&self) -> std::sync::MutexGuard<'_, usize> {
        self.active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Held by a connection from a load's request until its answer is sent.
    pub(crate) fn hold(&self) -> LoadHold<'_> {
        *self.lock() += 1;
        LoadHold(self)
    }

    /// The service is stopping: a load not begun is refused, one running
    /// stops where it safely can.
    pub(crate) fn stopping(&self) -> bool {
        self.stopping.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Asks every load to stop where it safely can, and waits until each
    /// has ended and sent its answer, however long that takes: a load is
    /// never abandoned, so its caller always gets the full report. Once
    /// `patience` has gone by (a stalled device), it is said in the log,
    /// and again each time as much more goes by.
    pub(crate) fn shut(&self, patience: std::time::Duration) {
        self.stopping
            .store(true, std::sync::atomic::Ordering::Release);
        let patience = patience.max(std::time::Duration::from_millis(1));
        let started = std::time::Instant::now();
        let mut active = self.lock();
        while *active > 0 {
            let (next, waited) = self
                .idle
                .wait_timeout(active, patience)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            active = next;
            if waited.timed_out() && *active > 0 {
                tracing::warn!(
                    loads = *active,
                    waited_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                    "the service is waiting to stop for a Stick load still finishing"
                );
            }
        }
    }
}

/// Answers one of these commands. `with` runs a closure on the service's
/// Library under its hold: only planning takes it; the export runs with it
/// free.
pub(crate) fn answer(
    state_dir: &std::path::Path,
    stems: &std::sync::Arc<crate::stems::Stems>,
    request: &crate::protocol::Request,
    with: &load::WithLibrary<'_>,
    loads: &Loads,
) -> Result<serde_json::Value, crate::protocol::ServiceError> {
    let value = match request.command.as_str() {
        commands::STICKS_LIST => serde_json::to_value(load::list(state_dir)),
        commands::STICKS_DRY_RUN => {
            serde_json::to_value(load::dry_run(state_dir, stems, request, with)?)
        }
        _ => serde_json::to_value(load::load(state_dir, stems, request, with, loads)?),
    };
    value.map_err(|e| {
        crate::protocol::ServiceError::new(crate::protocol::codes::SERVICE_FAILED, e.to_string())
    })
}
