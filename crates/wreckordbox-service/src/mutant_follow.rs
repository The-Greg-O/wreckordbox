//! What a Library edit means for the Mutants of its playlist entries
//! (wbx-80): the [`Follow`] the Ledger records with the edit's entry.
//!
//! The Ledger's recording point ([`crate::ledger::Keeper`]) sees every
//! recorded edit, the window's and every command's, and every Undo and
//! Redo of one, with the history's exact payload. From that payload it is
//! known which entries (`djmdSongPlaylist.ID`) left a playlist and which
//! came back; the Sidecar records the Mutants' consequences in the same
//! transaction as the entry ([`Sidecar::append_following`]).
//!
//! | Edit | Edit / Redo | Undo |
//! |---|---|---|
//! | Remove tracks from a playlist | removed entries left | they returned |
//! | Delete a playlist (and what is in it) | its entries left | they returned |
//! | Add tracks | nothing | added entries left (Redo: returned) |
//! | Copy a playlist (`playlists.copy`) | a copy of each source Mutant on its new entry | the copy's entries left (Redo: returned) |
//! | Replace contents (XML/iTunes "Replace existing") | old entries left; each Mutant carried to its track's new entry | new left, old returned (Redo: the reverse) |
//! | A folder drop's Replace existing | the replaced list's entries left; the list's own Mutants carried to the new playlist's entries of their tracks | new left, old returned (Redo: the reverse) |
//! | Reorder (by track or by entry) | nothing: entries keep their identifiers | nothing |
//!
//! A carried or copied Mutant is made with its source's edits as they were
//! when the Library was changed: the plan read before the write
//! ([`plan_before`]), kept with the Ledger's intent, says so. A write that
//! was never confirmed (the process died between the Library's commit and
//! the entry) is recovered from that plan and from the Library as it is
//! ([`recover`]); so is an edit with no payload (Remove from Collection),
//! by bringing every Mutant into line with its entry ([`reconcile`]).
//!
//! Reads of the Library here tell a missing row from a failed read: a
//! failure is an error, and the Ledger keeps the change waiting, never a
//! reason to take a Mutant off an entry that may still be there.
//!
//! Cost: the payload names the entries, so most edits read no Library
//! rows; a replacement or copy reads the new entries' tracks by primary
//! key, and only when a Mutant is involved; [`reconcile`] reads the Mutant
//! table once plus one primary-key lookup per Mutant. No audio is read.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};

use rbl_db::{LibraryLocation, OpenMode};
use rbl_library::entry_copy::CopyIds;
use rbl_library::history::LibraryEdit;
use rbl_library::ledger::Step;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wreckordbox_sidecar::mutant_follow::{actions, Follow, Followed, Made, Snapshot};
use wreckordbox_sidecar::Sidecar;

use crate::stems::{self, Stems};

/// What a write that replaces or copies entries planned for their Mutants,
/// read before the write and kept with the Ledger's intent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Plan {
    /// For a replacement: every playlist that had a live Mutant, as it was.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    replaced: Vec<Before>,
    /// For a copy: the source's Mutants and the identifiers chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    copy: Option<CopyPlan>,
}

impl Plan {
    fn is_empty(&self) -> bool {
        self.replaced.is_empty() && self.copy.is_none()
    }
}

/// A playlist with Mutants, before a write that may replace its entries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Before {
    playlist: String,
    parent: String,
    name: String,
    /// Its live entries.
    entries: Vec<String>,
    mutants: Vec<Snapshot>,
}

/// A copy, before it is written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct CopyPlan {
    source: String,
    playlist: String,
    /// `(source entry, new entry)`, in order.
    pairs: Vec<(String, String)>,
    mutants: Vec<Snapshot>,
}

thread_local! {
    /// The playlist a command on this thread is about to copy, and then
    /// the identifiers chosen for the copy ([`plan_before`]).
    static COPYING: RefCell<Option<(String, Option<CopyIds>)>> = const { RefCell::new(None) };
}

/// Says that the next write on this thread copies `source`: the Ledger
/// plans the copy's identifiers and snapshots the source's Mutants before
/// the write ([`plan_before`]); the write takes the identifiers with
/// [`copy_ids`]. [`done_copying`] forgets it.
pub(crate) fn copying(source: &str) {
    COPYING.with(|copying| *copying.borrow_mut() = Some((source.to_owned(), None)));
}

/// The identifiers planned for the copy on this thread, if the Ledger
/// planned them.
pub(crate) fn copy_ids() -> Option<CopyIds> {
    COPYING.with(|copying| copying.borrow().as_ref().and_then(|(_, ids)| ids.clone()))
}

pub(crate) fn done_copying() {
    COPYING.with(|copying| *copying.borrow_mut() = None);
}

/// The plan for a write about to be made through `db` (the writer's own
/// connection, before the write): a copy the thread has asked for, and,
/// when `replaces` (an import's or a folder drop's Replace existing), every
/// playlist with a live Mutant as it is. `None` when there is nothing to
/// plan: no copy, no Mutant.
pub(crate) fn plan_before(
    db: &rbl_db::Library,
    sidecar: &Sidecar,
    replaces: bool,
) -> Result<Option<Value>, String> {
    let conn = db.connection();
    let mut plan = Plan::default();
    let source =
        COPYING.with(|copying| copying.borrow().as_ref().map(|(source, _)| source.clone()));
    if let Some(source) = source {
        let entries =
            rbl_library::entry_order::entries(conn, &source).map_err(|e| e.to_string())?;
        let ids = CopyIds::fresh(conn, entries.len()).map_err(|e| e.to_string())?;
        let from: Vec<String> = entries.iter().map(|entry| entry.id.clone()).collect();
        let mutants = sidecar
            .mutant_snapshots_of(&from)
            .map_err(|e| e.to_string())?;
        plan.copy = Some(CopyPlan {
            source: source.clone(),
            playlist: ids.playlist.clone(),
            pairs: from.into_iter().zip(ids.entries.iter().cloned()).collect(),
            mutants,
        });
        COPYING.with(|copying| *copying.borrow_mut() = Some((source, Some(ids))));
    }
    if replaces {
        let playlists = sidecar.mutated_playlists().map_err(|e| e.to_string())?;
        let mutants = sidecar
            .mutant_snapshots_in(&playlists)
            .map_err(|e| e.to_string())?;
        for playlist in playlists {
            let Some((parent, name)) = node(conn, &playlist)? else {
                continue;
            };
            plan.replaced.push(Before {
                entries: live_entries_of(conn, &playlist)?
                    .into_iter()
                    .map(|(entry, _)| entry)
                    .collect(),
                mutants: mutants
                    .iter()
                    .filter(|m| m.playlist_id == playlist)
                    .cloned()
                    .collect(),
                playlist,
                parent,
                name,
            });
        }
    }
    if plan.is_empty() {
        return Ok(None);
    }
    serde_json::to_value(&plan)
        .map(Some)
        .map_err(|e| e.to_string())
}

fn read_plan(plan: Option<&Value>) -> Result<Plan, String> {
    plan.map_or_else(
        || Ok(Plan::default()),
        |value| {
            serde_json::from_value(value.clone()).map_err(|e| format!("a damaged Mutant plan: {e}"))
        },
    )
}

/// The Mutants' consequences of one recorded step of `edit`, given the
/// plan made before it was written (if any).
///
/// # Errors
/// The Library or the Sidecar could not be read; the Ledger then keeps
/// the entry and works its consequences out again later.
pub(crate) fn of_edit(
    location: &LibraryLocation,
    sidecar: Option<&Sidecar>,
    step: Step,
    edit: &LibraryEdit,
    plan: Option<&Value>,
) -> Result<Follow, String> {
    let plan = read_plan(plan)?;
    let mut follow = Follow::default();
    let undo = step == Step::Undo;
    let owned = |list: &[&str]| list.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>();
    match edit {
        LibraryEdit::RemovePlaylistTracks(removal) => {
            let parts = removal.parts();
            left_or_returned(&mut follow, undo, owned(&parts.moved), &[parts.playlist]);
        }
        LibraryEdit::DeletePlaylist(deletion) => {
            let playlists: Vec<&str> = deletion.playlist_ids.iter().map(String::as_str).collect();
            left_or_returned(
                &mut follow,
                undo,
                deletion.membership_ids.clone(),
                &playlists,
            );
        }
        LibraryEdit::AddPlaylistTracks { addition, .. } => {
            let parts = addition.parts();
            match step {
                Step::Edit => {}
                Step::Undo => follow.left = owned(&parts.moved),
                Step::Redo => returned(&mut follow, owned(&parts.moved), parts.playlist),
            }
        }
        LibraryEdit::CopyPlaylist(copy) => {
            let (_, playlist, entries) = copy.parts();
            let new: Vec<String> = entries.iter().map(|(_, to, _)| to.clone()).collect();
            match step {
                Step::Edit => {
                    let from: Vec<String> =
                        entries.iter().map(|(from, _, _)| from.clone()).collect();
                    let snapshots = match plan.copy {
                        Some(planned) => planned.mutants,
                        None => live_snapshots(sidecar, &from)?,
                    };
                    let by_entry: HashMap<&str, &Snapshot> =
                        snapshots.iter().map(|s| (s.entry_id.as_str(), s)).collect();
                    for (from, to, _) in entries {
                        if let Some(snapshot) = by_entry.get(from.as_str()) {
                            follow.made.push(Made::from_snapshot(
                                actions::COPIED,
                                snapshot,
                                playlist,
                                to,
                            ));
                        }
                    }
                }
                Step::Undo => follow.left = new,
                Step::Redo => returned(&mut follow, new, playlist),
            }
        }
        LibraryEdit::ReplacePlaylistContents(replacements) => {
            for replacement in replacements {
                let parts = replacement.parts();
                let (old, new) = (owned(&parts.moved), owned(&parts.around));
                match step {
                    Step::Edit => {
                        let snapshots = planned_or_live(&plan, sidecar, &old)?;
                        carry(location, &mut follow, &snapshots, parts.playlist, &new)?;
                        follow.left.extend(old);
                    }
                    Step::Undo => {
                        follow.left.extend(new);
                        returned(&mut follow, old, parts.playlist);
                    }
                    Step::Redo => {
                        follow.left.extend(old);
                        returned(&mut follow, new, parts.playlist);
                    }
                }
            }
        }
        LibraryEdit::ReplacePlaylist(replacement) => {
            let parts = replacement.parts();
            let old = parts.replaced.membership_ids.clone();
            let new = owned(&parts.members);
            match step {
                Step::Edit => {
                    // The replaced list's own Mutants go to the new playlist;
                    // those of lists inside it (a folder) only leave.
                    let root = parts
                        .replaced
                        .playlist_ids
                        .first()
                        .cloned()
                        .unwrap_or_default();
                    let snapshots: Vec<Snapshot> = planned_or_live(&plan, sidecar, &old)?
                        .into_iter()
                        .filter(|s| s.playlist_id == root)
                        .collect();
                    carry(location, &mut follow, &snapshots, parts.playlist, &new)?;
                    follow.left.extend(old);
                }
                Step::Undo => {
                    follow.left.extend(new);
                    follow.returned.extend(old);
                    follow
                        .returned_in
                        .extend(parts.replaced.playlist_ids.iter().cloned());
                }
                Step::Redo => {
                    follow.left.extend(old);
                    returned(&mut follow, new, parts.playlist);
                }
            }
        }
        _ => {}
    }
    Ok(follow)
}

fn left_or_returned(follow: &mut Follow, undo: bool, entries: Vec<String>, playlists: &[&str]) {
    if undo {
        follow.returned.extend(entries);
        follow
            .returned_in
            .extend(playlists.iter().map(|id| (*id).to_owned()));
    } else {
        follow.left.extend(entries);
    }
}

fn returned(follow: &mut Follow, entries: Vec<String>, playlist: &str) {
    follow.returned.extend(entries);
    follow.returned_in.push(playlist.to_owned());
}

/// The live Mutants of `entries`: from the plan when it has them, as they
/// were before the write; else as they are now.
fn planned_or_live(
    plan: &Plan,
    sidecar: Option<&Sidecar>,
    entries: &[String],
) -> Result<Vec<Snapshot>, String> {
    if !plan.replaced.is_empty() {
        let wanted: HashSet<&str> = entries.iter().map(String::as_str).collect();
        return Ok(plan
            .replaced
            .iter()
            .flat_map(|before| before.mutants.iter())
            .filter(|s| wanted.contains(s.entry_id.as_str()))
            .cloned()
            .collect());
    }
    live_snapshots(sidecar, entries)
}

fn live_snapshots(sidecar: Option<&Sidecar>, entries: &[String]) -> Result<Vec<Snapshot>, String> {
    sidecar.map_or_else(
        || Ok(Vec::new()),
        |sidecar| {
            sidecar
                .mutant_snapshots_of(entries)
                .map_err(|e| e.to_string())
        },
    )
}

/// Each snapshot made again on an entry of its track among `new`, each new
/// entry taken once, in order. Reads the new entries' tracks only when
/// there is something to carry.
fn carry(
    location: &LibraryLocation,
    follow: &mut Follow,
    snapshots: &[Snapshot],
    playlist: &str,
    new: &[String],
) -> Result<(), String> {
    if snapshots.is_empty() || new.is_empty() {
        return Ok(());
    }
    let db = open(location)?;
    let tracks = tracks_of(db.connection(), new)?;
    let mut by_track: HashMap<&str, VecDeque<&str>> = HashMap::new();
    for entry in new {
        if let Some(track) = tracks.get(entry) {
            by_track.entry(track.as_str()).or_default().push_back(entry);
        }
    }
    for snapshot in snapshots {
        if let Some(entry) = by_track
            .get_mut(snapshot.track_id.as_str())
            .and_then(VecDeque::pop_front)
        {
            follow.made.push(Made::from_snapshot(
                actions::CARRIED,
                snapshot,
                playlist,
                entry,
            ));
        }
    }
    Ok(())
}

fn open(location: &LibraryLocation) -> Result<rbl_db::Library, String> {
    rbl_db::Library::open(location.clone(), OpenMode::ReadOnly).map_err(|e| e.to_string())
}

/// The track of each of these entries that is live, by primary key; a
/// failed read is an error, never a missing entry.
fn tracks_of(conn: &Connection, entries: &[String]) -> Result<HashMap<String, String>, String> {
    let mut found = HashMap::new();
    let mut statement = conn
        .prepare("SELECT ContentID FROM djmdSongPlaylist WHERE ID = ?1 AND rb_local_deleted = 0")
        .map_err(|e| e.to_string())?;
    for entry in entries {
        let track: Option<Option<String>> = statement
            .query_row([entry], |row| row.get(0))
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(Some(track)) = track {
            found.insert(entry.clone(), track);
        }
    }
    Ok(found)
}

/// Whether each entry is one a Mutant can be on, as `entries::find` has
/// it: a live row, in a live plain playlist, of a track still in the
/// collection. By primary key; a failed read is an error.
fn attached(conn: &Connection, entries: &[String]) -> Result<HashSet<String>, String> {
    let mut found = HashSet::new();
    let mut statement = conn
        .prepare(
            "SELECT 1 FROM djmdSongPlaylist AS entry
             JOIN djmdPlaylist AS playlist
               ON playlist.ID = entry.PlaylistID AND playlist.rb_local_deleted = 0
              AND playlist.Attribute = ?2
             JOIN djmdContent AS track
               ON track.ID = entry.ContentID AND track.rb_local_deleted = 0
             WHERE entry.ID = ?1 AND entry.rb_local_deleted = 0",
        )
        .map_err(|e| e.to_string())?;
    for entry in entries {
        let live = statement
            .query_row(
                params![entry, rbl_db::write::ATTRIBUTE_PLAYLIST],
                |_| Ok(()),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if live.is_some() {
            found.insert(entry.clone());
        }
    }
    Ok(found)
}

/// A live playlist's parent and name.
fn node(conn: &Connection, playlist: &str) -> Result<Option<(String, String)>, String> {
    conn.query_row(
        "SELECT COALESCE(ParentID, ''), COALESCE(Name, '') FROM djmdPlaylist
         WHERE ID = ?1 AND rb_local_deleted = 0",
        [playlist],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// A playlist's live entries with their tracks, through the playlist index.
fn live_entries_of(conn: &Connection, playlist: &str) -> Result<Vec<(String, String)>, String> {
    let mut statement = conn
        .prepare(
            "SELECT ID, ContentID FROM djmdSongPlaylist
             WHERE PlaylistID = ?1 AND rb_local_deleted = 0 ORDER BY TrackNo, rowid",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([playlist], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|row| match row {
            Ok((Some(id), Some(track))) => Some(Ok((id, track))),
            Ok(_) => None,
            Err(e) => Some(Err(e.to_string())),
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(rows)
}

/// Brings every Mutant into line with the Library as it is: a live
/// Mutant whose entry is gone (the row, its playlist or its track) is
/// detached, and one a playlist edit detached whose entry is back is
/// reattached. For a change whose payload does not say which entries
/// moved. Reads nothing from the Library when there are no Mutants.
///
/// # Errors
/// The Sidecar or the Library could not be read: nothing is assumed.
pub(crate) fn reconcile(location: &LibraryLocation, sidecar: &Sidecar) -> Result<Follow, String> {
    let mut follow = Follow::default();
    let attachments = sidecar.mutant_attachments().map_err(|e| e.to_string())?;
    if attachments.is_empty() {
        return Ok(follow);
    }
    let db = open(location)?;
    let entries: Vec<String> = attachments.iter().map(|a| a.entry_id.clone()).collect();
    let live = attached(db.connection(), &entries)?;
    let mut playlists = HashSet::new();
    for attachment in attachments {
        match (attachment.live, live.contains(&attachment.entry_id)) {
            (true, false) => follow.left.push(attachment.entry_id),
            (false, true) => {
                playlists.insert(attachment.playlist_id);
                follow.returned.push(attachment.entry_id);
            }
            _ => {}
        }
    }
    follow.returned_in = playlists.into_iter().collect();
    Ok(follow)
}

/// The Mutants' consequences of a write whose entry was never recorded:
/// every Mutant brought into line with the Library ([`reconcile`]), and
/// what the write's plan says a replacement carries or a copy copies, made
/// only where the Library shows the write was made.
///
/// # Errors
/// The plan, the Sidecar or the Library could not be read: the write stays
/// unrecovered, to be tried again.
pub(crate) fn recover(
    location: &LibraryLocation,
    sidecar: &Sidecar,
    plan: Option<&Value>,
) -> Result<Follow, String> {
    let plan = read_plan(plan)?;
    let mut follow = reconcile(location, sidecar)?;
    if plan.is_empty() {
        return Ok(follow);
    }
    let db = open(location)?;
    let conn = db.connection();
    if let Some(copy) = &plan.copy {
        let to: Vec<String> = copy.pairs.iter().map(|(_, to)| to.clone()).collect();
        let live = attached(conn, &to)?;
        let by_entry: HashMap<&str, &Snapshot> = copy
            .mutants
            .iter()
            .map(|s| (s.entry_id.as_str(), s))
            .collect();
        for (from, to) in &copy.pairs {
            if let (true, Some(snapshot)) = (live.contains(to), by_entry.get(from.as_str())) {
                follow.made.push(Made::from_snapshot(
                    actions::COPIED,
                    snapshot,
                    &copy.playlist,
                    to,
                ));
            }
        }
    }
    for before in &plan.replaced {
        // Replaced in place (an import), or by a new list of the same name
        // under the same parent (a folder drop).
        let target = if node(conn, &before.playlist)?.is_some() {
            Some(before.playlist.clone())
        } else {
            conn.query_row(
                "SELECT ID FROM djmdPlaylist
                 WHERE ParentID = ?1 AND Name = ?2 AND ID != ?3 AND rb_local_deleted = 0
                 ORDER BY created_at DESC LIMIT 1",
                params![before.parent, before.name, before.playlist],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
        };
        let Some(target) = target else {
            continue;
        };
        let held: HashSet<&str> = before.entries.iter().map(String::as_str).collect();
        let new: Vec<String> = live_entries_of(conn, &target)?
            .into_iter()
            .map(|(entry, _)| entry)
            .filter(|entry| !held.contains(entry.as_str()))
            .collect();
        let gone: Vec<String> = before.mutants.iter().map(|m| m.entry_id.clone()).collect();
        let still = attached(conn, &gone)?;
        let moved: Vec<Snapshot> = before
            .mutants
            .iter()
            .filter(|m| !still.contains(&m.entry_id))
            .cloned()
            .collect();
        carry(location, &mut follow, &moved, &target, &new)?;
    }
    Ok(follow)
}

/// Tells every client and the host of the Mutant changes a Follow made,
/// as each Mutant command does. Queued, never sent from under the gate.
pub(crate) fn announce(stems: &Stems, followed: &[Followed]) {
    for change in followed {
        stems.publish(
            stems::events::SIDECAR_CHANGED,
            json!(crate::mutants::MutantEvent {
                track_id: change.track_id.clone(),
                kind: crate::mutants::KIND.to_owned(),
                fact_id: None,
                stale: Vec::new(),
                mutant_id: change.mutant_id,
                playlist_id: change.playlist_id.clone(),
                entry_id: change.entry_id.clone(),
                action: change.action.clone(),
            }),
        );
    }
}
