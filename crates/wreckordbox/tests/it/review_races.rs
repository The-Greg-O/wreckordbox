//! Review (wbx-27), round 1 of review: what can come between the DJ's
//! decision and its write, a batch that fails or is cut off part-way, the
//! same suggestion recorded again, undeclared accepts, and reopening a
//! rejection. Interleavings are made with the service's test-mode holds
//! (`WRECKORDBOX_REVIEW_GATE`): a file handshake, never a timed window.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
use crate::common;
use crate::review::{fact, facts_waiting, library_changes, ok, policy, rating, refused, waiting};

use std::path::Path;
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use common::Sandbox;
use rbl_db::write::TrackField;
use serde_json::{json, Value};

/// The service's test-mode holds: `hold-P` makes the accept stop at point
/// `P`, say so in `at-P`, and wait for `go-P`.
struct Gate {
    dir: tempfile::TempDir,
}

impl Gate {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn hold(&self, point: &str) {
        std::fs::write(self.path().join(format!("hold-{point}")), b"").unwrap();
    }

    fn reached(&self, name: &str) {
        let file = self.path().join(name);
        let deadline = Instant::now() + Duration::from_secs(30);
        while !file.exists() {
            assert!(Instant::now() < deadline, "{name} never appeared");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn go(&self, point: &str) {
        std::fs::write(self.path().join(format!("go-{point}")), b"").unwrap();
    }
}

fn stop_service(s: &Sandbox) {
    let socket = s.config().socket_path(s.location());
    if let Ok(mut client) = wreckordbox_service::Client::connect(&socket) {
        let _ = client.stop();
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while wreckordbox_service::Client::connect(&socket).is_ok() {
        assert!(Instant::now() < deadline, "the service did not stop");
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// A service of this test's, stopped and waited for when dropped.
struct Serving(Child);

impl Drop for Serving {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A service with the given test-mode environment, up and answering.
fn serve_with(s: &Sandbox, env: &[(&str, &str)]) -> Serving {
    let mut command = s.command();
    for (name, value) in env {
        command.env(name, value);
    }
    let child = command
        .arg("serve")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let socket = s.config().socket_path(s.location());
    let deadline = Instant::now() + Duration::from_secs(10);
    while wreckordbox_service::Client::connect(&socket).is_err() {
        assert!(Instant::now() < deadline, "the service did not come up");
        std::thread::sleep(Duration::from_millis(25));
    }
    Serving(child)
}

fn gated(s: &Sandbox, gate: &Gate) -> Serving {
    serve_with(
        s,
        &[("WRECKORDBOX_REVIEW_GATE", gate.path().to_str().unwrap())],
    )
}

/// A CLI command running alongside, its JSON output read when it ends.
fn spawn(s: &Sandbox, args: &[&str]) -> Child {
    let mut words = vec!["--json"];
    words.extend_from_slice(args);
    s.command()
        .args(&words)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn finished(child: Child) -> (bool, Value) {
    let output = child.wait_with_output().unwrap();
    let text = if output.status.success() {
        String::from_utf8(output.stdout).unwrap()
    } else {
        String::from_utf8(output.stderr).unwrap()
    };
    let value = serde_json::from_str(text.trim()).unwrap_or(json!({ "text": text }));
    (output.status.success(), value)
}

/// What the Library file holds, read past the service's index.
fn db_rating(s: &Sandbox, id: &str) -> i64 {
    let db = rbl_db::Library::open(s.location().clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    db.connection()
        .query_row(
            "SELECT COALESCE(Rating, 0) FROM djmdContent WHERE ID = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
}

fn standing(s: &Sandbox, id: &str) -> Value {
    ok(s, &["facts", "standing", id, "energy"])["standing"].clone()
}

#[test]
fn a_write_from_outside_the_edit_gate_after_the_selection_refuses_the_accept_and_is_kept() {
    let s = Sandbox::new();
    let gate = Gate::new();
    let _service = gated(&s, &gate);
    let f = fact(&s, "10001", "energy", "4", "agent:tagger", "0.7");
    let runs = ok(&s, &["runs", "list"]);

    gate.hold("selected");
    let accept = spawn(&s, &["review", "accept", &f.to_string(), "--source", "dj"]);
    gate.reached("at-selected");
    // A player or rekordbox writes the row: no edit gate, no Ledger entry.
    s.edit(|w| {
        w.set_rating("10001", 5).unwrap();
    });
    gate.go("selected");
    let (success, answer) = finished(accept);
    assert!(!success, "{answer}");
    assert_eq!(answer["error"]["code"], "review_changed", "{answer}");
    assert_eq!(db_rating(&s, "10001"), 5, "the other write is kept");
    assert_eq!(
        ok(&s, &["runs", "list"]),
        runs,
        "no Run for a refused accept"
    );
}

#[test]
fn an_accept_bound_to_what_the_dj_saw_refuses_a_superseded_fact_by_name() {
    let s = Sandbox::new();
    ok(&s, &["status"]);
    let f = fact(&s, "10001", "energy", "4", "agent:tagger", "0.7");
    let shown = waiting(&s);
    assert_eq!(shown.len(), 1);
    // The agent replaces its answer after the DJ looked.
    fact(&s, "10001", "energy", "2", "agent:tagger", "0.7");
    let mut client = s.client();
    let error = client
        .call::<Value>(
            "review.accept",
            json!({"facts": [f], "expect": shown, "source": {"kind": "dj"}}),
        )
        .unwrap_err();
    assert_eq!(error.code, "review_changed", "{error:?}");
    assert!(error.message.contains(&f.to_string()), "{error:?}");
    assert_eq!(db_rating(&s, "10001"), 0);
}

#[test]
fn an_accept_bound_to_what_the_dj_saw_refuses_a_fact_since_reattached_to_another_track() {
    let s = Sandbox::new();
    let id = s.import_track("Track.wav", 1);
    let describe = |s: &Sandbox, id: &str| {
        s.edit(|w| {
            w.set_field(id, TrackField::Title, "Night Journey (Original Mix)")
                .unwrap();
            w.set_field(id, TrackField::Artist, "Fixture Artist")
                .unwrap();
        });
    };
    describe(&s, &id);
    let mut service = s.serve();
    let f = fact(&s, &id, "energy", "4", "agent:tagger", "0.7");
    let shown = waiting(&s);
    assert_eq!(shown[0]["track"], json!(id));
    service.kill().unwrap();
    service.wait().unwrap();
    // The track is removed and added again: its Facts follow it.
    let mut next = String::new();
    s.edit(|w| {
        w.delete_track(&id).unwrap();
        next = w.import_file(&s.path().join("audio/Track.wav")).unwrap();
    });
    describe(&s, &next);
    let _service = Serving(s.serve());
    ok(&s, &["facts", "show", &next, "--check-audio"]);
    assert_eq!(waiting(&s)[0]["track"], json!(next), "reattached");

    let mut client = s.client();
    let error = client
        .call::<Value>(
            "review.accept",
            json!({"facts": [f], "expect": shown, "source": {"kind": "dj"}}),
        )
        .unwrap_err();
    assert_eq!(error.code, "review_changed", "{error:?}");
    assert!(error.message.contains(&f.to_string()), "{error:?}");
    assert_eq!(db_rating(&s, &next), 0);
}

#[test]
fn a_fact_recorded_while_an_accept_is_being_written_waits_until_the_write_is_done() {
    let s = Sandbox::new();
    let gate = Gate::new();
    let _service = gated(&s, &gate);
    let f = fact(&s, "10001", "energy", "4", "agent:tagger", "0.7");

    gate.hold("selected");
    let accept = spawn(&s, &["review", "accept", &f.to_string(), "--source", "dj"]);
    gate.reached("at-selected");
    let record = spawn(
        &s,
        &[
            "facts",
            "record",
            "10001",
            "energy",
            "2",
            "--source",
            "agent:tagger",
            "--confidence",
            "0.7",
        ],
    );
    // The recording is held back until the accept's write is done.
    gate.reached("facts-waiting");
    gate.go("selected");
    let (accepted, answer) = finished(accept);
    assert!(accepted, "{answer}");
    let (recorded, recording) = finished(record);
    assert!(recorded, "{recording}");
    assert_eq!(db_rating(&s, "10001"), 4);
    // The DJ's value stands; the agent's later answer is outranked by it.
    let standing = standing(&s, "10001");
    assert_eq!(standing["source"]["kind"], "dj", "{standing}");
    assert_eq!(standing["value"], 4);
}

#[test]
fn a_batch_that_fails_part_way_records_what_it_applied_and_says_which_were_not() {
    let s = Sandbox::new();
    let _service = serve_with(&s, &[("WRECKORDBOX_REVIEW_FAIL_AT", "2")]);
    let a = fact(&s, "10001", "energy", "4", "agent:tagger", "0.7");
    let b = fact(&s, "10002", "energy", "2", "agent:tagger", "0.7");
    let mut watcher = s.client();
    watcher.subscribe().unwrap();

    let error = refused(&s, &["review", "accept", "--all", "--source", "dj"]);
    assert_eq!(error["code"], "review_partial", "{error}");
    let message = error["message"].as_str().unwrap();
    assert!(message.contains(&format!("applied: [{a}]")), "{message}");
    assert!(
        message.contains(&format!("not applied: [{b}]")),
        "{message}"
    );
    // What was applied is in the index, recorded, the DJ's and told.
    assert_eq!(rating(&s, "10001"), 4);
    assert_eq!(rating(&s, "10002"), 0);
    assert_eq!(library_changes(&mut watcher), 1);
    assert_eq!(standing(&s, "10001")["source"]["kind"], "dj");
    let runs = ok(&s, &["runs", "list"]);
    let dj: Vec<&Value> = runs["runs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["source"]["kind"] == "dj")
        .collect();
    assert_eq!(dj.len(), 1, "{runs}");
    let shown = ok(&s, &["runs", "show", &dj[0]["id"].to_string()]);
    let library: Vec<&Value> = shown["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["store"] == "library")
        .collect();
    assert_eq!(library.len(), 1, "{shown}");
    assert_eq!(library[0]["values"].as_array().unwrap().len(), 1, "{shown}");
    // What was not applied still waits.
    assert_eq!(facts_waiting(&s), vec![b]);
}

#[test]
fn a_crash_between_setters_is_reconciled_to_the_djs_values_and_hides_no_other_fact() {
    let s = Sandbox::new();
    let gate = Gate::new();
    let mut service = gated(&s, &gate);
    fact(&s, "10001", "energy", "4", "agent:tagger", "0.7");
    fact(&s, "10002", "energy", "2", "agent:tagger", "0.7");
    let other = fact(&s, "10003", "energy", "5", "agent:tagger", "0.7");

    gate.hold("between");
    let accept = spawn(
        &s,
        &[
            "review", "accept", "--track", "10001", "--track", "10002", "--source", "dj",
        ],
    );
    gate.reached("at-between");
    service.0.kill().unwrap();
    service.0.wait().unwrap();
    let (success, _) = finished(accept);
    assert!(!success, "the accept was cut off");

    // The next service recovers it: the value written is the DJ's.
    assert_eq!(db_rating(&s, "10001"), 4);
    assert_eq!(db_rating(&s, "10002"), 0);
    let standing = standing(&s, "10001");
    assert_eq!(standing["source"]["kind"], "dj", "{standing}");
    assert_eq!(standing["value"], 4);
    // The cut-off write bars only its own tracks: the rest of the queue
    // still waits.
    assert!(facts_waiting(&s).contains(&other), "{:?}", waiting(&s));
    stop_service(&s);
}

#[test]
fn the_same_suggestion_recorded_again_after_a_rejection_is_never_offered_or_projected() {
    let s = Sandbox::new();
    ok(&s, &["status"]);
    let f = fact(&s, "10001", "energy", "4", "agent:tagger", "0.7");
    ok(&s, &["review", "reject", &f.to_string(), "--source", "dj"]);

    // The agent sends the same answer again: a new Fact, the same
    // suggestion.
    let again = fact(&s, "10001", "energy", "4", "agent:tagger", "0.8");
    assert_ne!(again, f);
    assert!(facts_waiting(&s).is_empty(), "{:?}", waiting(&s));
    policy(&s, "[energy]\napply = 0.1\n");
    let applied = ok(&s, &["project"]);
    assert_eq!(applied["changes"], json!([]), "{applied}");
    assert_eq!(rating(&s, "10001"), 0);

    // A different value from the same source is a new suggestion.
    policy(&s, "");
    let different = fact(&s, "10001", "energy", "3", "agent:tagger", "0.7");
    assert_eq!(facts_waiting(&s), vec![different]);
}

#[test]
fn an_accept_must_declare_the_dj_on_every_request_even_in_the_djs_run() {
    let s = Sandbox::new();
    ok(&s, &["status"]);
    let f = fact(&s, "10001", "energy", "4", "agent:tagger", "0.7");
    let run = ok(
        &s,
        &["runs", "start", "--source", "dj", "--description", "review"],
    )["id"]
        .to_string();
    assert_eq!(
        refused(&s, &["review", "accept", &f.to_string(), "--run", &run])["code"],
        "review_not_the_djs"
    );
    // The envelope says an agent, the arguments say the DJ: refused.
    let mut client = s.client();
    client.in_run(Some(wreckordbox_service::ledger::RunArg {
        source: Some(serde_json::from_value(json!({"kind": "agent", "name": "x"})).unwrap()),
        ..Default::default()
    }));
    let error = client
        .call::<Value>(
            "review.accept",
            json!({"facts": [f], "source": {"kind": "dj"}}),
        )
        .unwrap_err();
    assert_eq!(error.code, "review_not_the_djs", "{error:?}");
    assert_eq!(db_rating(&s, "10001"), 0);
    // Declared, in the DJ's Run: accepted.
    ok(
        &s,
        &[
            "review",
            "accept",
            &f.to_string(),
            "--run",
            &run,
            "--source",
            "dj",
        ],
    );
    assert_eq!(rating(&s, "10001"), 4);
}

#[test]
fn the_dj_reopens_a_rejection_singly_by_filter_or_a_whole_reject_and_the_facts_wait_again() {
    let s = Sandbox::new();
    ok(&s, &["status"]);
    let a = fact(&s, "10001", "energy", "4", "agent:tagger", "0.7");
    let b = fact(&s, "10002", "energy", "2", "agent:tagger", "0.7");

    // A mistaken reject --all, undone as a whole.
    let rejected = ok(&s, &["review", "reject", "--all", "--source", "dj"]);
    let entry = rejected["rejection"]["entry"].to_string();
    let run = rejected["rejection"]["run"].to_string();
    assert!(facts_waiting(&s).is_empty());
    let listed = ok(&s, &["review", "rejected"]);
    assert_eq!(listed["count"], 2, "{listed}");
    assert_eq!(
        refused(&s, &["review", "reopen", "--rejection", &entry])["code"],
        "review_not_the_djs"
    );
    let reopened = ok(
        &s,
        &["review", "reopen", "--rejection", &entry, "--source", "dj"],
    );
    assert_eq!(reopened["count"], 2, "{reopened}");
    let mut now = facts_waiting(&s);
    now.sort_unstable();
    assert_eq!(now, vec![a, b]);
    assert_eq!(ok(&s, &["review", "rejected"])["count"], 0);
    // Append-only: the rejection is still in the Ledger, as it was.
    let shown = ok(&s, &["runs", "show", &run]);
    assert_eq!(shown["entries"].as_array().unwrap().len(), 1, "{shown}");
    let reopening = ok(
        &s,
        &["runs", "show", &reopened["reopening"]["run"].to_string()],
    );
    assert_eq!(reopening["run"]["source"]["kind"], "dj", "{reopening}");

    // Singly, by the Fact rejected.
    ok(&s, &["review", "reject", &a.to_string(), "--source", "dj"]);
    assert_eq!(facts_waiting(&s), vec![b]);
    ok(&s, &["review", "reopen", &a.to_string(), "--source", "dj"]);
    assert_eq!(facts_waiting(&s).len(), 2);

    // By filter.
    ok(&s, &["review", "reject", "--all", "--source", "dj"]);
    ok(
        &s,
        &["review", "reopen", "--track", "10002", "--source", "dj"],
    );
    assert_eq!(facts_waiting(&s), vec![b]);
    // Rejected again after reopening: rejected.
    ok(&s, &["review", "reject", &b.to_string(), "--source", "dj"]);
    assert!(facts_waiting(&s).is_empty());
}
