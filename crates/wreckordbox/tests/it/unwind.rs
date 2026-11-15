//! Unwind through the real CLI on a disposable Library.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
use crate::common::Sandbox;
use serde_json::Value;

fn ok(s: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let out = s.cli(&all);
    assert!(out.status.success(), "{args:?}: {}", out.stderr);
    out.json()
}

#[test]
fn an_old_run_keeps_later_values_and_its_unwind_can_be_unwound() {
    let s = Sandbox::new();
    s.describe_track(3, "First", "Artist", "8A", 2);
    s.describe_track(4, "Second", "Artist", "8A", 1);
    let run = ok(
        &s,
        &[
            "runs",
            "start",
            "--source",
            "agent:test",
            "--description",
            "Earlier work",
        ],
    )["id"]
        .to_string();
    ok(&s, &["tracks", "rate", "10003", "4", "--run", &run]);
    ok(&s, &["tracks", "rate", "10004", "3", "--run", &run]);
    ok(&s, &["runs", "finish", &run]);
    ok(&s, &["tracks", "rate", "10004", "5"]);
    let plan = ok(&s, &["runs", "unwind", &run, "--dry-run"]);
    assert_eq!(plan["entries"][0]["status"], "changed_since");
    assert_eq!(plan["entries"][1]["status"], "will_reverse");
    let applied = ok(&s, &["runs", "unwind", &run]);
    assert_eq!(applied["reversed"], 1);
    let unwind = applied["run"]["id"].to_string();
    assert_eq!(ok(&s, &["tracks", "show", "10003"])["rating"], 2);
    assert_eq!(ok(&s, &["tracks", "show", "10004"])["rating"], 5);
    assert_eq!(ok(&s, &["runs", "unwind", &run])["reversed"], 0);
    assert_eq!(ok(&s, &["runs", "unwind", &unwind])["reversed"], 1);
    assert_eq!(ok(&s, &["tracks", "show", "10003"])["rating"], 4);
}

#[test]
fn protection_dry_run_and_forced_conflicts_are_explicit() {
    let s = Sandbox::new();
    let changed = ok(&s, &["tracks", "rate", "10003", "4"]);
    let run = changed["run"]["id"].to_string();
    ok(&s, &["tracks", "rate", "10003", "5"]);
    ok(&s, &["protection", "on"]);
    let before_library = std::fs::read(s.master_db()).unwrap();
    let before_sidecar = std::fs::read(s.sidecar_dir().join("sidecar.db")).unwrap();
    let plan = ok(&s, &["runs", "unwind", &run, "--dry-run"]);
    assert_eq!(plan["entries"][0]["status"], "changed_since");
    let refused = s.cli(&["--json", "runs", "unwind", &run, "--force"]);
    assert!(!refused.status.success());
    assert_eq!(refused.error_json()["error"]["code"], "library_protected");
    assert_eq!(std::fs::read(s.master_db()).unwrap(), before_library);
    assert_eq!(
        std::fs::read(s.sidecar_dir().join("sidecar.db")).unwrap(),
        before_sidecar
    );
    ok(&s, &["protection", "off"]);
    let applied = ok(&s, &["runs", "unwind", &run, "--force"]);
    assert_eq!(applied["reversed"], 1);
    assert_eq!(ok(&s, &["tracks", "show", "10003"])["rating"], 0);
    let inverse = applied["run"]["id"].to_string();
    ok(&s, &["runs", "unwind", &inverse]);
    assert_eq!(ok(&s, &["tracks", "show", "10003"])["rating"], 5);
}

#[test]
fn repeated_edits_in_one_run_and_named_only_entries_are_not_lost() {
    let s = Sandbox::new();
    let run = ok(
        &s,
        &[
            "runs",
            "start",
            "--source",
            "agent:test",
            "--description",
            "Repeated work",
        ],
    )["id"]
        .to_string();
    ok(&s, &["tracks", "rate", "10003", "2", "--run", &run]);
    ok(&s, &["tracks", "rate", "10003", "4", "--run", &run]);
    ok(
        &s,
        &["mytags", "create", "700001", "New tag", "--run", &run],
    );
    ok(&s, &["runs", "finish", &run]);
    let plan = ok(&s, &["runs", "unwind", &run, "--dry-run"]);
    assert_eq!(plan["entries"][0]["status"], "cannot_reverse");
    assert_eq!(plan["entries"][0]["kind"], "unrecorded");
    let applied = ok(&s, &["runs", "unwind", &run]);
    assert_eq!(applied["reversed"], 2);
    assert_eq!(ok(&s, &["tracks", "show", "10003"])["rating"], 0);
}

#[test]
fn facts_and_mytags_keep_their_original_sources_when_unwound() {
    let s = Sandbox::new();
    let tag = s.create_my_tag("Unwind tag");
    let first = ok(
        &s,
        &[
            "facts",
            "record",
            "10003",
            "genre",
            "House",
            "--source",
            "agent:test",
            "--confidence",
            "0.8",
        ],
    );
    let run = ok(
        &s,
        &[
            "runs",
            "start",
            "--source",
            "agent:test",
            "--description",
            "Facts and tags",
        ],
    )["id"]
        .to_string();
    ok(
        &s,
        &[
            "facts",
            "record",
            "10003",
            "genre",
            "Techno",
            "--source",
            "agent:test",
            "--confidence",
            "0.9",
            "--run",
            &run,
        ],
    );
    ok(&s, &["mytags", "assign", &tag, "10003", "--run", &run]);
    ok(&s, &["runs", "finish", &run]);
    let applied = ok(&s, &["runs", "unwind", &run]);
    assert_eq!(applied["reversed"], 2);
    let facts = ok(&s, &["facts", "show", "10003", "--kind", "genre"]);
    assert_eq!(facts["facts"][0]["value"], serde_json::json!(["House"]));
    assert_eq!(facts["facts"][0]["source"]["name"], "test");
    assert_eq!(facts["facts"][0]["run"], first["run"]["id"]);
    assert!(!ok(&s, &["tracks", "show", "10003"])["my_tags"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["id"] == tag));
    let inverse = applied["run"]["id"].to_string();
    ok(&s, &["runs", "unwind", &inverse]);
    assert_eq!(
        ok(&s, &["facts", "show", "10003", "--kind", "genre"])["facts"][0]["value"],
        serde_json::json!(["Techno"])
    );
}

#[test]
fn a_subscriber_hears_one_change_and_restart_does_not_lose_unwind() {
    let s = Sandbox::new();
    let run = ok(&s, &["tracks", "rate", "10003", "4"])["run"]["id"].to_string();
    let pid = s.status()["pid"].as_u64().unwrap();
    s.client().stop().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while crate::common::alive(pid) {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    ok(&s, &["status"]);
    let mut observer = s.client();
    observer.subscribe().unwrap();
    crate::common::barrier::settle(&mut observer);
    let plan = ok(&s, &["runs", "unwind", &run, "--dry-run"]);
    assert_eq!(plan["entries"][0]["status"], "will_reverse");
    crate::common::barrier::told_nothing(&mut observer);
    let answer = ok(&s, &["runs", "unwind", &run]);
    assert_eq!(answer["reversed"], 1);
    crate::common::barrier::one_library_change(&mut observer);
    assert_eq!(ok(&s, &["tracks", "show", "10003"])["rating"], 0);
}

#[test]
fn later_dj_facts_are_kept_and_repeated_dj_edits_unwind_together() {
    let s = Sandbox::new();
    let run = ok(
        &s,
        &[
            "runs",
            "start",
            "--source",
            "dj",
            "--description",
            "DJ choices",
        ],
    )["id"]
        .to_string();
    ok(&s, &["tracks", "rate", "10003", "2", "--run", &run]);
    ok(&s, &["tracks", "rate", "10003", "4", "--run", &run]);
    ok(&s, &["runs", "finish", &run]);
    let answer = ok(&s, &["runs", "unwind", &run]);
    assert_eq!(answer["reversed"], 2, "{answer}");
    assert_eq!(ok(&s, &["tracks", "show", "10003"])["rating"], 0);
    assert!(
        ok(&s, &["facts", "show", "10003", "--kind", "energy"])["facts"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let original =
        ok(&s, &["tracks", "rate", "10003", "4", "--source", "dj"])["run"]["id"].to_string();
    ok(
        &s,
        &["facts", "record", "10003", "energy", "5", "--source", "dj"],
    );
    let answer = ok(&s, &["runs", "unwind", &original]);
    assert_eq!(answer["reversed"], 0);
    assert_eq!(answer["entries"][0]["status"], "changed_since");
    assert_eq!(
        ok(&s, &["facts", "show", "10003", "--kind", "energy"])["facts"][0]["value"],
        5
    );
    assert_eq!(ok(&s, &["tracks", "show", "10003"])["rating"], 4);
}

#[test]
fn forced_native_unwind_replaces_later_facts_and_can_restore_them() {
    let s = Sandbox::new();
    let original =
        ok(&s, &["tracks", "rate", "10003", "4", "--source", "dj"])["run"]["id"].to_string();
    ok(
        &s,
        &["facts", "record", "10003", "energy", "5", "--source", "dj"],
    );
    let applied = ok(&s, &["runs", "unwind", &original, "--force"]);
    assert_eq!(applied["reversed"], 1);
    assert!(
        ok(&s, &["facts", "show", "10003", "--kind", "energy"])["facts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    ok(&s, &["runs", "unwind", &applied["run"]["id"].to_string()]);
    assert_eq!(
        ok(&s, &["facts", "show", "10003", "--kind", "energy"])["facts"][0]["value"],
        5
    );
}

#[test]
fn repeated_fact_replacements_restore_the_original_and_dry_run_changes_no_bytes() {
    let s = Sandbox::new();
    let initial = ok(
        &s,
        &[
            "facts",
            "record",
            "10003",
            "energy",
            "1",
            "--source",
            "agent:test",
            "--confidence",
            "0.8",
        ],
    );
    let original = ok(
        &s,
        &[
            "runs",
            "start",
            "--source",
            "agent:test",
            "--description",
            "Facts",
        ],
    )["id"]
        .to_string();
    for value in ["2", "3"] {
        ok(
            &s,
            &[
                "facts",
                "record",
                "10003",
                "energy",
                value,
                "--source",
                "agent:test",
                "--confidence",
                "0.8",
                "--run",
                &original,
            ],
        );
    }
    ok(&s, &["runs", "finish", &original]);
    let bytes = || {
        [
            s.master_db().to_path_buf(),
            s.sidecar_dir().join("sidecar.db"),
            s.sidecar_dir().join("sidecar.db-wal"),
            s.sidecar_dir().join("sidecar.db-shm"),
        ]
        .into_iter()
        .map(|p| (p.clone(), std::fs::read(p).ok()))
        .collect::<Vec<_>>()
    };
    let before = bytes();
    let plan = ok(&s, &["runs", "unwind", &original, "--dry-run"]);
    assert_eq!(bytes(), before);
    assert_eq!(plan["reversed"], 2);
    let applied = ok(&s, &["runs", "unwind", &original]);
    assert_eq!(applied["reversed"], 2);
    let facts = ok(&s, &["facts", "show", "10003", "--kind", "energy"]);
    assert_eq!(facts["facts"].as_array().unwrap().len(), 1);
    assert_eq!(facts["facts"][0]["value"], 1);
    assert_eq!(facts["facts"][0]["run"], initial["run"]["id"]);
    let after = bytes();
    assert_eq!(ok(&s, &["runs", "unwind", &original])["reversed"], 0);
    assert_eq!(bytes(), after);
}

#[test]
fn unwinding_an_unwind_keeps_a_new_opinion_in_a_previously_empty_slot() {
    let s = Sandbox::new();
    let original = ok(
        &s,
        &["facts", "record", "10003", "energy", "2", "--source", "dj"],
    )["run"]["id"]
        .to_string();
    let unwind = ok(&s, &["runs", "unwind", &original])["run"]["id"].to_string();
    ok(
        &s,
        &["facts", "record", "10003", "energy", "5", "--source", "dj"],
    );
    let kept = ok(&s, &["runs", "unwind", &unwind]);
    assert_eq!(kept["reversed"], 0, "{kept}");
    assert_eq!(kept["entries"][0]["status"], "changed_since");
    let forced = ok(&s, &["runs", "unwind", &unwind, "--force"]);
    assert_eq!(forced["reversed"], 1);
    let facts = ok(&s, &["facts", "show", "10003", "--kind", "energy"]);
    assert_eq!(facts["facts"].as_array().unwrap().len(), 1);
    assert_eq!(facts["facts"][0]["value"], 2);
    ok(&s, &["runs", "unwind", &forced["run"]["id"].to_string()]);
    assert_eq!(
        ok(&s, &["facts", "show", "10003", "--kind", "energy"])["facts"][0]["value"],
        5
    );
}

#[test]
fn unfinished_and_missing_runs_are_refused_without_creating_an_unwind() {
    let s = Sandbox::new();
    let original = ok(
        &s,
        &[
            "runs",
            "start",
            "--source",
            "agent:test",
            "--description",
            "Still working",
        ],
    )["id"]
        .to_string();
    ok(&s, &["tracks", "rate", "10003", "4", "--run", &original]);
    let before = ok(&s, &["runs", "list"]);
    for (id, code) in [(original.as_str(), "run_open"), ("99999", "run_not_found")] {
        for dry in [true, false] {
            let mut args = vec!["--json", "runs", "unwind", id];
            if dry {
                args.push("--dry-run");
            }
            let refused = s.cli(&args);
            assert!(!refused.status.success());
            assert_eq!(refused.error_json()["error"]["code"], code);
        }
    }
    assert_eq!(ok(&s, &["runs", "list"]), before);
    assert_eq!(ok(&s, &["tracks", "show", "10003"])["rating"], 4);
}

#[test]
fn tag_and_playlist_order_unwind_through_the_same_run() {
    let s = Sandbox::new();
    let playlist = rbl_db::fixture::playlist_id(0);
    let before_tags = ok(&s, &["mytags", "list"])["categories"].clone();
    let before_entries = ok(&s, &["playlists", "tracks", &playlist])["tracks"].clone();
    let original = ok(
        &s,
        &[
            "runs",
            "start",
            "--source",
            "agent:test",
            "--description",
            "Orders",
        ],
    )["id"]
        .to_string();
    ok(
        &s,
        &["mytags", "reorder", "700003", "700002", "--run", &original],
    );
    let ids: Vec<_> = before_entries
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .map(|row| row["entry_id"].as_str().unwrap())
        .collect();
    let mut args = vec!["playlists", "reorder", &playlist];
    args.extend(ids);
    args.extend(["--run", &original]);
    ok(&s, &args);
    ok(&s, &["runs", "finish", &original]);
    let plan = ok(&s, &["runs", "unwind", &original, "--dry-run"]);
    assert_eq!(plan["reversed"], 2, "{plan}");
    ok(&s, &["runs", "unwind", &original]);
    assert_eq!(ok(&s, &["mytags", "list"])["categories"], before_tags);
    assert_eq!(
        ok(&s, &["playlists", "tracks", &playlist])["tracks"],
        before_entries
    );
}

#[test]
fn an_accepted_unwind_reports_pending_recording_and_refuses_a_stale_retry() {
    let s = Sandbox::new();
    let original =
        ok(&s, &["tracks", "rate", "10003", "4", "--source", "dj"])["run"]["id"].to_string();
    s.client().stop().unwrap();
    crate::common::released::wait_until_released(&s, "service stopped before fault injection");
    let mut command = s.command();
    command
        .env("WRECKORDBOX_LEDGER_FAULTS", "fail_append=1")
        .args(["--json", "runs", "unwind", &original]);
    let accepted = s.run(&mut command);
    assert!(accepted.status.success(), "{}", accepted.stderr);
    let answer = accepted.json();
    assert_eq!(answer["recording_pending"], true, "{answer}");
    assert!(answer["warning"].as_str().unwrap().contains("Ledger"));
    assert_eq!(ok(&s, &["tracks", "show", "10003"])["rating"], 0);
    let refused = s.cli(&["--json", "runs", "unwind", &original]);
    assert!(!refused.status.success());
    assert_eq!(
        refused.error_json()["error"]["code"],
        "unwind_pending_recording"
    );
}
