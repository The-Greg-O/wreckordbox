//! Where the core reports the changes its edit history sees, for a Ledger
//! kept outside it (ADR 0004).
//!
//! The core knows nothing of where a Ledger is kept or of who made an edit.
//! Whoever holds the core gives the edit history a [`Ledger`]; from then on
//! every edit that enters the history ([`crate::history::EditHistory::record`],
//! and each slice of an Add Tracks to Playlist), and every Undo and Redo of
//! one ([`crate::LibraryCore::step_history`]), is handed to it once, with the
//! exact payload the history keeps. The [`Ledger`] answers the identifier it
//! gave the change; the history keeps it beside the entry, so a later Undo
//! or Redo can say which change it takes back or makes again.

use crate::history::LibraryEdit;

/// What happened to a history entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The edit was made and entered the history.
    Edit,
    /// The entry was taken back.
    Undo,
    /// The entry was made again after an Undo.
    Redo,
}

/// Keeps a durable record of the changes the edit history sees.
///
/// Called with the history's lock held and under the edit gate, on the
/// thread that made the change. It must not take the edit gate or the
/// history, read the core, or call a [`crate::Host`]; it may block briefly
/// to write. A failure to record cannot undo a write already made: it is
/// the implementation's to report, and it answers `None`.
pub trait Ledger: Send + Sync {
    /// Records one change: `edit` as the history keeps it, under `label`.
    /// `reverses` is, for an Undo or Redo, the identifier this Ledger gave
    /// the change it steps. Answers the identifier of this change.
    fn record(
        &self,
        step: Step,
        edit: &LibraryEdit,
        label: &str,
        reverses: Option<i64>,
    ) -> Option<i64>;
}
