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

#[test]
fn reading_old_or_newer_sidecars_never_migrates_or_tidies_them() {
    for version in [1, 999] {
        let sandbox = Sandbox::new();
        // Ledger startup performs recovery; isolate the status query from it.
        sandbox.status();
        let dir = sandbox.sidecar_dir();
        std::fs::create_dir_all(dir.join("work")).unwrap();
        let work = dir.join("work/unfinished");
        std::fs::write(&work, b"leave this alone").unwrap();
        let path = dir.join("sidecar.db");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(include_str!(
            "../../wreckordbox-sidecar/tests/fixtures/sidecar-v1.sql"
        ))
        .unwrap();
        db.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            [version.to_string()],
        )
        .unwrap();
        drop(db);
        let before = std::fs::read(&path).unwrap();
        sandbox.status();
        let result = sandbox.client().call::<Value>("runs.active", json!({}));
        assert!(work.exists(), "status must not tidy work");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "status must not migrate or write"
        );
        assert!(!dir.join("sidecar.db-wal").exists());
        assert!(!dir.join("sidecar.db-shm").exists());
        assert_eq!(
            result.unwrap(),
            Value::Null,
            "unsupported schemas are unknown"
        );
    }
}

#[test]
fn a_status_read_of_a_current_sidecar_preserves_work_and_database_files() {
    let sandbox = Sandbox::new();
    // Install the closed fixture after startup, before any Sidecar writer opens.
    sandbox.status();
    let dir = sandbox.sidecar_dir();
    let sidecar = wreckordbox_sidecar::Sidecar::open(&dir).unwrap();
    let id = sidecar
        .begin_run(
            &wreckordbox_sidecar::Source::new(wreckordbox_sidecar::SourceKind::Agent, "test"),
            "Already running",
        )
        .unwrap();
    drop(sidecar);
    std::fs::create_dir_all(dir.join("work")).unwrap();
    std::fs::write(dir.join("work/keep"), b"keep").unwrap();
    let before = std::fs::read(dir.join("sidecar.db")).unwrap();
    sandbox.status();
    let mut client = sandbox.client();
    for _ in 0..2 {
        assert_eq!(
            client.call::<Value>("runs.active", json!({})).unwrap(),
            json!([{"id":id.0,"description":"Already running"}])
        );
    }
    assert!(dir.join("work/keep").exists());
    assert_eq!(std::fs::read(dir.join("sidecar.db")).unwrap(), before);
    assert!(!dir.join("sidecar.db-wal").exists());
    assert!(!dir.join("sidecar.db-shm").exists());
}

#[test]
fn unavailable_status_backs_off_at_the_service_and_recovers() {
    let sandbox = Sandbox::new();
    // Install the closed fixture after startup, before any Sidecar writer opens.
    sandbox.status();
    let dir = sandbox.sidecar_dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("sidecar.db"), b"corrupt fixture").unwrap();
    sandbox.status();
    let mut client = sandbox.client();
    assert_eq!(
        client.call::<Value>("runs.active", json!({})).unwrap(),
        Value::Null
    );
    // Repair the fixture: the service must not retry every client's poll.
    std::fs::remove_file(dir.join("sidecar.db")).unwrap();
    drop(wreckordbox_sidecar::Sidecar::open(&dir).unwrap());
    assert_eq!(
        client.call::<Value>("runs.active", json!({})).unwrap(),
        Value::Null
    );
    std::thread::sleep(Duration::from_millis(10100));
    assert_eq!(
        client.call::<Value>("runs.active", json!({})).unwrap(),
        json!([])
    );
}
