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

const GENRE_CONTEXT: &str = r#"{"title": "Midnight Drive", "file_genre": "tech-house", "bpm": 126}"#;

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
    assert_eq!(ok(&sandbox, &["decisions", "show", &id.to_string()]), created);
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
