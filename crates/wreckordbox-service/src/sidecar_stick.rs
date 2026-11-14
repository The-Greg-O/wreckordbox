//! Long background work and a Sidecar kept on a Stick (wbx-45.14).
//!
//! Stem separation and Intensity measurement write the Sidecar for minutes
//! (a separation's workspace, its Stems, a Run and its Facts, and the
//! cleanup afterwards). For a Sidecar on a Stick that would mean holding the
//! Stick's authorization, and so its execution lock, for that whole time:
//! the DJ's lock or eject would wait behind a twenty-minute separation.
//! Decided instead: **work of that kind is refused outright** when the
//! Sidecar is on a Stick, locked or unlocked, before anything is written,
//! with a message saying why. Nothing shows the DJ relying on it: the
//! DJ's Library and its Sidecar live in the home folder, and a Library on a
//! Stick is supported for edits (each one short and authorized for its own
//! duration), not as a place to separate or measure.
//!
//! Short commands that write the Sidecar (a Fact recorded, a Decision) keep
//! the per-use check in [`crate::stems::Stems::sidecar`].

use std::path::Path;

use crate::protocol::{codes, ServiceError};

/// Refuses `what` (a separation, a measurement) when the Sidecar at
/// `sidecar_dir` is on a Stick.
pub(crate) fn refuse_background_work(sidecar_dir: &Path, what: &str) -> Result<(), ServiceError> {
    let Some(root) = wreckordbox_stick_protection::stick_root(sidecar_dir) else {
        return Ok(());
    };
    let name = root.file_name().map_or_else(
        || root.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    Err(ServiceError::new(
        codes::SIDECAR_ON_STICK,
        format!(
            "{what} is not done for a Library whose Sidecar is on the Stick {name}: it writes the Sidecar for minutes, \
             and a Stick must stay free to lock and eject. Nothing was written. Keep the Library on this computer for this."
        ),
    ))
}
