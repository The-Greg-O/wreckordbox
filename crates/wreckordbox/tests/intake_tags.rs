//! `wreckordbox intake tags` and `intake tag` (wbx-65): Intake's tag panel
//! through the real command, against a service on a fixture Library, with
//! a second client listening for what the service tells everyone.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod common;

use std::time::Duration;

use common::Sandbox;
use rbl_db::fixture::track_id;
use serde_json::{json, Value};
use wreckordbox_service::Client;

const SCHEMA: &str = r#"
version = 1

[genre]
terms = [
  { id = "house", name = "House" },
  { id = "house-deep", name = "Deep", parent = "house" },
  { id = "techno", name = "Techno" },
]

[dimensions.situation]
terms = [
  { id = "peak", name = "Peak" },
  { id = "warm-up", name = "Warm-up" },
]

[deck]
categories = [{ dimension = "situation", label = "Situation" }]
"#;

fn with_schema() -> Sandbox {
    let sandbox = Sandbox::new();
    std::fs::create_dir_all(sandbox.sidecar_dir()).unwrap();
    std::fs::write(sandbox.sidecar_dir().join("tag-schema.toml"), SCHEMA).unwrap();
    sandbox
}

fn ok(sandbox: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend(args);
    let run = sandbox.cli(&all);
    assert!(run.status.success(), "{args:?}: {}", run.stderr);
    run.json()
}

fn watcher(sandbox: &Sandbox) -> Client {
    sandbox.status();
    let mut watcher = sandbox.client();
    watcher.subscribe().unwrap();
    watcher
}

/// Every event `watcher` hears until it is quiet.
fn heard(watcher: &mut Client) -> Vec<Value> {
    let mut events = Vec::new();
    while let Some(event) = watcher.next_event(Duration::from_millis(700)).unwrap() {
        events.push(json!({ "event": event.event, "data": event.data }));
    }
    events
}

#[test]
fn intake_tags_shows_the_panel_and_intake_tag_sets_a_deck_dimension_heard_by_another_client() {
    let sandbox = with_schema();
    let track = track_id(0);
    let mut watcher = watcher(&sandbox);

    let set = ok(&sandbox, &["intake", "tag", &track, "situation", "peak", "--source", "dj"]);

    assert_eq!(set["changed"], true);
    assert_eq!(set["panel"]["track"]["set"]["situation"], json!(["peak"]));
    let events = heard(&mut watcher);
    assert!(
        events.iter().any(|e| e["event"] == "library_changed"),
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| e["event"] == "sidecar_changed" && e["data"]["kind"] == "situation"),
        "{events:?}"
    );
    // The MyTag is on the track as `mytags` reads it.
    let text = sandbox.cli(&["intake", "tags", &track]);
    assert!(text.status.success(), "{}", text.stderr);
    assert!(text.stdout.contains("◆ Situation: [Peak], Warm-up"), "{}", text.stdout);
}

#[test]
fn intake_tag_sets_a_vocabulary_dimension_as_the_djs_fact_and_shows_implied_parents() {
    let sandbox = with_schema();
    let track = track_id(1);

    ok(&sandbox, &["intake", "tag", &track, "genre", "house-deep", "--source", "dj"]);

    let facts = ok(&sandbox, &["facts", "show", &track]);
    let genre: Vec<&Value> = facts["facts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|fact| fact["kind"] == "genre")
        .collect();
    assert_eq!(genre.len(), 1, "{facts}");
    assert_eq!(genre[0]["value"], json!(["house-deep"]));
    assert_eq!(genre[0]["source"]["kind"], "dj");
    let text = sandbox.cli(&["intake", "tags", &track]).stdout;
    assert!(text.contains("  Genre: (House), [Deep], Techno"), "{text}");
}

#[test]
fn intake_tag_is_refused_under_library_protection_and_for_a_tag_the_dimension_lacks() {
    let sandbox = with_schema();
    let track = track_id(2);

    sandbox.protect(true);
    let refused = sandbox.cli(&["--json", "intake", "tag", &track, "genre", "techno", "--source", "dj"]);
    assert!(!refused.status.success());
    assert_eq!(refused.error_json()["error"]["code"], "library_protected");
    sandbox.protect(false);

    let unknown = sandbox.cli(&["--json", "intake", "tag", &track, "genre", "polka"]);
    assert!(!unknown.status.success());
    assert_eq!(unknown.error_json()["error"]["code"], "tag_not_in_dimension");
    let panel = ok(&sandbox, &["intake", "tags", &track]);
    assert_eq!(panel["track"]["set"]["genre"], json!([]));
}
