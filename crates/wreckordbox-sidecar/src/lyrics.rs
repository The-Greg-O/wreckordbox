//! Lyrics: the words of a track, kept as Facts of the reserved kind
//! [`LYRICS`] so an outside agent can read them and match tracks on their
//! themes (bead wbx-f6v).
//!
//! A Lyrics Fact is a **Judgement**: the words belong to the song, not to
//! one encoding of it, so they follow the music through a re-encode, a
//! retag or a re-add (the fingerprint is recorded alongside and re-attaches
//! them), and are never marked stale. Its value is a [`Lyrics`]: the text
//! (or an explicit "instrumental", so nobody keeps searching), whether it
//! is synced (with its timed lines), the language when known, and where it
//! came from: the audio file's own tags, or supplied by someone with the
//! web address and the date it was taken from.
//!
//! Being Facts, Lyrics get the Ledger, Runs, sources, supersession (a
//! source recording again replaces its own earlier record) and backups
//! with no machinery of their own. The one addition is the full-text index
//! [`SCHEMA`] (schema version 10): an SQLite FTS5 table kept by triggers on
//! `fact`, so it is written in the same transaction as the Fact itself and
//! can never disagree with it. Search reads only live records.
//!
//! Lyrics are never written into the Library's own fields and never to a
//! Stick: nothing outside the Sidecar reads this kind.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::store::{read_fact, select};
use crate::{Fact, FactsReader, Kind, Result, Sidecar};

/// The kind: a Judgement, recorded only by the lyrics commands.
pub const LYRICS: Kind<'static> = Kind::judgement("lyrics");

/// The search index: one row per Lyrics Fact that has text, keyed by the
/// Fact's identifier. Kept by triggers, so a Fact and its index row are
/// written together or not at all. Facts are never deleted; the delete
/// trigger is there so a future clean-up cannot leave a dangling row.
pub(crate) const SCHEMA: &str = "
CREATE VIRTUAL TABLE lyrics_search USING fts5(
    text,
    tokenize = 'unicode61 remove_diacritics 2'
);
CREATE TRIGGER lyrics_search_add AFTER INSERT ON fact
    WHEN NEW.kind = 'lyrics' AND json_extract(NEW.value, '$.text') IS NOT NULL
BEGIN
    INSERT INTO lyrics_search (rowid, text) VALUES (NEW.id, json_extract(NEW.value, '$.text'));
END;
CREATE TRIGGER lyrics_search_drop AFTER DELETE ON fact
    WHEN OLD.kind = 'lyrics'
BEGIN
    DELETE FROM lyrics_search WHERE rowid = OLD.id;
END;
INSERT INTO lyrics_search (rowid, text)
    SELECT id, json_extract(value, '$.text') FROM fact
    WHERE kind = 'lyrics' AND json_extract(value, '$.text') IS NOT NULL;
";

/// How search is done, as the commands report it.
pub const SEARCH_ENGINE: &str = "sqlite-fts5";

/// Where a track's lyrics came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Read from the audio file's own tags.
    Embedded,
    /// Handed in by someone: an agent, the DJ, from a web page.
    Supplied,
}

/// One timed line of synced lyrics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncedLine {
    /// From the start of the track, in milliseconds.
    pub at_ms: u32,
    pub text: String,
}

/// The value of a Lyrics Fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lyrics {
    /// The track has no words: an explicit "no lyrics", so nobody keeps
    /// looking. Such a record has no text.
    pub instrumental: bool,
    /// The words, lines separated by newlines. For synced lyrics, the
    /// lines' texts in order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Whether the words carry times.
    #[serde(default)]
    pub synced: bool,
    /// The timed lines, for synced lyrics.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<SyncedLine>,
    /// The language, as given (an ISO 639 code from a tag), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub origin: Origin,
    /// For embedded lyrics: the tag they were read from, such as
    /// `ID3v2 USLT` or `Vorbis LYRICS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// For supplied lyrics: the web address they were taken from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// The date (`YYYY-MM-DD`) they were taken from `address`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrieved: Option<String>,
}

impl Lyrics {
    /// Whether the record holds together; the reason in words if not.
    ///
    /// # Errors
    ///
    /// A record that is instrumental and has words, has no words and is
    /// not instrumental, is synced without lines (or the reverse), or has
    /// an address without a date, a date without an address, an address
    /// with spaces or a date not written `YYYY-MM-DD`.
    pub fn check(&self) -> std::result::Result<(), String> {
        let has_text = self.text.as_deref().is_some_and(|t| !t.trim().is_empty());
        if self.instrumental && (has_text || !self.lines.is_empty()) {
            return Err("an instrumental record has no words".to_owned());
        }
        if !self.instrumental && !has_text {
            return Err(
                "lyrics need words; for a track with none, record it as instrumental".to_owned(),
            );
        }
        if self.synced != !self.lines.is_empty() {
            return Err("synced lyrics are exactly those with timed lines".to_owned());
        }
        if let Some(language) = &self.language {
            let ok = (2..=8).contains(&language.len())
                && language.chars().all(|c| c.is_ascii_alphabetic() || c == '-');
            if !ok {
                return Err(format!(
                    "a language is a code such as eng or en-GB, not {language:?}"
                ));
            }
        }
        match (&self.address, &self.retrieved) {
            (Some(address), Some(date)) => {
                if address.trim().is_empty() || address.chars().any(char::is_whitespace) {
                    return Err(format!(
                        "a web address has no spaces and is not empty: {address:?}"
                    ));
                }
                if !is_date(date) {
                    return Err(format!(
                        "the date lyrics were taken is written YYYY-MM-DD, not {date:?}"
                    ));
                }
            }
            (None, None) => {}
            _ => {
                return Err(
                    "lyrics from the web need both the address and the date they were taken"
                        .to_owned(),
                )
            }
        }
        Ok(())
    }
}

fn is_date(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(at, b)| at == 4 || at == 7 || b.is_ascii_digit())
        && (1..=12).contains(&text[5..7].parse::<u8>().unwrap_or(0))
        && (1..=31).contains(&text[8..10].parse::<u8>().unwrap_or(0))
}

/// One search hit: a live Lyrics Fact whose words match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    pub fact: Fact,
    /// The matching words in context, the matches in `[` `]`.
    pub snippet: String,
    /// FTS5's bm25 rank: lower is a better match.
    pub rank: f64,
}

/// The query FTS5 is given for `text`: each word as a quoted string, all
/// of which must appear, so punctuation in what an agent typed is never
/// read as query syntax. `None` when `text` has no words.
#[must_use]
pub fn query_of(text: &str) -> Option<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .map(|word| word.trim_matches('\''))
        .filter(|word| !word.is_empty())
        .map(|word| format!("\"{}\"", word.replace('"', "\"\"")))
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

fn live_in(db: &Connection) -> Result<Vec<Fact>> {
    let mut statement = db.prepare_cached(&select(
        "WHERE fact.kind = 'lyrics' AND fact.superseded_at IS NULL ORDER BY fact.id DESC",
    ))?;
    let facts = statement
        .query_map([], read_fact)?
        .collect::<rusqlite::Result<Vec<Fact>>>()?;
    Ok(facts)
}

fn search_in(db: &Connection, text: &str, limit: usize) -> Result<Vec<Hit>> {
    let Some(query) = query_of(text) else {
        return Ok(Vec::new());
    };
    let mut statement = db.prepare_cached(
        "SELECT lyrics_search.rowid, snippet(lyrics_search, 0, '[', ']', '…', 12), bm25(lyrics_search)
         FROM lyrics_search JOIN fact ON fact.id = lyrics_search.rowid
         WHERE lyrics_search MATCH ?1 AND fact.superseded_at IS NULL
         ORDER BY bm25(lyrics_search), lyrics_search.rowid DESC LIMIT ?2",
    )?;
    let found = statement
        .query_map(
            params![query, i64::try_from(limit).unwrap_or(i64::MAX)],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, f64>(2)?,
                ))
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut fact = db.prepare_cached(&select("WHERE fact.id = ?1"))?;
    let mut hits = Vec::with_capacity(found.len());
    for (id, snippet, rank) in found {
        hits.push(Hit {
            fact: fact.query_row([id], read_fact)?,
            snippet,
            rank,
        });
    }
    Ok(hits)
}

impl Sidecar {
    /// Every live Lyrics Fact of every track, newest first.
    ///
    /// # Errors
    ///
    /// The database fails.
    pub fn lyrics_records(&self) -> Result<Vec<Fact>> {
        live_in(&self.db())
    }

    /// Live Lyrics Facts whose words contain every word of `text`, best
    /// match first, at most `limit`.
    ///
    /// # Errors
    ///
    /// The database fails.
    pub fn search_lyrics(&self, text: &str, limit: usize) -> Result<Vec<Hit>> {
        search_in(&self.db(), text, limit)
    }
}

impl FactsReader {
    /// As [`Sidecar::lyrics_records`], on the read-only snapshot.
    ///
    /// # Errors
    ///
    /// The database fails.
    pub fn lyrics_records(&self) -> Result<Vec<Fact>> {
        live_in(self.connection())
    }

    /// As [`Sidecar::search_lyrics`], on the read-only snapshot.
    ///
    /// # Errors
    ///
    /// The database fails.
    pub fn search_lyrics(&self, text: &str, limit: usize) -> Result<Vec<Hit>> {
        search_in(self.connection(), text, limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Lyrics {
        Lyrics {
            instrumental: false,
            text: Some(text.to_owned()),
            synced: false,
            lines: Vec::new(),
            language: None,
            origin: Origin::Supplied,
            tag: None,
            address: None,
            retrieved: None,
        }
    }

    #[test]
    fn a_query_quotes_every_word_so_punctuation_is_never_syntax() {
        assert_eq!(
            query_of("neon, \"tide\" OR -harbour*").as_deref(),
            Some("\"neon\" \"tide\" \"OR\" \"harbour\"")
        );
        assert_eq!(query_of(" ,.; "), None);
    }

    #[test]
    fn a_record_is_words_or_instrumental_never_both_or_neither() {
        assert!(words("glass rivers at dawn").check().is_ok());
        let mut both = words("glass");
        both.instrumental = true;
        assert!(both.check().is_err());
        let mut neither = words("  ");
        neither.text = Some("  ".to_owned());
        assert!(neither.check().is_err());
        let none = Lyrics {
            instrumental: true,
            text: None,
            ..words("")
        };
        assert!(none.check().is_ok());
    }

    #[test]
    fn web_lyrics_need_their_address_and_a_real_date() {
        let mut web = words("glass rivers");
        web.address = Some("https://example.test/lyrics/1".to_owned());
        assert!(web.check().is_err(), "no date");
        web.retrieved = Some("2026-11-15".to_owned());
        assert!(web.check().is_ok());
        web.retrieved = Some("2026-13-01".to_owned());
        assert!(web.check().is_err());
        web.retrieved = Some("2026-11-15".to_owned());
        web.address = Some("https://example.test/a b".to_owned());
        assert!(web.check().is_err());
    }
}
