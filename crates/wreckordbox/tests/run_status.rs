//! Current work as observed by a second client of the real CLI's service.
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

fn ok(sandbox: &Sandbox, args: &[&str]) -> Value {
    let run = sandbox.cli(&[&["--json"], args].concat());
    assert!(run.status.success(), "{}", run.stderr);
    run.json()
}

#[test]
fn current_work_includes_old_and_concurrent_runs_but_never_finished_work() {
    let sandbox = Sandbox::new();
    sandbox.status();
    let mut observer = sandbox.client();
    observer.subscribe().unwrap();
    assert_eq!(
        observer.call::<Value>("runs.active", json!({})).unwrap(),
        json!([])
    );
    assert!(
        !sandbox.sidecar_dir().exists(),
        "a status read must not create a Sidecar"
    );
    let first = ok(
        &sandbox,
        &[
            "runs",
            "start",
            "--source",
            "agent:tagger",
            "--description",
            "Tag the backlog",
        ],
    );
    let second = ok(
        &sandbox,
        &[
            "runs",
            "start",
            "--source",
            "tool:meter",
            "--description",
            "Measure Intensity",
        ],
    );
    // Completed history must not push an older, still-active Run out of view.
    for _ in 0..105 {
        let finished = observer
            .call::<Value>(
                "runs.start",
                json!({"source": {"kind": "tool", "name": "test"}, "description": "Finished work"}),
            )
            .unwrap();
        observer
            .call::<Value>("runs.finish", json!({"id": finished["id"]}))
            .unwrap();
    }
    assert_eq!(
        observer.call::<Value>("runs.active", json!({})).unwrap(),
        json!([
            {"id": first["id"], "description": "Tag the backlog"},
            {"id": second["id"], "description": "Measure Intensity"}
        ])
    );
    ok(&sandbox, &["runs", "finish", &first["id"].to_string()]);
    assert_eq!(
        observer.call::<Value>("runs.active", json!({})).unwrap(),
        json!([
            {"id": second["id"], "description": "Measure Intensity"}
        ])
    );
    ok(&sandbox, &["runs", "finish", &second["id"].to_string()]);
    assert_eq!(
        observer.call::<Value>("runs.active", json!({})).unwrap(),
        json!([])
    );
    assert!(
        observer
            .next_event(Duration::from_millis(100))
            .unwrap()
            .is_none(),
        "reading status and tracking Runs do not edit the Library"
    );
}
