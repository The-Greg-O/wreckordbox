//! The Ledger through `wreckordbox`: every edit to the Library and every
//! change to the Sidecar is recorded with its source, its Run and its
//! before and after values, and Runs can be listed and shown, also after
//! the service has restarted. The real binary against a service on a
//! throwaway Library and Sidecar.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod common;

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

/// The entries `runs show` lists for a Run.
fn entries(sandbox: &Sandbox, run: i64) -> Vec<Value> {
    ok(sandbox, &["runs", "show", &run.to_string()])["entries"]
        .as_array()
        .unwrap()
        .clone()
}

/// Stops the service a command started here and waits until it is gone, so
/// the next command starts another.
fn restart_service(sandbox: &Sandbox) {
    let pid = sandbox.status()["pid"].as_u64().unwrap();
    sandbox.client().stop().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while common::alive(pid) {
        assert!(std::time::Instant::now() < deadline, "the service did not stop");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn a_rating_set_from_the_command_is_a_ledger_entry_with_its_source_run_and_before_and_after() {
    let sandbox = Sandbox::new();
    sandbox.describe_track(3, "Midnight Drive", "Ada Lovelace", "8A", 2);

    let rated = ok(
        &sandbox,
        &[
            "tracks",
            "rate",
            "10003",
            "4",
            "--source",
            "agent:tagger",
            "--description",
            "Rate the warm-up tracks",
        ],
    );

    // The command reports the Run it made.
    let run = rated["run"]["id"].as_i64().expect("the command names its Run");
    assert_eq!(rated["run"]["description"], "Rate the warm-up tracks");
    assert_eq!(rated["run"]["source"], json!({"kind": "agent", "name": "tagger"}));

    let shown = ok(&sandbox, &["runs", "show", &run.to_string()]);
    assert_eq!(shown["run"]["id"], run);
    assert_eq!(shown["total"], 1);
    let entry = &shown["entries"][0];
    assert_eq!(entry["store"], "library");
    assert_eq!(entry["action"], "edit");
    assert_eq!(entry["kind"], "track");
    assert_eq!(entry["label"], "Track Edit");
    assert_eq!(entry["source"], json!({"kind": "agent", "name": "tagger"}));
    assert_eq!(entry["command"]["name"], "tracks.rate");
    assert_eq!(entry["reversible"], true);
    assert_eq!(
        entry["values"],
        json!([{"subject": "track 10003", "field": "Rating", "before": 2, "after": 4}])
    );
    assert!(entry["at"].as_i64().unwrap() > 1_600_000_000_000);
}
