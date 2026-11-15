//! A lookup check as the Sidecar keeps it (wbx-37): its Decision, its one
//! answer and an accepted candidate's Facts, in one Run and one
//! transaction.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use serde_json::json;
use wreckordbox_sidecar::decisions::{Candidate, NewDecision, QuestionType, RunOfAnswer};
use wreckordbox_sidecar::lookups::{LookupCheck, LookupFact};
use wreckordbox_sidecar::{Kind, Sidecar, SidecarError, Source, SourceKind};

const ADDRESS: &str = "https://shop.example.test/release/4471";

fn counts(folder: &std::path::Path) -> (i64, i64, i64, i64) {
    let db = rusqlite::Connection::open(folder.join("sidecar.db")).unwrap();
    let count = |table: &str| {
        db.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    };
    (
        count("run"),
        count("decision"),
        count("decision_answer"),
        count("fact"),
    )
}

/// Records an accepted check of track 10003 with two Facts.
fn record(sidecar: &Sidecar) -> Result<wreckordbox_sidecar::lookups::LookupRecorded, SidecarError> {
    let checker = Source::new(SourceKind::Tool, "lookup-check");
    let shop = Source::new(SourceKind::Web, ADDRESS);
    let facts = [
        LookupFact {
            kind: Kind::judgement("title"),
            value: json!("Night Signal"),
            source: &shop,
            confidence: 0.9,
        },
        LookupFact {
            kind: Kind::judgement("genre"),
            value: json!(["Tech House"]),
            source: &shop,
            confidence: 0.9,
        },
    ];
    sidecar.record_lookup_check(&LookupCheck {
        decision: NewDecision {
            question_type: QuestionType::Choice,
            topic: Some("lookup_match"),
            question: "Is the recording at the shop the track?",
            context: &json!({ "address": ADDRESS, "date": "2026-11-10" }),
            candidates: &[
                Candidate::named("accepted"),
                Candidate::named("held"),
                Candidate::named("rejected"),
            ],
            track_id: Some("10003"),
            run: None,
        },
        decider: &checker,
        choice: "accepted",
        choice_probability: 0.9,
        reason: "the title is the same",
        track_id: "10003",
        fingerprint: None,
        facts: &facts,
        run: RunOfAnswer::Made {
            description: "Lookup check of track 10003: the shop, looked up on 2026-11-10",
        },
    })
}

#[test]
fn a_check_its_answer_and_its_facts_are_one_run() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("sidecar");
    let sidecar = Sidecar::open(&folder).unwrap();

    let recorded = record(&sidecar).unwrap();

    assert_eq!(recorded.decision.run, Some(recorded.run));
    assert_eq!(
        recorded.decision.answers,
        std::slice::from_ref(&recorded.answer)
    );
    assert_eq!(recorded.answer.choice, "accepted");
    assert_eq!(recorded.answer.run, Some(recorded.run));
    assert_eq!(recorded.facts.len(), 2);
    for fact in &recorded.facts {
        assert_eq!(fact.run, recorded.run);
        assert_eq!(fact.source, Source::new(SourceKind::Web, ADDRESS));
    }
    let run = sidecar.run(recorded.run).unwrap().unwrap();
    assert!(run.is_finished());
    assert_eq!(run.source, Source::new(SourceKind::Tool, "lookup-check"));
    assert_eq!(counts(&folder), (1, 1, 1, 2));
}

#[test]
fn a_check_that_fails_part_way_leaves_no_decision_no_fact_and_no_run() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("sidecar");
    let sidecar = Sidecar::open(&folder).unwrap();
    let before = counts(&folder);
    // The answer is written last, after the Decision, the Run and the Facts.
    rusqlite::Connection::open(folder.join("sidecar.db"))
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER injected_failure BEFORE INSERT ON decision_answer
             BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
        )
        .unwrap();

    let failed = record(&sidecar);

    assert!(
        matches!(failed, Err(SidecarError::Db(_))),
        "{:?}",
        failed.err()
    );
    assert_eq!(counts(&folder), before);
}
