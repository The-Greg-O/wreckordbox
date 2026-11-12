//! A beat grid edit as a step of the one shared history (wbx-64).
//!
//! The window's grid edits rewrite the track's analysis files and its BPM
//! row. They are made, undone and redone by the window's grid editor, which
//! holds the file-edit guard (Library Protection, Stick Protection, the
//! analysis-file lock), journals the files and records each step in the
//! Ledger with the beats either side. The history keeps the step as data:
//! the beats before and after, so Undo and Redo of grid edits, Energy, tags,
//! cues and every other edit come back in the order the DJ made them.
//!
//! The core does not step one itself: it has no analysis files to write.
//! [`crate::history::LibraryEdit::check`] refuses it, so a caller that is
//! not the grid editor writes nothing ([`GridStep::NOT_HERE`]). The window
//! looks at the next entry ([`crate::history::EditHistory::next`]), steps a
//! grid entry through its grid editor, and then moves the entry
//! ([`crate::history::EditHistory::stepped_elsewhere`]).

use std::sync::Arc;

/// One beat of a grid, as the analysis file holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridBeat {
    pub time_ms: u32,
    /// 1 to 4, 1 the downbeat.
    pub beat_number: u16,
    /// BPM x100 at this beat.
    pub tempo_x100: u16,
}

/// The grid of one track either side of an edit, or of a run of edits
/// joined into one step (a tap run, a held nudge).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridStep {
    pub track: String,
    /// What Undo writes back, and what Redo expects to find.
    pub before: Arc<[GridBeat]>,
    /// What Redo writes, and what Undo expects to find.
    pub after: Arc<[GridBeat]>,
    /// The run the edit belongs to: the edits of one run, made one after
    /// another on one track, are one step.
    pub transaction: Option<String>,
}

impl GridStep {
    /// Why the core refuses to step a grid entry itself.
    pub const NOT_HERE: &'static str = "A beat grid step is made by the window's grid editor.";

    pub fn is_empty(&self) -> bool {
        self.before == self.after
    }

    /// Whether `next`, made just after this step, joins it.
    pub fn joins(&self, next: &Self) -> bool {
        self.track == next.track
            && self.transaction.is_some()
            && self.transaction == next.transaction
    }
}
