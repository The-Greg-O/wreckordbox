//! Where the core reports the changes its edit history sees, for a Ledger
//! kept outside it (ADR 0004).
//!
//! The core knows nothing of where a Ledger is kept or of who made an edit.
//! Whoever holds the core gives the edit history a [`Ledger`]
//! ([`crate::history::EditHistory::keep_ledger`]). From then on:
//!
//! - before any write is made, the Ledger is asked whether it can record
//!   ([`Ledger::ready`]); when it cannot, the write is refused and nothing
//!   changes, so the Library is never changed with nothing able to say so;
//! - every edit that enters the history
//!   ([`crate::history::EditHistory::record`], and each slice of an Add
//!   Tracks to Playlist), and every Undo and Redo of one
//!   ([`crate::LibraryCore::step_history`]), is handed to it once, after the
//!   write, with the exact payload the history keeps ([`Ledger::record`]).
//!   It answers the identifier it gave the change; the history keeps it
//!   beside the entry, so a later Undo or Redo can say which change it
//!   takes back or makes again;
//! - an edit the history does not keep is named to it ([`Ledger::note`]):
//!   one that only ends the Redo branch ([`Unkept::Unrecorded`]) and one
//!   that empties the history ([`Unkept::Permanent`]).
//!
//! A write that failed is never handed over: an entry says a change was
//! made.

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

/// An edit the history does not keep an entry for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unkept {
    /// The history records nothing for it; it ends the Redo branch.
    Unrecorded,
    /// It removed rows for good; the history was emptied after it.
    Permanent,
}

/// Keeps a durable record of the changes the edit history sees.
///
/// [`Self::record`] and [`Self::note`] are called with the history's lock
/// held and under the edit gate, on the thread that made the change;
/// [`Self::ready`] under the edit gate alone. None of them may take the
/// edit gate or the history, read the core, or call a [`crate::Host`]; they
/// may block briefly to write. A failure to record cannot undo a write
/// already made: it is the implementation's to keep (and to refuse the
/// next write for, in [`Self::ready`]).
pub trait Ledger: Send + Sync {
    /// Whether a change made now could be recorded. Asked before every
    /// write through the core; an error refuses the write with its text.
    fn ready(&self) -> Result<(), String>;

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

    /// Names an edit the history keeps no entry for. `name` is what the
    /// caller calls it, when the caller says; the Ledger may know more
    /// (which command it is answering).
    fn note(&self, kept: Unkept, name: Option<&str>);
}
