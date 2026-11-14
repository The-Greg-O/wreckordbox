//! Reviewing waiting Facts (wbx-27) through the real CLI and a service on a
//! fixture Library: the Facts Projection held back are listed; the DJ
//! accepts them (a hand set, the DJ's, through the edit path) or rejects
//! them (recorded, never offered again, nothing written to the Library).
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
use crate::common;

use std::time::Duration;

use common::Sandbox;
use serde_json::{json, Value};

fn ok(s: &Sandbox, args: &[&str]) -> Value {
    let mut words = vec!["--json"];
    words.extend_from_slice(args);
    let result = s.cli(&words);
    assert!(
        result.status.success(),
        "{args:?}: {}{}",
        result.stdout,
        result.stderr
    );
    result.json()
}

fn refused(s: &Sandbox, args: &[&str]) -> Value {
    let mut words = vec!["--json"];
    words.extend_from_slice(args);
    let result = s.cli(&words);
    assert!(!result.status.success(), "{args:?}: {}", result.stdout);
    result.error_json()["error"].clone()
}

fn policy(s: &Sandbox, text: &str) {
    std::fs::create_dir_all(s.sidecar_dir()).unwrap();
    std::fs::write(s.sidecar_dir().join("projection-policy.toml"), text).unwrap();
}

fn fact(s: &Sandbox, track: &str, kind: &str, value: &str, source: &str, confidence: &str) -> i64 {
    ok(
        s,
        &[
            "facts",
            "record",
            track,
            kind,
            value,
            "--source",
            source,
            "--confidence",
            confidence,
        ],
    )["facts"][0]["id"]
        .as_i64()
        .unwrap()
}

fn waiting(s: &Sandbox) -> Vec<Value> {
    ok(s, &["review", "waiting"])["waiting"]
        .as_array()
        .unwrap()
        .clone()
}

fn facts_waiting(s: &Sandbox) -> Vec<i64> {
    waiting(s)
        .iter()
        .map(|w| w["fact"].as_i64().unwrap())
        .collect()
}

fn rating(s: &Sandbox, id: &str) -> Value {
    ok(s, &["tracks", "show", id])["rating"].clone()
}

/// Every file in the Sidecar's folder with its bytes.
fn sidecar_bytes(s: &Sandbox) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(s.sidecar_dir()) {
        for entry in entries.flatten() {
            if entry.path().is_file() {
                out.push((
                    entry.file_name().to_string_lossy().into_owned(),
                    std::fs::read(entry.path()).unwrap(),
                ));
            }
        }
    }
    out.sort();
    out
}

fn drain(watcher: &mut wreckordbox_service::Client) {
    while watcher
        .next_event(Duration::from_millis(300))
        .unwrap()
        .is_some()
    {}
}

#[test]
fn facts_held_back_by_policy_are_listed_with_track_value_source_and_confidence_and_reading_writes_nothing(
) {
    let s = Sandbox::new();
    s.dj_prep_colours();
    policy(&s, "[energy]\napply = 0.8\n");
    ok(&s, &["status"]);
    let low = fact(&s, "10001", "energy", "4", "agent:tagger", "0.6");
    let colour = fact(&s, "10002", "prep_colour", "Green", "tool:meter", "0.9");
    // Above the threshold: it would apply, it is not waiting.
    fact(&s, "10003", "energy", "2", "agent:tagger", "0.95");
    let mut watcher = s.client();
    watcher.subscribe().unwrap();
    let runs_before = ok(&s, &["runs", "list"]);
    let bytes_before = sidecar_bytes(&s);
    let library_before = std::fs::read(s.master_db()).unwrap();

    let listed = waiting(&s);
    assert_eq!(listed.len(), 2, "{listed:?}");
    let energy = listed.iter().find(|w| w["fact"] == low).unwrap();
    assert_eq!(energy["track"], "10001");
    assert_eq!(energy["field"], "energy");
    assert_eq!(energy["before"], Value::Null);
    assert_eq!(energy["after"], 4);
    assert_eq!(energy["source"], json!({"kind": "agent", "name": "tagger"}));
    assert_eq!(energy["confidence"], 0.6);
    assert_eq!(energy["reason"], "below_confidence");
    let green = listed.iter().find(|w| w["fact"] == colour).unwrap();
    assert_eq!(green["track"], "10002");
    assert_eq!(green["after"]["colour"], "Green");
    assert_eq!(green["source"], json!({"kind": "tool", "name": "meter"}));
    assert_eq!(green["confidence"], 0.9);
    assert_eq!(green["reason"], "waits_for_dj");

    // The readable form names them too.
    let text = s.cli(&["review", "waiting"]);
    assert!(text.status.success(), "{}", text.stderr);
    assert!(text.stdout.contains("10001"), "{}", text.stdout);
    assert!(text.stdout.contains("agent:tagger"), "{}", text.stdout);
    assert!(text.stdout.contains("0.6"), "{}", text.stdout);

    // Reading wrote nothing: no Run, no Sidecar byte, no Library byte, no
    // event.
    assert_eq!(ok(&s, &["runs", "list"]), runs_before);
    assert_eq!(sidecar_bytes(&s), bytes_before);
    assert_eq!(std::fs::read(s.master_db()).unwrap(), library_before);
    assert!(watcher
        .next_event(Duration::from_millis(300))
        .unwrap()
        .is_none());
}
