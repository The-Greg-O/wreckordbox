//! The Ledger failing during a projection's apply (wbx-xme, part 2),
//! through the real CLI and a service on a fixture Library started with
//! Ledger faults. A service killed after the write and before its entry is
//! `projection_barriers`' case; here the Sidecar refuses the entry, so it
//! is kept on disk and written back by the next service.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
use crate::common;

use std::time::{Duration, Instant};

use common::Sandbox;
use serde_json::{json, Value};

const SPILL: &str = "ledger-spill.jsonl";

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

/// Runs a command with `faults` given to the service it starts.
fn with_faults(s: &Sandbox, faults: &str, args: &[&str]) -> common::Run {
    let mut command = s.command();
    command
        .env("WRECKORDBOX_LEDGER_FAULTS", faults)
        .arg("--json")
        .args(args);
    s.run(&mut command)
}

/// Stops the service here and waits until its process is gone.
fn stop_service(s: &Sandbox) {
    let Ok(mut client) =
        wreckordbox_service::Client::connect(&s.config().socket_path(s.location()))
    else {
        return;
    };
    let pid = client.status().ok().map(|status| u64::from(status.pid));
    let _ = client.stop();
    if let Some(pid) = pid {
        let deadline = Instant::now() + Duration::from_secs(10);
        while common::alive(pid) {
            assert!(Instant::now() < deadline, "the service did not stop");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn wait_for_no_service(s: &Sandbox) {
    let socket = s.config().socket_path(s.location());
    let deadline = Instant::now() + Duration::from_secs(10);
    while wreckordbox_service::Client::connect(&socket)
        .and_then(|mut client| client.status())
        .is_ok()
    {
        assert!(Instant::now() < deadline, "the service did not die");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn fact(s: &Sandbox, track: &str, value: &str, source: &str, confidence: &str) {
    ok(
        s,
        &[
            "facts",
            "record",
            track,
            "energy",
            value,
            "--source",
            source,
            "--confidence",
            confidence,
        ],
    );
}

fn rating(s: &Sandbox, id: &str) -> Value {
    ok(s, &["tracks", "show", id])["rating"].clone()
}

/// Every entry of every Run.
fn every_entry(s: &Sandbox) -> Vec<Value> {
    let runs = ok(s, &["runs", "list"]);
    let mut out = Vec::new();
    for run in runs["runs"].as_array().unwrap() {
        let shown = ok(s, &["runs", "show", &run["id"].to_string()]);
        out.extend(shown["entries"].as_array().unwrap().iter().cloned());
    }
    out
}

/// The entries a projection's apply recorded.
fn projection_entries(s: &Sandbox) -> Vec<Value> {
    every_entry(s)
        .into_iter()
        .filter(|e| e["command"]["name"] == "projection.apply")
        .collect()
}

fn changes(report: &Value) -> Vec<(String, Value, Value)> {
    let mut list: Vec<_> = report["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["track"].as_str().unwrap().to_owned(),
                c["before"].clone(),
                c["after"].clone(),
            )
        })
        .collect();
    list.sort_by(|a, b| a.0.cmp(&b.0));
    list
}

fn setup() -> Sandbox {
    let s = Sandbox::new();
    std::fs::create_dir_all(s.sidecar_dir()).unwrap();
    std::fs::write(
        s.sidecar_dir().join("projection-policy.toml"),
        "[energy]\napply = 0.5\n",
    )
    .unwrap();
    fact(&s, "10001", "3", "agent:tagger", "0.6");
    fact(&s, "10002", "2", "agent:tagger", "0.6");
    stop_service(&s);
    s
}

/// The Sidecar refuses the projection's entry: the write stands, the entry
/// waits on disk, and while it waits the written values count as the DJ's.
/// The next service writes it back once, after which the projection's
/// values are its own again, and no DJ Fact was made.
#[test]
fn a_projection_entry_the_sidecar_refused_is_kept_replayed_once_and_owns_nothing_while_it_waits() {
    let s = setup();
    let applied = with_faults(&s, "fail_append=1", &["project"]);
    assert!(applied.status.success(), "{}", applied.stderr);
    assert_eq!(
        changes(&applied.json()),
        vec![
            ("10001".to_owned(), Value::Null, json!(3)),
            ("10002".to_owned(), Value::Null, json!(2)),
        ]
    );
    assert!(
        s.sidecar_dir().join(SPILL).is_file(),
        "the entry waits on disk"
    );

    // While it waits (same service), nothing the Ledger cannot show is
    // the projection's: a better Fact does not replace the value.
    fact(&s, "10001", "5", "agent:other", "0.9");
    let dry = ok(&s, &["project", "--dry-run"]);
    assert_eq!(changes(&dry), vec![], "{dry}");
    assert_eq!(rating(&s, "10001"), 3);

    // The next service writes the entry back before answering, once.
    stop_service(&s);
    let entries = projection_entries(&s);
    assert_eq!(entries.len(), 1, "{entries:#?}");
    assert_eq!(entries[0]["kind"], "track");
    assert_eq!(
        entries[0]["source"],
        json!({"kind": "tool", "name": "projection"})
    );
    assert!(!s.sidecar_dir().join(SPILL).exists());
    // No DJ Fact came of the projection.
    let shown = ok(&s, &["facts", "show", "10002"]);
    assert!(
        shown["facts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["source"]["kind"] != "dj"),
        "{shown}"
    );
    // With its entry recorded the value is the projection's again.
    let again = ok(&s, &["project"]);
    assert_eq!(
        changes(&again),
        vec![("10001".to_owned(), json!(3), json!(5))],
        "{again}"
    );
    assert_eq!(rating(&s, "10001"), 5);
    assert_eq!(projection_entries(&s).len(), 2);
}

/// A replay of a kept projection entry cut off after writing it and before
/// removing the file: the next replay writes nothing twice.
#[test]
fn a_replay_of_a_kept_projection_entry_cut_off_part_way_writes_it_once() {
    let s = setup();
    let applied = with_faults(&s, "fail_append=1", &["project"]);
    assert!(applied.status.success(), "{}", applied.stderr);
    stop_service(&s);

    let killed = with_faults(&s, "abort_in_replay", &["runs", "list"]);
    assert!(!killed.status.success());
    wait_for_no_service(&s);
    assert!(s.sidecar_dir().join(SPILL).is_file());

    assert_eq!(projection_entries(&s).len(), 1);
    assert!(!s.sidecar_dir().join(SPILL).exists());
    // Projecting the same Facts again changes nothing.
    let again = ok(&s, &["project"]);
    assert_eq!(changes(&again), vec![]);
    assert_eq!(again["unchanged"], 2, "{again}");
}
