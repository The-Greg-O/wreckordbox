//! The Ledger when things go wrong, through `wreckordbox` against a real
//! service on a throwaway Library: a service killed between a Library
//! change and its entry, an entry the Sidecar refused, a replay of kept
//! entries interrupted part way, a damaged file of kept entries, and a Run
//! ended while an edit in it is still being made.
//!
//! Failures are injected into the service with `WRECKORDBOX_LEDGER_FAULTS`,
//! which a service honours only under `RBXPORT_TEST`.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::common;

use std::time::{Duration, Instant};

use common::Sandbox;
use serde_json::Value;

const FAULTS: &str = "WRECKORDBOX_LEDGER_FAULTS";
const SPILL: &str = "ledger-spill.jsonl";

/// Runs a command that must succeed and answers its JSON result.
fn ok(sandbox: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let run = sandbox.cli(&all);
    assert!(run.status.success(), "{args:?}: {}", run.stderr);
    run.json()
}

/// Runs a command with `faults` given to the service it starts.
fn with_faults(sandbox: &Sandbox, faults: &str, args: &[&str]) -> common::Run {
    let mut command = sandbox.command();
    command.env(FAULTS, faults).arg("--json").args(args);
    sandbox.run(&mut command)
}

/// Stops the service running here, if one is, and waits until it is gone,
/// so the next command starts another.
fn stop_service(sandbox: &Sandbox) {
    let Ok(mut client) =
        wreckordbox_service::Client::connect(&sandbox.config().socket_path(sandbox.location()))
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

/// Waits for a service that was made to die to be gone.
fn wait_for_no_service(sandbox: &Sandbox) {
    let socket = sandbox.config().socket_path(sandbox.location());
    let deadline = Instant::now() + Duration::from_secs(10);
    while wreckordbox_service::Client::connect(&socket)
        .and_then(|mut client| client.status())
        .is_ok()
    {
        assert!(Instant::now() < deadline, "the service did not die");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Every entry of every Run, oldest Run first.
fn all_entries(sandbox: &Sandbox) -> Vec<Value> {
    let listed = ok(sandbox, &["runs", "list"]);
    let mut runs: Vec<i64> = listed["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|run| run["id"].as_i64().unwrap())
        .collect();
    runs.sort_unstable();
    runs.iter()
        .flat_map(|run| {
            ok(sandbox, &["runs", "show", &run.to_string()])["entries"]
                .as_array()
                .unwrap()
                .clone()
        })
        .collect()
}

/// The entries that say a track's rating went from `before` to `after`.
fn ratings(entries: &[Value], track: &str, before: i64, after: i64) -> Vec<Value> {
    entries
        .iter()
        .filter(|entry| {
            entry["values"].as_array().is_some_and(|values| {
                values.iter().any(|value| {
                    value["subject"] == format!("track {track}")
                        && value["before"] == before
                        && value["after"] == after
                })
            })
        })
        .cloned()
        .collect()
}

#[test]
fn a_service_killed_after_a_library_change_and_before_its_entry_leaves_an_entry_saying_so() {
    let sandbox = Sandbox::new();
    sandbox.describe_track(3, "Midnight Drive", "Ada Lovelace", "8A", 2);

    // The service dies the moment the rating is in the Library.
    let killed = with_faults(
        &sandbox,
        "abort_after_write",
        &["tracks", "rate", "10003", "5"],
    );
    assert!(!killed.status.success());
    wait_for_no_service(&sandbox);

    // The next service finds the change made, and says it was begun and
    // never confirmed, in a Run of its own, before anything else is read.
    assert_eq!(ok(&sandbox, &["tracks", "show", "10003"])["rating"], 5);
    let entries = all_entries(&sandbox);
    let unconfirmed: Vec<&Value> = entries
        .iter()
        .filter(|entry| entry["kind"] == "unconfirmed")
        .collect();
    assert_eq!(unconfirmed.len(), 1, "{entries:#?}");
    assert_eq!(unconfirmed[0]["action"], "write");
    assert_eq!(unconfirmed[0]["store"], "library");
    assert_eq!(unconfirmed[0]["command"]["name"], "tracks.rate");
    assert_eq!(unconfirmed[0]["source"]["kind"], "tool");
    assert!(
        unconfirmed[0]["label"]
            .as_str()
            .unwrap()
            .contains("not confirmed"),
        "{}",
        unconfirmed[0]
    );

    // An edit that completes leaves no such entry.
    ok(&sandbox, &["tracks", "rate", "10003", "4"]);
    let entries = all_entries(&sandbox);
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry["kind"] == "unconfirmed")
            .count(),
        1
    );
    assert_eq!(ratings(&entries, "10003", 5, 4).len(), 1);
}

#[test]
fn an_entry_the_sidecar_refused_after_a_real_edit_is_kept_and_shown_after_a_restart() {
    let sandbox = Sandbox::new();
    sandbox.describe_track(2, "Low Tide", "Grace Hopper", "5A", 0);

    let rated = with_faults(&sandbox, "fail_append=1", &["tracks", "rate", "10002", "5"]);
    assert!(rated.status.success(), "{}", rated.stderr);
    // The change is made and the entry waits on disk.
    assert!(sandbox.sidecar_dir().join(SPILL).is_file());
    stop_service(&sandbox);

    // Only reading: the next service writes it before answering.
    let entries = all_entries(&sandbox);
    let kept = ratings(&entries, "10002", 0, 5);
    assert_eq!(kept.len(), 1, "{entries:#?}");
    assert_eq!(kept[0]["action"], "edit");
    assert_eq!(kept[0]["command"]["name"], "tracks.rate");
    assert!(!sandbox.sidecar_dir().join(SPILL).exists());
    // The Library and the Ledger agree.
    assert_eq!(ok(&sandbox, &["tracks", "show", "10002"])["rating"], 5);
    assert!(entries.iter().all(|entry| entry["kind"] != "unconfirmed"));
    // No Run is left open by the service that stopped.
    let listed = ok(&sandbox, &["runs", "list"]);
    assert!(listed["runs"]
        .as_array()
        .unwrap()
        .iter()
        .all(|run| !run["finished_at"].is_null()));
}

#[test]
fn a_replay_of_kept_entries_interrupted_part_way_writes_each_entry_once() {
    let sandbox = Sandbox::new();
    let rated = with_faults(&sandbox, "fail_append=1", &["tracks", "rate", "10001", "4"]);
    assert!(rated.status.success(), "{}", rated.stderr);
    assert!(sandbox.sidecar_dir().join(SPILL).is_file());
    stop_service(&sandbox);

    // The next service dies once it has written the kept entry, before it
    // has removed the file.
    let killed = with_faults(&sandbox, "abort_in_replay", &["runs", "list"]);
    assert!(!killed.status.success());
    wait_for_no_service(&sandbox);
    assert!(sandbox.sidecar_dir().join(SPILL).is_file());

    let entries = all_entries(&sandbox);
    assert_eq!(ratings(&entries, "10001", 0, 4).len(), 1, "{entries:#?}");
    assert!(!sandbox.sidecar_dir().join(SPILL).exists());
}

#[test]
fn a_damaged_file_of_kept_entries_is_kept_as_it_is_and_edits_stay_refused_until_it_is_dealt_with() {
    let sandbox = Sandbox::new();
    let rated = with_faults(&sandbox, "fail_append=1", &["tracks", "rate", "10001", "4"]);
    assert!(rated.status.success(), "{}", rated.stderr);
    stop_service(&sandbox);
    // A torn last line after a whole one.
    let spill = sandbox.sidecar_dir().join(SPILL);
    let mut text = std::fs::read_to_string(&spill).unwrap();
    text.push_str("{\"id\":");
    std::fs::write(&spill, &text).unwrap();

    for _ in 0..2 {
        let refused = sandbox.cli(&["--json", "tracks", "rate", "10002", "3"]);
        assert!(!refused.status.success());
        let message = refused.error_json()["error"]["message"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(message.contains(SPILL), "{message}");
        assert_eq!(
            std::fs::read_to_string(&spill).unwrap(),
            text,
            "kept as it was"
        );
    }
    assert_eq!(ok(&sandbox, &["tracks", "show", "10002"])["rating"], 0);

    // The DJ moves it aside: edits go on, and nothing of it was replayed.
    std::fs::rename(&spill, sandbox.sidecar_dir().join("set-aside.jsonl")).unwrap();
    ok(&sandbox, &["tracks", "rate", "10002", "3"]);
    let entries = all_entries(&sandbox);
    assert!(ratings(&entries, "10001", 0, 4).is_empty());
    assert_eq!(ratings(&entries, "10002", 0, 3).len(), 1);
}

#[test]
fn a_run_ended_while_an_edit_in_it_is_being_made_ends_after_that_edit_is_recorded() {
    let sandbox = Sandbox::new();
    // Every Library write waits a moment once it has been admitted.
    let started = with_faults(
        &sandbox,
        "delay_after_begin_ms=1500",
        &[
            "runs",
            "start",
            "--source",
            "agent:tagger",
            "--description",
            "Rate",
        ],
    );
    assert!(started.status.success(), "{}", started.stderr);
    let run = started.json()["id"].as_i64().unwrap().to_string();

    let editing = {
        let mut command = sandbox.command();
        command.args(["--json", "tracks", "rate", "10001", "3", "--run", &run]);
        std::thread::spawn(move || command.output().unwrap())
    };
    std::thread::sleep(Duration::from_millis(500));
    ok(&sandbox, &["runs", "finish", &run, "--outcome", "done"]);
    let edited = editing.join().unwrap();
    assert!(
        edited.status.success(),
        "{}",
        String::from_utf8_lossy(&edited.stderr)
    );

    let shown = ok(&sandbox, &["runs", "show", &run]);
    let entries = shown["entries"].as_array().unwrap();
    assert_eq!(ratings(entries, "10001", 0, 3).len(), 1, "{shown:#}");
    let finished = shown["run"]["finished_at"].as_i64().unwrap();
    assert!(
        finished >= entries[0]["at"].as_i64().unwrap(),
        "the Run ended before its edit: {shown:#}"
    );
    // Once ended, it takes no more.
    let refused = sandbox.cli(&["--json", "tracks", "rate", "10001", "4", "--run", &run]);
    assert!(!refused.status.success());
    assert_eq!(refused.error_json()["error"]["code"], "run_finished");
}

#[test]
fn an_edit_made_by_hand_says_which_run_it_is_in() {
    let sandbox = Sandbox::new();
    let rated = sandbox.cli(&["tracks", "rate", "10001", "3"]);
    assert!(rated.status.success(), "{}", rated.stderr);
    let last = rated.stdout.trim_end().lines().last().unwrap().to_owned();
    assert!(last.starts_with("Run "), "{}", rated.stdout);
    assert!(last.contains("(tool:wreckordbox)"), "{last}");
}

#[test]
fn a_crashed_dj_edit_reconciles_its_fact_from_the_library() {
    let s = Sandbox::new();
    let killed = with_faults(
        &s,
        "abort_after_write",
        &["tracks", "rate", "10003", "5", "--source", "dj"],
    );
    assert!(!killed.status.success());
    wait_for_no_service(&s);
    // An unrelated native change must not be attributed to the interrupted DJ edit.
    s.edit(|writer| {
        writer.set_rating("10004", 4).unwrap();
    });
    assert_eq!(ok(&s, &["tracks", "show", "10003"])["rating"], 5);
    let fact = ok(&s, &["facts", "standing", "10003", "energy"])["standing"].clone();
    assert_eq!(fact["value"], 5);
    assert_eq!(fact["reconciled"], true);
    assert!(ok(&s, &["facts", "standing", "10004", "energy"])["standing"].is_null());
    let entries = all_entries(&s);
    let entry = entries.iter().find(|e| e["kind"] == "unconfirmed").unwrap();
    assert_eq!(fact["ledger_entry"], entry["id"]);
    assert_eq!(fact["run"], entry["run"]);
    stop_service(&s);
    assert_eq!(
        ok(&s, &["facts", "standing", "10003", "energy"])["standing"],
        fact
    );
}
