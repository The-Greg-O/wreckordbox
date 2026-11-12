//! Audio identities and re-attachment, separate from immutable Fact provenance.
//! The caller supplies a current Library snapshot; this store never opens it.

use rusqlite::{params, Transaction};
use serde::{Deserialize, Serialize};

use crate::{store::now, Fingerprint, Result, RunId, Sidecar};

pub(crate) const SCHEMA: &str = "
CREATE TABLE track_identity (
    track_id TEXT PRIMARY KEY,
    fingerprint TEXT,
    title TEXT NOT NULL,
    artist TEXT NOT NULL,
    mix_name TEXT NOT NULL,
    duration INTEGER NOT NULL
) STRICT;
CREATE INDEX identity_audio ON track_identity(fingerprint);
CREATE INDEX identity_fields ON track_identity(title, artist, mix_name, duration);
INSERT INTO track_identity
    SELECT f.track_id, CASE WHEN NOT EXISTS (
        SELECT 1 FROM fact old WHERE old.track_id = f.track_id
        AND old.stale_at >= f.recorded_at
    ) THEN f.fingerprint ELSE NULL END, '', '', '', 0
    FROM fact f WHERE f.id = (
        SELECT max(latest.id) FROM fact latest
        WHERE latest.track_id = f.track_id AND latest.fingerprint IS NOT NULL
    );
CREATE TABLE attachment_change (
    id INTEGER PRIMARY KEY,
    from_track TEXT NOT NULL,
    to_track TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    attached_at INTEGER NOT NULL,
    run_id INTEGER REFERENCES run(id)
) STRICT;
CREATE TABLE fact_attachment (
    fact_id INTEGER NOT NULL REFERENCES fact(id),
    change_id INTEGER NOT NULL REFERENCES attachment_change(id),
    PRIMARY KEY(fact_id, change_id)
) STRICT;
CREATE TABLE fact_track (
    fact_id INTEGER PRIMARY KEY REFERENCES fact(id),
    track_id TEXT NOT NULL
) STRICT;
CREATE INDEX fact_track_by_track ON fact_track(track_id);
INSERT INTO fact_track SELECT id, track_id FROM fact;
CREATE TRIGGER fact_initial_track AFTER INSERT ON fact BEGIN
    INSERT INTO fact_track VALUES (NEW.id, NEW.track_id);
END;
CREATE VIEW attached_fact AS
    SELECT fact.*, fact_track.track_id AS effective_track_id
    FROM fact_track JOIN fact ON fact.id = fact_track.fact_id;
CREATE TABLE attachment_review (
    track_id TEXT NOT NULL,
    from_track TEXT NOT NULL,
    reason TEXT NOT NULL,
    PRIMARY KEY(track_id, from_track)
) STRICT;
";

/// Identifying fields are evidence for review only. Mix names are never stripped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentifyingFields {
    pub title: String,
    pub artist: String,
    pub mix_name: String,
    pub duration: u32,
}

impl IdentifyingFields {
    fn matches(&self, other: &Self) -> bool {
        let title = normalize(&self.title);
        let artist = normalize(&self.artist);
        !title.is_empty()
            && !artist.is_empty()
            && title == normalize(&other.title)
            && artist == normalize(&other.artist)
            && normalize(&self.mix_name) == normalize(&other.mix_name)
            && self.duration > 0
            && other.duration > 0
            && self.duration.abs_diff(other.duration) <= 2
    }
}

fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// An automatic attachment made by this observation. Fact IDs are unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub from_track: String,
    pub to_track: String,
    pub fact_ids: Vec<i64>,
}

/// A possible attachment, deliberately not applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    pub from_track: String,
    /// `identifying_fields`, `ambiguous_fingerprint` or `target_has_facts`.
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachments {
    pub reattached: Vec<Attachment>,
    pub review: Vec<Review>,
}

struct Identity {
    track: String,
    fingerprint: Option<String>,
    fields: IdentifyingFields,
}

impl Sidecar {
    /// Remember the latest observed identity, and attach Facts only from a
    /// removed track with one unambiguous fingerprint match. Missing audio
    /// preserves the last known fingerprint; it is not evidence of a swap.
    /// All attachment rows and review candidates are committed together.
    /// `present` must read an already captured Library snapshot, not take a lock.
    pub fn observe_audio(
        &self,
        track: &str,
        fingerprint: Option<&Fingerprint>,
        fields: &IdentifyingFields,
        present: impl Fn(&str) -> bool,
    ) -> Result<Attachments> {
        self.observe_audio_in_run(track, fingerprint, fields, present, None)
    }

    /// Observe before a producer writes, attributing attachments to its Run.
    pub fn observe_audio_in_run(
        &self,
        track: &str,
        fingerprint: Option<&Fingerprint>,
        fields: &IdentifyingFields,
        present: impl Fn(&str) -> bool,
        run: Option<RunId>,
    ) -> Result<Attachments> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let occupied: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM attached_fact WHERE effective_track_id = ?1)",
            [track],
            |row| row.get(0),
        )?;
        let mut result = Attachments::default();
        {
            let candidates = identities(&tx, fingerprint, fields)?;
            let removed: Vec<_> = candidates
                .iter()
                .filter(|candidate| candidate.track != track && !present(&candidate.track))
                .collect();
            let exact: Vec<_> = removed
                .iter()
                .copied()
                .filter(|candidate| {
                    fingerprint.is_some()
                        && candidate.fingerprint.as_deref() == fingerprint.map(Fingerprint::as_str)
                })
                .collect();
            if exact.len() == 1 && !occupied {
                let matched = exact[0];
                result
                    .reattached
                    .push(attach(&tx, &matched.track, track, fingerprint, run)?);
            } else if exact.is_empty() {
                result.review = removed
                    .iter()
                    .filter(|candidate| fields.matches(&candidate.fields))
                    .map(|candidate| Review {
                        from_track: candidate.track.clone(),
                        reason: "identifying_fields".to_owned(),
                    })
                    .collect();
            } else {
                result.review = exact
                    .iter()
                    .map(|candidate| Review {
                        from_track: candidate.track.clone(),
                        reason: if occupied {
                            "target_has_facts"
                        } else {
                            "ambiguous_fingerprint"
                        }
                        .to_owned(),
                    })
                    .collect();
            }
        }
        tx.execute("DELETE FROM attachment_review WHERE track_id = ?1", [track])?;
        for review in &result.review {
            tx.execute(
                "INSERT INTO attachment_review VALUES (?1, ?2, ?3)",
                params![track, review.from_track, review.reason],
            )?;
        }
        tx.execute(
            "INSERT INTO track_identity VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(track_id) DO UPDATE SET
                fingerprint = COALESCE(excluded.fingerprint, track_identity.fingerprint),
                title = excluded.title, artist = excluded.artist,
                mix_name = excluded.mix_name, duration = excluded.duration",
            params![
                track,
                fingerprint.map(Fingerprint::as_str),
                normalize(&fields.title),
                normalize(&fields.artist),
                normalize(&fields.mix_name),
                fields.duration
            ],
        )?;
        tx.commit()?;
        Ok(result)
    }
    /// Stored candidates for this track. No observation or mutation.
    pub fn attachment_reviews(&self, track: &str) -> Result<Vec<Review>> {
        let db = self.db();
        let mut query = db.prepare("SELECT from_track, reason FROM attachment_review WHERE track_id = ?1 ORDER BY from_track")?;
        let reviews = query
            .query_map([track], |row| {
                Ok(Review {
                    from_track: row.get(0)?,
                    reason: row.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(reviews)
    }
}

fn identities(
    tx: &Transaction<'_>,
    fingerprint: Option<&Fingerprint>,
    fields: &IdentifyingFields,
) -> Result<Vec<Identity>> {
    let mut statement = tx.prepare(
        "SELECT track_id, fingerprint, title, artist, mix_name, duration FROM track_identity
         WHERE (fingerprint = ?1 OR
             (?2 <> '' AND ?3 <> '' AND duration > 0 AND title = ?2 AND artist = ?3 AND mix_name = ?4 AND duration BETWEEN ?5 AND ?6))
         AND EXISTS (SELECT 1 FROM fact_track WHERE fact_track.track_id = track_identity.track_id)
         ORDER BY track_id",
    )?;
    let identities = statement
        .query_map(
            params![
                fingerprint.map(Fingerprint::as_str),
                normalize(&fields.title),
                normalize(&fields.artist),
                normalize(&fields.mix_name),
                fields.duration.saturating_sub(2),
                fields.duration.saturating_add(2)
            ],
            |row| {
                Ok(Identity {
                    track: row.get(0)?,
                    fingerprint: row.get(1)?,
                    fields: IdentifyingFields {
                        title: row.get(2)?,
                        artist: row.get(3)?,
                        mix_name: row.get(4)?,
                        duration: row.get(5)?,
                    },
                })
            },
        )?
        .collect::<rusqlite::Result<_>>()?;
    Ok(identities)
}

fn attach(
    tx: &Transaction<'_>,
    from: &str,
    to: &str,
    fingerprint: Option<&Fingerprint>,
    run: Option<RunId>,
) -> Result<Attachment> {
    let mut statement =
        tx.prepare("SELECT id FROM attached_fact WHERE effective_track_id = ?1 ORDER BY id")?;
    let fact_ids = statement
        .query_map([from], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    tx.execute("INSERT INTO attachment_change (from_track, to_track, fingerprint, attached_at, run_id) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![from, to, fingerprint.map(Fingerprint::as_str), now(), run.map(|run| run.0)])?;
    let change = tx.last_insert_rowid();
    for id in &fact_ids {
        tx.execute(
            "INSERT INTO fact_attachment VALUES (?1, ?2)",
            params![id, change],
        )?;
    }
    tx.execute(
        "UPDATE fact_track SET track_id = ?2 WHERE track_id = ?1",
        params![from, to],
    )?;
    Ok(Attachment {
        from_track: from.to_owned(),
        to_track: to.to_owned(),
        fact_ids,
    })
}
