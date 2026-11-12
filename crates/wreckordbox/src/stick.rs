//! `wreckordbox stick`: see and change Stick Protection (wbx-45.11), the
//! rule that nothing writes to a Stick until the DJ unlocks it.
//!
//! Kept in the state folder the window and the Library service share, so
//! an unlock here is the window's too, and the reverse. It needs no running
//! service.

use std::path::{Path, PathBuf};

use clap::Subcommand;
use serde_json::json;
use wreckordbox_service::ServiceError;
use wreckordbox_stick_protection::{self as protection, UnlockError};

use wreckordbox_service::codes;
use wreckordbox_service::native_stems::codes::STICK_NOT_FOUND;

#[derive(Subcommand)]
pub enum StickCommands {
    /// Say whether the Stick is protected.
    Status {
        /// Where the Stick is mounted.
        stick: PathBuf,
    },
    /// Let the Stick be written to until it is ejected, removed or put
    /// back, or locked again.
    ///
    /// A Spare stick unlocks as it is. Any other Stick is a Gig stick:
    /// give its name, as the window shows it, with `--name`.
    Unlock {
        /// Where the Stick is mounted.
        stick: PathBuf,
        /// The Stick's name, typed: required for a Gig stick.
        #[arg(long)]
        name: Option<String>,
    },
    /// Protect the Stick again.
    Lock {
        /// Where the Stick is mounted.
        stick: PathBuf,
    },
}

fn found(stick: &Path) -> Result<PathBuf, ServiceError> {
    let absolute = std::path::absolute(stick).unwrap_or_else(|_| stick.to_path_buf());
    if absolute.is_dir() {
        Ok(absolute)
    } else {
        Err(ServiceError::new(
            STICK_NOT_FOUND,
            format!("there is no Stick at {}", stick.display()),
        ))
    }
}

fn said(status: &protection::Status, as_json: bool) -> String {
    if as_json {
        return json!(status).to_string();
    }
    let kind = if status.spare {
        "Spare stick"
    } else {
        "Gig stick"
    };
    if status.protected {
        format!(
            "{} ({kind}) is protected: nothing is written to it.",
            status.name
        )
    } else {
        format!(
            "{} ({kind}) is unlocked until it is ejected, removed or locked.",
            status.name
        )
    }
}

/// What to print for `wreckordbox stick …`.
pub fn stick(
    state_dir: &Path,
    as_json: bool,
    command: &StickCommands,
) -> Result<String, ServiceError> {
    match command {
        StickCommands::Status { stick } => Ok(said(
            &protection::status(state_dir, &found(stick)?),
            as_json,
        )),
        StickCommands::Unlock { stick, name } => {
            let root = found(stick)?;
            protection::unlock(state_dir, &root, name.as_deref())
                .map(|status| said(&status, as_json))
                .map_err(|error| {
                    let code = match error {
                        UnlockError::NotFound => STICK_NOT_FOUND,
                        UnlockError::NameRequired { .. } | UnlockError::NameMismatch { .. } => {
                            codes::BAD_REQUEST
                        }
                        UnlockError::Io(_) => codes::WRITE_REFUSED,
                    };
                    ServiceError::new(code, error.to_string())
                })
        }
        StickCommands::Lock { stick } => {
            let root = found(stick)?;
            protection::lock(state_dir, &root)
                .map(|status| said(&status, as_json))
                .map_err(|e| {
                    ServiceError::new(
                        codes::WRITE_REFUSED,
                        format!("the Stick could not be locked: {e}"),
                    )
                })
        }
    }
}

/// What a person is told beside a refusal by Stick Protection.
pub fn hint(error: &ServiceError) -> Option<&'static str> {
    (error.code == codes::STICK_PROTECTED)
        .then_some("Unlock it first: wreckordbox stick unlock <stick> [--name <its name>]")
}
