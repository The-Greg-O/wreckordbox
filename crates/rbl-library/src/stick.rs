//! Stick Protection (wbx-45.11) at the Library's own boundaries: a Library
//! kept on a volume that is a Stick (`wreckordbox_stick_protection::classify`)
//! is written, edited or recovered only while the DJ has unlocked that
//! Stick, whoever asks (the window, the service, the CLI, background work).
//! A Library on an ordinary folder or an external disk with no player
//! export is never refused here.
//!
//! In the one lock order: taken under the Library edit gate, before the
//! analysis lock; held for the whole write.

use std::path::Path;

use wreckordbox_stick_protection::{Authorization, Refused};

/// The words every refusal here starts its reason with: the Stick's own.
const MARK: &str = "is protected: nothing is written to it until it is unlocked";

/// Leave to write the Library at `location` (its database's folder): held
/// for as long as it lives. `state_dir` is the shared state folder.
///
/// # Errors
///
/// A refusal ([`refused`] says so) when the Library is on a protected
/// Stick, or one removed or replaced; nothing may be written.
pub fn leave(state_dir: &Path, location: &rbl_db::LibraryLocation) -> Result<Authorization, rbl_db::DbError> {
    let folder = location.master_db.parent().unwrap_or(&location.master_db);
    wreckordbox_stick_protection::authorize(state_dir, folder).map_err(|why| error(&why))
}

/// The error a write refused by Stick Protection answers.
#[must_use]
pub fn error(why: &Refused) -> rbl_db::DbError {
    rbl_db::DbError::WriteRefused(why.to_string())
}

/// Whether `error` is a refusal by Stick Protection because the Stick is
/// protected.
#[must_use]
pub fn refused(error: &rbl_db::DbError) -> bool {
    matches!(error, rbl_db::DbError::WriteRefused(why) if why.contains(MARK))
}
