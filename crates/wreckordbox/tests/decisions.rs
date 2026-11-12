//! `wreckordbox decisions` against a Library service on a throwaway Library
//! and Sidecar: the real binary, its output and exit status, and what a
//! second connected client is told.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod common;

use std::time::Duration;

use common::Sandbox;
use serde_json::{json, Value};

/// Runs a command that must succeed and answers its JSON result.
fn ok(sandbox: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let run = sandbox.cli(&all);
    assert!(run.status.success(), "{args:?}: {}", run.stderr);
    assert_eq!(run.stderr, "", "{args:?}");
    run.json()
}

/// Runs a command that must fail and answers its JSON error.
fn refused(sandbox: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let run = sandbox.cli(&all);
    assert!(!run.status.success(), "{args:?}: {}", run.stdout);
    assert_eq!(run.stdout, "", "{args:?}");
    run.error_json()["error"].clone()
}

/// Runs a command that must succeed and answers its text.
fn text(sandbox: &Sandbox, args: &[&str]) -> String {
    let run = sandbox.cli(args);
    assert!(run.status.success(), "{args:?}: {}", run.stderr);
    run.stdout
}

const GENRE_CONTEXT: &str =
    r#"{"title": "Midnight Drive", "file_genre": "tech-house", "bpm": 126}"#;

/// Asks which genre track 10003 is, between three, and answers the
/// Decision as created.
fn ask_genre(sandbox: &Sandbox) -> Value {
    ok(
        sandbox,
        &[
            "decisions",
            "create",
            "--type",
            "choice",
            "--question",
            "Which genre is this track?",
            "--context",
            GENRE_CONTEXT,
            "--candidate",
            "Techno",
            "--candidate",
            "Tech House",
            "--candidate",
            "House",
            "--track",
            "10003",
        ],
    )
}

/// The identifiers of the Decisions a listing answers, in its order.
fn ids(listing: &Value) -> Vec<i64> {
    listing["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|decision| decision["id"].as_i64().unwrap())
        .collect()
}

#[test]
fn a_decision_is_created_with_a_question_context_and_candidates_and_is_pending_until_answered() {
    let sandbox = Sandbox::new();

    let created = ask_genre(&sandbox);

    let id = created["id"].as_i64().unwrap();
    assert_eq!(created["question_type"], "choice");
    assert_eq!(created["question"], "Which genre is this track?");
    assert_eq!(
        created["context"],
        json!({ "title": "Midnight Drive", "file_genre": "tech-house", "bpm": 126 })
    );
    assert_eq!(
        created["candidates"],
        json!([{ "name": "Techno" }, { "name": "Tech House" }, { "name": "House" }])
    );
    assert_eq!(created["track_id"], "10003");
    assert_eq!(created["pending"], true);
    assert_eq!(created["answers"], json!([]));
    assert!(created["created_at"].as_i64().unwrap() > 1_700_000_000_000);

    let pending = ok(&sandbox, &["decisions", "pending"]);
    assert_eq!(pending["total"], 1);
    assert_eq!(pending["decisions"][0], created);
    assert_eq!(
        ok(&sandbox, &["decisions", "show", &id.to_string()]),
        created
    );
    assert_eq!(ok(&sandbox, &["decisions", "answered"])["total"], 0);

    ok(
        &sandbox,
        &[
            "decisions",
            "answer",
            &id.to_string(),
            "Tech House",
            "--decider",
            "agent:tagger",
            "--probability",
            "0.8",
        ],
    );

    assert_eq!(ok(&sandbox, &["decisions", "pending"])["total"], 0);
    assert_eq!(ids(&ok(&sandbox, &["decisions", "answered"])), [id]);
}

fn answer_as(sandbox: &Sandbox, id: i64, choice: &str, decider: &str, p: &str) -> Value {
    ok(
        sandbox,
        &[
            "decisions",
            "answer",
            &id.to_string(),
            choice,
            "--decider",
            decider,
            "--probability",
            p,
        ],
    )
}

#[test]
fn an_answered_decision_is_returned_with_its_choice_how_sure_and_its_decider() {
    let sandbox = Sandbox::new();
    let id = ask_genre(&sandbox)["id"].as_i64().unwrap();

    let answered = ok(
        &sandbox,
        &[
            "decisions",
            "answer",
            &id.to_string(),
            "Tech House",
            "--decider",
            "agent:tagger",
            "--probabilities",
            r#"{"Techno": 0.2, "Tech House": 0.7, "House": 0.1}"#,
            "--reason",
            "The file's own tag says so.",
        ],
    );

    let answer = &answered["answer"];
    assert_eq!(answer["choice"], "Tech House");
    assert_eq!(answer["choice_probability"], 0.7);
    assert_eq!(
        answer["probabilities"],
        json!({ "Techno": 0.2, "Tech House": 0.7, "House": 0.1 })
    );
    assert_eq!(
        answer["decider"],
        json!({ "kind": "agent", "name": "tagger" })
    );
    assert_eq!(answer["reason"], "The file's own tag says so.");
    assert_eq!(answer["fact_id"], Value::Null);
    assert!(
        answer.get("confidence").is_none(),
        "how sure is the probability of the choice, never a confidence"
    );
    assert_eq!(answered["fact"], Value::Null);

    let shown = ok(&sandbox, &["decisions", "show", &id.to_string()]);
    assert_eq!(shown["pending"], false);
    assert_eq!(shown["answers"], json!([answer]));
    assert_eq!(shown, answered["decision"]);
}

#[test]
fn an_answer_outside_the_candidates_is_refused_and_the_decision_stays_pending() {
    let sandbox = Sandbox::new();
    let id = ask_genre(&sandbox)["id"].as_i64().unwrap();

    let error = refused(
        &sandbox,
        &[
            "decisions",
            "answer",
            &id.to_string(),
            "Trance",
            "--decider",
            "agent:tagger",
            "--probability",
            "0.9",
            "--fact",
            "genre",
        ],
    );

    assert_eq!(error["code"], "answer_not_a_candidate");
    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains("Trance") && message.contains("Tech House"),
        "{message}"
    );
    assert_eq!(ids(&ok(&sandbox, &["decisions", "pending"])), [id]);
    assert_eq!(
        ok(&sandbox, &["facts", "show", "10003"])["facts"],
        json!([]),
        "a refused answer records no Fact"
    );

    // Exactly as named: another spelling is not the candidate.
    let error = refused(
        &sandbox,
        &[
            "decisions",
            "answer",
            &id.to_string(),
            "tech house",
            "--decider",
            "agent:tagger",
            "--probability",
            "0.9",
        ],
    );
    assert_eq!(error["code"], "answer_not_a_candidate");
}

#[test]
fn how_sure_must_be_said_and_be_a_probability() {
    let sandbox = Sandbox::new();
    let id = ask_genre(&sandbox)["id"].as_i64().unwrap();
    let id = id.to_string();

    let missing = refused(
        &sandbox,
        &[
            "decisions",
            "answer",
            &id,
            "House",
            "--decider",
            "agent:tagger",
        ],
    );
    assert_eq!(missing["code"], "invalid_probability");
    let over = refused(
        &sandbox,
        &[
            "decisions",
            "answer",
            &id,
            "House",
            "--decider",
            "agent:tagger",
            "--probability",
            "1.5",
        ],
    );
    assert_eq!(over["code"], "invalid_probability");
    let nameless = refused(
        &sandbox,
        &[
            "decisions",
            "answer",
            &id,
            "House",
            "--decider",
            "agent:",
            "--probability",
            "0.5",
        ],
    );
    assert_eq!(nameless["code"], "invalid_source");
    let unknown = refused(
        &sandbox,
        &["decisions", "answer", "999", "House", "--decider", "dj"],
    );
    assert_eq!(unknown["code"], "decision_not_found");
    assert_eq!(ok(&sandbox, &["decisions", "pending"])["total"], 1);

    // The DJ's answer is the DJ's word.
    let dj = ok(
        &sandbox,
        &["decisions", "answer", &id, "House", "--decider", "dj"],
    );
    assert_eq!(dj["answer"]["choice_probability"], 1.0);
}

#[test]
fn a_yes_or_no_decision_has_the_candidates_yes_and_no_and_keeps_both_probabilities() {
    let sandbox = Sandbox::new();

    let created = ok(
        &sandbox,
        &[
            "decisions",
            "create",
            "--type",
            "yes-no",
            "--topic",
            "vocal",
            "--question",
            "This track has a vocal.",
            "--context",
            r#"{"vocal_seconds": 41.5}"#,
        ],
    );
    let id = created["id"].as_i64().unwrap();
    assert_eq!(created["question_type"], "yes_no");
    assert_eq!(created["topic"], "vocal");
    assert_eq!(
        created["candidates"],
        json!([{ "name": "yes" }, { "name": "no" }])
    );
    assert_eq!(created["track_id"], Value::Null);

    let answered = answer_as(&sandbox, id, "no", "agent:tagger", "0.75");
    assert_eq!(answered["answer"]["choice"], "no");
    assert_eq!(answered["answer"]["choice_probability"], 0.75);
    assert_eq!(
        answered["answer"]["probabilities"],
        json!({ "yes": 0.25, "no": 0.75 })
    );

    let error = refused(
        &sandbox,
        &[
            "decisions",
            "answer",
            &id.to_string(),
            "maybe",
            "--decider",
            "agent:tagger",
            "--probability",
            "0.5",
        ],
    );
    assert_eq!(error["code"], "answer_not_a_candidate");

    let error = refused(
        &sandbox,
        &[
            "decisions",
            "create",
            "--type",
            "yes-no",
            "--question",
            "Is it?",
            "--candidate",
            "perhaps",
        ],
    );
    assert_eq!(error["code"], "invalid_candidates");
    let error = refused(
        &sandbox,
        &[
            "decisions",
            "create",
            "--type",
            "choice",
            "--question",
            "Which?",
            "--candidate",
            "One",
        ],
    );
    assert_eq!(error["code"], "invalid_candidates");
    let error = refused(
        &sandbox,
        &[
            "decisions",
            "create",
            "--type",
            "choice",
            "--question",
            "Which?",
            "--candidate",
            "One",
            "--candidate",
            "Two",
            "--context",
            "[1]",
        ],
    );
    assert_eq!(error["code"], "invalid_decision");
    let error = refused(
        &sandbox,
        &[
            "decisions",
            "create",
            "--type",
            "choice",
            "--question",
            "Which?",
            "--candidate",
            "One",
            "--candidate",
            "Two",
            "--track",
            "424242",
        ],
    );
    assert_eq!(error["code"], "track_not_found");
    assert_eq!(ok(&sandbox, &["decisions", "list"])["total"], 1);
}

#[test]
fn the_rule_based_decider_answers_with_a_choice_and_how_sure() {
    let sandbox = Sandbox::new();
    let id = ask_genre(&sandbox)["id"].as_i64().unwrap();

    // The context's file_genre is "tech-house": the candidate "Tech House"
    // once both are normalised.
    let decided = ok(
        &sandbox,
        &[
            "decisions",
            "decide",
            &id.to_string(),
            "--match",
            "file_genre",
        ],
    );

    let answer = &decided["answer"];
    assert_eq!(answer["choice"], "Tech House");
    assert_eq!(answer["choice_probability"], 0.9);
    assert_eq!(
        answer["decider"],
        json!({ "kind": "tool", "name": "rule-based" })
    );
    assert!(answer["reason"].as_str().unwrap().contains("file_genre"));
    assert_eq!(decided["decision"]["pending"], false);

    // Deterministic: asked again, the same answer, and both are kept.
    let again = ok(
        &sandbox,
        &[
            "decisions",
            "decide",
            &id.to_string(),
            "--match",
            "file_genre",
        ],
    );
    assert_eq!(again["answer"]["choice"], answer["choice"]);
    assert_eq!(
        again["answer"]["choice_probability"],
        answer["choice_probability"]
    );
    assert_eq!(again["answer"]["reason"], answer["reason"]);
    assert_eq!(again["decision"]["answers"].as_array().unwrap().len(), 2);

    // A rule that cannot answer says so and records nothing.
    let error = refused(
        &sandbox,
        &["decisions", "decide", &id.to_string(), "--match", "title"],
    );
    assert_eq!(error["code"], "rule_not_applicable");
    let error = refused(
        &sandbox,
        &[
            "decisions",
            "decide",
            &id.to_string(),
            "--threshold",
            "bpm",
            "--at-least",
            "120",
        ],
    );
    assert_eq!(error["code"], "rule_not_applicable");
    assert_eq!(
        ok(&sandbox, &["decisions", "show", &id.to_string()])["answers"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let fast = ok(
        &sandbox,
        &[
            "decisions",
            "create",
            "--type",
            "yes-no",
            "--question",
            "This track is fast.",
            "--context",
            r#"{"bpm": 126}"#,
        ],
    )["id"]
        .as_i64()
        .unwrap();
    let yes = ok(
        &sandbox,
        &[
            "decisions",
            "decide",
            &fast.to_string(),
            "--threshold",
            "bpm",
            "--at-least",
            "120",
        ],
    );
    assert_eq!(yes["answer"]["choice"], "yes");
    assert_eq!(yes["answer"]["choice_probability"], 1.0);
    assert_eq!(
        yes["answer"]["probabilities"],
        json!({ "yes": 1.0, "no": 0.0 })
    );
    let no = ok(
        &sandbox,
        &[
            "decisions",
            "decide",
            &fast.to_string(),
            "--threshold",
            "bpm",
            "--at-most",
            "120",
        ],
    );
    assert_eq!(no["answer"]["choice"], "no");
}

#[test]
fn an_agent_lists_what_it_has_not_answered_and_answers_under_its_own_name() {
    let sandbox = Sandbox::new();
    let first = ask_genre(&sandbox)["id"].as_i64().unwrap();
    let second = ask_genre(&sandbox)["id"].as_i64().unwrap();
    answer_as(&sandbox, first, "Techno", "agent:tagger", "0.6");

    // Pending for anyone: only the second. Pending for another Decider:
    // both, so the same question can be answered again.
    assert_eq!(ids(&ok(&sandbox, &["decisions", "pending"])), [second]);
    assert_eq!(
        ids(&ok(
            &sandbox,
            &["decisions", "pending", "--decider", "agent:other"]
        )),
        [first, second]
    );
    assert_eq!(
        ids(&ok(
            &sandbox,
            &["decisions", "pending", "--decider", "agent:tagger"]
        )),
        [second]
    );

    let other = answer_as(&sandbox, first, "Tech House", "agent:other", "0.9");

    let answers = other["decision"]["answers"].as_array().unwrap();
    assert_eq!(
        answers.len(),
        2,
        "the earlier answer is kept beside the new one"
    );
    assert_eq!(answers[0]["decider"]["name"], "tagger");
    assert_eq!(answers[0]["choice"], "Techno");
    assert_eq!(answers[1]["decider"]["name"], "other");
    assert_eq!(answers[1]["choice"], "Tech House");
    assert_eq!(
        ids(&ok(
            &sandbox,
            &["decisions", "answered", "--decider", "agent:other"]
        )),
        [first]
    );
    assert_eq!(ids(&ok(&sandbox, &["decisions", "list"])), [first, second]);
    assert_eq!(
        ids(&ok(&sandbox, &["decisions", "list", "--track", "10004"])),
        Vec::<i64>::new()
    );
    assert_eq!(
        ids(&ok(&sandbox, &["decisions", "list", "--topic", "genre"])),
        Vec::<i64>::new()
    );

    let page = ok(&sandbox, &["decisions", "list", "--limit", "1"]);
    assert_eq!(ids(&page), [first]);
    assert_eq!(page["total"], 2);
    assert_eq!(page["next_offset"], 1);
}

#[test]
fn an_answer_can_produce_a_fact_whose_source_is_the_decider() {
    let sandbox = Sandbox::new();
    let id = ask_genre(&sandbox)["id"].as_i64().unwrap();

    let answered = ok(
        &sandbox,
        &[
            "decisions",
            "answer",
            &id.to_string(),
            "Tech House",
            "--decider",
            "agent:tagger",
            "--probability",
            "0.8",
            "--fact",
            "genre",
        ],
    );

    let fact = &answered["fact"];
    assert_eq!(fact["track_id"], "10003");
    assert_eq!(fact["kind"], "genre");
    assert_eq!(fact["value"], "Tech House");
    assert_eq!(fact["source"], json!({ "kind": "agent", "name": "tagger" }));
    assert_eq!(fact["confidence"], 0.8);
    assert_eq!(answered["answer"]["fact_id"], fact["id"]);
    assert_eq!(answered["answer"]["run"], fact["run"]);
    assert_eq!(answered["run"]["id"], fact["run"]);
    let facts = ok(&sandbox, &["facts", "show", "10003"]);
    assert_eq!(facts["facts"], json!([fact]));

    // The rule-based Decider's Fact is the tool's, about a track named on
    // the command, with a value given outright.
    let decided = ok(
        &sandbox,
        &[
            "decisions",
            "decide",
            &id.to_string(),
            "--match",
            "file_genre",
            "--fact",
            "mood",
            "--fact-track",
            "10004",
            "--fact-value",
            r#"["Driving"]"#,
        ],
    );
    assert_eq!(decided["fact"]["track_id"], "10004");
    assert_eq!(decided["fact"]["value"], json!(["Driving"]));
    assert_eq!(
        decided["fact"]["source"],
        json!({ "kind": "tool", "name": "rule-based" })
    );
    assert_eq!(decided["fact"]["confidence"], 0.9);

    // A Decision about no track needs the track said; a kind that is not
    // declared is refused; neither leaves an answer behind.
    let loose = ok(
        &sandbox,
        &[
            "decisions",
            "create",
            "--type",
            "choice",
            "--question",
            "Which?",
            "--candidate",
            "House",
            "--candidate",
            "Techno",
        ],
    )["id"]
        .as_i64()
        .unwrap();
    let error = refused(
        &sandbox,
        &[
            "decisions",
            "answer",
            &loose.to_string(),
            "House",
            "--decider",
            "agent:tagger",
            "--probability",
            "0.8",
            "--fact",
            "genre",
        ],
    );
    assert_eq!(error["code"], "fact_needs_track");
    let error = refused(
        &sandbox,
        &[
            "decisions",
            "answer",
            &loose.to_string(),
            "House",
            "--decider",
            "agent:tagger",
            "--probability",
            "0.8",
            "--fact",
            "flavour",
            "--fact-track",
            "10003",
        ],
    );
    assert_eq!(error["code"], "kind_not_declared");
    assert_eq!(ids(&ok(&sandbox, &["decisions", "pending"])), [loose]);
}

#[test]
fn decision_changes_send_the_sidecar_event_and_never_touch_the_library() {
    let sandbox = Sandbox::new();
    let track_before = ok(&sandbox, &["tracks", "show", "10003"]);
    let mut watcher = sandbox.client();
    watcher.subscribe().unwrap();

    let id = ask_genre(&sandbox)["id"].as_i64().unwrap();
    let answered = answer_as(&sandbox, id, "House", "agent:tagger", "0.5");
    ok(&sandbox, &["decisions", "pending"]);
    ok(&sandbox, &["decisions", "show", &id.to_string()]);

    let mut events = Vec::new();
    while let Some(event) = watcher.next_event(Duration::from_millis(500)).unwrap() {
        events.push(event);
    }
    let seen: Vec<(&str, &Value)> = events
        .iter()
        .map(|event| (event.event.as_str(), &event.data))
        .collect();
    assert_eq!(
        seen,
        [
            (
                "sidecar_changed",
                &json!({ "track_id": "10003", "kind": "decision", "fact_id": null, "stale": [],
                         "decision_id": id, "answer_id": null })
            ),
            (
                "sidecar_changed",
                &json!({ "track_id": "10003", "kind": "decision", "fact_id": null, "stale": [],
                         "decision_id": id, "answer_id": answered["answer"]["id"] })
            ),
        ],
        "one Sidecar event per change, none for a read, and no library_changed"
    );
    assert_eq!(ok(&sandbox, &["tracks", "show", "10003"]), track_before);
}

#[test]
fn decisions_print_as_tables() {
    let sandbox = Sandbox::new();

    assert_eq!(
        text(&sandbox, &["decisions", "pending"]),
        "No pending Decisions.\n"
    );
    let id = ask_genre(&sandbox)["id"].as_i64().unwrap();

    let pending = text(&sandbox, &["decisions", "pending"]);
    let lines: Vec<&str> = pending.lines().collect();
    assert_eq!(
        lines,
        [
            "DECISION  TYPE    TOPIC  TRACK  QUESTION                    CANDIDATES                 ANSWERS",
            "1         choice  -      10003  Which genre is this track?  Techno, Tech House, House  pending",
        ]
    );

    let said = text(
        &sandbox,
        &[
            "decisions",
            "answer",
            &id.to_string(),
            "Tech House",
            "--decider",
            "agent:tagger",
            "--probability",
            "0.8",
            "--fact",
            "genre",
        ],
    );
    assert!(
        said.starts_with("Decision 1 answered by agent:tagger: Tech House (probability 0.8).\n"),
        "{said}"
    );
    assert!(
        said.contains("Recorded Fact 1 in Run 1: track 10003 genre = \"Tech House\"."),
        "{said}"
    );

    let answered = text(&sandbox, &["decisions", "answered"]);
    assert!(
        answered
            .lines()
            .nth(1)
            .unwrap()
            .ends_with("Tech House 0.8 (agent:tagger)"),
        "{answered}"
    );

    let shown = text(&sandbox, &["decisions", "show", &id.to_string()]);
    assert!(
        shown.starts_with("Decision 1 (choice)\nQuestion    Which genre is this track?\n"),
        "{shown}"
    );
    let header = shown
        .lines()
        .find(|line| line.starts_with("ANSWER"))
        .unwrap();
    assert_eq!(
        header.split_whitespace().collect::<Vec<_>>(),
        [
            "ANSWER",
            "DECIDER",
            "CHOICE",
            "PROBABILITY",
            "FACT",
            "RUN",
            "ANSWERED",
            "REASON"
        ]
    );
    assert!(!shown.to_lowercase().contains("confidence"), "{shown}");

    // Refusals in text name the code's reason and exit non-zero.
    let run = sandbox.cli(&[
        "decisions",
        "answer",
        &id.to_string(),
        "Trance",
        "--decider",
        "agent:tagger",
        "--probability",
        "0.5",
    ]);
    assert!(!run.status.success());
    assert!(
        run.stderr
            .contains("\"Trance\" is not a candidate of Decision 1"),
        "{}",
        run.stderr
    );
}
