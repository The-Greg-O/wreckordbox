use super::*;

fn missing_manifest(case: &str, restart: bool, test: &str) {
    if std::env::var_os("WBX1411_ORPHAN_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--nocapture"])
            .env("WBX1411_ORPHAN_CHILD", "1")
            .env_remove("WRECKORDBOX_LEDGER_FAULTS")
            .env("RBXPORT_TEST", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let f = Fixture::new(1);
    // This child runs exactly one test. Inject only into the bulk recorder,
    // leaving the Keeper able to record the later DJ edit normally.
    std::env::set_var("WRECKORDBOX_LEDGER_FAULTS", "fail_append=999");
    let operation = Operation::new(&f.core, Kind::Analysis);
    std::env::remove_var("WRECKORDBOX_LEDGER_FAULTS");
    operation
        .run_many(
            json!({"track":fixture::track_id(0)}),
            128,
            || {
                f.core.write_then(
                    &NoHost,
                    History::Outside,
                    |w| {
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
                (0..128)
                    .map(|i| json!({"track":fixture::track_id(0),"part":i}))
                    .collect()
            },
        )
        .unwrap();
    assert!(operation.finish().unwrap().contains("accepted"));
    drop(operation);
    let ledger_root = std::fs::read_dir(f.dir.path().join("recovery/ledger"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let spill = std::fs::read_dir(ledger_root.join("ledger-bulk"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let batch = spill.join("batch-00000000000000000001.json");
    let accepted = std::fs::read(&batch).unwrap();
    // Construct the filesystem states at the two interrupted save/cleanup
    // boundaries, using a real completed operation's write-side retry queue.
    std::fs::remove_file(spill.join("manifest.json")).unwrap();
    if case != "batches" {
        std::fs::remove_file(&batch).unwrap();
    }
    if case == "temporary" {
        std::fs::write(spill.join("manifest.tmp"), b"{\"intent\":").unwrap();
    }
    let Fixture { core, service, dir } = f;
    let service = if restart {
        service.stop();
        service.stopped().wait();
        drop(service);
        drop(core);
        Service::start(
            rbl_db::LibraryLocation {
                master_db: dir.path().join("library/master.db"),
                share_root: dir.path().join("library/share"),
                passphrase: fixture::FIXTURE_PASSPHRASE.into(),
                is_real_install: false,
            },
            &Config {
                runtime_dir: dir.path().join("run"),
                recovery_dir: dir.path().join("recovery"),
                idle_exit: None,
            },
        )
        .unwrap()
    } else {
        // Service keeps the hosted core alive.
        service
    };
    if !restart && case != "batches" {
        let runs = service.call("runs.list", json!({})).unwrap();
        for run in runs["runs"].as_array().unwrap() {
            service.call("runs.show", json!({"id":run["id"]})).unwrap();
        }
        assert!(spill.is_dir(), "reads must leave the empty spill untouched");
        if case == "temporary" {
            assert_eq!(
                std::fs::read(spill.join("manifest.tmp")).unwrap(),
                b"{\"intent\":"
            );
        }
    }
    if restart && case != "batches" {
        let runs = service
            .call("runs.list", json!({}))
            .expect("restart must settle");
        for run in runs["runs"].as_array().unwrap() {
            service.call("runs.show", json!({"id":run["id"]})).unwrap();
            assert!(
                !run["finished_at"].is_null(),
                "startup must end the old Run"
            );
        }
    }
    service
        .call("protection.set", json!({"protect_library":false}))
        .unwrap();
    service
        .call("tracks.rate", json!({"id":fixture::track_id(0),"rating":4}))
        .expect("a later DJ edit must be accepted");
    let sidecar = rusqlite::Connection::open_with_flags(
        dir.path().join("library/wreckordbox-sidecar/sidecar.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let ratings: i64 = sidecar.query_row(
        "SELECT COUNT(*) FROM ledger WHERE json_extract(command, '$.name') = 'tracks.rate' AND source_kind = 'dj'",
        [], |r| r.get(0)).unwrap();
    assert_eq!(
        ratings, 1,
        "the later DJ edit must be recorded with the DJ source"
    );
    if case == "batches" {
        assert_eq!(
            std::fs::read(&batch).unwrap(),
            accepted,
            "accepted writes must be kept byte-for-byte"
        );
        let error = service.call("runs.list", json!({})).unwrap_err();
        assert!(
            error
                .message
                .contains(spill.file_name().unwrap().to_str().unwrap()),
            "{error:?}"
        );
        assert!(error.message.contains("manifest"), "{error:?}");
        assert_eq!(
            std::fs::read(&batch).unwrap(),
            accepted,
            "queries must not discard accepted writes"
        );
    } else {
        assert!(
            !spill.exists(),
            "empty spill must be removed on the write side"
        );
        let runs = service.call("runs.list", json!({})).unwrap();
        for run in runs["runs"].as_array().unwrap() {
            service.call("runs.show", json!({"id":run["id"]})).unwrap();
        }
    }
    service.stop();
}

#[test]
fn temporary_manifest_allows_later_dj_edit() {
    missing_manifest(
        "temporary",
        false,
        "orphans::temporary_manifest_allows_later_dj_edit",
    );
}
#[test]
fn empty_directory_allows_later_dj_edit() {
    missing_manifest(
        "empty",
        false,
        "orphans::empty_directory_allows_later_dj_edit",
    );
}
#[test]
fn temporary_manifest_allows_restart_to_settle() {
    missing_manifest(
        "temporary",
        true,
        "orphans::temporary_manifest_allows_restart_to_settle",
    );
}
#[test]
fn empty_directory_allows_restart_to_settle() {
    missing_manifest(
        "empty",
        true,
        "orphans::empty_directory_allows_restart_to_settle",
    );
}
#[test]
fn batches_without_manifest_survive_later_dj_edit_and_are_reported() {
    missing_manifest(
        "batches",
        false,
        "orphans::batches_without_manifest_survive_later_dj_edit_and_are_reported",
    );
}
#[test]
fn batches_without_manifest_survive_restart_and_are_reported() {
    missing_manifest(
        "batches",
        true,
        "orphans::batches_without_manifest_survive_restart_and_are_reported",
    );
}
