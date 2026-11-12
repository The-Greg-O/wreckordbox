//! Where the core reports the changes its edit history sees, for a Ledger
//! kept outside it (ADR 0004).
//!
//! The core knows nothing of where a Ledger is kept or of who made an edit.
//! Whoever holds the core gives the edit history a [`Ledger`]
//! ([`crate::history::EditHistory::keep_ledger`]). From then on:
//!
//! - before any write is made, the Ledger is told one is about to be made
//!   ([`Ledger::begin`]) and records that durably; when it cannot, the
//!   write is refused and nothing changes, so the Library is never changed
//!   with nothing able to say so. Once the write is done it is told whether
//!   any row changed ([`Ledger::written`]), before the core re-reads
//!   anything, so a change whose refresh then fails is still recorded;
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

/// How an entry is tied to entries made before it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Link {
    /// For an Undo or Redo: the identifier the Ledger gave the change it
    /// steps (the edit, or the latest Undo or Redo of it).
    pub reverses: Option<i64>,
    /// For a later slice of one Add Tracks to Playlist: the identifier of
    /// its first slice. An Undo of that first slice takes back every slice
    /// that names it.
    pub part_of: Option<i64>,
}

/// Bounded evidence for recovery if a recorded write dies before history.record.
/// Empty targets mean no inference is safe, never “all tracks”.
#[derive(Debug, Default)]
pub struct Preparation {
    pub tracks: Vec<String>,
    /// Exact track/column pairs when stepping an existing edit.
    pub fields: Vec<(String, String)>,
    pub undo_of: Option<i64>,
    pub redo_of: Option<i64>,
}
impl Preparation {
    pub fn touched(touched: &crate::Touched) -> Self {
        Self {
            tracks: match touched {
                crate::Touched::Metadata(ids) => ids.clone(),
                _ => Vec::new(),
            },
            ..Self::default()
        }
    }
    pub fn edit(edit: &LibraryEdit) -> Self {
        let fields = match edit {
            LibraryEdit::Track(edits) => edits
                .iter()
                .map(|edit| {
                    let parts = edit.parts();
                    (parts.track, parts.column)
                })
                .filter(|(_, column)| {
                    matches!(column.as_str(), "Rating" | "ColorID" | "Title" | "ArtistID")
                })
                .collect(),
            LibraryEdit::TrackTags { tags, .. } => tags
                .iter()
                .map(|tag| (tag.parts().track, "MyTags".to_owned()))
                .collect(),
            _ => Vec::new(),
        };
        Self {
            fields,
            ..Self::default()
        }
    }
}

/// Keeps a durable record of the changes the edit history sees.
///
/// [`Self::record`] and [`Self::note`] are called with the history's lock
/// held and under the edit gate, on the thread that made the change;
/// [`Self::begin`] and [`Self::written`] under the edit gate alone, on the
/// thread making the write. None of them may take the
/// edit gate or the history, read the core, or call a [`crate::Host`]; they
/// may block briefly to write. A failure to record cannot undo a write
/// already made: it is the implementation's to keep (and to refuse the
/// next write for, in [`Self::ready`]).
pub trait Ledger: Send + Sync {
    /// A bounded operation recorder. Creating it does not write anything.
    fn bulk(&self, _kind: crate::bulk::Kind) -> Option<std::sync::Arc<dyn crate::bulk::Recorder>> { None }

    /// A write is about to be made: record durably that it was begun, so a
    /// process that dies before its entry is written leaves a trace. Asked
    /// before every write through the core; an error refuses the write
    /// with its text and nothing is changed.
    fn begin(&self) -> Result<(), String>;

    /// A row write outside Undo history. Bulk analysis/import scopes may join
    /// these writes; recorded playlist edits retain their ordinary intent,
    /// preparation and transactional consequences (including Mutant Follows).
    fn begin_unkept(&self) -> Result<(), String> { self.begin() }


    /// A file-only change, after the file-edit guards and before publication.
    /// Keep the kind and track with the intent for crash recovery. No audio,
    /// file snapshots or database scans belong here.
    fn begin_file(
        &self,
        _kind: &str,
        _track: &str,
        _values: &serde_json::Value,
    ) -> Result<(), String> {
        self.begin()
    }

    /// A file-only change was published. Recording is infallible to the
    /// caller, just like recording a committed row edit.
    fn file_written(&self, kind: &str, track: &str) {
        self.note(Unkept::Unrecorded, Some(&format!("{kind}: track {track}")));
    }

    /// The write [`Self::begin`] announced is over: `wrote` is whether it
    /// changed any row. Called before the core re-reads what it touched.
    fn written(&self, wrote: bool);

    /// Best-effort bounded recovery evidence, on the writer's existing connection.
    /// This must never refuse an accepted Library write or read any audio.
    fn prepare(&self, _db: &rbl_db::Library, _targets: &Preparation) {}

    /// Records one change: `edit` as the history keeps it, under `label`,
    /// tied to earlier entries by `link`. Answers the identifier of this
    /// change, which stays its identifier even when its entry has to be
    /// written later.
    fn record(&self, step: Step, edit: &LibraryEdit, label: &str, link: Link) -> Option<i64>;

    /// Names an edit the history keeps no entry for. `name` is what the
    /// caller calls it, when the caller says; the Ledger may know more
    /// (which command it is answering).
    fn note(&self, kept: Unkept, name: Option<&str>);
}
