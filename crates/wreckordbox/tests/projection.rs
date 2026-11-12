//! Projecting Energy and Prep colour (wbx-26) through the real CLI and a
//! service on a fixture Library, with a second client watching.
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
    result.error_json()
}

fn policy(s: &Sandbox, text: &str) {
    std::fs::create_dir_all(s.sidecar_dir()).unwrap();
    std::fs::write(s.sidecar_dir().join("projection.toml"), text).unwrap();
}

fn fact(s: &Sandbox, track: &str, kind: &str, value: &str, source: &str, confidence: &str) -> Value {
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
    )["facts"][0]
        .clone()
}

fn track(s: &Sandbox, id: &str) -> Value {
    ok(s, &["tracks", "show", id])
}

/// The changes of a report as `(track, field, before, after)`, sorted.
fn changes(report: &Value, key: &str) -> Vec<(String, String, Value, Value)> {
    let mut list: Vec<_> = report[key]
        .as_array()
        .unwrap_or_else(|| panic!("no {key} in {report}"))
        .iter()
        .map(|change| {
            (
                change["track"].as_str().unwrap().to_owned(),
                change["field"].as_str().unwrap().to_owned(),
                change["before"].clone(),
                change["after"].clone(),
            )
        })
        .collect();
    list.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    list
}

fn drain(watcher: &mut wreckordbox_service::Client) {
    while watcher
        .next_event(Duration::from_millis(300))
        .unwrap()
        .is_some()
    {}
}

#[test]
fn a_dry_run_lists_exactly_what_applying_then_does_as_one_run_and_twice_changes_nothing() {
    let s = Sandbox::new();
    s.dj_prep_colours();
    policy(
        &s,
        "[energy]\napply = 0.8\n\n[prep_colour]\napply = 0.5\n",
    );
    ok(&s, &["status"]);
    let energy = fact(&s, "10001", "energy", "4", "agent:tagger", "0.9");
    fact(&s, "10002", "energy", "2", "tool:meter", "0.8");
    fact(&s, "10003", "prep_colour", "Green", "agent:tagger", "0.6");
    let mut watcher = s.client();
    watcher.subscribe().unwrap();
    let runs_before = ok(&s, &["runs", "list"]);

    let dry = ok(&s, &["project", "--dry-run"]);
    assert_eq!(dry["dry_run"], true);
    assert_eq!(
        changes(&dry, "changes"),
        vec![
            ("10001".into(), "energy".into(), json!(null), json!(4)),
            ("10002".into(), "energy".into(), json!(null), json!(2)),
            (
                "10003".into(),
                "prep_colour".into(),
                json!(null),
                json!({"id": 5, "colour": "Green", "label": "Clean"})
            ),
        ]
    );
    let first = dry["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|change| change["track"] == "10001")
        .unwrap();
    assert_eq!(first["fact"], energy["id"]);
    assert_eq!(first["source"], json!({"kind": "agent", "name": "tagger"}));
    // A dry run writes nothing: no Library change, no Run, no event.
    assert_eq!(track(&s, "10001")["rating"], 0);
    assert_eq!(ok(&s, &["runs", "list"]), runs_before);
    assert!(watcher
        .next_event(Duration::from_millis(300))
        .unwrap()
        .is_none());

    let applied = ok(&s, &["project"]);
    assert_eq!(applied["dry_run"], false);
    assert_eq!(changes(&applied, "changes"), changes(&dry, "changes"));
    assert_eq!(applied["plan"], dry["plan"]);
    assert_eq!(track(&s, "10001")["rating"], 4);
    assert_eq!(track(&s, "10002")["rating"], 2);
    assert_eq!(track(&s, "10003")["colour_id"], 5);
    // One edit, so the window hears of it once and live.
    let event = watcher.next_event(Duration::from_secs(2)).unwrap().unwrap();
    assert_eq!(event.event, "library_changed");
    // One Run, by a tool and never the DJ, holding exactly those changes.
    let run = &applied["run"];
    assert_eq!(run["source"]["kind"], "tool");
    assert_eq!(run["source"]["name"], "projection");
    let shown = ok(&s, &["runs", "show", &run["id"].to_string()]);
    let library: Vec<&Value> = shown["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["store"] == "library")
        .collect();
    assert_eq!(library.len(), 1, "{shown}");
    assert_eq!(library[0]["reversible"], true);
    let mut values: Vec<(String, String)> = library[0]["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| (value["subject"].to_string(), value["after"].to_string()))
        .collect();
    values.sort();
    assert_eq!(values.len(), 3, "{shown}");
    // No DJ Fact was made by the Projection's edit.
    let standing = ok(&s, &["facts", "standing", "10001", "energy"]);
    assert_eq!(standing["standing"]["source"]["kind"], "agent");

    drain(&mut watcher);
    let runs_after = ok(&s, &["runs", "list"]);
    let again = ok(&s, &["project"]);
    assert_eq!(again["changes"], json!([]));
    assert!(again.get("run").is_none() || again["run"].is_null(), "{again}");
    assert_eq!(ok(&s, &["runs", "list"]), runs_after);
    assert!(watcher
        .next_event(Duration::from_millis(300))
        .unwrap()
        .is_none());
}
