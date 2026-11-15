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
    if case == "active" {
        let listed = f.service.call("runs.spills.list", json!({})).unwrap();
        let id = &listed["spills"][0]["id"];
        for command in ["runs.spills.replay", "runs.spills.discard"] {
            let refused = f
                .service
                .call(
                    command,
                    json!({"id":id,"source":{"kind":"dj","name":"dj"},"reason":"fixture"}),
                )
                .unwrap_err();
            assert!(
                refused.message.contains("still being written"),
                "{refused:?}"
            );
        }
        assert_eq!(
            f.service
                .call("runs.spills.show", json!({"id":id}))
                .unwrap()["pending"]["total"],
            128
        );
        assert!(operation.finish().is_some());
        f.service.stop();
        return;
    }
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
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(spill.join("manifest.json")).unwrap()).unwrap();
    std::fs::remove_file(spill.join("manifest.json")).unwrap();
    if !case.starts_with("batches") {
        std::fs::remove_file(&batch).unwrap();
    }
    if case == "temporary" {
        std::fs::write(spill.join("manifest.tmp"), b"{\"intent\":").unwrap();
    }
    let Fixture { core, service, dir } = f;
    let mut service = if restart {
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
    if !restart && !case.starts_with("batches") {
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
    if restart && !case.starts_with("batches") {
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
    if case.starts_with("batches") {
        assert_eq!(
            std::fs::read(&batch).unwrap(),
            accepted,
            "accepted writes must be kept byte-for-byte"
        );
        let runs = service
            .call("runs.list", json!({}))
            .expect("pending spills must not hide Runs");
        assert!(!runs["recovery"]["spills"].as_array().unwrap().is_empty());
        let id = spill.file_name().unwrap().to_str().unwrap();
        let listed = service.call("runs.spills.list", json!({})).unwrap();
        assert_eq!(listed["total"], 1);
        let inspected = service
            .call("runs.spills.show", json!({"id":id,"limit":2}))
            .unwrap();
        assert_eq!(inspected["pending"]["total"], 128);
        assert_eq!(inspected["pending"]["records"].as_array().unwrap().len(), 2);
        assert_eq!(
            std::fs::read(&batch).unwrap(),
            accepted,
            "queries must not discard accepted writes"
        );
        if case == "batches-replay" {
            assert!(service
                .call("runs.spills.replay", json!({"id":id}))
                .is_err());
            let answer = service
                .call("runs.spills.replay", json!({"id":id,"manifest":manifest}))
                .unwrap();
            assert_eq!(answer["replayed"], true);
            assert!(!spill.exists());
            let recorded: i64 = sidecar.query_row("SELECT COUNT(*) FROM ledger WHERE kind='analysis' AND source_kind='tool' AND source_name='wreckordbox-analysis'", [], |r|r.get(0)).unwrap();
            assert_eq!(recorded, 128);
            assert_eq!(
                service.call("runs.spills.list", json!({})).unwrap()["total"],
                0
            );
        }
        if case.starts_with("batches-discard") {
            for source in [
                json!({"kind":"tool","name":"test"}),
                json!({"kind":"agent","name":"test"}),
            ] {
                let refused = service
                    .call(
                        "runs.spills.discard",
                        json!({"id":id,"source":source,"reason":"unrecoverable"}),
                    )
                    .unwrap_err();
                assert_eq!(refused.code, "bad_request");
                assert_eq!(std::fs::read(&batch).unwrap(), accepted);
            }
            if case.starts_with("batches-discard-failure") {
                let writable = rusqlite::Connection::open(
                    dir.path().join("library/wreckordbox-sidecar/sidecar.db"),
                )
                .unwrap();
                writable.execute_batch("CREATE TRIGGER refuse_discard BEFORE INSERT ON ledger WHEN NEW.kind='ledger_spill_discard' BEGIN SELECT RAISE(ABORT,'fixture refusal'); END;").unwrap();
                assert!(service.call("runs.spills.discard", json!({"id":id,"source":{"kind":"dj","name":"dj"},"reason":"explicit fixture discard"})).is_err());
                assert_eq!(
                    std::fs::read(&batch).unwrap(),
                    accepted,
                    "a failed recording cannot discard accepted writes"
                );
                let receipt = std::fs::read(spill.join("discard.json")).unwrap();
                let read = service.call("runs.spills.show", json!({"id":id})).unwrap();
                assert_eq!(read["discard_pending"], true);
                assert_eq!(std::fs::read(spill.join("discard.json")).unwrap(), receipt);
                assert!(service
                    .call("runs.spills.replay", json!({"id":id,"manifest":manifest}))
                    .is_err());
                if case.ends_with("restart") {
                    service.stop();
                    service.stopped().wait();
                    service = Service::start(
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
                    .unwrap();
                    assert_eq!(
                        std::fs::read(&batch).unwrap(),
                        accepted,
                        "startup must not finish a discard"
                    );
                    assert_eq!(std::fs::read(spill.join("discard.json")).unwrap(), receipt);
                    assert_eq!(
                        service.call("runs.spills.show", json!({"id":id})).unwrap()
                            ["discard_pending"],
                        true
                    );
                    assert_eq!(
                        sidecar
                            .query_row(
                                "SELECT COUNT(*) FROM ledger WHERE kind='ledger_spill_discard'",
                                [],
                                |r| r.get::<_, i64>(0)
                            )
                            .unwrap(),
                        0
                    );
                }
                writable
                    .execute_batch("DROP TRIGGER refuse_discard")
                    .unwrap();
            }
            service.call("runs.spills.discard", json!({"id":id,"source":{"kind":"dj"},"reason":"I choose to discard these accepted-write records"})).unwrap();
            assert!(!spill.exists());
            let recorded: i64 = sidecar.query_row("SELECT COUNT(*) FROM ledger WHERE kind='ledger_spill_discard' AND source_kind='dj'", [], |r|r.get(0)).unwrap();
            assert_eq!(recorded, 1);
            assert_eq!(
                service.call("runs.spills.list", json!({})).unwrap()["total"],
                0
            );
        }
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

#[test]
fn orphan_replay_preserves_original_source_and_clears_diagnostic() {
    missing_manifest(
        "batches-replay",
        false,
        "orphans::orphan_replay_preserves_original_source_and_clears_diagnostic",
    );
}
#[test]
fn orphan_discard_is_a_recorded_explicit_dj_act() {
    missing_manifest(
        "batches-discard",
        false,
        "orphans::orphan_discard_is_a_recorded_explicit_dj_act",
    );
}

#[test]
fn discard_recording_failure_preserves_bytes_and_retries_once() {
    missing_manifest(
        "batches-discard-failure",
        false,
        "orphans::discard_recording_failure_preserves_bytes_and_retries_once",
    );
}

#[test]
fn explicit_recovery_refuses_an_operation_still_writing_its_spill() {
    missing_manifest(
        "active",
        false,
        "orphans::explicit_recovery_refuses_an_operation_still_writing_its_spill",
    );
}

#[test]
fn startup_preserves_a_pending_discard_until_the_dj_explicitly_retries() {
    missing_manifest(
        "batches-discard-failure-restart",
        false,
        "orphans::startup_preserves_a_pending_discard_until_the_dj_explicitly_retries",
    );
}
