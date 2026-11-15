//! Cue conversion records every accepted slice while keeping one Undo entry.
use super::{EditHistory, HistoryEntry, LibraryEdit, Link, Step};
use rbl_db::write::CueRevision;
impl EditHistory {
    /// Join only the exact preceding aggregate. An intervening edit/Undo starts
    /// a separate history entry, so unrelated work can never be swallowed.
    pub fn record_cue_slice(
        &mut self,
        revision: CueRevision,
        previous: Option<&CueRevision>,
        label: &'static str,
    ) -> CueRevision {
        if let Some(HistoryEntry {
            edit: LibraryEdit::Cues(open),
            ledger: first,
            ..
        }) = self.undo.last_mut()
        {
            if previous == Some(open) && open.content() == revision.content() {
                open.extend(revision.clone());
                self.redo.clear();
                if let Some(ledger) = &self.ledger {
                    ledger.record(
                        Step::Edit,
                        &LibraryEdit::Cues(revision),
                        label,
                        Link {
                            reverses: None,
                            part_of: *first,
                        },
                    );
                }
                return open.clone();
            }
        }
        self.record(LibraryEdit::Cues(revision.clone()), label);
        revision
    }
}
