//! Background Fact producers hold the shared Fact coordination through
//! their commit (wbx-j8k): Review's accept never runs while a separation,
//! an Intensity measuring or a Tag schema conversion is recording, and the
//! hold deadlocks with nothing (an accept, Projection's apply, a Stick
//! load). Interleavings are made with the service's test-mode holds
//! (`WRECKORDBOX_REVIEW_GATE`, `hold-producer-<name>`): a file handshake,
//! never a timed window. No real separation is run: the engine's fake
//! worker stands in.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
use crate::common;
use crate::review::{fact, ok, token};

use std::path::Path;
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use common::Sandbox;
use serde_json::{json, Value};

/// The service's test-mode holds: `hold-P` stops at point `P`, says so in
/// `at-P`, and waits for `go-P`. A Fact command kept waiting by the
/// coordination writes `facts-waiting`.
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

    fn go(&self, point: &str) {
        std::fs::write(self.path().join(format!("go-{point}")), b"").unwrap();
    }

    fn has(&self, name: &str) -> bool {
        self.path().join(name).exists()
    }

    fn reached(&self, name: &str) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !self.has(name) {
            assert!(Instant::now() < deadline, "{name} never appeared");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// A service of this test's, killed and waited for when dropped.
struct Serving(Child);

impl Drop for Serving {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn serve_with(s: &Sandbox, env: &[(&str, &Path)]) -> Serving {
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
    serve_with(s, &[("WRECKORDBOX_REVIEW_GATE", gate.path())])
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

/// Waits for `child` to end (at most a minute: a deadlock fails the test
/// rather than hanging it) and reads its JSON answer.
fn finished(mut child: Child) -> (bool, Value) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("the command never finished: a deadlock");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let output = child.wait_with_output().unwrap();
    let text = if output.status.success() {
        String::from_utf8(output.stdout).unwrap()
    } else {
        String::from_utf8(output.stderr).unwrap()
    };
    let value = serde_json::from_str(text.trim()).unwrap_or(json!({ "text": text }));
    (output.status.success(), value)
}

/// Waits until the accept is kept waiting by the coordination, failing if
/// it finishes first (it ran while the producer was recording).
fn kept_waiting(gate: &Gate, accept: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !gate.has("facts-waiting") {
        if let Some(status) = accept.try_wait().unwrap() {
            panic!("the accept ran while a producer was recording ({status})");
        }
        assert!(Instant::now() < deadline, "the accept never waited");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(accept.try_wait().unwrap().is_none());
}

/// What the Library file holds, read past the service.
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

/// How many Facts of `kind` the Sidecar holds for `track`, read past the
/// service (a read through it would wait for the hold too).
fn facts_in_sidecar(s: &Sandbox, track: &str, kind: &str) -> i64 {
    let db = rusqlite::Connection::open_with_flags(
        s.sidecar_dir().join("sidecar.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    db.query_row(
        "SELECT COUNT(*) FROM fact WHERE track_id = ?1 AND kind = ?2",
        [track, kind],
        |r| r.get(0),
    )
    .unwrap()
}

/// `review accept` of one Fact, by the DJ, with the listing's token.
fn accept(s: &Sandbox, fact: i64, token: &str) -> Child {
    spawn(
        s,
        &[
            "review",
            "accept",
            &fact.to_string(),
            "--token",
            token,
            "--source",
            "dj",
        ],
    )
}

#[test]
fn an_accept_waits_while_a_separation_records_its_stems_and_then_is_written() {
    let s = Sandbox::new();
    let track = s.import_track("Night Bus.wav", 1);
    let gate = Gate::new();
    let _service = gated(&s, &gate);
    // No policy: the agent's Energy waits for the DJ.
    let energy = fact(&s, "10001", "energy", "4", "agent:tagger", "0.7");
    let t = token(&s);

    gate.hold("producer-stems");
    let separating = spawn(&s, &["stems", "separate", &track]);
    gate.reached("at-producer-stems");
    let mut accepting = accept(&s, energy, &t);

    // The separation is between its last observation and its commit: the
    // accept is kept out, nothing of it written.
    kept_waiting(&gate, &mut accepting);
    assert_eq!(db_rating(&s, "10001"), 0);
    assert_eq!(facts_in_sidecar(&s, &track, "stems"), 0);

    gate.go("producer-stems");
    let (separated, answer) = finished(separating);
    assert!(separated, "{answer}");
    let (accepted, answer) = finished(accepting);
    assert!(accepted, "{answer}");
    assert_eq!(facts_in_sidecar(&s, &track, "stems"), 1);
    assert_eq!(db_rating(&s, "10001"), 4);
}
