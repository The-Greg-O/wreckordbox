//! A Mutant follows its playlist entry (wbx-80).
//!
//! A Mutant is keyed to a playlist entry (`djmdSongPlaylist.ID`). When a
//! Library edit takes entries out of a playlist (a removal, a playlist
//! deleted, a replacement of its contents) their Mutants are **detached**;
//! when an Undo or Redo brings the same entries back, the Mutants that were
//! detached from them are **reattached**, with the same identifier and
//! edits. A replacement that writes new entries for tracks it keeps
//! **carries** each Mutant to the new entry of its track, and a copied
//! playlist gets **copies** of its source's Mutants.
//!
//! Nothing here reads the Library. The caller declares what happened to
//! which entries ([`Follow`]) and this module records the Mutants'
//! consequences inside the transaction that appends the Library change's
//! Ledger entry ([`Sidecar::append_following`]), so the change's entry and
//! its Mutant consequences are kept together or not at all, under the
//! entry's Run and source. A Follow that cannot be applied is rolled back
//! on its own and the entry is kept, saying so (`mutants_not_followed` in
//! its payload): a Ledger entry is never lost to a Mutant.
//!
//! Every lookup is by entry through an index (the live Mutant of an entry)
//! or by playlist through an index (the Mutants detached from a playlist's
//! entries), never a scan of the table per entry.
//!
//! Detaching is a removal (`removed_at` is set), so a detached Mutant is
//! not listed and not loaded to a Stick. Reattaching clears `removed_at`
//! again: the one exception to "set once", and only for a Mutant whose
//! newest change is `detached`. A Mutant the DJ or an agent removed on
//! purpose (`removed`) is never brought back by a playlist edit.

use std::collections::HashSet;

use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::{Result, RunId, Source};

/// The actions this module records in `mutant_change.action`.
pub mod actions {
    /// The entry left its playlist; the Mutant went with it.
    pub const DETACHED: &str = "detached";
    /// The entry came back (an Undo or Redo); so did the Mutant.
    pub const REATTACHED: &str = "reattached";
    /// A new Mutant: a replacement wrote a new entry for the parent, and
    /// the Mutant of its old entry was carried to it.
    pub const CARRIED: &str = "carried";
    /// A new Mutant: its playlist is a copy, and this is the copy of the
    /// source entry's Mutant.
    pub const COPIED: &str = "copied";
}

/// A replacement of one playlist's entries: each Mutant of an entry in
/// `from` moves to the entry of the same track in `to`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Carry {
    pub playlist: String,
    /// The entries replaced.
    pub from: Vec<String>,
    /// The entries written in their place, with their tracks, in order.
    pub to: Vec<(String, String)>,
}

/// A copy of one entry: its live Mutant is copied to `to`, an entry of
/// `playlist` (the same track).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Copy {
    pub from: String,
    pub playlist: String,
    pub to: String,
}

/// What a Library change did to playlist entries, as far as Mutants care.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Follow {
    /// Entries that left the Library's playlists: their live Mutants are
    /// detached.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub left: Vec<String>,
    /// Entries that came back: the Mutants detached from them are
    /// reattached.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub returned: Vec<String>,
    /// The playlists the returning entries are in, where their detached
    /// Mutants are looked for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub returned_in: Vec<String>,
    /// Replacements whose Mutants move to the new entries. The replaced
    /// entries belong in `left` too.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carried: Vec<Carry>,
    /// Entries copied to another playlist.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub copied: Vec<Copy>,
}

impl Follow {
    /// Nothing to follow.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.left.is_empty()
            && self.returned.is_empty()
            && self.carried.is_empty()
            && self.copied.is_empty()
    }
}

/// One Mutant change a [`Follow`] made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Followed {
    pub mutant_id: i64,
    pub playlist_id: String,
    pub entry_id: String,
    pub track_id: String,
    /// One of [`actions`].
    pub action: String,
}

/// A Mutant as a Follow needs it.
struct Held {
    id: i64,
    playlist: String,
    entry: String,
    track: String,
    fingerprint: Option<String>,
    edits: String,
}

/// Records `follow`'s consequences inside `tx`, as changes by `source` in
/// `run` at `at`. Answers what changed, in the order it changed.
pub(crate) fn apply_in(
    tx: &Transaction,
    follow: &Follow,
    source: &Source,
    run: RunId,
    at: i64,
) -> Result<Vec<Followed>> {
    let mut done = Vec::new();
    // What the carried and copied Mutants start from, read before any of
    // them is detached.
    let mut carried: Vec<(Held, String, String)> = Vec::new();
    for carry in &follow.carried {
        let mut used: HashSet<&str> = HashSet::new();
        for from in &carry.from {
            let Some(held) = live_of(tx, from)? else {
                continue;
            };
            let to = carry
                .to
                .iter()
                .find(|(entry, track)| *track == held.track && !used.contains(entry.as_str()));
            if let Some((entry, _)) = to {
                used.insert(entry);
                carried.push((held, carry.playlist.clone(), entry.clone()));
            }
        }
    }
    let mut copied: Vec<(Held, String, String)> = Vec::new();
    for copy in &follow.copied {
        if let Some(held) = live_of(tx, &copy.from)? {
            copied.push((held, copy.playlist.clone(), copy.to.clone()));
        }
    }

    for entry in &follow.left {
        let Some(held) = live_of(tx, entry)? else {
            continue;
        };
        change(tx, &held, actions::DETACHED, source, run, at)?;
        tx.execute(
            "UPDATE mutant SET removed_at = ?2, removed_run = ?3 WHERE id = ?1",
            params![held.id, at, run.0],
        )?;
        done.push(followed(&held, actions::DETACHED));
    }

    // The Mutants detached from the returning entries, newest first, read
    // a playlist at a time through its index.
    let wanted: HashSet<&str> = follow.returned.iter().map(String::as_str).collect();
    let playlists: HashSet<&str> = if wanted.is_empty() {
        HashSet::new()
    } else {
        follow.returned_in.iter().map(String::as_str).collect()
    };
    for playlist in playlists {
        let mut seen: HashSet<String> = HashSet::new();
        for (held, latest) in in_playlist(tx, playlist)? {
            if !wanted.contains(held.entry.as_str()) || !seen.insert(held.entry.clone()) {
                // Only the newest Mutant of an entry can come back.
                continue;
            }
            if latest != actions::DETACHED || live_of(tx, &held.entry)?.is_some() {
                continue;
            }
            change(tx, &held, actions::REATTACHED, source, run, at)?;
            tx.execute(
                "UPDATE mutant SET removed_at = NULL, removed_run = NULL WHERE id = ?1",
                [held.id],
            )?;
            done.push(followed(&held, actions::REATTACHED));
        }
    }

    for (action, list) in [(actions::CARRIED, carried), (actions::COPIED, copied)] {
        for (from, playlist, entry) in list {
            if live_of(tx, &entry)?.is_some() {
                continue;
            }
            tx.execute(
                "INSERT INTO mutant (playlist_id, entry_id, track_id, fingerprint, created_at, created_run)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![playlist, entry, from.track, from.fingerprint, at, run.0],
            )?;
            let made = Held {
                id: tx.last_insert_rowid(),
                playlist,
                entry,
                track: from.track,
                fingerprint: from.fingerprint,
                edits: from.edits,
            };
            change(tx, &made, action, source, run, at)?;
            done.push(followed(&made, action));
        }
    }
    Ok(done)
}

fn followed(held: &Held, action: &str) -> Followed {
    Followed {
        mutant_id: held.id,
        playlist_id: held.playlist.clone(),
        entry_id: held.entry.clone(),
        track_id: held.track.clone(),
        action: action.to_owned(),
    }
}

const HELD: &str = "SELECT mutant.id, mutant.playlist_id, mutant.entry_id, mutant.track_id,
        mutant.fingerprint,
        (SELECT edits FROM mutant_change WHERE mutant_id = mutant.id ORDER BY id DESC LIMIT 1),
        (SELECT action FROM mutant_change WHERE mutant_id = mutant.id ORDER BY id DESC LIMIT 1)
     FROM mutant";

fn held(row: &rusqlite::Row) -> rusqlite::Result<(Held, String)> {
    Ok((
        Held {
            id: row.get(0)?,
            playlist: row.get(1)?,
            entry: row.get(2)?,
            track: row.get(3)?,
            fingerprint: row.get(4)?,
            edits: row
                .get::<_, Option<String>>(5)?
                .unwrap_or_else(|| "{}".to_owned()),
        },
        row.get::<_, Option<String>>(6)?.unwrap_or_default(),
    ))
}

/// The entry's live Mutant (through the live-entry index).
fn live_of(tx: &Transaction, entry: &str) -> Result<Option<Held>> {
    Ok(tx
        .query_row(
            &format!("{HELD} WHERE entry_id = ?1 AND removed_at IS NULL"),
            [entry],
            held,
        )
        .optional()?
        .map(|(held, _)| held))
}

/// A playlist's Mutants, newest first, with each one's newest action.
fn in_playlist(tx: &Transaction, playlist: &str) -> Result<Vec<(Held, String)>> {
    let mut statement = tx.prepare(&format!(
        "{HELD} WHERE playlist_id = ?1 ORDER BY mutant.id DESC"
    ))?;
    let rows = statement
        .query_map([playlist], held)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn change(
    tx: &Transaction,
    held: &Held,
    action: &str,
    source: &Source,
    run: RunId,
    at: i64,
) -> Result<()> {
    tx.execute(
        "INSERT INTO mutant_change
            (mutant_id, action, edits, fingerprint, source_kind, source_name, run_id, recorded_at)
         VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7)",
        params![
            held.id,
            action,
            held.edits,
            source.kind.as_str(),
            source.name,
            run.0,
            at
        ],
    )?;
    Ok(())
}

/// One Mutant's entry, and whether a playlist edit could still move it:
/// live, or detached by one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub mutant_id: i64,
    pub playlist_id: String,
    pub entry_id: String,
    pub track_id: String,
    /// Live (not removed); otherwise its newest change is `detached`.
    pub live: bool,
}

impl crate::Sidecar {
    /// Records a Library change's Ledger entry and its Mutant consequences
    /// in one transaction; see the module's notes. Answers the entry's
    /// identifier and the Mutant changes made (none when the entry was
    /// already there: a replay changes nothing twice).
    ///
    /// # Errors
    /// The entry could not be written; nothing is kept.
    pub fn append_following(
        &self,
        entry: &crate::ledger::NewEntry,
        facts: &crate::dj_facts::NativeFacts,
        reconciled: bool,
        follow: &Follow,
    ) -> Result<(i64, Vec<Followed>)> {
        self.append_in_one(entry, facts, reconciled, follow)
    }

    /// Every Mutant a playlist edit could still move (live, or detached by
    /// one), with its entry: for bringing Mutants back into line with the
    /// Library after a change whose entries are not known (a write that
    /// was never confirmed, a track removed from the collection). Reads the
    /// whole Mutant table once; never on a per-entry path.
    ///
    /// # Errors
    /// The store could not be read.
    pub fn mutant_attachments(&self) -> Result<Vec<Attachment>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT mutant.id, mutant.playlist_id, mutant.entry_id, mutant.track_id,
                    mutant.removed_at IS NULL,
                    (SELECT action FROM mutant_change WHERE mutant_id = mutant.id
                     ORDER BY id DESC LIMIT 1)
             FROM mutant ORDER BY mutant.id DESC",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    Attachment {
                        mutant_id: row.get(0)?,
                        playlist_id: row.get(1)?,
                        entry_id: row.get(2)?,
                        track_id: row.get(3)?,
                        live: row.get(4)?,
                    },
                    row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut seen = HashSet::new();
        Ok(rows
            .into_iter()
            // Only the newest Mutant of an entry counts.
            .filter(|(attachment, _)| seen.insert(attachment.entry_id.clone()))
            .filter(|(attachment, action)| attachment.live || action == actions::DETACHED)
            .map(|(attachment, _)| attachment)
            .collect())
    }
}
