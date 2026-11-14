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
    fact_of(s, track, "energy", value);
}

fn fact_of(s: &Sandbox, track: &str, kind: &str, value: &str) {
    ok(
        s,
        &[
            "facts",
            "record",
            track,
            kind,
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
    wait_for_no_service(&s);
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

/// The same for Prep colour: a second projection cut off after its write.
#[test]
fn a_crash_during_a_second_prep_colour_projection_leaves_its_value_the_djs_cleared_or_not() {
    let s = Sandbox::new();
    std::fs::create_dir_all(s.sidecar_dir()).unwrap();
    std::fs::write(
        s.sidecar_dir().join("projection-policy.toml"),
        "[prep_colour]\napply = 0.5\n",
    )
    .unwrap();
    fact_of(&s, "10001", "prep_colour", "Green");
    ok(&s, &["project"]);
    fact_of(&s, "10001", "prep_colour", "Red");
    stop_service(&s);
    let mut command = s.command();
    command
        .env("WRECKORDBOX_LEDGER_FAULTS", "abort_after_write")
        .args(["--json", "project"]);
    assert!(!s.run(&mut command).status.success());
    wait_for_no_service(&s);
    fact_of(&s, "10001", "prep_colour", "Blue");
    let kept = |report: &Value| {
        assert_eq!(report["changes"], serde_json::json!([]), "{report}");
        report["kept"][0]["reason"].as_str().unwrap().to_owned()
    };
    assert_eq!(kept(&ok(&s, &["project", "--dry-run"])), "written_since");
    let barriers = ok(&s, &["projection", "barriers"]);
    let seq = barriers["barriers"][0]["seq"].to_string();
    ok(&s, &["projection", "acknowledge", &seq, "--source", "dj"]);
    assert_eq!(kept(&ok(&s, &["project", "--dry-run"])), "written_since");
}

/// A mark kept before marks named their confirmed write (wbx-29 review)
/// is not evidence: the value is the DJ's, under its own reason, and
/// nothing is written or recorded as the DJ's Fact for it. No migration
/// guesses ownership from the old evidence.
#[test]
fn a_mark_kept_before_confirmed_marks_leaves_energy_and_prep_colour_the_djs() {
    let s = Sandbox::new();
    std::fs::create_dir_all(s.sidecar_dir()).unwrap();
    std::fs::write(
        s.sidecar_dir().join("projection-policy.toml"),
        "[energy]\napply = 0.5\n\n[prep_colour]\napply = 0.5\n",
    )
    .unwrap();
    energy(&s, "10001", "3");
    fact_of(&s, "10001", "prep_colour", "Green");
    ok(&s, &["project"]);
    stop_service(&s);
    {
        // The mark as the previous version kept it: no entries.
        let sidecar = wreckordbox_sidecar::Sidecar::open(&s.sidecar_dir()).unwrap();
        let mut mark = sidecar.provenance(&["Rating", "ColorID"]).unwrap().marks["10001"].clone();
        assert!(!mark.field_entries.is_empty(), "this version names them");
        mark.field_entries.clear();
        sidecar
            .set_projection_marks(&[("10001".to_owned(), mark)])
            .unwrap();
    }
    energy(&s, "10001", "4");
    fact_of(&s, "10001", "prep_colour", "Red");
    let mut reasons: Vec<(String, String)> = ok(&s, &["project", "--dry-run"])["kept"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| {
            (
                k["field"].as_str().unwrap().to_owned(),
                k["reason"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    reasons.sort();
    assert_eq!(
        reasons,
        [
            ("energy".to_owned(), "predates_confirmed_marks".to_owned()),
            (
                "prep_colour".to_owned(),
                "predates_confirmed_marks".to_owned()
            ),
        ]
    );
    let applied = ok(&s, &["project"]);
    assert_eq!(applied["changes"], serde_json::json!([]), "{applied}");
    let track = ok(&s, &["tracks", "show", "10001"]);
    assert_eq!(track["rating"], 3);
    assert_eq!(track["colour_id"], 5);
    for kind in ["energy", "prep_colour"] {
        let standing = ok(&s, &["facts", "standing", "10001", kind]);
        assert_eq!(
            standing["standing"]["source"]["kind"], "agent",
            "{standing}"
        );
    }
}

/// Waits until the service a crashed command was answered by is gone, so
/// the next command starts (and recovers in) a service of its own.
fn wait_for_no_service(s: &Sandbox) {
    let socket = s.config().socket_path(s.location());
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while wreckordbox_service::Client::connect(&socket)
        .and_then(|mut client| client.status())
        .is_ok()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the service did not die"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
