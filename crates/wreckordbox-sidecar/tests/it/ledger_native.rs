#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
use wreckordbox_sidecar::{RunId, Sidecar};
#[test]
fn legacy_mutant_and_decision_history_is_visible_once_in_its_original_run() {
    let dir = tempfile::tempdir().unwrap();
    let db = rusqlite::Connection::open(dir.path().join("sidecar.db")).unwrap();
    db.execute_batch(include_str!("../fixtures/sidecar-v1.sql"))
        .unwrap();
    db.execute_batch(include_str!("../fixtures/sidecar-v2.sql"))
        .unwrap();
    db.execute_batch(include_str!("../fixtures/sidecar-v3.sql"))
        .unwrap();
    drop(db);
    let sidecar = Sidecar::open(dir.path()).unwrap();
    let (mutants, total) = sidecar.run_entries(RunId(8), 50, 0).unwrap();
    assert_eq!(
        total, 1,
        "legacy Mutant history cannot disappear from its Run"
    );
    assert_eq!(mutants[0].kind, "mutant_change");
    let (decisions, total) = sidecar.run_entries(RunId(7), 50, 0).unwrap();
    assert_eq!(
        total, 3,
        "the Decision must be visible beside the Run's two Facts"
    );
    assert_eq!(decisions[2].kind, "decision");
    assert_eq!(
        decisions[2].payload.as_ref().unwrap()["row"]["question"],
        "Genre?"
    );
    let counts = sidecar.run_summary(RunId(7)).unwrap().unwrap().counts;
    assert_eq!(counts.iter().map(|c| c.count).sum::<u64>(), 3);
    drop(sidecar);
    let reopened = Sidecar::open(dir.path()).unwrap();
    assert_eq!(
        reopened.run_entries(RunId(7), 50, 0).unwrap().0,
        decisions,
        "opening twice must not duplicate recovered history"
    );
}

#[test]
fn mixed_new_history_uses_one_order_and_counts_each_native_change_once() {
    use wreckordbox_sidecar::decisions::{NewAnswer, NewDecision, QuestionType};
    use wreckordbox_sidecar::mutants::NewMutant;
    use wreckordbox_sidecar::{Kind, NewFact, Source, SourceKind};
    let dir = tempfile::tempdir().unwrap();
    let sidecar = Sidecar::open(dir.path()).unwrap();
    let source = Source::new(SourceKind::Agent, "fixture");
    let run = sidecar.begin_run(&source, "Mixed work").unwrap();
    let m = sidecar
        .create_mutant(&NewMutant {
            playlist_id: "1",
            entry_id: "2",
            track_id: "3",
            fingerprint: None,
            edits: serde_json::json!({"gain_db":-3}),
            source: &source,
            run: run.into(),
        })
        .unwrap();
    sidecar
        .record(&NewFact {
            track_id: "3",
            fingerprint: None,
            kind: Kind::judgement("genre"),
            value: serde_json::json!("Techno"),
            source: &source,
            confidence: 1.,
            run,
        })
        .unwrap();
    let d = sidecar
        .create_decision(&NewDecision {
            question_type: QuestionType::YesNo,
            question: "Keep?",
            topic: None,
            context: &serde_json::json!({}),
            candidates: &[],
            track_id: Some("3"),
            run: Some(run),
        })
        .unwrap();
    sidecar
        .answer_decision(&NewAnswer {
            decision: d.id,
            decider: &source,
            choice: "yes",
            choice_probability: Some(1.),
            probabilities: None,
            reason: Some("fixture"),
            fact_id: None,
            run: Some(run),
        })
        .unwrap();
    let (entries, total) = sidecar.run_entries(run, 50, 0).unwrap();
    assert_eq!(total, 4);
    assert_eq!(
        entries.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
        ["mutant_change", "genre", "decision", "decision_answer"]
    );
    assert!(entries.iter().all(|e| e.source == source));
    assert_eq!(
        entries[0].payload.as_ref().unwrap()["row"]["mutant_id"],
        m.mutant.id
    );
    assert_eq!(
        sidecar
            .run_summary(run)
            .unwrap()
            .unwrap()
            .counts
            .iter()
            .map(|c| c.count)
            .sum::<u64>(),
        4
    );
    assert_eq!(sidecar.run_entries(run, 2, 2).unwrap().0, entries[2..]);
}

#[test]
fn historical_history_is_merged_by_timestamp_without_reordering_shared_ids() {
    let dir = tempfile::tempdir().unwrap();
    let db = rusqlite::Connection::open(dir.path().join("sidecar.db")).unwrap();
    db.execute_batch(include_str!("../fixtures/sidecar-v1.sql"))
        .unwrap();
    db.execute_batch(include_str!("../fixtures/sidecar-v2.sql"))
        .unwrap();
    db.execute_batch("UPDATE mutant_change SET run_id=7, recorded_at=1")
        .unwrap();
    drop(db);
    let sidecar = Sidecar::open(dir.path()).unwrap();
    let (entries, _) = sidecar.run_entries(RunId(7), 10, 0).unwrap();
    assert_eq!(entries[0].kind, "mutant_change");
    assert_eq!(entries[1].id, 3);
    assert_eq!(entries[2].id, 4);
}

#[test]
fn recovery_run_and_entry_commit_together_and_retry_does_not_create_an_empty_run() {
    use wreckordbox_sidecar::{
        ledger::{Action, NewEntry, Store},
        Source, SourceKind,
    };
    let dir = tempfile::tempdir().unwrap();
    let sidecar = Sidecar::open(dir.path()).unwrap();
    let source = Source::new(SourceKind::Tool, "fixture");
    let entry = NewEntry {
        id: Some(800),
        run: RunId(0),
        store: Store::Library,
        source: &source,
        at: Some(100),
        action: Action::Write,
        kind: "recovered_fixture",
        label: "Accepted write",
        command: None,
        reverses: None,
        part_of: None,
        reversible: false,
        format: 1,
        payload: None,
    };
    let raw = rusqlite::Connection::open(dir.path().join("sidecar.db")).unwrap();
    raw.execute_batch("CREATE TRIGGER reject_fixture BEFORE INSERT ON ledger BEGIN SELECT RAISE(ABORT,'fixture append failure'); END").unwrap();
    assert!(sidecar
        .append_recovered_following(
            &entry,
            &Default::default(),
            false,
            &Default::default(),
            Some("Recovered work")
        )
        .is_err());
    assert_eq!(
        sidecar.runs(100, 0).unwrap().1,
        0,
        "a failed append cannot leave an empty recovery Run"
    );
    raw.execute_batch("DROP TRIGGER reject_fixture").unwrap();
    for _ in 0..2 {
        assert_eq!(
            sidecar
                .append_recovered_following(
                    &entry,
                    &Default::default(),
                    false,
                    &Default::default(),
                    Some("Recovered work")
                )
                .unwrap()
                .0,
            800
        );
    }
    let (runs, total) = sidecar.runs(100, 0).unwrap();
    assert_eq!(total, 1);
    assert_eq!(sidecar.run_entries(runs[0].run.id, 100, 0).unwrap().1, 1);
}
