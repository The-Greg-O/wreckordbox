//! The Ledger in the Sidecar's database: entries for edits to the Library,
//! read with a Run's Facts as one list, counted by kind, never changed, and
//! brought into a store made before it existed.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use serde_json::json;
use wreckordbox_sidecar::ledger::{Action, Count, NewEntry, Store};
use wreckordbox_sidecar::{Kind, NewFact, RunId, Sidecar, Source, SourceKind};

fn agent() -> Source {
    Source::new(SourceKind::Agent, "tagger")
}

fn entry<'a>(
    run: RunId,
    source: &'a Source,
    payload: &'a serde_json::Value,
    command: Option<&'a serde_json::Value>,
) -> NewEntry<'a> {
    NewEntry {
        run,
        store: Store::Library,
        source,
        at: None,
        action: Action::Edit,
        kind: "track",
        label: "Track Edit",
        command,
        reverses: None,
        reversible: true,
        format: 1,
        payload: Some(payload),
    }
}

#[test]
fn a_runs_library_edits_and_facts_are_read_back_as_one_list_in_the_order_made() {
    let dir = tempfile::tempdir().unwrap();
    let sidecar = Sidecar::open(dir.path()).unwrap();
    let source = agent();
    let run = sidecar.begin_run(&source, "Rate and tag").unwrap();
    let other = sidecar.begin_run(&source, "Something else").unwrap();

    let payload = json!({"values": [{"field": "Rating", "before": 2, "after": 4}]});
    let command = json!({"name": "tracks.rate", "args": {"id": "10003", "rating": 4}});
    let first = sidecar
        .append(&entry(run, &source, &payload, Some(&command)))
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(3));
    let genre = |value: &str| NewFact {
        track_id: "10003",
        fingerprint: None,
        kind: Kind::judgement("genre"),
        value: json!(value),
        source: &source,
        confidence: 0.8,
        run,
    };
    let old = sidecar.record(&genre("House")).unwrap();
    let new = sidecar.record(&genre("Tech House")).unwrap();
    sidecar
        .append(&entry(other, &source, &payload, None))
        .unwrap();

    let (entries, total) = sidecar.run_entries(run, 50, 0).unwrap();
    assert_eq!(total, 3);
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].store, Store::Library);
    assert_eq!(entries[0].id, first);
    assert_eq!(entries[0].action, Action::Edit);
    assert_eq!(entries[0].source, source);
    assert_eq!(entries[0].command, Some(command));
    assert_eq!(entries[0].payload, Some(payload));
    assert!(entries[0].reversible);
    assert_eq!(entries[1].store, Store::Sidecar);
    assert_eq!(entries[1].fact.as_ref().unwrap().id, old.id);
    assert_eq!(entries[1].replaced, None);
    assert_eq!(entries[2].fact.as_ref().unwrap().id, new.id);
    assert_eq!(entries[2].action, Action::Record);
    assert_eq!(
        entries[2].replaced.as_ref().map(|fact| fact.id),
        Some(old.id),
        "a Fact's entry carries the one it took the place of"
    );

    // A page of it.
    let (page, total) = sidecar.run_entries(run, 1, 1).unwrap();
    assert_eq!(total, 3);
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].store, Store::Sidecar);

    let summary = sidecar.run_summary(run).unwrap().unwrap();
    assert_eq!(summary.run.description, "Rate and tag");
    assert_eq!(
        summary.counts,
        vec![
            Count {
                store: Store::Library,
                kind: "track".into(),
                count: 1
            },
            Count {
                store: Store::Sidecar,
                kind: "genre".into(),
                count: 2
            },
        ]
    );

    let (runs, total) = sidecar.runs(10, 0).unwrap();
    assert_eq!(total, 2);
    assert_eq!(runs[0].run.id, other, "newest first");
    assert_eq!(runs[1].run.id, run);
    assert!(sidecar.run_summary(RunId(999)).unwrap().is_none());
    assert_eq!(sidecar.ledger_entry(first).unwrap().unwrap().id, first);
    assert!(sidecar.ledger_entry(999).unwrap().is_none());
}

#[test]
fn an_entry_is_never_changed_or_removed_and_survives_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let source = agent();
    let payload = json!({"values": []});
    let (run, id) = {
        let sidecar = Sidecar::open(dir.path()).unwrap();
        let run = sidecar.begin_run(&source, "Rate").unwrap();
        let id = sidecar
            .append(&entry(run, &source, &payload, None))
            .unwrap();
        (run, id)
    };
    let raw = rusqlite::Connection::open(dir.path().join("sidecar.db")).unwrap();
    assert!(raw
        .execute("UPDATE ledger SET label = 'other' WHERE id = ?1", [id])
        .is_err());
    assert!(raw
        .execute("DELETE FROM ledger WHERE id = ?1", [id])
        .is_err());
    drop(raw);

    let sidecar = Sidecar::open(dir.path()).unwrap();
    let (entries, total) = sidecar.run_entries(run, 50, 0).unwrap();
    assert_eq!(total, 1);
    assert_eq!(entries[0].label, "Track Edit");
}

#[test]
fn a_store_made_before_the_ledger_is_given_one_and_keeps_its_facts() {
    let dir = tempfile::tempdir().unwrap();
    let source = agent();
    let fact = {
        let sidecar = Sidecar::open(dir.path()).unwrap();
        let run = sidecar.begin_run(&source, "Tag").unwrap();
        sidecar
            .record(&NewFact {
                track_id: "10001",
                fingerprint: None,
                kind: Kind::judgement("genre"),
                value: json!("Dub"),
                source: &source,
                confidence: 1.0,
                run,
            })
            .unwrap()
    };
    // Back to what the first version of the store held.
    let raw = rusqlite::Connection::open(dir.path().join("sidecar.db")).unwrap();
    raw.execute_batch(
        "DROP TRIGGER ledger_is_never_changed;
         DROP TRIGGER ledger_is_never_removed;
         DROP TABLE ledger;
         UPDATE meta SET value = '1' WHERE key = 'schema_version';",
    )
    .unwrap();
    drop(raw);

    let sidecar = Sidecar::open(dir.path()).unwrap();
    let run = sidecar.begin_run(&source, "Rate").unwrap();
    let payload = json!({"values": []});
    sidecar
        .append(&entry(run, &source, &payload, None))
        .unwrap();
    assert_eq!(sidecar.facts_of_track("10001").unwrap()[0].id, fact.id);
    let version: String = rusqlite::Connection::open(dir.path().join("sidecar.db"))
        .unwrap()
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, "2");
}

#[test]
fn runs_left_open_by_a_process_that_died_are_ended() {
    let dir = tempfile::tempdir().unwrap();
    let sidecar = Sidecar::open(dir.path()).unwrap();
    let dj = Source::new(SourceKind::Dj, "dj");
    let left = sidecar.begin_run(&dj, "Edits in the window").unwrap();
    let other = sidecar.begin_run(&dj, "Something else").unwrap();
    assert_eq!(
        sidecar
            .finish_abandoned_runs(&dj, "Edits in the window", "the window closed")
            .unwrap(),
        1
    );
    assert_eq!(
        sidecar.run(left).unwrap().unwrap().outcome.as_deref(),
        Some("the window closed")
    );
    assert!(!sidecar.run(other).unwrap().unwrap().is_finished());
}

#[test]
fn marking_measurements_stale_is_an_entry_in_a_run_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let sidecar = Sidecar::open(dir.path()).unwrap();
    let source = agent();
    let run = sidecar.begin_run(&source, "Measure").unwrap();
    let audio = dir.path().join("a.bin");
    std::fs::write(&audio, b"one").unwrap();
    let before = wreckordbox_sidecar::fingerprint_file(&audio).unwrap();
    std::fs::write(&audio, b"another").unwrap();
    let after = wreckordbox_sidecar::fingerprint_file(&audio).unwrap();
    let fact = sidecar
        .record(&NewFact {
            track_id: "10004",
            fingerprint: Some(&before),
            kind: Kind::measurement("loudness"),
            value: json!(0.5),
            source: &source,
            confidence: 1.0,
            run,
        })
        .unwrap();
    let (_, runs_before) = sidecar.runs(10, 0).unwrap();

    // The same audio marks nothing, and so records nothing.
    assert!(sidecar.mark_stale("10004", &before).unwrap().is_empty());
    assert_eq!(sidecar.runs(10, 0).unwrap().1, runs_before);

    assert_eq!(sidecar.mark_stale("10004", &after).unwrap(), vec![fact.id]);
    let (runs, total) = sidecar.runs(10, 0).unwrap();
    assert_eq!(total, runs_before + 1);
    let made = &runs[0];
    assert!(made.run.is_finished());
    assert_eq!(made.run.source, Source::new(SourceKind::Tool, "wreckordbox"));
    assert_eq!(
        made.counts,
        vec![Count {
            store: Store::Sidecar,
            kind: "stale".into(),
            count: 1
        }]
    );
    let (entries, _) = sidecar.run_entries(made.run.id, 10, 0).unwrap();
    assert_eq!(entries[0].store, Store::Sidecar);
    assert_eq!(entries[0].action, Action::Edit);
    assert_eq!(
        entries[0].payload,
        Some(json!({"track": "10004", "fingerprint": after.as_str(), "facts": [fact.id]}))
    );
    sidecar.ledger_ready().unwrap();
}
