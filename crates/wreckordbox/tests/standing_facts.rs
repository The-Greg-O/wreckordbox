//! Standing Facts through the real CLI and a fixture service.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
mod common;
use common::Sandbox;
use serde_json::{json, Value};
use std::time::Duration;

fn ok(s: &Sandbox, args: &[&str]) -> Value {
    let mut command = vec!["--json"];
    command.extend_from_slice(args);
    let result = s.cli(&command);
    assert!(result.status.success(), "{args:?}: {}", result.stderr);
    result.json()
}
fn record(s: &Sandbox, source: &str, value: &str, confidence: &str) -> Value {
    ok(
        s,
        &[
            "facts",
            "record",
            "10003",
            "energy",
            value,
            "--source",
            source,
            "--confidence",
            confidence,
        ],
    )["facts"][0]
        .clone()
}
fn standing(s: &Sandbox) -> Value {
    ok(s, &["facts", "standing", "10003", "energy"])
}
#[test]
fn the_dj_stands_over_every_source_and_the_corrected_answers_remain_readable() {
    let s = Sandbox::new();
    ok(&s, &["status"]);
    let mut watcher = s.client();
    watcher.subscribe().unwrap();
    let agent = record(&s, "agent:tagger", "5", "1");
    let dj = record(&s, "dj", "2", "0.1");
    let tool = record(&s, "tool:meter", "4", "1");
    let web = record(&s, "web:https://example.test/track", "3", "1");
    for _ in 0..4 {
        assert_eq!(
            watcher
                .next_event(Duration::from_secs(2))
                .unwrap()
                .unwrap()
                .event,
            "sidecar_changed"
        );
    }
    let before = ok(&s, &["runs", "list"]);
    let result = standing(&s);
    assert_eq!(result["standing"]["id"], dj["id"]);
    assert_eq!(result["reason"]["code"], "dj");
    for old in [&agent, &tool, &web] {
        let kept = result["superseded"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["id"] == old["id"])
            .unwrap();
        assert_eq!(kept["value"], old["value"]);
        assert_eq!(kept["source"], old["source"]);
        assert_eq!(kept["superseded_by"], dj["id"]);
    }
    assert_eq!(ok(&s, &["runs", "list"]), before);
    assert!(watcher
        .next_event(Duration::from_millis(200))
        .unwrap()
        .is_none());
    assert_eq!(ok(&s, &["tracks", "show", "10003"])["rating"], 0);
}
#[test]
fn source_rules_reload_and_confidence_breaks_ties_without_rewriting_facts() {
    let s = Sandbox::new();
    let trusted = record(&s, "agent:trusted", "2", "0.2");
    let certain = record(&s, "tool:meter", "4", "0.99");
    assert_eq!(standing(&s)["standing"]["id"], certain["id"]);
    assert_eq!(standing(&s)["reason"]["code"], "confidence");
    let file = s.sidecar_dir().join("source-precedence.toml");
    std::fs::write(
        &file,
        "[sources]\n\"agent:trusted\" = 10\n\"tool:meter\" = -2\n",
    )
    .unwrap();
    let result = standing(&s);
    assert_eq!(result["standing"]["id"], trusted["id"]);
    assert_eq!(result["reason"]["code"], "source_precedence");
    assert_eq!(result["rules_file"], file.to_str().unwrap());
    std::fs::write(&file, "[sources]\n\"tool:meter\" = 20\n").unwrap();
    assert_eq!(standing(&s)["standing"]["id"], certain["id"]);
    let dj = record(&s, "dj", "1", "0");
    assert_eq!(standing(&s)["standing"]["id"], dj["id"]);
    std::fs::write(&file, "[sources]\n\"agent:trusted\" = \"oops\"\n").unwrap();
    let refused = s.cli(&["--json", "facts", "standing", "10003", "energy"]);
    assert!(!refused.status.success());
    assert_eq!(
        refused.error_json()["error"]["code"],
        "source_precedence_invalid"
    );
    assert!(refused.error_json()["error"]["message"]
        .as_str()
        .unwrap()
        .contains("source-precedence.toml"));
}
#[test]
fn querying_an_empty_sidecar_creates_nothing_and_is_readable_with_protection_on() {
    let s = Sandbox::new();
    s.protect(true);
    let result = standing(&s);
    assert_eq!(result["standing"], Value::Null);
    assert_eq!(result["reason"]["code"], "no_eligible_fact");
    assert_eq!(result["superseded"], json!([]));
    assert!(!s.sidecar_dir().exists());
    let refused = s.cli(&["--json", "tracks", "--source", "dj", "rate", "10003", "4"]);
    assert!(!refused.status.success());
    assert_eq!(refused.error_json()["error"]["code"], "library_protected");
    assert!(!s.sidecar_dir().exists());
}
