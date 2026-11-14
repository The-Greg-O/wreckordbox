//! Seeing and clearing Projection's barriers (wbx-xme) through the real CLI
//! and a service on a fixture Library: a write the Ledger cannot account
//! for stops Projection filling fields; the DJ, and only the DJ, can clear
//! it, the clearing is recorded as the DJ's, and it makes only empty fields
//! fillable again.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
use crate::common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
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

/// `(track, field, before, after)` of a report's changes, sorted.
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

/// `(track, reason)` of a report's kept values, sorted.
fn kept(report: &Value) -> Vec<(String, String)> {
    let mut list: Vec<_> = report["kept"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| {
            (
                k["track"].as_str().unwrap().to_owned(),
                k["reason"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    list.sort();
    list
}

fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                todo.push(path);
            } else {
                out.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    out
}

/// Starts a service here with `faults` given to it, and waits until it is up.
fn serve_with_faults(s: &Sandbox, faults: &str) -> std::process::Child {
    let child = s
        .command()
        .env("WRECKORDBOX_LEDGER_FAULTS", faults)
        .arg("serve")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let socket = s.config().socket_path(s.location());
    assert!(
        (0..400).any(|_| {
            std::thread::sleep(Duration::from_millis(25));
            wreckordbox_service::Client::connect(&socket).is_ok()
        }),
        "the service did not come up"
    );
    child
}

fn stop_service(s: &Sandbox) {
    let socket = s.config().socket_path(s.location());
    if let Ok(mut client) = wreckordbox_service::Client::connect(&socket) {
        let _ = client.stop();
    }
    let gone = (0..400).any(|_| {
        std::thread::sleep(Duration::from_millis(25));
        wreckordbox_service::Client::connect(&socket).is_err()
    });
    assert!(gone, "the service did not stop");
}

/// Every entry of every Run, oldest Run first.
fn every_entry(s: &Sandbox) -> Vec<Value> {
    let runs = ok(s, &["runs", "list"]);
    let mut out = Vec::new();
    for run in runs["runs"].as_array().unwrap() {
        let shown = ok(s, &["runs", "show", &run["id"].to_string()]);
        out.extend(shown["entries"].as_array().unwrap().iter().cloned());
    }
    out
}

/// A projection killed after its write and before its entry: the next
/// service records an unconfirmed write that names no tracks, a barrier
/// over every track. Track 10003 was projected before it; the crashed
/// apply wrote 10001.
fn crashed_apply(s: &Sandbox) {
    policy(s, "[energy]\napply = 0.5\n");
    ok(s, &["status"]);
    fact(s, "10003", "2", "agent:tagger", "0.6");
    ok(s, &["project"]);
    assert_eq!(rating(s, "10003"), 2);
    fact(s, "10001", "3", "agent:tagger", "0.6");
    stop_service(s);
    let mut service = serve_with_faults(s, "abort_after_write");
    let applied = s.cli(&["--json", "project"]);
    assert!(!applied.status.success(), "the service should have died");
    service.wait().unwrap();
}

#[test]
fn the_dj_sees_a_crashed_applys_barrier_and_clearing_it_lets_only_empty_fields_fill() {
    let s = Sandbox::new();
    crashed_apply(&s);
    assert_eq!(rating(&s, "10001"), 3, "the crashed write was made");

    // The DJ sees it: which write, when, which tracks.
    let listed = ok(&s, &["projection", "barriers"]);
    let barriers = listed["barriers"].as_array().unwrap();
    assert_eq!(barriers.len(), 1, "{listed}");
    let barrier = &barriers[0];
    assert_eq!(barrier["what"], "unconfirmed", "{barrier}");
    assert_eq!(barrier["command"], "projection.apply");
    assert_eq!(
        barrier["source"],
        json!({"kind": "tool", "name": "projection"})
    );
    assert_eq!(barrier["tracks"], Value::Null, "it names no tracks: all");
    assert!(barrier["at"].as_i64().unwrap() > 0);
    assert!(barrier["run"].as_i64().is_some());
    assert_eq!(barrier["cleared"], Value::Null);
    let seq = barrier["seq"].as_i64().unwrap().to_string();

    // While it stands nothing is filled, and nothing owned is replaced.
    fact(&s, "10002", "4", "agent:other", "0.9");
    fact(&s, "10001", "5", "agent:other", "0.9");
    fact(&s, "10003", "5", "agent:other", "0.9");
    let expected_kept = vec![
        ("10001".to_owned(), "no_record".to_owned()),
        ("10002".to_owned(), "provenance_incomplete".to_owned()),
        ("10003".to_owned(), "provenance_incomplete".to_owned()),
    ];

    // Reading barriers and a dry run clear nothing and write nothing.
    let before = files(&s.sidecar_dir());
    let dry = ok(&s, &["project", "--dry-run"]);
    assert_eq!(changes(&dry), vec![], "{dry}");
    assert_eq!(kept(&dry), expected_kept);
    assert_eq!(ok(&s, &["projection", "barriers"]), listed);
    assert_eq!(files(&s.sidecar_dir()), before, "a read wrote the Sidecar");
    let entries_before = every_entry(&s).len();

    // A tool or an agent is refused, however it asks; nothing is recorded.
    for source in [None, Some("agent:helper"), Some("tool:wreckordbox")] {
        let mut args = vec!["projection", "acknowledge", seq.as_str()];
        if let Some(source) = source {
            args.extend(["--source", source]);
        }
        let error = refused(&s, &args);
        assert_eq!(error["code"], "barrier_not_the_djs", "{source:?}: {error}");
    }
    assert_eq!(every_entry(&s).len(), entries_before);
    assert_eq!(ok(&s, &["projection", "barriers"]), listed);

    // The DJ clears it, deliberately: recorded as the DJ's, with what.
    let cleared = ok(&s, &["projection", "acknowledge", &seq, "--source", "dj"]);
    let entry_id = cleared["entry"].as_i64().unwrap();
    let entries = every_entry(&s);
    assert_eq!(entries.len(), entries_before + 1);
    let entry = entries
        .iter()
        .find(|e| e["kind"] == "barrier_cleared")
        .unwrap_or_else(|| panic!("no clearing entry in {entries:?}"));
    assert_eq!(entry["id"], entry_id);
    assert_eq!(entry["store"], "sidecar");
    assert_eq!(entry["source"], json!({"kind": "dj", "name": "dj"}));
    assert_eq!(entry["payload"]["barrier"]["seq"], barrier["seq"]);
    assert_eq!(entry["payload"]["barrier"]["command"], "projection.apply");
    assert_eq!(entry["payload"]["barrier"]["tracks"], Value::Null);

    let after = ok(&s, &["projection", "barriers"]);
    assert_eq!(
        after["barriers"][0]["cleared"]["entry"], entry_id,
        "{after}"
    );

    // Once only, and only a barrier there is.
    assert_eq!(
        refused(&s, &["projection", "acknowledge", &seq, "--source", "dj"])["code"],
        "barrier_already_cleared"
    );
    assert_eq!(
        refused(
            &s,
            &["projection", "acknowledge", "999999", "--source", "dj"]
        )["code"],
        "barrier_not_found"
    );

    // Now the empty field is filled; the value the crashed write left has no
    // record and stays the DJ's; the projected value under the cleared
    // barrier is not the projection's again.
    let applied = ok(&s, &["project"]);
    assert_eq!(
        changes(&applied),
        vec![("10002".to_owned(), Value::Null, json!(4))],
        "{applied}"
    );
    assert_eq!(
        kept(&applied),
        vec![
            ("10001".to_owned(), "no_record".to_owned()),
            ("10003".to_owned(), "provenance_incomplete".to_owned()),
        ]
    );
    assert_eq!(rating(&s, "10001"), 3);
    assert_eq!(rating(&s, "10002"), 4);
    assert_eq!(rating(&s, "10003"), 2);
}

/// Asked through the service directly: a request in a Run (even the DJ's),
/// a source that is not the DJ, or none, is refused, and a write still in
/// progress cannot be cleared at all.
#[test]
fn clearing_is_refused_in_a_run_without_the_djs_name_and_while_the_write_is_in_progress() {
    use wreckordbox_service::facts::SourceArg;
    use wreckordbox_service::ledger::RunArg;
    use wreckordbox_sidecar::SourceKind;

    let s = Sandbox::new();
    crashed_apply(&s);
    let listed = ok(&s, &["projection", "barriers"]);
    let seq = listed["barriers"][0]["seq"].as_i64().unwrap();
    let dj = SourceArg {
        kind: SourceKind::Dj,
        name: String::new(),
    };
    let entries_before = every_entry(&s).len();
    let refused_code = |client: &mut wreckordbox_service::Client, args: Value| {
        client
            .call::<Value>("projection.acknowledge", args)
            .expect_err("should be refused")
            .code
    };

    let mut client = s.client();
    assert_eq!(
        refused_code(&mut client, json!({"barrier": seq})),
        "barrier_not_the_djs"
    );
    assert_eq!(
        refused_code(
            &mut client,
            json!({"barrier": seq, "source": {"kind": "agent", "name": "helper"}})
        ),
        "barrier_not_the_djs"
    );
    // Not a Library edit: it takes no Run, whoever's.
    let dj_run = ok(
        &s,
        &[
            "runs",
            "start",
            "--source",
            "dj",
            "--description",
            "DJ work",
        ],
    )["id"]
        .as_i64()
        .unwrap();
    for run in [
        RunArg {
            id: Some(dj_run),
            source: None,
            description: None,
        },
        RunArg {
            id: None,
            source: Some(dj.clone()),
            description: None,
        },
    ] {
        let mut in_run = s.client();
        in_run.in_run(Some(run));
        assert_eq!(
            refused_code(&mut in_run, json!({"barrier": seq, "source": dj})),
            "bad_request"
        );
    }
    // Arguments it does not know are refused, not ignored.
    assert_eq!(
        refused_code(
            &mut client,
            json!({"barrier": seq, "source": dj, "all": true})
        ),
        "bad_request"
    );
    assert_eq!(every_entry(&s).len(), entries_before);
    assert_eq!(ok(&s, &["projection", "barriers"])["standing"], 1);

    // A write in progress is listed, and cannot be cleared.
    stop_service(&s);
    let hold = tempfile::tempdir().unwrap();
    let mut service = serve_with_faults(&s, &format!("hold_after_begin={}", hold.path().display()));
    let rating = s
        .command()
        .args(["--json", "tracks", "rate", "10002", "1", "--source", "dj"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    // Held once its intent is saved, until this test lets it go.
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !hold.path().join("begun").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the rating never began"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut watcher = s.client();
    let listed: Value = watcher.call("projection.barriers", json!({})).unwrap();
    let in_progress = listed["barriers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["what"] == "in_progress")
        .cloned()
        .unwrap_or_else(|| panic!("the held write is not listed: {listed}"));
    assert_eq!(in_progress["tracks"], json!(["10002"]), "{in_progress}");
    assert_eq!(
        refused_code(
            &mut watcher,
            json!({"barrier": in_progress["seq"], "source": dj})
        ),
        "barrier_in_progress"
    );
    std::fs::write(hold.path().join("go"), b"").unwrap();
    let mut rating = rating;
    assert!(rating.wait().unwrap().success());
    stop_service(&s);
    service.wait().unwrap();
    // Once confirmed it is no barrier; the crashed apply's still stands.
    let after = ok(&s, &["projection", "barriers"]);
    assert_eq!(after["barriers"].as_array().unwrap().len(), 1, "{after}");
    assert_eq!(after["standing"], 1);
}

/// The cross-model review's case, on ONE track: a projection sets Energy;
/// the DJ asserts the same Energy and the service dies before the
/// assertion is recorded (a barrier on that track); the DJ clears the
/// barrier; an apply fills the track's empty Prep colour; a later apply
/// proposing another Energy must still keep the DJ's: filling one field
/// gives the projection back no other field.
#[test]
fn filling_an_empty_field_after_a_cleared_barrier_gives_the_projection_back_no_other_field() {
    let s = Sandbox::new();
    s.dj_prep_colours();
    policy(&s, "[energy]\napply = 0.5\n\n[prep_colour]\napply = 0.5\n");
    ok(&s, &["status"]);
    fact(&s, "10001", "3", "agent:tagger", "0.6");
    ok(&s, &["project"]);
    assert_eq!(rating(&s, "10001"), 3);
    stop_service(&s);

    // The DJ's same-value set, cut off before its assertion is recorded.
    let mut service = serve_with_faults(&s, "abort_before_assert");
    let rated = s.cli(&["--json", "tracks", "rate", "10001", "3", "--source", "dj"]);
    assert!(!rated.status.success(), "the service should have died");
    service.wait().unwrap();

    let listed = ok(&s, &["projection", "barriers"]);
    let barriers = listed["barriers"].as_array().unwrap();
    assert_eq!(barriers.len(), 1, "{listed}");
    assert_eq!(barriers[0]["tracks"], json!(["10001"]));
    let seq = barriers[0]["seq"].as_i64().unwrap().to_string();
    ok(&s, &["projection", "acknowledge", &seq, "--source", "dj"]);

    // The empty Prep colour is filled, as clearing allows.
    ok(
        &s,
        &[
            "facts",
            "record",
            "10001",
            "prep_colour",
            "Green",
            "--source",
            "agent:tagger",
            "--confidence",
            "0.6",
        ],
    );
    let filled = ok(&s, &["project"]);
    let fields: Vec<&str> = filled["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["field"].as_str().unwrap())
        .collect();
    assert_eq!(fields, vec!["prep_colour"], "{filled}");

    // The Energy the DJ asserted stays the DJ's: the barrier came after the
    // projection wrote it, so the refreshed mark did not keep it.
    fact(&s, "10001", "4", "agent:other", "0.9");
    let applied = ok(&s, &["project"]);
    assert_eq!(changes(&applied), vec![], "{applied}");
    assert_eq!(
        kept(&applied),
        vec![("10001".to_owned(), "written_since".to_owned())]
    );
    assert_eq!(rating(&s, "10001"), 3);
}
