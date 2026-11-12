//! Decisions in the Sidecar: asked once, answered any number of times, and
//! never changed.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::collections::BTreeMap;

use serde_json::json;
use wreckordbox_sidecar::decisions::{
    Candidate, DecisionQuery, DecisionState, NewAnswer, NewDecision, QuestionType,
};
use wreckordbox_sidecar::{Kind, NewFact, Sidecar, SidecarError, Source, SourceKind};

fn agent(name: &str) -> Source {
    Source::new(SourceKind::Agent, name)
}

fn genre_question(sidecar: &Sidecar) -> i64 {
    sidecar
        .create_decision(&NewDecision {
            question_type: QuestionType::Choice,
            topic: Some("genre"),
            question: "Which genre is this track?",
            context: &json!({ "file_genre": "tech-house" }),
            candidates: &[Candidate::named("Techno"), Candidate::named("Tech House")],
            track_id: Some("10003"),
            run: None,
        })
        .unwrap()
        .id
}

fn answer<'a>(decision: i64, decider: &'a Source, choice: &'a str, p: f64) -> NewAnswer<'a> {
    NewAnswer {
        decision,
        decider,
        choice,
        choice_probability: Some(p),
        probabilities: None,
        reason: None,
        fact_id: None,
        run: None,
    }
}

fn pending_for(sidecar: &Sidecar, decider: Option<Source>) -> Vec<i64> {
    sidecar
        .decisions(&DecisionQuery {
            state: DecisionState::Pending,
            decider,
            limit: 50,
            ..DecisionQuery::default()
        })
        .unwrap()
        .0
        .iter()
        .map(|decision| decision.id)
        .collect()
}

#[test]
fn a_store_from_before_decisions_is_brought_up_to_date_and_keeps_its_facts() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("sidecar");
    {
        let sidecar = Sidecar::open(&folder).unwrap();
        let run = sidecar.begin_run(&agent("tagger"), "Tag").unwrap();
        sidecar
            .record(&NewFact {
                track_id: "10003",
                fingerprint: None,
                kind: Kind::judgement("genre"),
                value: json!("Techno"),
                source: &agent("tagger"),
                confidence: 0.7,
                run,
            })
            .unwrap();
    }
    // Take the store back to what version 1 held: only its four tables.
    {
        let db = rusqlite::Connection::open(folder.join("sidecar.db")).unwrap();
        let later: Vec<String> = db
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table'
                 AND name NOT IN ('meta', 'kind', 'run', 'fact') AND name NOT LIKE 'sqlite_%'",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(later.contains(&"decision".to_owned()), "{later:?}");
        db.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        for table in later {
            db.execute_batch(&format!("DROP TABLE \"{table}\""))
                .unwrap();
        }
        db.execute_batch("UPDATE meta SET value = '1' WHERE key = 'schema_version'")
            .unwrap();
    }

    let sidecar = Sidecar::open(&folder).unwrap();

    assert_eq!(sidecar.facts("10003", "genre").unwrap().len(), 1);
    let id = genre_question(&sidecar);
    assert!(sidecar.decision(id).unwrap().unwrap().pending);
    let db = rusqlite::Connection::open(folder.join("sidecar.db")).unwrap();
    let version: String = db
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(version.parse::<u32>().unwrap() >= 2, "{version}");
}

#[test]
fn a_store_from_a_newer_wreckordbox_is_still_refused() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("sidecar");
    drop(Sidecar::open(&folder).unwrap());
    rusqlite::Connection::open(folder.join("sidecar.db"))
        .unwrap()
        .execute(
            "UPDATE meta SET value = '99' WHERE key = 'schema_version'",
            [],
        )
        .unwrap();

    assert!(matches!(
        Sidecar::open(&folder),
        Err(SidecarError::Newer { found: 99, .. })
    ));
}

#[test]
fn a_second_decider_answers_the_same_decision_and_both_answers_are_kept() {
    let dir = tempfile::tempdir().unwrap();
    let sidecar = Sidecar::open(&dir.path().join("sidecar")).unwrap();
    let id = genre_question(&sidecar);
    let (first, second) = (agent("tagger"), agent("other"));

    sidecar
        .answer_decision(&answer(id, &first, "Techno", 0.6))
        .unwrap();
    assert_eq!(pending_for(&sidecar, None), Vec::<i64>::new());
    assert_eq!(pending_for(&sidecar, Some(second.clone())), [id]);
    assert_eq!(
        pending_for(&sidecar, Some(first.clone())),
        Vec::<i64>::new()
    );

    let (decision, _) = sidecar
        .answer_decision(&answer(id, &second, "Tech House", 0.9))
        .unwrap();

    let choices: Vec<(&str, &str)> = decision
        .answers
        .iter()
        .map(|one| (one.decider.name.as_str(), one.choice.as_str()))
        .collect();
    assert_eq!(choices, [("tagger", "Techno"), ("other", "Tech House")]);
    assert_eq!(pending_for(&sidecar, Some(second)), Vec::<i64>::new());
}

#[test]
fn decisions_and_answers_cannot_be_changed_or_deleted_in_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("sidecar");
    let sidecar = Sidecar::open(&folder).unwrap();
    let id = genre_question(&sidecar);
    sidecar
        .answer_decision(&answer(id, &agent("tagger"), "Techno", 0.6))
        .unwrap();

    let db = rusqlite::Connection::open(folder.join("sidecar.db")).unwrap();
    for statement in [
        "UPDATE decision SET question = 'Another?'",
        "DELETE FROM decision_answer",
        "UPDATE decision_answer SET choice = 'Tech House'",
        "DELETE FROM decision",
    ] {
        assert!(db.execute(statement, []).is_err(), "{statement}");
    }
    let decision = sidecar.decision(id).unwrap().unwrap();
    assert_eq!(decision.question, "Which genre is this track?");
    assert_eq!(decision.answers[0].choice, "Techno");
}

#[test]
fn probabilities_must_be_of_candidates_and_agree_with_the_choice() {
    let dir = tempfile::tempdir().unwrap();
    let sidecar = Sidecar::open(&dir.path().join("sidecar")).unwrap();
    let id = genre_question(&sidecar);
    let decider = agent("tagger");
    let with = |pairs: &[(&str, f64)], stated: Option<f64>| {
        let all: BTreeMap<String, f64> = pairs.iter().map(|(n, p)| ((*n).to_owned(), *p)).collect();
        sidecar
            .answer_decision(&NewAnswer {
                choice_probability: stated,
                probabilities: Some(&all),
                ..answer(id, &decider, "Techno", 0.0)
            })
            .map(|(_, answer)| answer)
    };

    assert!(matches!(
        with(&[("Techno", 0.7), ("Trance", 0.3)], None),
        Err(SidecarError::Probability(_))
    ));
    assert!(matches!(
        with(&[("Techno", 0.7), ("Tech House", 0.6)], None),
        Err(SidecarError::Probability(_))
    ));
    assert!(matches!(
        with(&[("Techno", 0.7), ("Tech House", 0.3)], Some(0.9)),
        Err(SidecarError::Probability(_))
    ));
    assert!(sidecar.decision(id).unwrap().unwrap().pending);

    let kept = with(&[("Techno", 0.7), ("Tech House", 0.3)], None).unwrap();
    assert_eq!(kept.choice_probability, 0.7);
    assert_eq!(kept.probabilities.unwrap()["Tech House"], 0.3);
}
