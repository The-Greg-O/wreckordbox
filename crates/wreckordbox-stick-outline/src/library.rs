//! A Stick library read in full (its tracks too) without settling anything,
//! and the one explicit action that does settle an unfinished write.
//!
//! `rbl_export::device_library::read` first runs `rbl_export::recover`, which
//! finishes or rolls back an interrupted publication: it renames, replaces
//! and deletes files on the Stick. Looking at a Stick (opening it, opening
//! one of its playlists, a device-list refresh) must never do that, so every
//! such read goes through [`library`] instead, and an unfinished write comes
//! back as [`ReadError::UnfinishedWrite`] for the interface to show. Settling
//! it is [`settle`], called only when the DJ asks for it, or by a write the DJ
//! asked for (those already recover as their first step).
//!
//! How it avoids recovery without reshaping `rbl-export` (ADR 0003): the
//! publication journal is looked for while holding the shared write lease,
//! so none of our writers can start or be part way through meanwhile, and
//! `device_library::read`, whose recovery does nothing when there is no
//! journal, is called only when there is none. The lease is taken only on a
//! lock file that already exists; nothing is created.

use std::path::Path;

use rbl_export::device_library::{self as device, Format, Library};

/// Why a Stick library could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    /// An earlier write to the Stick did not finish. Nothing is read until
    /// the DJ settles it ([`settle`]) or a write they asked for does.
    #[error("A write to this Stick did not finish.")]
    UnfinishedWrite,
    /// The library is missing or cannot be read.
    #[error(transparent)]
    Export(#[from] rbl_export::ExportError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Whether an earlier write to the Stick at `mount` did not finish. Reads
/// nothing but the journal's presence.
#[must_use]
pub fn unfinished_write(mount: &Path) -> bool {
    rbl_export::recovery_pending(mount)
}

/// Reads one library of the Stick mounted at `mount`, tracks included,
/// writing nothing to the Stick.
///
/// # Errors
///
/// [`ReadError::UnfinishedWrite`] when an earlier write did not finish;
/// otherwise why the library could not be read.
pub fn library(mount: &Path, format: Format) -> Result<Library, ReadError> {
    // Held across the check and the read: a writer of ours takes the lock
    // exclusively for as long as its journal exists.
    let _lease = rbl_core::durable::read_lease(mount)?;
    if unfinished_write(mount) {
        return Err(ReadError::UnfinishedWrite);
    }
    Ok(device::read(mount, format)?)
}

/// Settles an unfinished write on the Stick mounted at `mount`: finishes the
/// interrupted export when it can, otherwise rolls the Stick back to the
/// files it held before, keeping whatever another program wrote since. This
/// writes to the Stick: call it only for an action the DJ chose.
///
/// # Errors
///
/// Why recovery failed; the Stick is left for another try.
pub fn settle(mount: &Path) -> std::io::Result<()> {
    rbl_export::recover(mount)
}
