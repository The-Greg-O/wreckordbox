//! The DJ's per-operation confirmation of a write to a Stick (wbx-45.4),
//! in the window's words.
//!
//! The plan id, the token and the checks are kept in
//! `wreckordbox_stick_write::approval`, shared with the Library service's
//! Stick load (wbx-70), so the window and the service confirm a write by
//! one rule and one store. This module answers the window's errors.
//!
//! This is the window's confirmation, enforced in the backend so that a
//! script or a stale button cannot skip it; it is not the guarantee that a
//! Stick is safe. That is Stick Protection (`stick_guard`, wbx-45.11),
//! which every writer checks first, under the Stick's lock.

use std::path::Path;

pub use wreckordbox_stick_write::approval::{
    bound, stick_revision, StickLock, StickOperation, APPROVAL_LIFETIME,
};

use crate::error::AppResult;

/// Takes the execution lock of the Stick at `stick`, waiting for a write
/// already holding it.
pub fn lock(state_dir: &Path, stick: &Path) -> AppResult<StickLock> {
    Ok(wreckordbox_stick_write::approval::lock(state_dir, stick)?)
}

/// The mounted directory's identity: another volume at the same path differs.
pub fn volume_identity(stick: &Path) -> AppResult<String> {
    Ok(wreckordbox_stick_write::approval::volume_identity(stick)?)
}

/// Records a plan for `op` on the Stick at `stick` and answers its plan id:
/// not a token. Called by `stick_write_plan` alone.
pub(crate) fn propose(
    stick: &Path,
    op: StickOperation,
    generation: Option<u64>,
) -> AppResult<String> {
    Ok(wreckordbox_stick_write::approval::propose(
        stick, op, generation,
    )?)
}

/// The DJ confirmed the plan `plan` (the window's hold): answers the token
/// its writer needs.
pub fn approve(state_dir: &Path, plan: &str) -> AppResult<String> {
    Ok(wreckordbox_stick_write::approval::approve(state_dir, plan)?)
}

/// The DJ cancelled the plan `plan`: it can never be confirmed.
pub fn revoke(plan: &str) {
    wreckordbox_stick_write::approval::revoke(plan);
}

/// The plan `plan`, while it waits for the DJ: for tests.
#[doc(hidden)]
#[must_use]
pub fn planned(plan: &str) -> Option<StickOperation> {
    wreckordbox_stick_write::approval::planned(plan)
}

/// Checks and uses up the token for `op` on the Stick at `stick`. Call it
/// holding the Stick's lock (`stick_guard`), before any recovery or write.
pub fn consume(
    token: Option<&str>,
    op: &StickOperation,
    stick: &Path,
    generation: u64,
) -> AppResult<()> {
    Ok(wreckordbox_stick_write::approval::consume(
        token, op, stick, generation,
    )?)
}

/// The commands that write to a Stick. Each takes an approval token and
/// consumes it under the Stick's lock before anything else; the source
/// guard in the tests holds this list to the code.
pub const STICK_WRITING_COMMANDS: &[&str] = &[
    "export_playlist",
    "device_playlist_add_library_tracks",
    "export_tracks_to_device",
    "sync_devices",
    "device_playlist_edit",
    "device_playlist_add_files",
    "save_device_settings",
    "ensure_device_library",
    "stick_settle",
    "format_device",
];
