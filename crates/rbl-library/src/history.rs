//! The edit history: what Undo and Redo of Library edits run on.
//!
//! One bounded stack per session, held by the [`crate::LibraryCore`] beside
//! the edit path, so an edit made by a client of the Library service and an
//! edit made in the window are entries of the same history. Not saved.
//!
//! [`EditHistory::record`] is the one place every recorded edit passes
//! through. A [`Ledger`] given to the history
//! ([`EditHistory::keep_ledger`]) is told of each edit there, and of each
//! Undo and Redo, so the durable record does not depend on the history
//! being kept.

use std::sync::Arc;

use rbl_db::DbError;

use crate::ledger::{Ledger, Link, Step, Unkept};
use crate::Touched;

/// The history label of an Info panel, rating, colour or comment edit.
pub const TRACK_EDIT: &str = "Track Edit";

/// What an entry changed outside the Library, stepped with it: the audio
/// files a key edit wrote its key into (#507). The core does not know what
/// they are; whoever recorded the entry supplies them.
pub trait OutsideSteps: Send + Sync {
    /// Nothing outside the Library was changed.
    fn is_empty(&self) -> bool;

    /// Refuses, before anything is written, an Undo (`undo`) or Redo when
    /// what was changed no longer holds what the step expects:
    /// [`DbError::Conflict`], as a changed row is.
    fn check(&self, undo: bool, label: &str) -> Result<(), DbError>;

    /// Undoes (`undo`) or redoes every change, or none.
    fn step(&self, undo: bool, label: &str) -> Result<(), DbError>;

    /// Puts back what [`Self::step`] changed, after the Library's own step
    /// failed; answers `(could not be put back, of how many)`.
    fn revert(&self, undo: bool) -> (usize, usize);
}

/// One reversible library operation. The window's grid edits are entries too
/// (`Grid`), stepped by its grid editor because they also rewrite analysis
/// files; text fields keep `WebKit`'s native history.
#[derive(Clone)]
pub enum LibraryEdit {
    DeletePlaylist(rbl_db::write::PlaylistDeletion),
    RenamePlaylist(rbl_db::write::PlaylistRename),
    MovePlaylist(rbl_db::write::PlaylistMove),
    /// A folder drop's Replace existing (#426): the replaced list and the
    /// playlist made in its place, undone and redone as one step.
    ReplacePlaylist(rbl_db::write::ListReplacement),
    /// An XML or iTunes import's Replace existing (#501): the members of
    /// every playlist it replaced, undone and redone as one step. The tracks
    /// and lists the import added are not part of it.
    ReplacePlaylistContents(Vec<rbl_db::write::MembershipReplacement>),
    SortPlaylists(rbl_db::write::PlaylistSort),
    RemovePlaylistTracks(rbl_db::write::PlaylistTrackRemoval),
    /// One Add Tracks to Playlist (#499): every slice of one drop or menu
    /// choice, joined while they share `transaction`.
    AddPlaylistTracks {
        addition: rbl_db::write::PlaylistTrackAddition,
        transaction: Option<String>,
    },
    /// One reorder or Renumber Track Order of a playlist (#499).
    ReorderPlaylist(rbl_db::write::PlaylistReorder),
    /// One reorder of a playlist by entry (wbx-59): each row of a track
    /// that is in the playlist twice moves on its own.
    ReorderEntries(crate::entry_order::EntryReorder),
    /// A playlist copied with its entries, in one write (wbx-80): Undo
    /// tombstones the copy and its entries, Redo brings them back.
    CopyPlaylist(crate::entry_copy::PlaylistCopy),
    Track(Vec<rbl_db::write::TrackEdit>),
    /// A key edit that also wrote the key into audio files, with "Write the
    /// value to the ID3 tag" on (#507): the library's steps, as `Track`, and
    /// each file written with the key its tag held. Undo and Redo put both
    /// back or neither. A key edit that wrote no file is a `Track` entry.
    KeyEdit {
        tracks: Vec<rbl_db::write::TrackEdit>,
        files: Arc<dyn OutsideSteps>,
    },
    /// One My Tag set per track: the Info tab's toggles edit one track,
    /// a tick in the My Tag panel every selected track. `comments` are the
    /// comments Add "My Tag" to the "Comments" rewrote with it (#605), put
    /// back and written again with the tags. Turning that setting on or off
    /// is an entry with comments and no tags.
    TrackTags {
        tags: Vec<rbl_db::write::TrackTagEdit>,
        comments: Vec<rbl_db::write::TrackEdit>,
    },
    /// The categories, or one category's tags, put in a new order (#584).
    MyTagOrder(rbl_db::write::MyTagOrder),
    /// One cue or loop command on one track (#504): an add, move, slot,
    /// colour, comment, Active Loop or delete, or a whole Convert Memory
    /// Cues to Hot Cues.
    Cues(rbl_db::write::CueRevision),
    /// A playlist, folder or intelligent playlist made (#500). Undo
    /// tombstones the same row unless something has been put in it since.
    CreatePlaylist(rbl_db::write::PlaylistCreation),
    /// An intelligent playlist's rule saved, or a playlist's or folder's
    /// artwork reference changed (#500).
    PlaylistEdit(rbl_db::write::PlaylistEdit),
    /// One Relocate, Relocate by location or Auto Relocate run (#502):
    /// every track's file link before and after, put back in one step.
    Relocate(rbl_db::write::TrackRelocation),
    /// One Remove from Tag List, Clear Tag List or Remove from History
    /// (#508): the rows taken off the list and its order before and after.
    ListRemoval(rbl_db::write::ListRemoval),
    /// The DJ's judgements of kinds the Library cannot hold (wbx-65): no
    /// row is written, Undo and Redo write none; the Ledger records,
    /// withdraws and records again the Facts (`crate::judgements`).
    Judgements(crate::judgements::Judgements),
    /// A beat grid edit made in the window, or a run of them (a tap run, a
    /// held nudge) (wbx-64): the beats either side. Stepped by the window's
    /// grid editor, never by [`crate::LibraryCore::step_history`]; see
    /// `crate::grid_step`.
    Grid(crate::grid_step::GridStep),
}

impl LibraryEdit {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::DeletePlaylist(edit) => edit.is_empty(),
            Self::RemovePlaylistTracks(edit) => edit.is_empty(),
            Self::ReplacePlaylistContents(edits) => edits.is_empty(),
            Self::AddPlaylistTracks { addition, .. } => addition.is_empty(),
            Self::ReorderPlaylist(edit) => edit.is_empty(),
            Self::ReorderEntries(edit) => edit.is_empty(),
            Self::Track(edits) | Self::KeyEdit { tracks: edits, .. } => {
                edits.is_empty() || edits.iter().all(rbl_db::write::TrackEdit::is_empty)
            }
            Self::TrackTags { tags, comments } => {
                tags.iter().all(rbl_db::write::TrackTagEdit::is_empty)
                    && comments.iter().all(rbl_db::write::TrackEdit::is_empty)
            }
            Self::MyTagOrder(edit) => edit.is_empty(),
            Self::RenamePlaylist(_) | Self::MovePlaylist(_) | Self::ReplacePlaylist(_) | Self::CreatePlaylist(_)
            | Self::CopyPlaylist(_) => false,
            Self::SortPlaylists(edit) => edit.is_empty(),
            Self::Cues(edit) => edit.is_empty(),
            Self::PlaylistEdit(edit) => edit.is_empty(),
            Self::Relocate(edit) => edit.is_empty(),
            Self::ListRemoval(edit) => edit.is_empty(),
            Self::Judgements(judgements) => judgements.is_empty(),
            Self::Grid(step) => step.is_empty(),
        }
    }

    /// What Undo or Redo of this entry leaves to re-read.
    pub fn touched(&self) -> Touched {
        match self {
            Self::DeletePlaylist(_) | Self::RenamePlaylist(_) |
            Self::MovePlaylist(_) | Self::SortPlaylists(_) | Self::RemovePlaylistTracks(_) |
            Self::CreatePlaylist(_) | Self::PlaylistEdit(_) |
            Self::AddPlaylistTracks { .. } | Self::ReorderPlaylist(_) | Self::ReorderEntries(_) |
            Self::CopyPlaylist(_) |
            // The tracks a replacement imported stay in the collection.
            Self::ReplacePlaylist(_) | Self::ReplacePlaylistContents(_) => Touched::Playlists,
            // Tokens keep their database row ids private; a full reload after an
            // undo is uncommon and guarantees every view and sort follows it.
            // The index's My Tag list (filters, panel) is read in a full load.
            Self::Track(_) | Self::KeyEdit { .. } | Self::TrackTags { .. } | Self::Relocate(_)
            | Self::MyTagOrder(_) => Touched::Tracks,
            Self::Cues(revision) => Touched::Cues(revision.content().to_owned()),
            Self::ListRemoval(removal) => match removal.list() {
                rbl_db::write::RemovalList::TagList => Touched::TagList,
                rbl_db::write::RemovalList::History(_) => Touched::Histories(Vec::new()),
            },
            // No row changes: the cheapest refresh, which still tells the
            // clients (and the window) that something changed.
            Self::Judgements(_) => Touched::Metadata(Vec::new()),
            // The track's BPM row; the window re-reads its grid itself.
            Self::Grid(step) => Touched::Metadata(vec![step.track.clone()]),
        }
    }

    /// Refuses, with [`DbError::Conflict`] and before anything is written,
    /// an entry of several steps any one of which no longer finds the value
    /// it expects. An entry of one step, or one written in a single
    /// transaction, is checked by the writer as it runs.
    pub fn check(&self, writer: &rbl_db::write::Writer, undo: bool) -> Result<(), DbError> {
        match self {
            Self::Track(values) => values.iter().try_for_each(|value| writer.check_track_edit(value, undo)),
            // The library's rows and every file the edit wrote, before either is
            // written (#507).
            Self::KeyEdit { tracks, files } => {
                tracks.iter().try_for_each(|value| writer.check_track_edit(value, undo))?;
                files.check(undo, TRACK_EDIT)
            }
            Self::TrackTags { tags, comments } => {
                tags.iter().try_for_each(|value| writer.check_tag_edit(value, undo))?;
                comments.iter().try_for_each(|value| writer.check_track_edit(value, undo))
            }
            Self::DeletePlaylist(_) | Self::RenamePlaylist(_) | Self::MovePlaylist(_)
            | Self::ReplacePlaylist(_) | Self::SortPlaylists(_) | Self::RemovePlaylistTracks(_)
            | Self::AddPlaylistTracks { .. } | Self::ReorderPlaylist(_) | Self::ReorderEntries(_)
            | Self::CopyPlaylist(_)
            // Checked in the one transaction that writes it (#508).
            | Self::ListRemoval(_)
            // Checked in the transaction that writes it.
            | Self::MyTagOrder(_)
            // Every playlist of a replacement is checked in the one transaction
            // that writes them.
            | Self::ReplacePlaylistContents(_)
            // Nothing of the Library's to find changed.
            | Self::Judgements(_) => Ok(()),
            // One transaction, but the pad check reads rows the entry does not
            // name, so it is done here as well as in the writer (#504).
            Self::Cues(revision) => match writer.cue_revision_conflict(revision, undo)? {
                Some(reason) => Err(DbError::Conflict(reason)),
                None => Ok(()),
            },
            // Checked again in the writer's transaction (#500).
            Self::CreatePlaylist(creation) => match writer.creation_conflict(creation, undo)? {
                Some(reason) => Err(DbError::Conflict(reason)),
                None => Ok(()),
            },
            Self::PlaylistEdit(value) => writer.check_playlist_edit(value, undo),
            // Checked again in the writer's transaction (#502).
            Self::Relocate(relocation) => match writer.relocation_conflict(relocation, undo)? {
                Some(reason) => Err(DbError::Conflict(reason)),
                None => Ok(()),
            },
            // Its files are the window's grid editor's to write: refused
            // before anything is written, and the entry stays.
            Self::Grid(_) => Err(DbError::WriteRefused(crate::grid_step::GridStep::NOT_HERE.to_owned())),
        }
    }

    /// Undoes (`undo`) or redoes this entry through `writer`.
    pub fn apply(&self, writer: &mut rbl_db::write::Writer, undo: bool) -> Result<(), DbError> {
        match self {
            Self::DeletePlaylist(value) => if undo { writer.restore_playlist(value) } else { writer.redo_playlist_deletion(value) }.map(|_| ()),
            Self::RenamePlaylist(value) => if undo { writer.undo_rename(value) } else { writer.redo_rename(value) }.map(|_| ()),
            Self::MovePlaylist(value) => if undo { writer.undo_move(value) } else { writer.redo_move(value) }.map(|_| ()),
            Self::ReplacePlaylist(value) => if undo { writer.undo_replacement(value) } else { writer.redo_replacement(value) }.map(|_| ()),
            Self::ReplacePlaylistContents(values) => if undo {
                writer.undo_membership_replacements(values)
            } else {
                writer.redo_membership_replacements(values)
            }.map(|_| ()),
            Self::SortPlaylists(value) => if undo { writer.undo_sort(value) } else { writer.redo_sort(value) }.map(|_| ()),
            Self::RemovePlaylistTracks(value) => if undo { writer.undo_track_removal(value) } else { writer.redo_track_removal(value) }.map(|_| ()),
            Self::AddPlaylistTracks { addition, .. } => if undo { writer.undo_track_addition(addition) } else { writer.redo_track_addition(addition) }.map(|_| ()),
            Self::ReorderPlaylist(value) => if undo { writer.undo_reorder(value) } else { writer.redo_reorder(value) }.map(|_| ()),
            Self::ReorderEntries(value) => crate::entry_order::step(writer, value, undo),
            Self::CopyPlaylist(value) => crate::entry_copy::step(writer, value, undo),
            Self::Track(values) => {
                let ordered: Box<dyn Iterator<Item = _>> = if undo {
                    Box::new(values.iter().rev())
                } else {
                    Box::new(values.iter())
                };
                for value in ordered {
                    if undo { writer.undo_track_edit(value)?; } else { writer.redo_track_edit(value)?; }
                }
                Ok(())
            }
            // The files first: they are what can still fail after the check (a
            // file made read-only since). A failure there has put back the files
            // it wrote and leaves the library as it was; a failure in the library
            // puts the files and the tracks already written back (#507).
            Self::KeyEdit { tracks, files } => {
                files.step(undo, TRACK_EDIT)?;
                let error = match step_track_edits(writer, tracks, undo) {
                    Ok(()) => return Ok(()),
                    Err(error) => error,
                };
                let (stranded, of) = files.revert(undo);
                if stranded == 0 {
                    return Err(error);
                }
                Err(DbError::Io(std::io::Error::other(format!(
                    "{error}; {stranded} of {of} files could not be changed back"
                ))))
            }
            Self::TrackTags { tags, comments } => step_track_tags(writer, tags, comments, undo),
            Self::MyTagOrder(value) => if undo { writer.undo_my_tag_order(value) } else { writer.redo_my_tag_order(value) }.map(|_| ()),
            Self::Cues(value) => if undo { writer.undo_cue_revision(value) } else { writer.redo_cue_revision(value) }.map(|_| ()),
            Self::CreatePlaylist(value) => if undo { writer.undo_creation(value) } else { writer.redo_creation(value) }.map(|_| ()),
            Self::PlaylistEdit(value) => if undo { writer.undo_playlist_edit(value) } else { writer.redo_playlist_edit(value) }.map(|_| ()),
            Self::Relocate(value) => if undo { writer.undo_relocation(value) } else { writer.redo_relocation(value) }.map(|_| ()),
            Self::ListRemoval(value) => if undo { writer.undo_list_removal(value) } else { writer.redo_list_removal(value) }.map(|_| ()),
            Self::Judgements(_) => Ok(()),
            Self::Grid(_) => Err(DbError::WriteRefused(crate::grid_step::GridStep::NOT_HERE.to_owned())),
        }
    }
}

/// Undoes or redoes a My Tag entry: the tags, and the comments Add "My
/// Tag" to the "Comments" rewrote with them (#605), the comments in one
/// transaction. Undo puts the comments back before the tags, Redo writes
/// them after, so a refused comment step leaves the tags as they were; a
/// tag step that fails after the comments has them stepped back again.
fn step_track_tags(
    writer: &mut rbl_db::write::Writer,
    tags: &[rbl_db::write::TrackTagEdit],
    comments: &[rbl_db::write::TrackEdit],
    undo: bool,
) -> Result<(), DbError> {
    let step_tags = |writer: &mut rbl_db::write::Writer| -> Result<(), DbError> {
        let ordered: Box<dyn Iterator<Item = _>> = if undo { Box::new(tags.iter().rev()) } else { Box::new(tags.iter()) };
        for value in ordered {
            if undo { writer.undo_tag_edit(value)?; } else { writer.redo_tag_edit(value)?; }
        }
        Ok(())
    };
    if !undo {
        step_tags(writer)?;
        if comments.is_empty() {
            return Ok(());
        }
        if let Err(error) = writer.step_track_edits_at_once(comments, false) {
            for value in tags.iter().rev() {
                if let Err(back) = writer.undo_tag_edit(value) {
                    tracing::error!(%back, "a My Tag edit could not be put back after a failed Redo");
                }
            }
            return Err(error);
        }
        return Ok(());
    }
    if !comments.is_empty() {
        writer.step_track_edits_at_once(comments, true)?;
    }
    if let Err(error) = step_tags(writer) {
        if !comments.is_empty() {
            if let Err(back) = writer.step_track_edits_at_once(comments, false) {
                tracing::error!(%back, "the My Tag comments could not be written again after a failed Undo");
            }
        }
        return Err(error);
    }
    Ok(())
}

/// Undoes (`undo`, last first) or redoes the steps of a track edit, all or
/// none: a step that fails has the ones before it put back.
fn step_track_edits(
    writer: &mut rbl_db::write::Writer,
    values: &[rbl_db::write::TrackEdit],
    undo: bool,
) -> Result<(), DbError> {
    let ordered: Vec<&rbl_db::write::TrackEdit> = if undo { values.iter().rev().collect() } else { values.iter().collect() };
    for (at, value) in ordered.iter().enumerate() {
        let step = if undo { writer.undo_track_edit(value) } else { writer.redo_track_edit(value) };
        if let Err(error) = step {
            let mut stranded = 0usize;
            for done in ordered.get(..at).unwrap_or_default().iter().rev() {
                let back = if undo { writer.redo_track_edit(done) } else { writer.undo_track_edit(done) };
                if let Err(back) = back {
                    tracing::error!(error = %back, "could not put a track back after a failed Undo or Redo");
                    stranded += 1;
                }
            }
            if stranded == 0 {
                return Err(error);
            }
            return Err(DbError::Io(std::io::Error::other(format!(
                "{error}; {stranded} of {} already-changed tracks could not be changed back",
                ordered.len()
            ))));
        }
    }
    Ok(())
}

#[derive(Clone)]
pub struct HistoryEntry {
    pub edit: LibraryEdit,
    pub label: &'static str,
    /// What the history's [`Ledger`] called this edit, or its latest Undo
    /// or Redo, when there is a Ledger and it recorded it.
    pub ledger: Option<i64>,
}

/// Bounded session history for reversible library edits. The payloads retain
/// exact database values or row ids, rather than reconstructing state from the
/// interface when an action is undone.
#[derive(Default)]
pub struct EditHistory {
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    /// Told of every edit recorded here and every step of one.
    ledger: Option<Arc<dyn Ledger>>,
}

impl EditHistory {
    const LIMIT: usize = 50;

    /// Records one edit as the next Undo and ends the Redo branch.
    ///
    /// The single recording point: every edit that enters the history, the
    /// window's or a service client's, passes through here once. (A later
    /// slice of one Add Tracks to Playlist extends the entry its first
    /// slice recorded here; see [`Self::record_addition`].)
    pub fn record(&mut self, edit: LibraryEdit, label: &'static str) {
        let ledger = self.ledger.as_ref().and_then(|ledger| ledger.record(Step::Edit, &edit, label, Link::default()));
        self.undo.push(HistoryEntry { edit, label, ledger });
        if self.undo.len() > Self::LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Records a window grid edit (wbx-64). An edit of the run on top of
    /// Undo (the same track and `transaction`: a tap run, a held nudge)
    /// joins it, so the run is one Undo back to the grid before its first
    /// edit; the Ledger records each write, the later ones as part of the
    /// first, as the slices of one Add Tracks to Playlist are.
    pub fn record_grid(&mut self, step: crate::grid_step::GridStep, label: &'static str) {
        if step.is_empty() {
            return;
        }
        if let Some(HistoryEntry { edit: LibraryEdit::Grid(open), ledger: first, .. }) = self.undo.last_mut() {
            if open.joins(&step) {
                open.after = std::sync::Arc::clone(&step.after);
                self.redo.clear();
                if let Some(ledger) = &self.ledger {
                    ledger.record(Step::Edit, &LibraryEdit::Grid(step), label, Link { reverses: None, part_of: *first });
                }
                return;
            }
        }
        self.record(LibraryEdit::Grid(step), label);
    }

    /// Moves the next Undo (`undo`) or Redo entry across once something
    /// other than the core has made its step: the window's grid editor, for
    /// a [`LibraryEdit::Grid`] entry. The Ledger is told, as for any step.
    /// Call it under the edit gate, after the step was written; answers the
    /// entry, or `None` when there was none.
    pub fn stepped_elsewhere(&mut self, undo: bool) -> Option<&HistoryEntry> {
        let entry = self.drop_next(undo)?;
        self.stepped(entry, undo);
        self.next(!undo)
    }

    /// Records one slice of an Add Tracks to Playlist (#499). A slice of the
    /// same add (the same `transaction`) that carries on from the entry on
    /// top of Undo joins it, so a drop of thousands, sent a hundred at a
    /// time, is still one Undo. Anything else is an entry of its own.
    pub fn record_addition(
        &mut self,
        addition: rbl_db::write::PlaylistTrackAddition,
        transaction: Option<String>,
        label: &'static str,
    ) {
        if addition.is_empty() {
            return;
        }
        if let (Some(id), Some(HistoryEntry { edit: LibraryEdit::AddPlaylistTracks { addition: open, transaction: Some(open_id) }, ledger: first, .. }))
            = (&transaction, self.undo.last_mut())
        {
            if open_id == id && open.absorb(&addition) {
                self.redo.clear();
                // The Ledger records each slice as it is made, as part of
                // the first: Undo takes back the whole add, and its entry
                // reverses the first slice and so every slice part of it.
                if let Some(ledger) = &self.ledger {
                    let slice = LibraryEdit::AddPlaylistTracks { addition, transaction };
                    ledger.record(Step::Edit, &slice, label, Link { reverses: None, part_of: *first });
                }
                return;
            }
        }
        self.record(LibraryEdit::AddPlaylistTracks { addition, transaction }, label);
    }

    /// Gives the history a [`Ledger`] to tell of every edit and step from
    /// now on, or takes it away (`None`).
    pub fn keep_ledger(&mut self, ledger: Option<Arc<dyn Ledger>>) {
        self.ledger = ledger;
    }

    /// The [`Ledger`] the history tells, when it has one. Take it out and
    /// let the history go before asking it anything.
    pub fn ledger(&self) -> Option<Arc<dyn Ledger>> {
        self.ledger.clone()
    }

    pub fn clear_redo(&mut self) {
        self.redo.clear();
    }

    /// An edit the history records nothing for was made: the Redo branch
    /// ends ([`Self::clear_redo`]) and the [`Ledger`] is told, by `name`
    /// when the caller has one for it. Call it once the write is made.
    pub fn unrecorded(&mut self, name: Option<&str>) {
        self.redo.clear();
        if let Some(ledger) = &self.ledger {
            ledger.note(Unkept::Unrecorded, name);
        }
    }

    /// An edit that removed rows for good was made: both stacks are
    /// emptied ([`Self::clear`]) and the [`Ledger`] is told, by `name` when
    /// the caller has one for it. Call it once the write is made.
    pub fn permanent(&mut self, name: Option<&str>) {
        self.clear();
        if let Some(ledger) = &self.ledger {
            ledger.note(Unkept::Permanent, name);
        }
    }

    /// Takes the next Undo (`undo`) or Redo entry off its stack and leaves
    /// every other entry where it is: what a refused, conflicting entry
    /// comes to (#506). Each remaining entry checks its own rows when it
    /// runs, so keeping them cannot overwrite anything either.
    pub fn drop_next(&mut self, undo: bool) -> Option<HistoryEntry> {
        if undo { self.undo.pop() } else { self.redo.pop() }
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    /// The entry Undo (`undo`) or Redo would run next.
    pub fn next(&self, undo: bool) -> Option<&HistoryEntry> {
        if undo { self.undo.last() } else { self.redo.last() }
    }

    /// The label of the edit Undo would take back, when there is one.
    /// What the Ledger called the latest entry Undo would take back, when
    /// there is one and the Ledger recorded it.
    #[must_use]
    pub fn latest_ledger_entry(&self) -> Option<i64> {
        self.undo.last().and_then(|entry| entry.ledger)
    }

    pub fn undo_label(&self) -> Option<&'static str> {
        self.undo.last().map(|entry| entry.label)
    }

    /// The label of the edit Redo would make again, when there is one.
    pub fn redo_label(&self) -> Option<&'static str> {
        self.redo.last().map(|entry| entry.label)
    }

    /// Moves an entry that has just been undone (`undone`) to Redo, or one
    /// redone back to Undo. Not a recording: the edit is already in the
    /// history. The Ledger is told of the step.
    fn stepped(&mut self, mut entry: HistoryEntry, undone: bool) {
        if let Some(ledger) = &self.ledger {
            let step = if undone { Step::Undo } else { Step::Redo };
            // The step's own identifier, so the next step names it; never
            // the one before, which would make a Redo name the edit rather
            // than the Undo it reverses.
            entry.ledger = ledger.record(step, &entry.edit, entry.label, Link { reverses: entry.ledger, part_of: None });
        }
        if undone { self.redo.push(entry); } else { self.undo.push(entry); }
    }
}

/// What an Undo or Redo did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stepped {
    /// The entry's label.
    pub label: &'static str,
    /// What the step left to re-read, which the core has re-read.
    pub touched: Touched,
    /// The generation the index is at now.
    pub generation: u32,
}

/// Why an Undo or Redo did not run.
#[derive(Debug)]
pub enum StepError {
    /// There is no entry to undo, or to redo.
    Nothing,
    /// Something outside the history changed what the entry would write
    /// (#506). Nothing was written, the newer value stays, and only this
    /// entry has left the history.
    Conflict { label: &'static str, reason: String },
    /// The write failed; the entry stays to be tried again.
    Write(DbError),
}

impl crate::LibraryCore {
    /// The edit history. A leaf lock: take it under the edit gate, hold it
    /// for the bookkeeping alone, and never write, read the core, or call a
    /// [`crate::Host`] while holding it.
    pub fn history(&self) -> &parking_lot::Mutex<EditHistory> {
        &self.history
    }

    /// One recorded edit, whole: the guarded write, the refresh of what it
    /// `touched`, and the entry `action` answers put in the history under
    /// `label`, all under the edit gate. An entry that changed nothing is
    /// not recorded. The host is told the history changed
    /// ([`crate::Host::history_changed`]).
    ///
    /// Blocking.
    pub fn recorded_edit<T>(
        &self,
        host: &(impl crate::Host + ?Sized),
        touched: Touched,
        label: &'static str,
        action: impl FnOnce(&mut rbl_db::write::Writer) -> Result<(T, LibraryEdit), DbError>,
    ) -> Result<(T, crate::Change), DbError> {
        let _gate = self.edit_gate().lock();
        let kind = touched.change_kind();
        // The refresh's own failure is kept aside: the edit is made by then
        // and goes into the history (and the Ledger) whatever it says.
        let preparation = crate::ledger::Preparation::touched(&touched);
        let (value, reversible, refreshed) = self.write_then_prepared(host, crate::History::Caller, &preparation, action, |db, (value, reversible)| {
            Ok((value, reversible, self.refresh_after_edit(host, db, touched)))
        })?;
        if !reversible.is_empty() {
            self.history.lock().record(reversible, label);
            host.history_changed();
        }
        let generation = refreshed?;
        Ok((value, crate::Change { kind, generation }))
    }

    /// One edit that Undo must never traverse, whole: the guarded write and
    /// the refresh of what it `touched`, then both stacks emptied, because
    /// an older entry could name rows the write removed for good. The host
    /// is told when there was a history to empty.
    ///
    /// Blocking.
    pub fn permanent_edit<T>(
        &self,
        host: &(impl crate::Host + ?Sized),
        touched: Touched,
        action: impl FnOnce(&mut rbl_db::write::Writer) -> Result<T, DbError>,
    ) -> Result<(T, crate::Change), DbError> {
        let _gate = self.edit_gate().lock();
        let done = self.edit(host, crate::History::Caller, touched, action)?;
        let had = {
            let mut history = self.history.lock();
            let had = history.undo_label().is_some() || history.redo_label().is_some();
            history.permanent(None);
            had
        };
        if had {
            host.history_changed();
        }
        Ok(done)
    }

    /// Runs the next Undo (`undo`) or Redo entry of the history, and
    /// re-reads what it touched.
    ///
    /// Every row it would write is checked first against the value the entry
    /// expects there: the edit's result for Undo, what it replaced for Redo.
    /// When something outside the history (a player over LINK, an import,
    /// rekordbox) changed one since, nothing is written, the newer value
    /// stays, and only that entry leaves the history
    /// ([`StepError::Conflict`]). The rest of the history stays usable: each
    /// entry checks its own rows when it runs.
    ///
    /// The caller tells its own clients what the history offers now; the
    /// host is not told.
    ///
    /// Blocking.
    pub fn step_history(&self, host: &(impl crate::Host + ?Sized), undo: bool) -> Result<Stepped, StepError> {
        let _gate = self.edit_gate().lock();
        let entry = self.history.lock().next(undo).cloned().ok_or(StepError::Nothing)?;
        // The window's grid editor steps a grid entry (its files are not the
        // core's to write): refused before any write or Ledger intent, and
        // the entry stays.
        if matches!(entry.edit, LibraryEdit::Grid(_)) {
            return Err(StepError::Write(DbError::WriteRefused(crate::grid_step::GridStep::NOT_HERE.to_owned())));
        }
        let touched = entry.edit.touched();
        let mut preparation = crate::ledger::Preparation::edit(&entry.edit);
        if undo { preparation.undo_of = entry.ledger; } else { preparation.redo_of = entry.ledger; }
        let result = self.write_then_prepared(
            host,
            crate::History::Caller,
            &preparation,
            |w| {
                entry.edit.check(w, undo)?;
                entry.edit.apply(w, undo)
            },
            // Kept aside: the step is made by then and is recorded whatever
            // the refresh says.
            |db, ()| Ok(self.refresh_after_edit(host, db, touched.clone())),
        );
        let mut history = self.history.lock();
        match result {
            Ok(refreshed) => {
                history.drop_next(undo);
                let label = entry.label;
                history.stepped(entry, undo);
                let generation = refreshed.map_err(StepError::Write)?;
                Ok(Stepped { label, touched, generation })
            }
            Err(DbError::Conflict(reason)) => {
                history.drop_next(undo);
                tracing::info!(label = entry.label, undo, %reason, "library history entry refused: changed since");
                Err(StepError::Conflict { label: entry.label, reason })
            }
            Err(error) => Err(StepError::Write(error)),
        }
    }
}
