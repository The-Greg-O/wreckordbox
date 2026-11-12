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
    Candidate, DecisionQuery, DecisionState, FactOfAnswer, NewAnswer, NewDecision, QuestionType,
    RunOfAnswer,
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

/// A Sidecar folder holding the frozen version 1 store, never opened by
/// this version of the crate.
fn version_one_store(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let folder = dir.path().join("sidecar");
    std::fs::create_dir_all(&folder).unwrap();
    rusqlite::Connection::open(folder.join("sidecar.db"))
        .unwrap()
        .execute_batch(include_str!("fixtures/sidecar-v1.sql"))
        .unwrap();
    folder
}

/// A Sidecar folder holding the frozen version 2 store (Mutants, before
/// Decisions), never opened by this version of the crate.
fn version_two_store(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let folder = version_one_store(dir);
    raw(&folder)
        .execute_batch(include_str!("fixtures/sidecar-v2.sql"))
        .unwrap();
    folder
}

/// Every column of every row of `table`, as text.
fn rows_of(folder: &std::path::Path, table: &str) -> Vec<String> {
    let db = raw(folder);
    let mut statement = db
        .prepare(&format!("SELECT * FROM {table} ORDER BY id"))
        .unwrap();
    let width = statement.column_count();
    let mut found = statement.query([]).unwrap();
    let mut rows = Vec::new();
    while let Some(row) = found.next().unwrap() {
        let cells: Vec<String> = (0..width)
            .map(|at| format!("{:?}", row.get_ref(at).unwrap()))
            .collect();
        rows.push(cells.join("|"));
    }
    rows
}

/// Every table, index and trigger the store holds, by type and name.
fn shape(folder: &std::path::Path) -> Vec<(String, String)> {
    raw(folder)
        .prepare(
            "SELECT type, name FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'
             ORDER BY type, name",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn raw(folder: &std::path::Path) -> rusqlite::Connection {
    rusqlite::Connection::open(folder.join("sidecar.db")).unwrap()
}

fn schema_version(folder: &std::path::Path) -> u32 {
    raw(folder)
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap()
        .parse()
        .unwrap()
}

fn tables(folder: &std::path::Path) -> Vec<String> {
    raw(folder)
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
             ORDER BY name",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Every column of every row the version 1 store held, as text.
fn version_one_rows(folder: &std::path::Path) -> Vec<String> {
    let db = raw(folder);
    let mut rows = Vec::new();
    for query in [
        "SELECT 'kind', name, nature, '', '', '', '', '', '', '', '', '', '', '' FROM kind ORDER BY name",
        "SELECT 'run', id, source_kind, source_name, description, started_at, finished_at, outcome,
                '', '', '', '', '', '' FROM run ORDER BY id",
        "SELECT 'fact', id, track_id, fingerprint, kind, value, source_kind, source_name, confidence,
                run_id, recorded_at, stale_at, superseded_at, superseded_by FROM fact ORDER BY id",
    ] {
        let mut statement = db.prepare(query).unwrap();
        let mut found = statement.query([]).unwrap();
        while let Some(row) = found.next().unwrap() {
            let cells: Vec<String> = (0..14)
                .map(|at| format!("{:?}", row.get_ref(at).unwrap()))
                .collect();
            rows.push(cells.join("|"));
        }
    }
    rows
}

#[test]
fn a_version_one_store_is_brought_up_to_date_with_every_record_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let folder = version_one_store(&dir);
    let before = version_one_rows(&folder);
    assert_eq!(before.len(), 5, "two kinds, a Run and two Facts");
    assert_eq!(schema_version(&folder), 1);

    let sidecar = Sidecar::open(&folder).unwrap();

    assert_eq!(schema_version(&folder), 5, "the current version");
    for table in ["mutant", "mutant_change", "decision", "decision_answer"] {
        assert!(tables(&folder).contains(&table.to_owned()), "{table}");
    }
    assert_eq!(version_one_rows(&folder), before);
    // And as the crate reads them: value, provenance, Run and supersession.
    let facts = sidecar.facts("10003", "genre").unwrap();
    assert_eq!(facts.len(), 2);
    let (newer, older) = (&facts[0], &facts[1]);
    assert_eq!(
        (
            newer.id,
            &newer.value,
            newer.confidence,
            newer.run.0,
            newer.recorded_at
        ),
        (4, &json!("Tech House"), 0.8, 7, 1_794_400_000_500)
    );
    assert_eq!(newer.source, agent("tagger"));
    assert_eq!(newer.fingerprint.as_ref().unwrap().as_str(), "a1:00ff");
    assert!(newer.is_live() && !newer.is_stale());
    assert_eq!(
        (
            older.id,
            &older.value,
            older.confidence,
            older.superseded_at,
            older.superseded_by
        ),
        (3, &json!("Techno"), 0.7, Some(1_794_400_000_500), Some(4))
    );
    assert_eq!(older.fingerprint, None);
    let run = sidecar.run(wreckordbox_sidecar::RunId(7)).unwrap().unwrap();
    assert_eq!(run.description, "Tag the new arrivals");
    assert_eq!(run.outcome.as_deref(), Some("recorded 2 Facts"));
    assert_eq!(run.finished_at, Some(1_794_400_001_000));

    // The migrated store takes Decisions, and opens again as it is.
    let id = genre_question(&sidecar);
    sidecar
        .answer_decision(&answer(id, &agent("tagger"), "Techno", 0.6))
        .unwrap();
    let version = schema_version(&folder);
    let shape = tables(&folder);
    drop(sidecar);

    let again = Sidecar::open(&folder).unwrap();

    assert_eq!(schema_version(&folder), version);
    assert_eq!(tables(&folder), shape);
    assert_eq!(version_one_rows(&folder), before);
    let decision = again.decision(id).unwrap().unwrap();
    assert_eq!(decision.answers[0].choice, "Techno");
}

#[test]
fn a_migration_that_fails_part_way_leaves_version_one_intact_and_can_be_retried() {
    let dir = tempfile::tempdir().unwrap();
    let folder = version_one_store(&dir);
    let before = version_one_rows(&folder);
    // Something already has the name of the last thing the Decision tables'
    // statements make, so they fail after the tables themselves were made.
    raw(&folder)
        .execute_batch("CREATE TABLE decision_answer_by_decider (in_the_way INTEGER)")
        .unwrap();
    let shape = tables(&folder);

    let failed = Sidecar::open(&folder);

    assert!(
        matches!(failed, Err(SidecarError::Db(_))),
        "{:?}",
        failed.err()
    );
    assert_eq!(schema_version(&folder), 1);
    assert_eq!(
        tables(&folder),
        shape,
        "no table of a later version was left behind"
    );
    assert_eq!(version_one_rows(&folder), before);

    raw(&folder)
        .execute_batch("DROP TABLE decision_answer_by_decider")
        .unwrap();
    let sidecar = Sidecar::open(&folder).unwrap();

    assert_eq!(schema_version(&folder), 5, "the current version");
    assert_eq!(version_one_rows(&folder), before);
    let id = genre_question(&sidecar);
    assert!(sidecar.decision(id).unwrap().unwrap().pending);
}

#[test]
fn a_new_store_is_made_at_version_three_in_the_shape_an_old_one_is_brought_to() {
    let dir = tempfile::tempdir().unwrap();
    let fresh = dir.path().join("fresh");
    let sidecar = Sidecar::open(&fresh).unwrap();
    assert_eq!(schema_version(&fresh), 5, "the current version");
    let id = genre_question(&sidecar);
    sidecar
        .answer_decision(&answer(id, &agent("tagger"), "Techno", 0.6))
        .unwrap();
    drop(sidecar);

    let (one, two) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (from_one, from_two) = (version_one_store(&one), version_two_store(&two));
    drop(Sidecar::open(&from_one).unwrap());
    drop(Sidecar::open(&from_two).unwrap());

    assert_eq!(shape(&from_one), shape(&fresh));
    assert_eq!(shape(&from_two), shape(&fresh));
    let again = Sidecar::open(&fresh).unwrap();
    assert_eq!(schema_version(&fresh), 5, "the current version");
    assert_eq!(again.decision(id).unwrap().unwrap().answers.len(), 1);
}

#[test]
fn a_version_two_store_keeps_its_mutants_when_brought_to_version_three() {
    let dir = tempfile::tempdir().unwrap();
    let folder = version_two_store(&dir);
    assert_eq!(schema_version(&folder), 2);
    let before = (
        version_one_rows(&folder),
        rows_of(&folder, "mutant"),
        rows_of(&folder, "mutant_change"),
    );
    assert_eq!((before.1.len(), before.2.len()), (1, 1));

    let sidecar = Sidecar::open(&folder).unwrap();

    assert_eq!(schema_version(&folder), 5, "the current version");
    let after = (
        version_one_rows(&folder),
        rows_of(&folder, "mutant"),
        rows_of(&folder, "mutant_change"),
    );
    assert_eq!(after, before);
    // And as the crate reads it.
    let mutant = sidecar.mutant(2).unwrap().unwrap();
    assert_eq!(
        (
            mutant.entry_id.as_str(),
            mutant.track_id.as_str(),
            &mutant.edits,
            mutant.run.0
        ),
        ("400", "10003", &json!({ "gain_db": -3 }), 8)
    );
    assert!(mutant.is_live());
    assert_eq!(sidecar.mutant_changes(2).unwrap().len(), 1);
    let id = genre_question(&sidecar);
    sidecar
        .answer_decision(&answer(id, &agent("tagger"), "Techno", 0.6))
        .unwrap();
    drop(sidecar);

    let again = Sidecar::open(&folder).unwrap();

    assert_eq!(schema_version(&folder), 5, "the current version");
    assert_eq!(rows_of(&folder, "mutant"), before.1);
    assert_eq!(
        again.decision(id).unwrap().unwrap().answers[0].choice,
        "Techno"
    );
}

#[test]
fn a_migration_from_version_two_that_fails_part_way_leaves_version_two_intact() {
    let dir = tempfile::tempdir().unwrap();
    let folder = version_two_store(&dir);
    raw(&folder)
        .execute_batch("CREATE TABLE decision_answer_by_decider (in_the_way INTEGER)")
        .unwrap();
    let before = (
        shape(&folder),
        rows_of(&folder, "mutant"),
        version_one_rows(&folder),
    );

    assert!(matches!(Sidecar::open(&folder), Err(SidecarError::Db(_))));

    assert_eq!(schema_version(&folder), 2);
    let after = (
        shape(&folder),
        rows_of(&folder, "mutant"),
        version_one_rows(&folder),
    );
    assert_eq!(after, before, "nothing of version 3 was left behind");
    raw(&folder)
        .execute_batch("DROP TABLE decision_answer_by_decider")
        .unwrap();
    let sidecar = Sidecar::open(&folder).unwrap();
    assert_eq!(schema_version(&folder), 5, "the current version");
    assert!(sidecar.mutant(2).unwrap().unwrap().is_live());
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
        "INSERT OR REPLACE INTO decision (id, question_type, question, context, candidates, created_at)
         VALUES (1, 'yes_no', 'Replaced?', '{}', '[]', 0)",
        "REPLACE INTO decision_answer (id, decision_id, decider_kind, decider_name, choice,
                                       choice_probability, answered_at)
         VALUES (1, 1, 'agent', 'other', 'Tech House', 1.0, 0)",
        "INSERT OR REPLACE INTO decision_answer (id, decision_id, decider_kind, decider_name, choice,
                                       choice_probability, answered_at)
         SELECT id, decision_id, decider_kind, decider_name, 'Tech House', 1.0, 0 FROM decision_answer",
        "UPDATE decision SET question = 'Another?'",
        "DELETE FROM decision_answer",
        "UPDATE decision_answer SET choice = 'Tech House'",
        "DELETE FROM decision",
    ] {
        assert!(db.execute(statement, []).is_err(), "{statement}");
    }
    let decision = sidecar.decision(id).unwrap().unwrap();
    assert_eq!(decision.question, "Which genre is this track?");
    assert_eq!(decision.answers.len(), 1);
    assert_eq!(decision.answers[0].choice, "Techno");
    assert_eq!(decision.answers[0].decider, agent("tagger"));
    // A new row with an identifier of its own is still taken.
    db.execute(
        "INSERT INTO decision (id, question_type, question, context, candidates, created_at)
         VALUES (50, 'yes_no', 'New?', '{}', '[]', 0)",
        [],
    )
    .unwrap();
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

#[test]
fn the_choice_counts_once_in_the_total_when_it_is_left_out_of_the_probabilities() {
    let dir = tempfile::tempdir().unwrap();
    let sidecar = Sidecar::open(&dir.path().join("sidecar")).unwrap();
    let id = genre_question(&sidecar);
    let decider = agent("tagger");
    let with = |other: f64, stated: f64| {
        let all = BTreeMap::from([("Tech House".to_owned(), other)]);
        sidecar
            .answer_decision(&NewAnswer {
                probabilities: Some(&all),
                ..answer(id, &decider, "Techno", stated)
            })
            .map(|(_, answer)| answer)
    };

    // 0.8 for the choice and 0.7 for the other candidate is 1.5 in all.
    assert!(matches!(with(0.7, 0.8), Err(SidecarError::Probability(_))));
    assert!(sidecar.decision(id).unwrap().unwrap().pending);

    let kept = with(0.2, 0.8).unwrap();
    assert_eq!(kept.choice_probability, 0.8);
    assert_eq!(
        kept.probabilities.unwrap(),
        BTreeMap::from([("Tech House".to_owned(), 0.2), ("Techno".to_owned(), 0.8)]),
        "the choice's own probability is kept among the candidates'"
    );
}

fn genre_fact<'a>(run: RunOfAnswer<'a>) -> FactOfAnswer<'a> {
    FactOfAnswer {
        track_id: "10003",
        fingerprint: None,
        kind: Kind::judgement("genre"),
        value: json!("Tech House"),
        run,
    }
}

fn counts(folder: &std::path::Path) -> (i64, i64, i64) {
    let db = raw(folder);
    let count = |table: &str| {
        db.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    };
    (count("run"), count("fact"), count("decision_answer"))
}

#[test]
fn an_answer_and_its_fact_are_recorded_together_with_the_run_made_for_them() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("sidecar");
    let sidecar = Sidecar::open(&folder).unwrap();
    let id = genre_question(&sidecar);
    let decider = agent("tagger");

    let (decision, given, fact) = sidecar
        .answer_decision_with_fact(
            &answer(id, &decider, "Tech House", 0.8),
            &genre_fact(RunOfAnswer::Made {
                description: "Decision answered by tagger",
            }),
        )
        .unwrap();

    assert_eq!(given.fact_id, Some(fact.id));
    assert_eq!(given.run, Some(fact.run));
    assert_eq!(fact.source, decider);
    assert_eq!(fact.confidence, 0.8);
    assert_eq!(fact.value, json!("Tech House"));
    assert_eq!(decision.answers, [given]);
    let run = sidecar.run(fact.run).unwrap().unwrap();
    assert_eq!(run.description, "Decision answered by tagger");
    assert_eq!(run.source, decider);
    assert!(run.is_finished());

    // In an open Run: it is joined and stays open.
    let open = sidecar.begin_run(&decider, "A pass").unwrap();
    let (_, second, newer) = sidecar
        .answer_decision_with_fact(
            &answer(id, &decider, "Techno", 0.6),
            &FactOfAnswer {
                value: json!("Techno"),
                ..genre_fact(RunOfAnswer::Open(open))
            },
        )
        .unwrap();
    assert_eq!((second.run, newer.run), (Some(open), open));
    assert!(!sidecar.run(open).unwrap().unwrap().is_finished());
    assert_eq!(
        sidecar.fact(fact.id).unwrap().unwrap().superseded_by,
        Some(newer.id)
    );
}

#[test]
fn when_the_answer_cannot_be_written_no_fact_is_recorded_and_nothing_is_superseded() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("sidecar");
    let sidecar = Sidecar::open(&folder).unwrap();
    let id = genre_question(&sidecar);
    let decider = agent("tagger");
    let run = sidecar.begin_run(&decider, "Earlier").unwrap();
    let earlier = sidecar
        .record(&NewFact {
            track_id: "10003",
            fingerprint: None,
            kind: Kind::judgement("genre"),
            value: json!("Techno"),
            source: &decider,
            confidence: 0.7,
            run,
        })
        .unwrap();
    let before = counts(&folder);
    // The store refuses the answer's row, after the Fact's would be written.
    raw(&folder)
        .execute_batch(
            "CREATE TRIGGER injected_failure BEFORE INSERT ON decision_answer
             BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
        )
        .unwrap();

    let failed = sidecar.answer_decision_with_fact(
        &answer(id, &decider, "Tech House", 0.8),
        &genre_fact(RunOfAnswer::Made {
            description: "Decision answered by tagger",
        }),
    );

    assert!(
        matches!(failed, Err(SidecarError::Db(_))),
        "{:?}",
        failed.err()
    );
    assert_eq!(counts(&folder), before, "no Run, no Fact, no answer");
    let still = sidecar.fact(earlier.id).unwrap().unwrap();
    assert!(still.is_live(), "the earlier Fact was not superseded");
    assert_eq!(sidecar.facts("10003", "genre").unwrap(), [still]);
    assert!(sidecar.decision(id).unwrap().unwrap().pending);
}
