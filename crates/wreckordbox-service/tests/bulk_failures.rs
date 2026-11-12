//! Real process interruption, and a recording failure independent of the accepted write.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
use rbl_db::{
    fixture::{self, Shape},
    LibraryLocation,
};
use rbl_library::{
    bulk::{Kind, Operation},
    History, LibraryCore, NoHost,
};
use serde_json::{json, Value};
use std::{path::Path, process::Command, sync::Arc};
use wreckordbox_service::{Config, Service};
fn config(root: &Path) -> Config {
    Config {
        runtime_dir: root.join("run"),
        recovery_dir: root.join("recovery"),
        idle_exit: None,
    }
}
fn location(root: &Path) -> LibraryLocation {
    LibraryLocation {
        master_db: root.join("library/master.db"),
        share_root: root.join("library/share"),
        passphrase: fixture::FIXTURE_PASSPHRASE.into(),
        is_real_install: false,
    }
}
#[test]
fn worker() {
    let Ok(root) = std::env::var("WBX1411_CHILD") else {
        return;
    };
    let root = Path::new(&root);
    std::fs::create_dir_all(root.join("library")).unwrap();
    let location = fixture::build(
        &root.join("library"),
        Shape {
            tracks: 2,
            ..Shape::default()
        },
    )
    .unwrap();
    let core = Arc::new(LibraryCore::new(root.join("recovery")));
    core.set_protection(false).unwrap();
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (index, _) = rbl_index::load(&db).unwrap();
    core.set_library(
        &NoHost,
        index,
        false,
        db.schema().db_version,
        0,
        location.clone(),
    );
    drop(db);
    let host: Arc<dyn rbl_library::Host + Send + Sync> = Arc::new(NoHost);
    let service = Service::host(&location, &config(root), &core, &host).unwrap();
    let operation = Operation::new(&core, Kind::Analysis);
    let mode = std::env::var("WBX1411_MODE").unwrap();
    operation
        .run_many(
            json!({"track":fixture::track_id(0)}),
            129,
            || {
                core.write_then(
                    &NoHost,
                    History::Outside,
                    |w| {
                        if mode == "before" {
                            std::process::abort();
                        }
                        w.set_analysis(
                            &fixture::track_id(0),
                            &rbl_db::write::AnalysisWrite {
                                bpm_x100: 12345,
                                key: None,
                                analysis_path: "/fixture/ANLZ0000.DAT",
                                length_sec: None,
                            },
                        )
                    },
                    |_, v| Ok(v),
                )
            },
            |_| {
                (0..129)
                    .map(|i| json!({"track":fixture::track_id(0),"part":i}))
                    .collect()
            },
        )
        .unwrap();
    if mode == "after_batch" {
        std::process::abort();
    }
    let warning = operation.finish();
    assert!(warning.as_deref().unwrap().contains("accepted"));
    let sidecar = rusqlite::Connection::open_with_flags(
        root.join("library/wreckordbox-sidecar/sidecar.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let before: i64 = sidecar
        .query_row("SELECT COUNT(*) FROM ledger_intent", [], |r| r.get(0))
        .unwrap();
    assert_eq!(before, 1);
    for _ in 0..3 {
        assert_eq!(service.call("runs.list", json!({})).unwrap()["total"], 1);
    }
    assert_eq!(
        sidecar
            .query_row("SELECT COUNT(*) FROM ledger_intent", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        before,
        "reads must not recover"
    );
    assert_eq!(
        core.open_read_only()
            .unwrap()
            .connection()
            .query_row(
                "SELECT BPM FROM djmdContent WHERE ID=?1",
                [fixture::track_id(0)],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
        12345
    );
    service.stop();
}
#[test]
fn interrupted_operations_recover_in_their_own_run_and_recording_failure_does_not_fail_the_write() {
    for mode in ["before", "after", "after_batch", "append_failure"] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "worker", "--nocapture"])
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", root.join("home"))
            .env("RBXPORT_TEST", "1")
            .env("WBX1411_CHILD", root)
            .env("WBX1411_MODE", mode)
            .env(
                "WRECKORDBOX_LEDGER_FAULTS",
                match mode {
                    "after" => "abort_after_write",
                    "append_failure" => "fail_append=99",
                    _ => "",
                },
            )
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            mode == "append_failure",
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let location = location(root);
        let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
        let bpm: u32 = db
            .connection()
            .query_row(
                "SELECT BPM FROM djmdContent WHERE ID=?1",
                [fixture::track_id(0)],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(bpm == 12345, mode != "before");
        drop(db);
        let service = Service::start(location, &config(root)).unwrap();
        let runs = service.call("runs.list", json!({})).unwrap();
        assert_eq!(runs["total"], 1, "{mode}: {runs}");
        let shown = service
            .call(
                "runs.show",
                json!({"id":runs["runs"][0]["id"],"limit":1000}),
            )
            .unwrap();
        let entries = shown["entries"].as_array().unwrap();
        assert_eq!(
            entries
                .iter()
                .filter(|e| e["kind"] == "unconfirmed")
                .count(),
            if mode == "append_failure" { 0 } else { 1 },
            "{mode}: {shown}"
        );
        assert!(entries.iter().all(|e| e["source"]["kind"] == "tool"));
        if mode == "after_batch" {
            assert_eq!(entries.len(), 129);
        }
        if mode == "append_failure" {
            let parts: std::collections::BTreeSet<_> = entries
                .iter()
                .flat_map(|e| e["payload"]["records"].as_array().into_iter().flatten())
                .filter_map(|r| r["part"].as_u64())
                .collect();
            assert_eq!(
                parts,
                (0..129).collect(),
                "every spilled accepted result must replay"
            );
        }
        let snapshot: Value = service
            .call(
                "runs.show",
                json!({"id":runs["runs"][0]["id"],"limit":1000}),
            )
            .unwrap();
        assert_eq!(shown, snapshot);
        service.stop();
    }
}
