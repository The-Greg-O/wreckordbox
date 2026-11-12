//! What a Library edit means for the Mutants of its playlist entries
//! (wbx-80): the [`Follow`] the Ledger records with the edit's entry.
//!
//! The Ledger's recording point ([`crate::ledger::Keeper`]) sees every
//! recorded edit, the window's and every command's, and every Undo and
//! Redo of one, with the history's exact payload. From that payload alone
//! it is known which entries (`djmdSongPlaylist.ID`) left a playlist and
//! which came back; the Sidecar records the Mutants' consequences in the
//! same transaction as the entry ([`Sidecar::append_following`]).
//!
//! | Edit | Edit / Redo | Undo |
//! |---|---|---|
//! | Remove tracks from a playlist | removed entries left | they returned |
//! | Delete a playlist (and what is in it) | its entries left | they returned |
//! | Add tracks (a copy's second step) | copies of the source's Mutants | added entries left (Redo: returned) |
//! | Replace a playlist's contents (XML/iTunes "Replace existing") | old entries left; each Mutant carried to its track's new entry | new left, old returned (Redo: the reverse) |
//! | Reorder (by track or by entry) | nothing: entries keep their identifiers | nothing |
//!
//! An edit that removes rows for good (Remove from Collection) has no
//! payload: the Mutants are brought into line with the Library instead
//! ([`reconcile`]), as they are for a write whose entry was never
//! confirmed (a crash between the write and its entry). Not followed:
//! a folder drop's Replace existing (`ReplacePlaylist`), whose payload this
//! crate cannot read; its Mutants are reconciled at the next permanent edit
//! or restart.
//!
//! Cost: the payload names the entries, so a Follow reads no Library rows
//! at all, except a replacement with a Mutant on one of its old entries
//! (its new entries' tracks, by primary key) and [`reconcile`] (one
//! primary-key lookup per Mutant). No audio is read.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use rbl_db::{LibraryLocation, OpenMode};
use rbl_library::history::LibraryEdit;
use rbl_library::ledger::Step;
use serde_json::json;
use wreckordbox_sidecar::mutant_follow::{Carry, Copy, Follow, Followed};
use wreckordbox_sidecar::Sidecar;

use crate::stems::{self, Stems};

thread_local! {
    /// Entries a command on this thread is copying, for the addition it
    /// is about to record ([`expect_copies`]).
    static COPIES: RefCell<Vec<Copy>> = const { RefCell::new(Vec::new()) };
}

/// Says that the next addition recorded on this thread is a copy of these
/// entries: their Mutants are copied with it, in its Ledger entry's
/// transaction. Called from inside the write, which runs on the thread
/// that records it.
pub(crate) fn expect_copies(copies: Vec<Copy>) {
    COPIES.with(|pending| *pending.borrow_mut() = copies);
}

/// Forgets copies no addition took (the write failed).
pub(crate) fn forget_copies() {
    COPIES.with(|pending| pending.borrow_mut().clear());
}

fn take_copies() -> Vec<Copy> {
    COPIES.with(|pending| std::mem::take(&mut *pending.borrow_mut()))
}

/// The Mutants' consequences of one recorded step of `edit`.
pub(crate) fn of_edit(
    location: &LibraryLocation,
    sidecar: Option<&Sidecar>,
    step: Step,
    edit: &LibraryEdit,
) -> Follow {
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
                Step::Edit => follow.copied = take_copies(),
                // The addition taken back: its entries leave; made again:
                // they return.
                Step::Undo => follow.left = owned(&parts.moved),
                Step::Redo => {
                    follow.returned = owned(&parts.moved);
                    follow.returned_in = vec![parts.playlist.to_owned()];
                }
            }
        }
        LibraryEdit::ReplacePlaylistContents(replacements) => {
            for replacement in replacements {
                let parts = replacement.parts();
                let (old, new) = (owned(&parts.moved), owned(&parts.around));
                match step {
                    Step::Edit => {
                        if let Some(carry) = carry(location, sidecar, parts.playlist, &old, &new) {
                            follow.carried.push(carry);
                        }
                        follow.left.extend(old);
                    }
                    Step::Undo => {
                        follow.left.extend(new);
                        follow.returned.extend(old);
                        follow.returned_in.push(parts.playlist.to_owned());
                    }
                    Step::Redo => {
                        follow.left.extend(old);
                        follow.returned.extend(new);
                        follow.returned_in.push(parts.playlist.to_owned());
                    }
                }
            }
        }
        _ => {}
    }
    follow
}

fn left_or_returned(follow: &mut Follow, undo: bool, entries: Vec<String>, playlists: &[&str]) {
    if undo {
        follow.returned = entries;
        follow.returned_in = playlists.iter().map(|id| (*id).to_owned()).collect();
    } else {
        follow.left = entries;
    }
}

/// A replacement's carry: only when one of its old entries has a live
/// Mutant, and then the new entries' tracks, read by primary key.
fn carry(
    location: &LibraryLocation,
    sidecar: Option<&Sidecar>,
    playlist: &str,
    old: &[String],
    new: &[String],
) -> Option<Carry> {
    let sidecar = sidecar?;
    let mutated = old
        .iter()
        .any(|entry| matches!(sidecar.mutant_of_entry(entry), Ok(Some(_))));
    if !mutated || new.is_empty() {
        return None;
    }
    let tracks = match tracks_of(location, new) {
        Ok(tracks) => tracks,
        Err(error) => {
            tracing::warn!(%error, "a replacement's new entries could not be read; its Mutants are detached, not carried");
            return None;
        }
    };
    Some(Carry {
        playlist: playlist.to_owned(),
        from: old.to_vec(),
        to: new
            .iter()
            .filter_map(|entry| {
                tracks
                    .get(entry)
                    .map(|track| (entry.clone(), track.clone()))
            })
            .collect(),
    })
}

/// The track of each of these entries, live ones only, by primary key.
fn tracks_of(
    location: &LibraryLocation,
    entries: &[String],
) -> Result<HashMap<String, String>, String> {
    let db =
        rbl_db::Library::open(location.clone(), OpenMode::ReadOnly).map_err(|e| e.to_string())?;
    let mut found = HashMap::new();
    let mut statement = db
        .connection()
        .prepare("SELECT ContentID FROM djmdSongPlaylist WHERE ID = ?1 AND rb_local_deleted = 0")
        .map_err(|e| e.to_string())?;
    for entry in entries {
        let track: Option<String> = statement.query_row([entry], |row| row.get(0)).ok();
        if let Some(track) = track {
            found.insert(entry.clone(), track);
        }
    }
    Ok(found)
}

/// Brings every Mutant into line with the Library as it is: a live
/// Mutant whose entry is gone is detached, and one a playlist edit
/// detached whose entry is back is reattached. For a change whose payload
/// does not say which entries moved. Reads nothing from the Library when
/// there are no Mutants.
pub(crate) fn reconcile(location: &LibraryLocation, sidecar: Option<&Sidecar>) -> Follow {
    let mut follow = Follow::default();
    let Some(sidecar) = sidecar else {
        return follow;
    };
    let attachments = match sidecar.mutant_attachments() {
        Ok(list) if !list.is_empty() => list,
        Ok(_) => return follow,
        Err(error) => {
            tracing::warn!(%error, "the Mutants could not be read to bring them into line");
            return follow;
        }
    };
    let entries: Vec<String> = attachments.iter().map(|a| a.entry_id.clone()).collect();
    let live = match tracks_of(location, &entries) {
        Ok(live) => live,
        Err(error) => {
            tracing::warn!(%error, "the Library could not be read to bring the Mutants into line");
            return follow;
        }
    };
    let mut playlists = HashSet::new();
    for attachment in attachments {
        match (attachment.live, live.contains_key(&attachment.entry_id)) {
            (true, false) => follow.left.push(attachment.entry_id),
            (false, true) => {
                playlists.insert(attachment.playlist_id);
                follow.returned.push(attachment.entry_id);
            }
            _ => {}
        }
    }
    follow.returned_in = playlists.into_iter().collect();
    follow
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
