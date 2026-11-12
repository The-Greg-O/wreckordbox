//! Mutant edits through the real CLI and fixture service.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
mod common;
use common::Sandbox;
use serde_json::{json, Value};

fn ok(s: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let out = s.cli(&all);
    assert!(out.status.success(), "{args:?}: {}", out.stderr);
    out.json()
}
fn refused(s: &Sandbox, args: &[&str]) -> String {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let out = s.cli(&all);
    assert!(!out.status.success(), "{args:?}: {}", out.stdout);
    out.error_json()["error"]["code"].as_str().unwrap().into()
}
fn make(s: &Sandbox, playlist: &str) -> String {
    ok(
        s,
        &[
            "mutants",
            "create",
            "--playlist",
            playlist,
            "--position",
            "1",
            "--source",
            "dj",
        ],
    )["mutant"]["id"]
        .to_string()
}
fn edit(s: &Sandbox, id: &str, edits: Value) -> Value {
    ok(
        s,
        &[
            "mutants",
            "edit",
            id,
            "--edits",
            &edits.to_string(),
            "--source",
            "agent:tester",
        ],
    )
}
#[test]
fn edits_combine_derive_and_clear_without_changing_a_protected_parent() {
    let s = Sandbox::new();
    s.describe_track(2, "Parent", "Artist", "8A", 3);
    s.edit(|w| {
        w.add_cue("10002", 1, 250).unwrap();
        w.add_cue("10002", 0, 500).unwrap();
    });
    let playlist = s.create_playlist("Set", &[2]);
    s.protect(true);
    let parent = ok(&s, &["tracks", "show", "10002"]);
    let id = make(&s, &playlist);
    let initial = ok(&s, &["mutants", "show", &id]);
    assert_eq!(
        initial["mutant"]["derived"]["cues"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        refused(&s, &["tracks", "rate", "10002", "5"]),
        "library_protected"
    );
    let edited = edit(&s, &id, json!({"key_shift":1,"tempo":1.1}));
    assert_eq!(
        edited["mutant"]["edits"],
        json!({"key_shift":1,"tempo":1.1})
    );
    assert_eq!(edited["mutant"]["derived"]["key"], "3A");
    assert!((edited["mutant"]["derived"]["bpm"].as_f64().unwrap() - 128.02 * 1.1).abs() < 1e-8);
    assert_eq!(
        edit(&s, &id, json!({"cues":[]}))["mutant"]["derived"]["cues"],
        json!([])
    );
    for field in ["key_shift", "tempo", "cues"] {
        ok(&s, &["mutants", "clear", &id, field, "--source", "dj"]);
    }
    let cleared = ok(&s, &["mutants", "show", &id]);
    assert_eq!(cleared["mutant"]["edits"], json!({}));
    assert_eq!(cleared["mutant"]["derived"], initial["mutant"]["derived"]);
    assert_eq!(ok(&s, &["tracks", "show", "10002"]), parent);
}

#[test]
fn reads_do_not_create_a_sidecar() {
    let s = Sandbox::new();
    assert_eq!(ok(&s, &["mutants", "list"])["total"], 0);
    assert_eq!(refused(&s, &["mutants", "show", "99"]), "mutant_not_found");
    assert!(!s.sidecar_dir().exists());
}
