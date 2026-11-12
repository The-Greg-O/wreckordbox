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
use wreckordbox_sidecar::ledger::{Action, Count, Intent, NewEntry, Store};
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
        id: None,
        reverses: None,
        part_of: None,
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

/// Takes a store back to what an older wreckordbox made: the shape of
/// `version`, with what it held then.
fn downgrade(dir: &std::path::Path, version: u32) {
    let raw = rusqlite::Connection::open(dir.join("sidecar.db")).unwrap();
    let mut drop = String::new();
    if version < 4 {
        drop.push_str(
            "DROP TRIGGER fact_takes_its_place_in_order;
             DROP TABLE ledger_intent; DROP TABLE ledger_owned_run;
             DROP TABLE ledger; DROP TABLE ledger_order;
             DROP INDEX fact_by_successor;",
        );
    }
    if version < 3 {
        drop.push_str("DROP TABLE decision_answer; DROP TABLE decision;");
    }
    if version < 2 {
        drop.push_str("DROP TABLE mutant_change; DROP TABLE mutant;");
    }
    drop.push_str(&format!(
        "UPDATE meta SET value = '{version}' WHERE key = 'schema_version';"
    ));
    raw.execute_batch(&drop).unwrap();
}

#[test]
fn a_store_from_each_earlier_version_is_given_the_ledger_and_keeps_its_facts_in_order() {
    for version in [1, 2, 3] {
        let dir = tempfile::tempdir().unwrap();
        let source = agent();
        let (run, facts) = {
            let sidecar = Sidecar::open(dir.path()).unwrap();
            let run = sidecar.begin_run(&source, "Tag").unwrap();
            let fact = |value: &str| {
                sidecar
                    .record(&NewFact {
                        track_id: "10001",
                        fingerprint: None,
                        kind: Kind::judgement("genre"),
                        value: json!(value),
                        source: &source,
                        confidence: 1.0,
                        run,
                    })
                    .unwrap()
                    .id
            };
            (run, vec![fact("Dub"), fact("Dub Techno")])
        };
        downgrade(dir.path(), version);

        let sidecar = Sidecar::open(dir.path()).unwrap();
        let payload = json!({"values": []});
        let id = sidecar
            .append(&entry(run, &source, &payload, None))
            .unwrap();
        // The Facts it held keep their order, before what is added now.
        let (entries, total) = sidecar.run_entries(run, 50, 0).unwrap();
        assert_eq!(total, 3, "from version {version}");
        let ids: Vec<i64> = entries.iter().map(|entry| entry.id).collect();
        assert_eq!(ids, vec![facts[0], facts[1], id], "from version {version}");
        let stored: String = rusqlite::Connection::open(dir.path().join("sidecar.db"))
            .unwrap()
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, "5", "from version {version}");
    }
}

#[test]
fn library_entries_and_facts_made_in_the_same_millisecond_are_read_in_the_order_made() {
    let dir = tempfile::tempdir().unwrap();
    let sidecar = Sidecar::open(dir.path()).unwrap();
    let source = agent();
    let run = sidecar.begin_run(&source, "Tag and rate").unwrap();
    let fact = sidecar
        .record(&NewFact {
            track_id: "10002",
            fingerprint: None,
            kind: Kind::judgement("genre"),
            value: json!("Dub"),
            source: &source,
            confidence: 1.0,
            run,
        })
        .unwrap();
    let payload = json!({"values": []});
    let edit = sidecar
        .append(&NewEntry {
            at: Some(fact.recorded_at),
            ..entry(run, &source, &payload, None)
        })
        .unwrap();
    let later = sidecar
        .record(&NewFact {
            track_id: "10003",
            fingerprint: None,
            kind: Kind::judgement("genre"),
            value: json!("Dub"),
            source: &source,
            confidence: 1.0,
            run,
        })
        .unwrap();
    // An entry whose identifier was taken before the Fact was recorded is
    // read before it, whenever it was written.
    let reserved = sidecar
        .begin_intent(
            &Intent {
                source: &source,
                run: Some(run),
                description: "Tag and rate",
                command: None,
                at: fact.recorded_at,
            },
            &[],
        )
        .unwrap();
    let last = sidecar
        .record(&NewFact {
            track_id: "10004",
            fingerprint: None,
            kind: Kind::judgement("genre"),
            value: json!("Dub"),
            source: &source,
            confidence: 1.0,
            run,
        })
        .unwrap();
    sidecar
        .append(&NewEntry {
            id: Some(reserved),
            at: Some(fact.recorded_at),
            ..entry(run, &source, &payload, None)
        })
        .unwrap();

    let (entries, _) = sidecar.run_entries(run, 50, 0).unwrap();
    let order: Vec<(Store, i64)> = entries.iter().map(|e| (e.store, e.id)).collect();
    assert_eq!(
        order,
        vec![
            (Store::Sidecar, fact.id),
            (Store::Library, edit),
            (Store::Sidecar, later.id),
            (Store::Library, reserved),
            (Store::Sidecar, last.id),
        ]
    );
    // Paged by the same order.
    let (page, _) = sidecar.run_entries(run, 2, 1).unwrap();
    assert_eq!(
        page.iter().map(|e| e.id).collect::<Vec<_>>(),
        vec![edit, later.id]
    );
}

#[test]
fn the_fact_a_recorded_fact_replaced_is_found_by_an_index_not_a_scan() {
    let dir = tempfile::tempdir().unwrap();
    let sidecar = Sidecar::open(dir.path()).unwrap();
    let source = agent();
    let run = sidecar.begin_run(&source, "Tag").unwrap();
    for track in 0..400 {
        for value in ["Dub", "Dub Techno"] {
            sidecar
                .record(&NewFact {
                    track_id: &format!("{}", 20_000 + track),
                    fingerprint: None,
                    kind: Kind::judgement("genre"),
                    value: json!(value),
                    source: &source,
                    confidence: 1.0,
                    run,
                })
                .unwrap();
        }
    }
    let (page, total) = sidecar.run_entries(run, 50, 700).unwrap();
    assert_eq!(total, 800);
    assert_eq!(page.len(), 50);
    // Each track's second Fact replaced its first.
    for pair in page.chunks(2) {
        assert!(pair[0].replaced.is_none());
        assert_eq!(pair[1].replaced.as_ref().unwrap().id, pair[0].id);
    }

    let raw = rusqlite::Connection::open(dir.path().join("sidecar.db")).unwrap();
    let plan: Vec<String> = raw
        .prepare(&format!(
            "EXPLAIN QUERY PLAN {}",
            wreckordbox_sidecar::ledger::PAGE_QUERY
        ))
        .unwrap()
        .query_map(rusqlite::params![run.0, 50, 0], |row| {
            row.get::<_, String>(3)
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(
        // No table is scanned; only the page's own rows are sorted.
        plan.iter()
            .all(|step| !step.starts_with("SCAN") || step.starts_with("SCAN (subquery")),
        "{plan:#?}"
    );
}

#[test]
fn an_entry_appended_again_under_its_identifier_is_kept_once_and_settles_its_intent() {
    let dir = tempfile::tempdir().unwrap();
    let sidecar = Sidecar::open(dir.path()).unwrap();
    let source = agent();
    let run = sidecar.begin_run(&source, "Rate").unwrap();
    let command = json!({"name": "tracks.rate"});
    let intent = Intent {
        source: &source,
        run: Some(run),
        description: "Rate",
        command: Some(&command),
        at: 1_000,
    };
    let first = sidecar.begin_intent(&intent, &[]).unwrap();
    let second = sidecar.begin_intent(&intent, &[]).unwrap();
    assert!(second > first);
    assert_eq!(
        sidecar
            .open_intents()
            .unwrap()
            .iter()
            .map(|open| open.id)
            .collect::<Vec<_>>(),
        vec![first, second]
    );

    let payload = json!({"values": []});
    let kept = NewEntry {
        id: Some(first),
        ..entry(run, &source, &payload, None)
    };
    assert_eq!(sidecar.append(&kept).unwrap(), first);
    assert_eq!(sidecar.append(&kept).unwrap(), first, "a replay");
    assert_eq!(sidecar.run_entries(run, 50, 0).unwrap().1, 1);
    // Its intent went with it; the other is settled without an entry.
    assert_eq!(sidecar.open_intents().unwrap()[0].id, second);
    let third = sidecar.begin_intent(&intent, &[second]).unwrap();
    assert_eq!(
        sidecar
            .open_intents()
            .unwrap()
            .iter()
            .map(|open| open.id)
            .collect::<Vec<_>>(),
        vec![third]
    );
    sidecar.settle_intents(&[third]).unwrap();
    assert!(sidecar.open_intents().unwrap().is_empty());
}

#[test]
fn runs_wreckordbox_made_for_itself_and_left_open_by_a_process_that_died_are_ended() {
    let dir = tempfile::tempdir().unwrap();
    let sidecar = Sidecar::open(dir.path()).unwrap();
    let dj = Source::new(SourceKind::Dj, "dj");
    let left = sidecar.begin_owned_run(&dj, "Edits in the window").unwrap();
    let command = sidecar.begin_owned_run(&agent(), "tracks.rate").unwrap();
    let ended = sidecar.begin_owned_run(&dj, "Edits in the window").unwrap();
    sidecar.finish_run(ended, "the DJ paused").unwrap();
    // A Run a client started stays open for it.
    let client = sidecar.begin_run(&dj, "Edits in the window").unwrap();
    assert_eq!(sidecar.finish_owned_runs("wreckordbox stopped").unwrap(), 2);
    for run in [left, command] {
        assert_eq!(
            sidecar.run(run).unwrap().unwrap().outcome.as_deref(),
            Some("wreckordbox stopped")
        );
    }
    assert_eq!(
        sidecar.run(ended).unwrap().unwrap().outcome.as_deref(),
        Some("the DJ paused")
    );
    assert!(!sidecar.run(client).unwrap().unwrap().is_finished());
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
    assert_eq!(
        made.run.source,
        Source::new(SourceKind::Tool, "wreckordbox")
    );
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
}
