//! Lyrics in the Sidecar (wbx-f6v): Facts of the reserved kind `lyrics`,
//! searchable through an FTS5 index kept with the Fact itself, and a store
//! from before the index brought up to date with its lyrics searchable.
//! The lyrics are synthetic, written for these tests.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use rusqlite::Connection;
use serde_json::json;
use wreckordbox_sidecar::lyrics::LYRICS;
use wreckordbox_sidecar::{NewFact, Sidecar, Source, SourceKind, SCHEMA_VERSION};

fn agent(name: &str) -> Source {
    Source::new(SourceKind::Agent, name)
}

fn record(sidecar: &Sidecar, track: &str, source: &Source, text: &str) -> i64 {
    let run = sidecar.begin_run(source, "Lyrics").unwrap();
    sidecar
        .record(&NewFact {
            track_id: track,
            fingerprint: None,
            kind: LYRICS,
            value: json!({"instrumental": false, "text": text, "origin": "supplied"}),
            source,
            confidence: 1.0,
            run,
        })
        .unwrap()
        .id
}

fn hits(sidecar: &Sidecar, text: &str) -> Vec<(String, i64)> {
    sidecar
        .search_lyrics(text, 10)
        .unwrap()
        .into_iter()
        .map(|hit| (hit.fact.track_id, hit.fact.id))
        .collect()
}

#[test]
fn search_finds_every_word_in_live_lyrics_only_and_never_the_replaced_words() {
    let dir = tempfile::tempdir().unwrap();
    let sidecar = Sidecar::open(&dir.path().join("sidecar")).unwrap();
    let first = record(
        &sidecar,
        "10001",
        &agent("a"),
        "Paper lanterns over the harbour\nwe sail at dawn",
    );
    let other = record(
        &sidecar,
        "10002",
        &agent("a"),
        "Copper wires hum in the city",
    );
    assert_eq!(
        hits(&sidecar, "harbour lanterns"),
        [("10001".to_owned(), first)]
    );
    assert_eq!(hits(&sidecar, "harbour city"), []);
    assert_eq!(hits(&sidecar, "Harbour!"), [("10001".to_owned(), first)]);

    // The same source again replaces its own words: the old ones are not found.
    let second = record(
        &sidecar,
        "10001",
        &agent("a"),
        "Salt and static on the radio",
    );
    assert_eq!(hits(&sidecar, "harbour"), []);
    assert_eq!(hits(&sidecar, "static"), [("10001".to_owned(), second)]);
    let snippet = &sidecar.search_lyrics("static", 1).unwrap()[0].snippet;
    assert!(snippet.contains("[static]"), "{snippet}");

    let live: Vec<i64> = sidecar
        .lyrics_records()
        .unwrap()
        .iter()
        .map(|f| f.id)
        .collect();
    assert_eq!(live, [second, other]);
}

#[test]
fn a_store_from_before_the_index_is_migrated_with_its_lyrics_searchable() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("sidecar");
    let id = {
        let sidecar = Sidecar::open(&folder).unwrap();
        record(&sidecar, "10003", &agent("a"), "Velvet thunder rolls")
    };
    // Back to the version before the index, as such a store was.
    {
        let db = Connection::open(folder.join("sidecar.db")).unwrap();
        db.execute_batch(&format!(
            "DROP TRIGGER lyrics_search_add; DROP TRIGGER lyrics_search_drop;
             DROP TABLE lyrics_search;
             UPDATE meta SET value = '{}' WHERE key = 'schema_version';",
            SCHEMA_VERSION - 1
        ))
        .unwrap();
    }
    let sidecar = Sidecar::open(&folder).unwrap();
    let version: String = Connection::open(folder.join("sidecar.db"))
        .unwrap()
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION.to_string());
    assert_eq!(hits(&sidecar, "thunder"), [("10003".to_owned(), id)]);
}
