//! The Library service (ADR 0002): one process owns the Library, and every
//! front end is a client of it.
//!
//! - [`Service::start`] takes the Library's lock, opens it and listens on a
//!   Unix socket in the user's runtime directory.
//! - [`Client`] connects to that socket; [`Client::connect_or_start`] starts
//!   a service first when none is running.
//! - [`protocol`] is what goes over the socket.
//!
//! This crate has no dependency on the desktop framework and needs no
//! display.

mod client;
mod library;
pub mod mytag_edits;
pub mod protection;
pub mod protocol;
pub mod reading;
mod separation;
mod server;
pub mod stems;
pub mod tracks;

use std::path::{Path, PathBuf};
use std::time::Duration;

use rbl_db::LibraryLocation;

pub use client::{Client, Launcher, DEFAULT_LOADING_WAIT, LOADING_RETRY};
pub use protocol::{
    codes, events, ChangeKind, Event, LibraryChanged, RateTrack, RatedTrack, ServiceError, Status,
    Track, TrackList, TrackQuery, DEFAULT_TRACK_LIMIT,
};
pub use server::{serve_spawned, Service, Stopped};

/// Names the directory the socket goes in, instead of the user's runtime
/// directory. What a test points at a private folder.
pub const RUNTIME_DIR_ENV: &str = "WRECKORDBOX_RUNTIME_DIR";
/// Seconds a service stays up with no client connected. `0` means forever.
pub const IDLE_EXIT_ENV: &str = "WRECKORDBOX_IDLE_EXIT_SECS";

/// Seconds a `wreckordbox` command waits for a window that is still reading
/// the Library, instead of [`DEFAULT_LOADING_WAIT`]. `0` does not wait.
pub const LOADING_WAIT_ENV: &str = "WRECKORDBOX_LOADING_WAIT_SECS";

/// How long a command waits while the Library is being read:
/// [`LOADING_WAIT_ENV`] when it is set to a number of seconds, else
/// [`DEFAULT_LOADING_WAIT`].
#[must_use]
pub fn loading_wait_from_env() -> Duration {
    std::env::var(LOADING_WAIT_ENV)
        .ok()
        .and_then(|secs| secs.trim().parse::<u64>().ok())
        .map_or(DEFAULT_LOADING_WAIT, Duration::from_secs)
}

/// Where a service listens and how long it outlives its clients.
#[derive(Debug, Clone)]
pub struct Config {
    /// The directory holding the socket. Made on start, for this user only.
    pub runtime_dir: PathBuf,
    /// Stop after this long with no client connected. `None` never stops,
    /// which is what a front end hosting the service in its own process
    /// wants.
    pub idle_exit: Option<Duration>,
    /// Where the Library's recovery journals are kept: the same folder the
    /// window uses, so an unfinished restore stops an edit from either.
    pub recovery_dir: PathBuf,
}

impl Config {
    /// [`RUNTIME_DIR_ENV`] and [`IDLE_EXIT_ENV`] when set, else
    /// `wreckordbox/` in the user's runtime directory and no idle exit.
    pub fn from_env() -> Result<Self, ServiceError> {
        let runtime_dir = match std::env::var_os(RUNTIME_DIR_ENV) {
            Some(dir) => PathBuf::from(dir),
            // macOS has no runtime directory; its cache directory is the
            // nearest per-user place, and the folder is made private anyway.
            None => dirs::runtime_dir().or_else(dirs::cache_dir).map(|dir| dir.join("wreckordbox")).ok_or_else(
                || {
                    ServiceError::new(
                        codes::RUNTIME_DIR_UNUSABLE,
                        format!("there is no runtime directory for the service's socket; set XDG_RUNTIME_DIR or {RUNTIME_DIR_ENV}"),
                    )
                },
            )?,
        };
        let idle_exit = std::env::var(IDLE_EXIT_ENV)
            .ok()
            .and_then(|secs| secs.parse::<u64>().ok())
            .filter(|secs| *secs > 0)
            .map(Duration::from_secs);
        Ok(Self {
            runtime_dir,
            idle_exit,
            recovery_dir: rbl_backup::state_dir(),
        })
    }

    /// The socket of the service for `library`: one per Library, so two
    /// Libraries can each have a service.
    #[must_use]
    pub fn socket_path(&self, library: &LibraryLocation) -> PathBuf {
        let master_db = library
            .master_db
            .canonicalize()
            .unwrap_or_else(|_| library.master_db.clone());
        self.runtime_dir.join(format!(
            "{:016x}.sock",
            fnv1a(master_db.as_os_str().as_encoded_bytes())
        ))
    }
}

/// The Library this machine is set to use, found the way the window finds
/// it (`rbl_db::locate`, which a test steers with `RBXPORT_OPTIONS`).
pub fn locate_library() -> Result<LibraryLocation, ServiceError> {
    rbl_db::detect().map_err(|e| server::open_error(&e))
}

/// A hash that is the same in every build, unlike the standard library's:
/// a client and a service from different builds must agree on the socket.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The file whose lock says a service has the Library: beside `master.db`,
/// so the rule holds whatever runtime directory a service was given.
fn lock_path(master_db: &Path) -> PathBuf {
    master_db
        .parent()
        .map_or_else(|| PathBuf::from(LOCK_FILE), |dir| dir.join(LOCK_FILE))
}

const LOCK_FILE: &str = ".wreckordbox-service.lock";
