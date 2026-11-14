//! What the Review view (wbx-74) needs from the service beyond wbx-27,
//! through the real CLI and a service on a fixture Library: each waiting
//! Fact's Run, so the view can group by Run (still a read: nothing
//! written, no event), and an event when the DJ's decision changes the
//! queue without touching the Library (a rejection or a reopening), so a
//! window showing the queue hears another client's decision.
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

use super::review::{fact, ok};

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

fn quiet(watcher: &mut wreckordbox_service::Client) {
    while watcher
        .next_event(Duration::from_millis(300))
        .unwrap()
        .is_some()
    {}
}

/// The events that arrive before the line is quiet, by name.
fn heard(watcher: &mut wreckordbox_service::Client) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    let mut wait = Duration::from_secs(2);
    while let Some(event) = watcher.next_event(wait).unwrap() {
        out.push((event.event, event.data));
        wait = Duration::from_millis(300);
    }
    out
}

#[test]
fn each_waiting_fact_names_its_run_and_the_listing_lists_those_runs_writing_nothing() {
    let s = Sandbox::new();
    s.dj_prep_colours();
    ok(&s, &["status"]);
    let run = ok(
        &s,
        &[
            "runs",
            "start",
            "--source",
            "agent:tagger",
            "--description",
            "tagging pass",
        ],
    );
    let run_id = run["id"].as_i64().unwrap();
    let in_run = |track: &str, value: &str, confidence: &str| {
        ok(
            &s,
            &[
                "facts",
                "record",
                track,
                "energy",
                value,
                "--run",
                &run_id.to_string(),
                "--confidence",
                confidence,
            ],
        )["facts"][0]["id"]
            .as_i64()
            .unwrap()
    };
    let a = in_run("10001", "4", "0.9");
    let b = in_run("10002", "2", "0.5");
    // Recorded on its own: a Run of its own, finished with the command.
    let alone = fact(&s, "10003", "prep_colour", "Green", "tool:meter", "0.8");
    let mut watcher = s.client();
    watcher.subscribe().unwrap();
    quiet(&mut watcher);
    let bytes_before = sidecar_bytes(&s);
    let library_before = std::fs::read(s.master_db()).unwrap();

    let listing = ok(&s, &["review", "waiting"]);

    let runs = listing["runs"]
        .as_array()
        .expect("the listing names its Runs");
    assert_eq!(runs.len(), 2, "{runs:?}");
    let tagging = runs.iter().find(|r| r["id"] == run_id).unwrap();
    assert_eq!(
        tagging["source"],
        json!({"kind": "agent", "name": "tagger"})
    );
    assert_eq!(tagging["description"], "tagging pass");
    assert!(tagging["started_at"].as_i64().unwrap() > 0);
    // Still working: nothing finished it.
    assert_eq!(tagging["finished_at"], Value::Null);
    assert_eq!(tagging["facts"], json!([a, b]));
    let other = runs.iter().find(|r| r["id"] != run_id).unwrap();
    assert_eq!(other["source"], json!({"kind": "tool", "name": "meter"}));
    assert!(other["finished_at"].as_i64().is_some(), "{other:?}");
    assert_eq!(other["facts"], json!([alone]));
    // Every waiting Fact is in exactly one listed Run.
    let mut grouped: Vec<i64> = runs
        .iter()
        .flat_map(|r| {
            r["facts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f.as_i64().unwrap())
        })
        .collect();
    grouped.sort_unstable();
    let mut listed: Vec<i64> = listing["waiting"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["fact"].as_i64().unwrap())
        .collect();
    listed.sort_unstable();
    assert_eq!(grouped, listed);

    // A read: no Sidecar byte, no Library byte, no event.
    assert_eq!(sidecar_bytes(&s), bytes_before);
    assert_eq!(std::fs::read(s.master_db()).unwrap(), library_before);
    let stray = watcher.next_event(Duration::from_millis(300)).unwrap();
    assert!(stray.is_none(), "{stray:?}");

    // Finishing the Run is seen by the next listing.
    ok(&s, &["runs", "finish", &run_id.to_string()]);
    let again = ok(&s, &["review", "waiting"]);
    let tagging = again["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == run_id)
        .unwrap()
        .clone();
    assert!(tagging["finished_at"].as_i64().is_some(), "{tagging:?}");
    // The token covers what an accept would do, not the Runs: unchanged.
    assert_eq!(again["token"], listing["token"]);
}

#[test]
fn a_rejection_and_a_reopening_are_told_to_every_client_and_nothing_else_is() {
    let s = Sandbox::new();
    s.dj_prep_colours();
    ok(&s, &["status"]);
    let energy = fact(&s, "10001", "energy", "4", "agent:tagger", "0.7");
    let mut watcher = s.client();
    watcher.subscribe().unwrap();
    quiet(&mut watcher);

    let rejected = ok(
        &s,
        &["review", "reject", &energy.to_string(), "--source", "dj"],
    );
    let events = heard(&mut watcher);
    let decided: Vec<&Value> = events
        .iter()
        .filter(|(name, _)| name == "review_decided")
        .map(|(_, data)| data)
        .collect();
    assert_eq!(decided.len(), 1, "{events:?}");
    assert_eq!(decided[0]["decision"], "rejected");
    assert_eq!(decided[0]["facts"], json!([energy]));
    assert!(
        events.iter().all(|(name, _)| name != "library_changed"),
        "{events:?}"
    );

    let entry = rejected["rejection"]["entry"].as_i64().unwrap();
    ok(
        &s,
        &[
            "review",
            "reopen",
            "--rejection",
            &entry.to_string(),
            "--source",
            "dj",
        ],
    );
    let events = heard(&mut watcher);
    let decided: Vec<&Value> = events
        .iter()
        .filter(|(name, _)| name == "review_decided")
        .map(|(_, data)| data)
        .collect();
    assert_eq!(decided.len(), 1, "{events:?}");
    assert_eq!(decided[0]["decision"], "reopened");
    assert_eq!(decided[0]["facts"], json!([energy]));

    // A refused decision tells nobody.
    let refused = s.cli(&["--json", "review", "reject", &energy.to_string()]);
    assert!(!refused.status.success());
    let stray = watcher.next_event(Duration::from_millis(300)).unwrap();
    assert!(stray.is_none(), "{stray:?}");
}
