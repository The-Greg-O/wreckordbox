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
