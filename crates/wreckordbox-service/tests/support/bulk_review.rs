use super::*;

#[test]
fn one_append_failure_accounts_for_all_three_hundred_accepted_tracks() {
    if std::env::var_os("WBX1411_REVIEW_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "review::one_append_failure_accounts_for_all_three_hundred_accepted_tracks",
                "--nocapture",
            ])
            .env("WBX1411_REVIEW_CHILD", "1")
            .env("WRECKORDBOX_LEDGER_FAULTS", "fail_append=1")
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
    let f = Fixture::new(300);
    // The write result must remain successful even if recording degrades.
    let operation = Operation::new(&f.core, Kind::Analysis);
    for i in 0..300 {
        register(&f, &operation, i);
    }
    let _warning = operation.finish();
    let runs = f.runs();
    assert_eq!(runs["total"], 1);
    let entries = f
        .service
        .call(
            "runs.show",
            json!({"id":runs["runs"][0]["id"],"limit":1000}),
        )
        .unwrap();
    let tracks: std::collections::BTreeSet<_> = entries["entries"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|entry| entry["payload"]["records"].as_array().into_iter().flatten())
        .filter_map(|record| record["track"].as_str())
        .map(str::to_owned)
        .collect();
    assert_eq!(
        tracks,
        (0..300).map(fixture::track_id).collect(),
        "every accepted target must be named"
    );
}

fn register(f: &Fixture, operation: &Operation, i: usize) {
    operation
        .run(
            json!({"track":fixture::track_id(i)}),
            || {
                f.core.write_then(
                    &NoHost,
                    History::Outside,
                    |w| {
                        w.set_analysis(
                            &fixture::track_id(i),
                            &rbl_db::write::AnalysisWrite {
                                bpm_x100: 12345,
                                key: None,
                                analysis_path: "/fixture/ANLZ0000.DAT",
                                length_sec: None,
                            },
                        )
                    },
                    |_, value| Ok(value),
                )
            },
            |_| vec![json!({"track":fixture::track_id(i),"after":{"bpm_x100":12345}})],
        )
        .unwrap();
}

#[test]
fn acceptance_order_and_time_survive_an_intervening_dj_edit() {
    let f = Fixture::new(3);
    let operation = Operation::new(&f.core, Kind::Analysis);
    register(&f, &operation, 0);
    std::thread::sleep(std::time::Duration::from_millis(30));
    f.service
        .call("tracks.rate", json!({"id":fixture::track_id(2),"rating":3}))
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(30));
    register(&f, &operation, 1);
    assert_eq!(operation.finish(), None);
    let mut entries = Vec::new();
    for run in f.runs()["runs"].as_array().unwrap() {
        entries.extend(
            f.service
                .call("runs.show", json!({"id":run["id"]}))
                .unwrap()["entries"]
                .as_array()
                .unwrap()
                .clone(),
        );
    }
    let first = entries
        .iter()
        .find(|e| {
            e["payload"]["records"]
                .as_array()
                .is_some_and(|r| r.iter().any(|r| r["track"] == fixture::track_id(0)))
        })
        .unwrap();
    let last = entries
        .iter()
        .find(|e| {
            e["payload"]["records"]
                .as_array()
                .is_some_and(|r| r.iter().any(|r| r["track"] == fixture::track_id(1)))
        })
        .unwrap();
    let rating = entries
        .iter()
        .find(|e| e["command"]["name"] == "tracks.rate")
        .unwrap();
    assert!(
        first["id"].as_i64().unwrap() < rating["id"].as_i64().unwrap(),
        "first analysis must precede rating: {entries:?}"
    );
    assert!(rating["id"].as_i64().unwrap() < last["id"].as_i64().unwrap());
    assert!(first["at"].as_i64().unwrap() < rating["at"].as_i64().unwrap());
    assert!(rating["at"].as_i64().unwrap() < last["at"].as_i64().unwrap());
}

#[test]
fn unresolved_startup_work_refuses_bulk_admission_without_a_live_intent() {
    let f = Fixture::prepared(3, |root| {
        let sidecar =
            wreckordbox_sidecar::Sidecar::open(&root.join("wreckordbox-sidecar")).unwrap();
        std::fs::write(
            sidecar.dir().join(wreckordbox_service::ledger::SPILL_FILE),
            "damaged earlier entry\n",
        )
        .unwrap();
    });
    let operation = Operation::new(&f.core, Kind::Analysis);
    let result = operation.run(
        json!({"track":fixture::track_id(0)}),
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
                |_, value| Ok(value),
            )
        },
        |_| vec![json!({"track":fixture::track_id(0)})],
    );
    assert!(
        result.is_err(),
        "bulk must catch up earlier work before admission"
    );
    assert_eq!(operation.finish(), None);
    let db = rusqlite::Connection::open_with_flags(
        f.dir.path().join("library/wreckordbox-sidecar/sidecar.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM ledger_intent", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    std::fs::remove_file(
        f.dir
            .path()
            .join("library/wreckordbox-sidecar/ledger-spill.jsonl"),
    )
    .unwrap();
    f.service
        .call("tracks.rate", json!({"id":fixture::track_id(2),"rating":3}))
        .unwrap();
    let live = Operation::new(&f.core, Kind::Analysis);
    for _ in 0..129 {
        register(&f, &live, 0);
    }
    f.service
        .call("tracks.rate", json!({"id":fixture::track_id(2),"rating":4}))
        .unwrap();
    for run in f.runs()["runs"].as_array().unwrap() {
        let shown = f
            .service
            .call("runs.show", json!({"id":run["id"],"limit":1000}))
            .unwrap();
        assert!(shown["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["kind"] != "unconfirmed"));
        if run["source"]["kind"] == "tool" {
            assert!(run["finished_at"].is_null());
        }
    }
    assert_eq!(live.finish(), None);
}

fn counts(f: &Fixture) -> (i64, i64, i64) {
    let db = rusqlite::Connection::open_with_flags(
        f.dir.path().join("library/wreckordbox-sidecar/sidecar.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    (
        db.query_row("SELECT COUNT(*) FROM ledger_intent", [], |r| r.get(0))
            .unwrap(),
        db.query_row("SELECT COUNT(*) FROM ledger_order", [], |r| r.get(0))
            .unwrap(),
        db.query_row("SELECT COUNT(*) FROM dj_fact", [], |r| r.get(0))
            .unwrap(),
    )
}

#[test]
fn protected_stick_refuses_both_bulk_paths_without_records_or_reservations() {
    let f = Fixture::prepared(3, |root| {
        wreckordbox_sidecar::Sidecar::open(&root.join("wreckordbox-sidecar")).unwrap();
    });
    let before = counts(&f);
    std::fs::write(
        f.dir
            .path()
            .join("library")
            .join(wreckordbox_stick_protection::STAND_IN_MARKER),
        "",
    )
    .unwrap();
    for kind in [Kind::Analysis, Kind::Import] {
        let operation = Operation::new(&f.core, kind);
        let paths = f.paths(1);
        let result = operation.run(
            json!({"track":fixture::track_id(0)}),
            || {
                f.core.write_then(
                    &NoHost,
                    History::Outside,
                    |w| match kind {
                        Kind::Analysis => w
                            .set_analysis(
                                &fixture::track_id(0),
                                &rbl_db::write::AnalysisWrite {
                                    bpm_x100: 12345,
                                    key: None,
                                    analysis_path: "/fixture/ANLZ0000.DAT",
                                    length_sec: None,
                                },
                            )
                            .map(|_| ()),
                        Kind::Import => w.import_files(&paths).map(|_| ()),
                        Kind::Link | Kind::RoundTrip => unreachable!("tested separately"),
                    },
                    |_, v| Ok(v),
                )
            },
            |_| vec![json!({"unexpected":true})],
        );
        assert!(rbl_library::stick::refused(&result.unwrap_err()));
        assert_eq!(operation.finish(), None);
    }
    assert_eq!(counts(&f), before);
    assert_eq!(f.runs()["total"], 0);
}

#[test]
fn dj_sourced_imports_create_no_dj_facts() {
    let f = Fixture::new(3);
    f.import(&f.paths(3));
    assert_eq!(f.runs()["runs"][0]["source"]["kind"], "dj");
    assert_eq!(counts(&f).2, 0);
}

#[test]
fn persistent_failure_names_pending_tracks_and_reads_leave_the_spill_untouched() {
    if std::env::var_os("WBX1411_PERSISTENT_CHILD").is_none() {
        let output=std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact","review::persistent_failure_names_pending_tracks_and_reads_leave_the_spill_untouched","--nocapture"])
            .env("WBX1411_PERSISTENT_CHILD","1").env("WRECKORDBOX_LEDGER_FAULTS","fail_append=999").env("RBXPORT_TEST","1").output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let f = Fixture::new(300);
    let operation = Operation::new(&f.core, Kind::Analysis);
    for i in 0..300 {
        register(&f, &operation, i);
    }
    assert!(operation.finish().unwrap().contains("accepted"));
    let before = counts(&f);
    let runs = f.runs();
    assert_eq!(runs["total"], 1);
    let args = json!({"id":runs["runs"][0]["id"],"limit":1000});
    let shown = f.service.call("runs.show", args.clone()).unwrap();
    assert_eq!(shown["pending_recording"]["total"], 300);
    let named: std::collections::BTreeSet<_> = shown["pending_recording"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["result"]["track"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(named, (0..300).map(fixture::track_id).collect());
    assert_eq!(f.service.call("runs.show", args).unwrap(), shown);
    assert_eq!(counts(&f), before);
    let typed: wreckordbox_service::ledger::RunShown =
        serde_json::from_value(shown.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(typed).unwrap()["pending_recording"],
        shown["pending_recording"],
        "typed CLI response must retain named pending writes"
    );
    let page = f
        .service
        .call(
            "runs.show",
            json!({"id":runs["runs"][0]["id"],"limit":128,"offset":128}),
        )
        .unwrap();
    assert_eq!(
        page["pending_recording"]["records"]
            .as_array()
            .unwrap()
            .len(),
        128
    );
    assert_eq!(page["pending_recording"]["next_offset"], 256);
    // A later admitted normal edit catches up completed work only.
    f.service
        .call("tracks.rate", json!({"id":fixture::track_id(0),"rating":4}))
        .unwrap();
    let recovered = f
        .service
        .call(
            "runs.show",
            json!({"id":runs["runs"][0]["id"],"limit":1000}),
        )
        .unwrap();
    assert_eq!(recovered["total"], 300);
    assert_eq!(recovered["pending_recording"]["total"], 0);
    assert!(recovered["entries"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["kind"] != "unconfirmed"));
}

#[test]
fn protected_existing_sidecar_reports_the_startup_refusal() {
    let f = Fixture::prepared(1, |root| {
        wreckordbox_sidecar::Sidecar::open(&root.join("wreckordbox-sidecar")).unwrap();
        std::fs::write(root.join(wreckordbox_stick_protection::STAND_IN_MARKER), "").unwrap();
    });
    let error = f.service.call("runs.list", json!({})).unwrap_err();
    assert_eq!(
        error.code,
        wreckordbox_service::protocol::codes::STICK_PROTECTED
    );
    assert!(!error.message.contains("appeared after"));
}

#[test]
fn protecting_a_stick_before_flush_keeps_recovery_off_the_stick() {
    let f = Fixture::new(129);
    let operation = Operation::new(&f.core, Kind::Analysis);
    for i in 0..129 {
        register(&f, &operation, i);
    }
    let run = f.runs()["runs"][0]["id"].clone();
    let marker = f
        .dir
        .path()
        .join("library")
        .join(wreckordbox_stick_protection::STAND_IN_MARKER);
    std::fs::write(&marker, "").unwrap();
    let before = counts(&f);
    assert!(operation.finish().is_some());
    assert!(
        !f.dir
            .path()
            .join("library/wreckordbox-sidecar/ledger-bulk")
            .exists(),
        "a deferred recording must not write recovery files onto a protected Stick"
    );
    assert_eq!(counts(&f), before);
    let shown = f
        .service
        .call("runs.show", json!({"id":run,"limit":1000}))
        .unwrap();
    assert_eq!(shown["total"], 128);
    assert_eq!(shown["pending_recording"]["total"], 1);
    assert_eq!(
        shown["pending_recording"]["records"][0]["result"]["track"],
        fixture::track_id(128)
    );
    assert_eq!(
        counts(&f),
        before,
        "reading pending results must not write either store"
    );
    // Fixture-only remount as an ordinary folder; never unlock a real Stick.
    std::fs::remove_file(marker).unwrap();
    f.service
        .call("tracks.rate", json!({"id":fixture::track_id(0),"rating":4}))
        .unwrap();
    let shown = f
        .service
        .call("runs.show", json!({"id":run,"limit":1000}))
        .unwrap();
    assert_eq!(shown["total"], 129);
    assert_eq!(shown["pending_recording"]["total"], 0);
}

#[test]
fn a_busy_stick_does_not_hold_up_the_accepted_write_for_recording() {
    use std::sync::mpsc::channel;
    let f = Fixture::new(1);
    let operation = Operation::new(&f.core, Kind::Analysis);
    register(&f, &operation, 0);
    let root = f.dir.path().join("library");
    let marker = root.join(wreckordbox_stick_protection::STAND_IN_MARKER);
    std::fs::write(&marker, "").unwrap();
    let state = f.dir.path().join("recovery");
    let (held, ready) = channel();
    let (release, wait) = channel();
    let holder = std::thread::spawn(move || {
        let _lock = wreckordbox_stick_protection::exec_lock(&state, &root).unwrap();
        held.send(()).unwrap();
        wait.recv().unwrap();
    });
    ready.recv().unwrap();
    let (done, finished) = channel();
    let recording = std::thread::spawn(move || done.send(operation.finish()).unwrap());
    let outcome = finished.recv_timeout(std::time::Duration::from_secs(2));
    // Always release, even on regression, so the test cannot wedge the queue.
    release.send(()).unwrap();
    holder.join().unwrap();
    recording.join().unwrap();
    assert!(
        outcome.is_ok(),
        "recording must spill promptly instead of waiting for a Stick writer"
    );
    assert!(outcome.unwrap().is_some());
    let pending = f.service.call("runs.list", json!({})).unwrap_err();
    assert!(
        pending.message.contains(&fixture::track_id(0)),
        "the reader must name the pending accepted track"
    );
    std::fs::remove_file(marker).unwrap();
    f.service
        .call("tracks.rate", json!({"id":fixture::track_id(0),"rating":4}))
        .unwrap();
    let runs = f.runs();
    let run = runs["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["source"]["kind"] == "tool" && r["source"]["name"] == "wreckordbox-analysis")
        .unwrap();
    let shown = f
        .service
        .call("runs.show", json!({"id":run["id"]}))
        .unwrap();
    assert_eq!(
        shown["entries"][0]["payload"]["records"][0]["track"],
        fixture::track_id(0)
    );
}

#[test]
fn an_unavailable_sidecar_parent_is_not_reported_as_an_empty_ledger() {
    let f = Fixture::new(1);
    let sidecar = f.dir.path().join("library/wreckordbox-sidecar");
    std::fs::write(&sidecar, "fixture obstruction").unwrap();
    let operation = Operation::new(&f.core, Kind::Analysis);
    register(&f, &operation, 0);
    assert!(operation.finish().is_some());
    let pending = f.service.call("runs.list", json!({})).unwrap_err();
    assert!(
        pending.message.contains(&fixture::track_id(0)),
        "the reader must name accepted results that have no Run yet"
    );
    std::fs::remove_file(sidecar).unwrap();
    f.service
        .call("tracks.rate", json!({"id":fixture::track_id(0),"rating":4}))
        .unwrap();
    let runs = f.runs();
    assert_eq!(runs["total"], 2);
    let run = runs["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["source"]["kind"] == "tool" && r["source"]["name"] == "wreckordbox-analysis")
        .unwrap();
    let shown = f
        .service
        .call("runs.show", json!({"id":run["id"]}))
        .unwrap();
    assert_eq!(
        shown["entries"][0]["payload"]["records"][0]["track"],
        fixture::track_id(0)
    );
}
