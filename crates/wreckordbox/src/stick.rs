//! `wreckordbox stick`: see and change Stick Protection (wbx-45.11), the
//! rule that nothing writes to a Stick until the DJ unlocks it.
//!
//! Kept in the state folder the window and the Library service share, so
//! an unlock here is the window's too, and the reverse. It needs no running
//! service. Every path names the Stick that holds it: a folder inside a
//! Stick is that Stick, never unlocked on its own. `lock` waits for any
//! write to the Stick to finish before it answers.
//!
//! An agent can run `wreckordbox stick unlock` like anyone else; a Gig
//! stick (no Spare stick marker) needs its name. Agents must never unlock
//! a Stick unless the DJ asked for that Stick, by name, in the conversation
//! (AGENTS.md, "Safety").

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
    /// List the mounted Sticks: each one's kind (a Spare stick carries the
    /// Spare stick marker, an empty `.rbxport-disposable` file at its root;
    /// any other is a Gig stick), whether it is protected, when it was last
    /// loaded and how much room it has. Needs the Library service.
    List,
    /// Show what loading playlists onto a Stick would do, and write
    /// nothing: tracks added, changed (by field) and removed; playlists
    /// created, reordered and removed; Native stems written; the room
    /// before and after. Answers the plan id `stick load` needs.
    DryRun(crate::stick_load::DryRunArgs),
    /// Load playlists onto a Stick: exactly what `stick dry-run` reported,
    /// with the export the window uses. The Stick ends up holding these
    /// playlists and no other playlist of the Library.
    ///
    /// Refused unless the Stick is unlocked (`stick unlock`) and `--plan`
    /// carries the plan id of a dry run of this very load, made in the last
    /// ten minutes, with nothing changed since. Refused before anything is
    /// written when it would not fit. Prints each track as it is written;
    /// afterwards says what the Stick holds of the plan and what it does
    /// not, and fails unless everything planned was written.
    Load(crate::stick_load::LoadArgs),
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
                        UnlockError::NotAStick => STICK_NOT_FOUND,
                        UnlockError::NoIdentity => codes::STICK_PROTECTED,
                        UnlockError::NameRequired { .. } | UnlockError::NameMismatch { .. } => {
                            codes::BAD_REQUEST
                        }
                        UnlockError::Io(_) => codes::WRITE_REFUSED,
                    };
                    ServiceError::new(code, error.to_string())
                })
        }
        StickCommands::List | StickCommands::DryRun(_) | StickCommands::Load(_) => {
            Err(ServiceError::new(
                codes::BAD_REQUEST,
                "this stick command needs the Library service",
            ))
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
