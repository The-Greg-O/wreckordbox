//! The messages on the service's socket: one JSON object per line.
//!
//! A client sends a [`Request`]; the service answers with a [`Response`]
//! carrying the same `id`. A client that has sent the `subscribe` command is
//! also sent [`Event`]s, which carry no `id`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Raised when a message's meaning changes; reported by `status`.
pub const PROTOCOL_VERSION: u32 = 1;

/// The names of the commands the service answers.
pub mod commands {
    /// Reports the service and the Library it has open; answers a
    /// [`Status`](super::Status).
    pub const STATUS: &str = "status";
    /// Asks for [`Event`](super::Event)s on this connection from now on.
    pub const SUBSCRIBE: &str = "subscribe";
    /// Lists tracks in the Library: takes a [`TrackQuery`](super::TrackQuery),
    /// answers a [`TrackList`](super::TrackList).
    pub const TRACKS_LIST: &str = "tracks.list";
}

/// The stable codes an error carries. A client decides on the code and shows
/// the message.
pub mod codes {
    /// No Library where one is configured.
    pub const LIBRARY_MISSING: &str = "library_missing";
    /// The Library's database is locked by another program.
    pub const LIBRARY_LOCKED: &str = "library_locked";
    /// The Library is there and cannot be read as one.
    pub const LIBRARY_DAMAGED: &str = "library_damaged";
    /// Another service already has this Library.
    pub const SERVICE_ALREADY_RUNNING: &str = "service_already_running";
    /// No service answers on the socket.
    pub const SERVICE_NOT_RUNNING: &str = "service_not_running";
    /// A service could not be started, or stopped answering.
    pub const SERVICE_FAILED: &str = "service_failed";
    /// The runtime directory cannot hold the socket.
    pub const RUNTIME_DIR_UNUSABLE: &str = "runtime_dir_unusable";
    /// The service does not know the command.
    pub const UNKNOWN_COMMAND: &str = "unknown_command";
    /// A message that is not valid for this protocol.
    pub const BAD_REQUEST: &str = "bad_request";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub command: String,
    #[serde(default)]
    pub args: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(flatten)]
    pub outcome: Outcome,
}

/// `"result": …` or `"error": {"code": …, "message": …}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Result(Value),
    Error(ServiceError),
}

/// Something that happened in the service, pushed to subscribed clients:
/// `{"event": "<name>", "data": {…}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub event: String,
    #[serde(default)]
    pub data: Value,
}

/// Why a command, or reaching the service at all, failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceError {
    /// One of [`codes`]; stable across releases.
    pub code: String,
    /// For a person; may change.
    pub message: String,
}

impl ServiceError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self { code: code.to_owned(), message: message.into() }
    }
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ServiceError {}

/// What `status` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// Always true: a service that is not running does not answer.
    pub running: bool,
    pub pid: u32,
    pub version: String,
    pub protocol: u32,
    pub library: LibraryStatus,
    pub socket: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryStatus {
    /// The Library's `master.db`.
    pub path: PathBuf,
    /// Tracks in the Library, not counting deleted ones.
    pub tracks: u32,
}

/// The arguments of `tracks.list`. Every one may be left out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackQuery {
    /// Text to search for, as typed into the window's search box: every
    /// word must appear somewhere in the track's details. Empty matches
    /// every track.
    #[serde(default)]
    pub filter: String,
    /// The most tracks to return; [`DEFAULT_TRACK_LIMIT`] when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// How many tracks `tracks.list` returns when the client names no limit:
/// enough to browse, few enough that a large Library does not flood the
/// caller.
pub const DEFAULT_TRACK_LIMIT: usize = 50;

/// What `tracks.list` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackList {
    /// In the collection's own order, as the window lists them.
    pub tracks: Vec<Track>,
    /// How many tracks matched, which is more than were returned when the
    /// limit cut the list short.
    pub total: usize,
}

/// One track, as a row of the window's track list shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    /// The track's identifier in the Library; what an edit names it by.
    pub id: String,
    pub title: String,
    /// Empty when the track has none.
    pub artist: String,
    /// Beats per minute; `null` for a track with no tempo.
    pub bpm: Option<f64>,
    /// As the Library spells it, for example `8A`; empty when not analysed.
    pub key: String,
    /// Stars, 0 to 5.
    pub rating: u8,
}
