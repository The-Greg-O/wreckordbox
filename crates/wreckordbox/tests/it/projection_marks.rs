//! A projection's mark is evidence of its ownership only once the write it
//! was made for is confirmed in the Ledger (wbx-29 review): Energy's, as
//! the MyTag projection's, through the real CLI and a service on a fixture
//! Library started with Ledger faults.
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

fn energy(s: &Sandbox, track: &str, value: &str) {
    ok(
        s,
        &[
            "facts",
            "record",
            track,
            "energy",
            value,
            "--source",
            "agent:tagger",
            "--confidence",
            "0.9",
        ],
    );
}

/// A second Energy projection killed after its write and before its Ledger
/// entry: the value it wrote is not the projection's, whether or not the
/// DJ clears the barrier it left.
#[test]
fn a_crash_during_a_second_energy_projection_leaves_its_value_the_djs_cleared_or_not() {
    let s = Sandbox::new();
    std::fs::create_dir_all(s.sidecar_dir()).unwrap();
    std::fs::write(
        s.sidecar_dir().join("projection-policy.toml"),
        "[energy]\napply = 0.5\n",
    )
    .unwrap();
    energy(&s, "10001", "3");
    ok(&s, &["project"]);
    energy(&s, "10001", "4");
    stop_service(&s);
    let mut command = s.command();
    command
        .env("WRECKORDBOX_LEDGER_FAULTS", "abort_after_write")
        .args(["--json", "project"]);
    assert!(!s.run(&mut command).status.success());
    energy(&s, "10001", "5");
    let reason = |report: &Value| {
        assert_eq!(report["changes"], serde_json::json!([]), "{report}");
        report["kept"][0]["reason"].as_str().unwrap().to_owned()
    };
    let dry = ok(&s, &["project", "--dry-run"]);
    assert_eq!(reason(&dry), "written_since");
    let barriers = ok(&s, &["projection", "barriers"]);
    let seq = barriers["barriers"][0]["seq"].to_string();
    ok(&s, &["projection", "acknowledge", &seq, "--source", "dj"]);
    let cleared = ok(&s, &["project", "--dry-run"]);
    assert_eq!(reason(&cleared), "written_since");
    assert_eq!(ok(&s, &["tracks", "show", "10001"])["rating"], 4);
}
