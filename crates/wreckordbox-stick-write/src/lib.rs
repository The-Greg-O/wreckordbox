//! A write to a Stick, worked out and confirmed before anything is written:
//! the one implementation the window (`src-tauri`) and the Library service
//! (`sticks.dry_run`, `sticks.load`) share, so there is one planner and one
//! confirmation rule, whoever asks (wbx-45.4, wbx-70).
//!
//! - [`selection`]: what an export or sync writes, built from the Library
//!   as the window's commands build it ([`selection::ExportSelection`]).
//! - [`plan`]: what writing a selection would do to a Stick, worked out
//!   without writing or settling anything ([`plan::export_plan`]).
//! - [`approval`]: the plan id, the token it turns into once confirmed, and
//!   the check every writer makes under the Stick's execution lock.
//!
//! Stick Protection (`wreckordbox_stick_protection`) is the guarantee
//! beneath all of this: every writer checks it first. Nothing here unlocks
//! a Stick.
//!
//! ADR 0003: the export crates are used as they are; this sits beside them.

pub mod approval;
pub mod plan;
pub mod selection;

use std::path::PathBuf;

/// Why a plan, a selection or a confirmation could not be made. The window
/// and the service each say it in their own words and codes.
#[derive(Debug)]
pub enum StickWriteError {
    /// The Stick, a playlist or a track is not there.
    NotFound(String),
    /// Refused: the write was not confirmed, or the Stick or the Library
    /// changed since. Nothing was written.
    Conflict(String),
    /// Reading the Library failed.
    Db(rbl_db::DbError),
    /// Reading the Stick without recovering it failed (wbx-45.5).
    Read {
        mount: PathBuf,
        error: wreckordbox_stick_outline::ReadError,
    },
    /// Anything else, with what failed.
    Internal {
        message: String,
        detail: Option<String>,
    },
}

impl StickWriteError {
    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
            detail: None,
        }
    }
}

impl std::fmt::Display for StickWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(message) | Self::Conflict(message) => f.write_str(message),
            Self::Db(error) => write!(f, "{error}"),
            Self::Read { mount, error } => write!(f, "{}: {error}", mount.display()),
            Self::Internal { message, detail } => match detail {
                Some(detail) => write!(f, "{message} ({detail})"),
                None => f.write_str(message),
            },
        }
    }
}

impl std::error::Error for StickWriteError {}

impl From<rbl_db::DbError> for StickWriteError {
    fn from(error: rbl_db::DbError) -> Self {
        Self::Db(error)
    }
}

/// The result of this crate's functions.
pub type Result<T> = std::result::Result<T, StickWriteError>;
