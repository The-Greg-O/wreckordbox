#![allow(clippy::pedantic, clippy::unwrap_used, clippy::expect_used)]
use crate::common::Sandbox;
use serde_json::Value;
fn ok(s: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let result = s.cli(&all);
    assert!(result.status.success(), "{}", result.stderr);
    result.json()
}
#[test]
fn runs_show_names_mutant_changes_and_decisions_with_the_original_source() {
    let s = Sandbox::new();
    let playlist = s.create_playlist("Fixture", &[1]);
    let entries = ok(&s, &["playlists", "tracks", &playlist]);
    let entry = entries["tracks"][0]["entry_id"].as_str().unwrap();
    let run = ok(
        &s,
        &[
            "runs",
            "start",
            "--source",
            "agent:fixture",
            "--description",
            "Native Sidecar history",
        ],
    )["id"]
        .to_string();
    let mutant = ok(
        &s,
        &[
            "mutants",
            "create",
            entry,
            "--source",
            "agent:fixture",
            "--run",
            &run,
        ],
    );
    let decision = ok(
        &s,
        &[
            "decisions",
            "create",
            "--type",
            "yes-no",
            "--question",
            "Keep?",
            "--source",
            "agent:fixture",
            "--run",
            &run,
        ],
    );
    let shown = ok(&s, &["runs", "show", &run]);
    assert_eq!(shown["total"], 2);
    assert_eq!(shown["entries"][0]["kind"], "mutant_change");
    assert_eq!(
        shown["entries"][0]["payload"]["row"]["mutant_id"],
        mutant["mutant"]["id"]
    );
    assert_eq!(shown["entries"][1]["kind"], "decision");
    assert_eq!(shown["entries"][1]["payload"]["row"]["id"], decision["id"]);
    assert!(shown["entries"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["source"]["name"] == "fixture"));
    assert_eq!(shown["run"]["changes"], 2);
}

#[test]
fn an_unspecified_decision_asker_never_inherits_a_dj_run() {
    let s = Sandbox::new();
    let run = ok(
        &s,
        &[
            "runs",
            "start",
            "--source",
            "dj",
            "--description",
            "DJ work",
        ],
    )["id"]
        .to_string();
    let before = ok(&s, &["decisions", "list"]);
    let refused = s.cli(&[
        "--json",
        "decisions",
        "create",
        "--type",
        "yes-no",
        "--question",
        "Agent question",
        "--run",
        &run,
    ]);
    assert!(
        !refused.status.success(),
        "unspecified tool source must not join a DJ Run: {}",
        refused.stdout
    );
    assert_eq!(ok(&s, &["decisions", "list"]), before);
    assert_eq!(ok(&s, &["runs", "show", &run])["total"], 0);
}

#[test]
fn a_decision_entry_records_the_asker_instead_of_the_runs_owner() {
    let s = Sandbox::new();
    let run = ok(
        &s,
        &[
            "runs",
            "start",
            "--source",
            "agent:coordinator",
            "--description",
            "Questions",
        ],
    )["id"]
        .to_string();
    let decision = ok(
        &s,
        &[
            "decisions",
            "create",
            "--type",
            "yes-no",
            "--question",
            "Keep?",
            "--run",
            &run,
        ],
    );
    let shown = ok(&s, &["runs", "show", &run]);
    assert_eq!(
        shown["entries"][0]["source"]["kind"], "tool",
        "an undeclared asker must not borrow the Run owner"
    );
    assert_eq!(shown["entries"][0]["payload"]["row"]["id"], decision["id"]);
}

#[test]
fn declared_decision_sources_are_recorded_and_non_djs_cannot_join_dj_runs() {
    let s = Sandbox::new();
    let run = ok(
        &s,
        &[
            "runs",
            "start",
            "--source",
            "dj",
            "--description",
            "DJ decisions",
        ],
    )["id"]
        .to_string();
    for source in ["agent:asker", "tool:analyzer"] {
        let out = s.cli(&[
            "--json",
            "decisions",
            "create",
            "--type",
            "yes-no",
            "--question",
            "Keep?",
            "--source",
            source,
            "--run",
            &run,
        ]);
        assert!(!out.status.success(), "{source} must not borrow a DJ Run");
    }
    let decision = ok(
        &s,
        &[
            "decisions",
            "create",
            "--type",
            "yes-no",
            "--question",
            "Keep?",
            "--source",
            "dj",
            "--run",
            &run,
        ],
    );
    let entry = ok(&s, &["runs", "show", &run]);
    assert_eq!(entry["entries"][0]["source"]["kind"], "dj");
    let before = ok(&s, &["decisions", "show", &decision["id"].to_string()]);
    for source in ["agent:answerer", "tool:rules"] {
        let out = s.cli(&[
            "--json",
            "decisions",
            "answer",
            &decision["id"].to_string(),
            "yes",
            "--decider",
            source,
            "--probability",
            "1",
            "--run",
            &run,
        ]);
        assert!(!out.status.success());
    }
    assert_eq!(
        ok(&s, &["decisions", "show", &decision["id"].to_string()]),
        before
    );
    let own = ok(
        &s,
        &[
            "decisions",
            "create",
            "--type",
            "yes-no",
            "--question",
            "Independent?",
            "--source",
            "agent:asker",
        ],
    );
    let shown = ok(&s, &["runs", "show", &own["run"].to_string()]);
    assert_eq!(
        shown["entries"][0]["source"],
        serde_json::json!({"kind":"agent","name":"asker"})
    );
    assert_eq!(shown["entries"][0]["payload"]["row"]["id"], own["id"]);
}
