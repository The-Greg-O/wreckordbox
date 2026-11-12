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
//! which entries ([`Follow`]), with every new Mutant spelled out whole
//! ([`Made`]: its parent, edits and fingerprint as they were when the
//! Library change was made), and this module records the consequences
//! inside the transaction that appends the change's Ledger entry
//! ([`Sidecar::append_following`]): the entry and its Mutant consequences
//! are kept together or not at all. A Follow that cannot be applied fails
//! the whole append, so the Ledger keeps the entry, Follow and all, and
//! writes both later (its spill file), exactly as it does any entry it
//! could not write.
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
//!
//! A write whose entries are not known in advance and would lose Mutants
//! if it were never confirmed (a replacement, a copy) leaves a plan with
//! its Ledger intent before the Library is changed
//! ([`Sidecar::capture_mutant_plan`]); recovery reads it back with the
//! intent ([`crate::ledger::OpenIntent::mutant_plan`]).

use std::collections::HashSet;

use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

/// A live Mutant as it stands, whole: what a plan keeps and a [`Made`] is
/// made from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub mutant_id: i64,
    pub playlist_id: String,
    pub entry_id: String,
    pub track_id: String,
    /// Its edits as they stand (the newest change's).
    pub edits: Value,
    pub fingerprint: Option<String>,
}

/// A new Mutant a Follow makes: carried to a replacement's new entry, or
/// copied to a copy's entry, from `from` as it was then.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Made {
    /// [`actions::CARRIED`] or [`actions::COPIED`].
    pub action: String,
    /// The Mutant it is made from.
    pub from: i64,
    pub playlist_id: String,
    pub entry_id: String,
    pub track_id: String,
    pub edits: Value,
    pub fingerprint: Option<String>,
}

impl Made {
    /// `snapshot` made again on `entry` of `playlist`.
    #[must_use]
    pub fn from_snapshot(action: &str, snapshot: &Snapshot, playlist: &str, entry: &str) -> Self {
        Self {
            action: action.to_owned(),
            from: snapshot.mutant_id,
            playlist_id: playlist.to_owned(),
            entry_id: entry.to_owned(),
            track_id: snapshot.track_id.clone(),
            edits: snapshot.edits.clone(),
            fingerprint: snapshot.fingerprint.clone(),
        }
    }
}

/// What a Library change did to playlist entries, as far as Mutants care.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
    /// New Mutants, carried or copied, spelled out whole.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub made: Vec<Made>,
}

impl Follow {
    /// Nothing to follow.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.left.is_empty() && self.returned.is_empty() && self.made.is_empty()
    }

    /// Takes `other`'s consequences too.
    pub fn extend(&mut self, other: Self) {
        self.left.extend(other.left);
        self.returned.extend(other.returned);
        self.returned_in.extend(other.returned_in);
        self.made.extend(other.made);
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

    for made in &follow.made {
        // Made once: a recovery that runs again finds it there.
        if live_of(tx, &made.entry_id)?.is_some() {
            continue;
        }
        let edits = stored_text(&made.edits);
        tx.execute(
            "INSERT INTO mutant (playlist_id, entry_id, track_id, fingerprint, created_at, created_run)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![made.playlist_id, made.entry_id, made.track_id, made.fingerprint, at, run.0],
        )?;
        let held = Held {
            id: tx.last_insert_rowid(),
            playlist: made.playlist_id.clone(),
            entry: made.entry_id.clone(),
            track: made.track_id.clone(),
            edits,
        };
        change(tx, &held, &made.action, source, run, at)?;
        done.push(followed(&held, &made.action));
    }
    Ok(done)
}

/// Edits as `mutant_change.edits` holds them: a damaged value read as text
/// is written back as the same text.
fn stored_text(edits: &Value) -> String {
    match edits {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
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
            edits: row
                .get::<_, Option<String>>(5)?
                .unwrap_or_else(|| "{}".to_owned()),
        },
        row.get::<_, Option<String>>(6)?.unwrap_or_default(),
    ))
}

fn snapshot(row: &rusqlite::Row) -> rusqlite::Result<Snapshot> {
    let edits = row
        .get::<_, Option<String>>(5)?
        .unwrap_or_else(|| "{}".to_owned());
    Ok(Snapshot {
        mutant_id: row.get(0)?,
        playlist_id: row.get(1)?,
        entry_id: row.get(2)?,
        track_id: row.get(3)?,
        fingerprint: row.get(4)?,
        // Kept as stored, even if damaged: a copy repeats what is there.
        edits: serde_json::from_str(&edits).unwrap_or(Value::String(edits)),
    })
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
    let mut statement =
        tx.prepare(&format!("{HELD} WHERE playlist_id = ?1 ORDER BY mutant.id DESC"))?;
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

/// The table a plan is kept in, beside its Ledger intent; removed with it.
pub(crate) const SCHEMA: &str = "
CREATE TABLE mutant_intent (
    id   INTEGER PRIMARY KEY REFERENCES ledger_intent (id) ON DELETE CASCADE,
    plan TEXT NOT NULL
) STRICT;
";

impl crate::Sidecar {
    /// Records a Library change's Ledger entry and its Mutant consequences
    /// in one transaction; see the module's notes. Answers the entry's
    /// identifier and the Mutant changes made (none when the entry was
    /// already there: a replay changes nothing twice).
    ///
    /// # Errors
    /// The entry or its consequences could not be written; nothing is kept.
    pub fn append_following(
        &self,
        entry: &crate::ledger::NewEntry,
        facts: &crate::dj_facts::NativeFacts,
        reconciled: bool,
        follow: &Follow,
    ) -> Result<(i64, Vec<Followed>)> {
        self.append_in_one(entry, facts, reconciled, follow)
    }

    /// Keeps `plan` with the intent `id` until the intent is settled.
    ///
    /// # Errors
    /// The plan could not be written.
    pub fn capture_mutant_plan(&self, id: i64, plan: &Value) -> Result<()> {
        self.db().execute(
            "INSERT OR REPLACE INTO mutant_intent (id, plan) VALUES (?1, ?2)",
            params![id, plan.to_string()],
        )?;
        Ok(())
    }

    /// The live Mutants of these playlists, whole (through the playlist
    /// index).
    ///
    /// # Errors
    /// The store could not be read.
    pub fn mutant_snapshots_in(&self, playlists: &[String]) -> Result<Vec<Snapshot>> {
        let db = self.db();
        let mut statement = db.prepare(&format!(
            "{HELD} WHERE playlist_id = ?1 AND removed_at IS NULL ORDER BY mutant.id"
        ))?;
        let mut found = Vec::new();
        for playlist in playlists {
            for row in statement.query_map([playlist], snapshot)? {
                found.push(row?);
            }
        }
        Ok(found)
    }

    /// The live Mutants of these entries, whole (through the live-entry
    /// index).
    ///
    /// # Errors
    /// The store could not be read.
    pub fn mutant_snapshots_of(&self, entries: &[String]) -> Result<Vec<Snapshot>> {
        let db = self.db();
        let mut statement = db.prepare(&format!(
            "{HELD} WHERE entry_id = ?1 AND removed_at IS NULL"
        ))?;
        let mut found = Vec::new();
        for entry in entries {
            if let Some(row) = statement.query_row([entry], snapshot).optional()? {
                found.push(row);
            }
        }
        Ok(found)
    }

    /// The playlists that have a live Mutant.
    ///
    /// # Errors
    /// The store could not be read.
    pub fn mutated_playlists(&self) -> Result<Vec<String>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT DISTINCT playlist_id FROM mutant WHERE removed_at IS NULL ORDER BY playlist_id",
        )?;
        let found = statement
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(found)
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
