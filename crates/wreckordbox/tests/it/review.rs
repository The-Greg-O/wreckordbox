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

#[test]
fn accepting_a_fact_is_the_djs_hand_set_in_the_djs_run_and_it_leaves_the_queue() {
    let s = Sandbox::new();
    s.dj_prep_colours();
    // No policy file: everything waits for the DJ.
    ok(&s, &["status"]);
    let energy = fact(&s, "10001", "energy", "4", "agent:tagger", "0.7");
    let other = fact(&s, "10002", "energy", "2", "agent:tagger", "0.7");
    let mut watcher = s.client();
    watcher.subscribe().unwrap();

    let accepted = ok(
        &s,
        &["review", "accept", &energy.to_string(), "--source", "dj"],
    );
    assert_eq!(accepted["count"], 1, "{accepted}");
    assert_eq!(accepted["decided"][0]["fact"], energy);
    assert_eq!(rating(&s, "10001"), 4);
    // Only the one named: the other still waits, its track untouched.
    assert_eq!(rating(&s, "10002"), 0);
    assert_eq!(facts_waiting(&s), vec![other]);
    // The window hears of the edit, once.
    let event = watcher.next_event(Duration::from_secs(2)).unwrap().unwrap();
    assert_eq!(event.event, "library_changed");
    drain(&mut watcher);

    // In the DJ's Run, one Library entry, Undo-able, by the DJ.
    let run = &accepted["run"];
    assert_eq!(run["source"]["kind"], "dj", "{accepted}");
    let shown = ok(&s, &["runs", "show", &run["id"].to_string()]);
    let library: Vec<&Value> = shown["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["store"] == "library")
        .collect();
    assert_eq!(library.len(), 1, "{shown}");
    assert_eq!(library[0]["reversible"], true);
    // The value is the DJ's now: the DJ's Fact stands.
    let standing = ok(&s, &["facts", "standing", "10001", "energy"]);
    assert_eq!(standing["standing"]["source"]["kind"], "dj", "{standing}");
    assert_eq!(standing["standing"]["value"], 4);

    // And a stronger agent Fact is never projected over it.
    policy(&s, "[energy]\napply = 0.1\n");
    fact(&s, "10001", "energy", "1", "agent:tagger", "0.99");
    ok(&s, &["project"]);
    assert_eq!(rating(&s, "10001"), 4);
}

#[test]
fn only_the_dj_accepts_and_library_protection_refuses_an_accept_with_nothing_written() {
    let s = Sandbox::new();
    ok(&s, &["status"]);
    let energy = fact(&s, "10001", "energy", "4", "agent:tagger", "0.7");
    let id = energy.to_string();
    let runs = ok(&s, &["runs", "list"]);

    for source in [None, Some("agent:tagger"), Some("tool:meter")] {
        let mut args = vec!["review", "accept", id.as_str()];
        if let Some(source) = source {
            args.extend(["--source", source]);
        }
        assert_eq!(
            refused(&s, &args)["code"],
            "review_not_the_djs",
            "{source:?}"
        );
    }
    assert_eq!(rating(&s, "10001"), 0);
    assert_eq!(ok(&s, &["runs", "list"]), runs);

    s.protect(true);
    let listed = ok(&s, &["review", "waiting"]);
    assert_eq!(listed["library_protected"], true);
    assert_eq!(
        refused(&s, &["review", "accept", &id, "--source", "dj"])["code"],
        "library_protected"
    );
    assert_eq!(rating(&s, "10001"), 0);
    assert_eq!(ok(&s, &["runs", "list"]), runs);
    // Still waiting: a refused accept decided nothing.
    assert_eq!(facts_waiting(&s), vec![energy]);
    s.protect(false);

    // A Fact that is not waiting is refused, and nothing else is accepted
    // with it.
    let error = refused(&s, &["review", "accept", &id, "999999", "--source", "dj"]);
    assert_eq!(error["code"], "review_not_waiting", "{error}");
    assert_eq!(rating(&s, "10001"), 0);
    // A selection must say what it chooses.
    assert_eq!(
        refused(&s, &["review", "accept", "--source", "dj"])["code"],
        "bad_request"
    );
}

#[test]
fn rejecting_records_the_djs_decision_writes_nothing_to_the_library_and_is_never_offered_again() {
    let s = Sandbox::new();
    ok(&s, &["status"]);
    let rejected = fact(&s, "10001", "energy", "4", "agent:tagger", "0.7");
    let library_before = std::fs::read(s.master_db()).unwrap();
    let mut watcher = s.client();
    watcher.subscribe().unwrap();

    // Not the DJ's: refused, nothing recorded.
    let runs = ok(&s, &["runs", "list"]);
    for source in [None, Some("agent:tagger")] {
        let mut args = vec!["review".to_owned(), "reject".into(), rejected.to_string()];
        if let Some(source) = source {
            args.extend(["--source".into(), source.into()]);
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        assert_eq!(refused(&s, &args)["code"], "review_not_the_djs");
    }
    assert_eq!(ok(&s, &["runs", "list"]), runs);

    let done = ok(
        &s,
        &["review", "reject", &rejected.to_string(), "--source", "dj"],
    );
    assert_eq!(done["count"], 1, "{done}");
    // Recorded in the Ledger as the DJ's, in a Run of the DJ's.
    let run = done["rejection"]["run"].to_string();
    let shown = ok(&s, &["runs", "show", &run]);
    assert_eq!(shown["run"]["source"]["kind"], "dj", "{shown}");
    let entries = shown["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{shown}");
    assert_eq!(entries[0]["store"], "sidecar");
    // The Library is not written, and no client hears of a Library change.
    assert_eq!(std::fs::read(s.master_db()).unwrap(), library_before);
    assert_eq!(rating(&s, "10001"), 0);
    assert!(watcher
        .next_event(Duration::from_millis(300))
        .unwrap()
        .is_none());

    // Never offered again, never projected, even under a policy that would
    // apply it; a second rejection of it is refused.
    assert!(facts_waiting(&s).is_empty());
    policy(&s, "[energy]\napply = 0.1\n");
    assert!(facts_waiting(&s).is_empty());
    let applied = ok(&s, &["project"]);
    assert_eq!(applied["changes"], json!([]), "{applied}");
    assert_eq!(applied["rejected"][0]["fact"], rejected, "{applied}");
    assert_eq!(rating(&s, "10001"), 0);
    assert_eq!(
        refused(
            &s,
            &["review", "reject", &rejected.to_string(), "--source", "dj"]
        )["code"],
        "review_not_waiting"
    );

    // Another source's answer for the same field is a new suggestion.
    policy(&s, "");
    let other = fact(&s, "10001", "energy", "3", "tool:meter", "0.5");
    assert_eq!(facts_waiting(&s), vec![other]);
}

#[test]
fn bulk_decisions_by_filter_and_all_empty_the_queue_in_one_undo_step() {
    let s = Sandbox::new();
    s.dj_prep_colours();
    ok(&s, &["status"]);
    let a = fact(&s, "10001", "energy", "4", "agent:tagger", "0.9");
    let b = fact(&s, "10002", "energy", "2", "agent:tagger", "0.4");
    let c = fact(&s, "10003", "energy", "5", "tool:meter", "0.9");
    let d = fact(&s, "10004", "prep_colour", "Green", "agent:tagger", "0.9");
    let e = fact(&s, "10005", "prep_colour", "Red", "agent:tagger", "0.3");
    assert_eq!(facts_waiting(&s).len(), 5);

    // A filter: the tagger's Energy at or above 0.5 — only a.
    let one = ok(
        &s,
        &[
            "review",
            "waiting",
            "--field",
            "energy",
            "--from",
            "agent:tagger",
            "--at-least",
            "0.5",
        ],
    );
    assert_eq!(one["count"], 1, "{one}");
    assert_eq!(one["waiting"][0]["fact"], a);

    // Accept everything above 0.5 at once: one Library entry, one Undo
    // step, in one DJ Run.
    let accepted = ok(
        &s,
        &["review", "accept", "--at-least", "0.5", "--source", "dj"],
    );
    let mut decided: Vec<i64> = accepted["decided"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["fact"].as_i64().unwrap())
        .collect();
    decided.sort_unstable();
    assert_eq!(decided, vec![a, c, d], "{accepted}");
    assert_eq!(rating(&s, "10001"), 4);
    assert_eq!(rating(&s, "10003"), 5);
    assert_eq!(ok(&s, &["tracks", "show", "10004"])["colour_id"], 5);
    let shown = ok(&s, &["runs", "show", &accepted["run"]["id"].to_string()]);
    let library: Vec<&Value> = shown["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["store"] == "library")
        .collect();
    assert_eq!(library.len(), 1, "one Undo step: {shown}");
    assert_eq!(library[0]["values"].as_array().unwrap().len(), 3, "{shown}");

    // Reject the rest, all at once; the queue is then empty.
    let mut left = facts_waiting(&s);
    left.sort_unstable();
    assert_eq!(left, vec![b, e]);
    let rejected = ok(&s, &["review", "reject", "--all", "--source", "dj"]);
    assert_eq!(rejected["count"], 2, "{rejected}");
    assert!(facts_waiting(&s).is_empty());
    let text = s.cli(&["review", "waiting"]);
    assert!(text.stdout.contains("Nothing is waiting"), "{}", text.stdout);
    // Deciding all of an empty queue does nothing.
    let runs = ok(&s, &["runs", "list"]);
    assert_eq!(
        ok(&s, &["review", "accept", "--all", "--source", "dj"])["count"],
        0
    );
    assert_eq!(ok(&s, &["runs", "list"]), runs);
}
