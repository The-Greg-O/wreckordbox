//! Mutants in the Sidecar, and bringing a store from before them up to
//! date.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use serde_json::json;
use wreckordbox_sidecar::mutants::{InRun, MutantUpdate, NewMutant};
use wreckordbox_sidecar::RunId;
use wreckordbox_sidecar::{Kind, NewFact, Sidecar, SidecarError, Source, SourceKind};

/// The store as version 1 made it, before Mutants: what a Sidecar written
/// by an earlier wreckordbox holds.
const VERSION_1: &str = "
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;
CREATE TABLE kind (
    name TEXT PRIMARY KEY,
    nature TEXT NOT NULL CHECK (nature IN ('judgement', 'measurement'))
) STRICT;
CREATE TABLE run (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('dj', 'agent', 'tool', 'web')),
    source_name TEXT NOT NULL,
    description TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    outcome TEXT
) STRICT;
CREATE TABLE fact (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id TEXT NOT NULL,
    fingerprint TEXT,
    kind TEXT NOT NULL REFERENCES kind (name),
    value TEXT NOT NULL,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('dj', 'agent', 'tool', 'web')),
    source_name TEXT NOT NULL,
    confidence REAL NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
    run_id INTEGER NOT NULL REFERENCES run (id),
    recorded_at INTEGER NOT NULL,
    stale_at INTEGER,
    superseded_at INTEGER,
    superseded_by INTEGER REFERENCES fact (id)
) STRICT;
CREATE INDEX fact_by_track ON fact (track_id, kind);
CREATE INDEX fact_by_fingerprint ON fact (fingerprint, kind);
CREATE INDEX fact_by_run ON fact (run_id);
INSERT INTO meta (key, value) VALUES ('schema_version', '1');
INSERT INTO kind (name, nature) VALUES ('genre', 'judgement');
INSERT INTO run (source_kind, source_name, description, started_at) VALUES ('dj', 'dj', 'By hand', 1000);
INSERT INTO fact (track_id, kind, value, source_kind, source_name, confidence, run_id, recorded_at)
VALUES ('10003', 'genre', '\"Dub Techno\"', 'dj', 'dj', 1.0, 1, 1000);
";

fn agent() -> Source {
    Source::new(SourceKind::Agent, "preparer")
}

#[test]
fn a_store_from_before_mutants_is_brought_up_to_date_keeping_what_it_held() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("sidecar");
    std::fs::create_dir_all(&folder).unwrap();
    {
        let old = rusqlite::Connection::open(folder.join("sidecar.db")).unwrap();
        old.execute_batch(VERSION_1).unwrap();
    }

    let sidecar = Sidecar::open(&folder).unwrap();

    // What it held is still there, and still takes more.
    let facts = sidecar.facts("10003", "genre").unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].value, json!("Dub Techno"));
    let run = sidecar.begin_run(&agent(), "Prepare Friday").unwrap();
    sidecar
        .record(&NewFact {
            track_id: "10003",
            fingerprint: None,
            kind: Kind::judgement("genre"),
            value: json!("Dub"),
            source: &agent(),
            confidence: 0.5,
            run,
        })
        .unwrap();
    // And it holds Mutants now.
    let mutant = sidecar
        .create_mutant(&NewMutant {
            playlist_id: "900001",
            entry_id: "entry-a",
            track_id: "10003",
            fingerprint: None,
            edits: json!({}),
            source: &agent(),
            run: run.into(),
        })
        .unwrap()
        .mutant;
    drop(sidecar);

    // Opening it again changes nothing more.
    let again = Sidecar::open(&folder).unwrap();
    assert_eq!(again.mutant(mutant.id).unwrap(), Some(mutant));
    assert_eq!(again.facts("10003", "genre").unwrap().len(), 2);
}

#[test]
fn a_mutants_edits_are_those_of_its_newest_change_and_every_change_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let sidecar = Sidecar::open(&dir.path().join("sidecar")).unwrap();
    let run = sidecar.begin_run(&agent(), "Prepare Friday").unwrap();
    let dj = Source::new(SourceKind::Dj, "dj");
    let new = NewMutant {
        playlist_id: "900001",
        entry_id: "entry-a",
        track_id: "10003",
        fingerprint: None,
        edits: json!({}),
        source: &agent(),
        run: run.into(),
    };
    let made = sidecar.create_mutant(&new).unwrap().mutant;
    assert!(matches!(
        sidecar.create_mutant(&new),
        Err(SidecarError::AlreadyMutant { mutant, .. }) if mutant == made.id
    ));

    let by_dj = MutantUpdate {
        fingerprint: None,
        source: &dj,
        run: run.into(),
    };
    let edited = sidecar
        .edit_mutant(made.id, &json!({ "key_shift": 1 }), &by_dj)
        .unwrap()
        .mutant;
    assert_eq!(edited.edits, json!({ "key_shift": 1 }));
    assert_eq!(edited.source, dj);
    assert!(matches!(
        sidecar.edit_mutant(made.id, &json!([1]), &by_dj),
        Err(SidecarError::Invalid(_))
    ));
    assert_eq!(sidecar.mutant_of_entry("entry-a").unwrap(), Some(edited));

    let removed = sidecar.remove_mutant(made.id, &by_dj).unwrap().mutant;
    assert!(!removed.is_live());
    // What was removed is kept, with the edits it had.
    assert_eq!(removed.edits, json!({ "key_shift": 1 }));
    assert_eq!(sidecar.mutant_of_entry("entry-a").unwrap(), None);
    assert_eq!(sidecar.mutants(None).unwrap(), vec![]);
    assert!(matches!(
        sidecar.remove_mutant(made.id, &by_dj),
        Err(SidecarError::NoMutant(id)) if id == made.id
    ));
    let history = sidecar.mutant_changes(made.id).unwrap();
    let told: Vec<(&str, serde_json::Value)> = history
        .iter()
        .map(|change| (change.action.as_str(), change.edits.clone()))
        .collect();
    assert_eq!(
        told,
        [
            ("created", json!({})),
            ("edited", json!({ "key_shift": 1 })),
            ("removed", json!({ "key_shift": 1 })),
        ]
    );
    assert_eq!(sidecar.mutant_changes_in_run(run).unwrap(), history);

    // The entry can be mutated afresh: a new Mutant, as its parent.
    let afresh = sidecar.create_mutant(&new).unwrap().mutant;
    assert_ne!(afresh.id, made.id);
    assert_eq!(afresh.edits, json!({}));
    assert_eq!(sidecar.mutants_of_track("10003").unwrap(), vec![afresh]);
}

fn new_mutant<'a>(entry: &'a str, source: &'a Source, run: InRun<'a>) -> NewMutant<'a> {
    NewMutant {
        playlist_id: "900001",
        entry_id: entry,
        track_id: "10003",
        fingerprint: None,
        edits: json!({}),
        source,
        run,
    }
}

/// How many Runs the store holds.
fn runs(folder: &std::path::Path) -> i64 {
    rusqlite::Connection::open(folder.join("sidecar.db"))
        .unwrap()
        .query_row("SELECT count(*) FROM run", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn a_mutant_change_is_refused_in_a_run_that_has_ended_or_is_not_there() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("sidecar");
    let sidecar = Sidecar::open(&folder).unwrap();
    let source = agent();
    let open = sidecar.begin_run(&source, "Prepare Friday").unwrap();
    let made = sidecar
        .create_mutant(&new_mutant("entry-a", &source, open.into()))
        .unwrap();
    assert_eq!(made.run.id, open);
    assert!(!made.run.is_finished());
    sidecar.finish_run(open, "done").unwrap();

    // The Run is checked in the transaction that would record the change.
    assert!(matches!(
        sidecar.create_mutant(&new_mutant("entry-b", &source, open.into())),
        Err(SidecarError::RunFinished(run)) if run == open
    ));
    let update = MutantUpdate {
        fingerprint: None,
        source: &source,
        run: open.into(),
    };
    assert!(matches!(
        sidecar.remove_mutant(made.mutant.id, &update),
        Err(SidecarError::RunFinished(run)) if run == open
    ));
    assert!(matches!(
        sidecar.edit_mutant(made.mutant.id, &json!({ "key_shift": 1 }), &update),
        Err(SidecarError::RunFinished(run)) if run == open
    ));
    assert!(matches!(
        sidecar.create_mutant(&new_mutant("entry-b", &source, RunId(99).into())),
        Err(SidecarError::NoRun(RunId(99)))
    ));

    // Nothing changed.
    assert_eq!(sidecar.mutants(None).unwrap(), vec![made.mutant.clone()]);
    assert_eq!(sidecar.mutant_changes(made.mutant.id).unwrap().len(), 1);
    assert_eq!(runs(&folder), 1);
}

#[test]
fn a_run_made_for_one_change_is_made_and_ended_with_it_or_not_at_all() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("sidecar");
    let sidecar = Sidecar::open(&folder).unwrap();
    let source = agent();

    let made = sidecar
        .create_mutant(&new_mutant(
            "entry-a",
            &source,
            InRun::Own {
                description: "Mutate entry 1",
            },
        ))
        .unwrap();
    assert_eq!(made.mutant.run, made.run.id);
    assert_eq!(made.run.description, "Mutate entry 1");
    assert_eq!(made.run.source, source);
    assert_eq!(
        made.run.outcome.as_deref(),
        Some(format!("created Mutant {}", made.mutant.id).as_str())
    );
    assert!(made.run.is_finished());
    assert_eq!(runs(&folder), 1);

    // A store that refuses to end a Run: the change is not kept either.
    let other = rusqlite::Connection::open(folder.join("sidecar.db")).unwrap();
    other
        .execute_batch(
            "CREATE TRIGGER no_ending BEFORE UPDATE OF finished_at ON run
             BEGIN SELECT RAISE(ABORT, 'no ending'); END;",
        )
        .unwrap();
    let own = || InRun::Own {
        description: "By hand",
    };
    assert!(matches!(
        sidecar.create_mutant(&new_mutant("entry-b", &source, own())),
        Err(SidecarError::Db(_))
    ));
    let removal = MutantUpdate {
        fingerprint: None,
        source: &source,
        run: own(),
    };
    assert!(matches!(
        sidecar.remove_mutant(made.mutant.id, &removal),
        Err(SidecarError::Db(_))
    ));
    assert_eq!(sidecar.mutant_of_entry("entry-b").unwrap(), None);
    assert_eq!(
        sidecar.mutant_of_entry("entry-a").unwrap(),
        Some(made.mutant.clone())
    );
    assert_eq!(sidecar.mutant_changes(made.mutant.id).unwrap().len(), 1);
    assert_eq!(runs(&folder), 1);

    // Once it can, both go through, each in a Run of its own.
    other.execute_batch("DROP TRIGGER no_ending").unwrap();
    let removed = sidecar.remove_mutant(made.mutant.id, &removal).unwrap();
    assert!(!removed.mutant.is_live());
    assert_eq!(
        removed.run.outcome.as_deref(),
        Some(format!("removed Mutant {}", made.mutant.id).as_str())
    );
    assert!(sidecar
        .create_mutant(&new_mutant("entry-b", &source, own()))
        .is_ok());
    assert_eq!(runs(&folder), 3);
}
