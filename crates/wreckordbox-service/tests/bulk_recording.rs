//! Bulk registration/import cost and persisted Ledger effects on fixture Libraries.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
use rbl_db::fixture::{self, Shape};
use rbl_library::bulk::{Kind, Operation};
use rbl_library::{History, LibraryCore, NoHost};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc, time::Instant};
use wreckordbox_service::{Config, Service};

struct Fixture {
    core: Arc<LibraryCore>,
    service: Service,
    dir: tempfile::TempDir,
}
impl Fixture {
    fn new(tracks: usize) -> Self {
        Self::prepared(tracks, |_| {})
    }
    fn prepared(tracks: usize, prepare: impl FnOnce(&std::path::Path)) -> Self {
        Self::with_host(tracks, prepare, |_| Arc::new(NoHost))
    }
    fn with_host(
        tracks: usize,
        prepare: impl FnOnce(&std::path::Path),
        host: impl FnOnce(&Arc<LibraryCore>) -> Arc<dyn rbl_library::Host + Send + Sync>,
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("library");
        std::fs::create_dir_all(&root).unwrap();
        let location = fixture::build(
            &root,
            Shape {
                tracks,
                ..Shape::default()
            },
        )
        .unwrap();
        prepare(&root);
        let config = Config {
            runtime_dir: dir.path().join("run"),
            idle_exit: None,
            recovery_dir: dir.path().join("recovery"),
        };
        let core = Arc::new(LibraryCore::new(config.recovery_dir.clone()));
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
        let host = host(&core);
        let service = Service::host(&location, &config, &core, &host).unwrap();
        Self { core, service, dir }
    }
    fn registration(&self, n: usize) {
        let operation = Operation::new(&self.core, Kind::Analysis);
        for i in 0..n {
            operation
                .run(
                    json!({"track":fixture::track_id(i)}),
                    || {
                        self.core.write_then(
                            &NoHost,
                            History::Outside,
                            |w| {
                                w.set_analysis(
                                    &fixture::track_id(i),
                                    &rbl_db::write::AnalysisWrite {
                                        bpm_x100: 12345,
                                        key: None,
                                        analysis_path: "/fixture/ANLZ0000.DAT",
                                        length_sec: Some(60),
                                    },
                                )
                            },
                            |_, v| Ok(v),
                        )
                    },
                    |_| vec![json!({"track":fixture::track_id(i),"after":{"bpm_x100":12345}})],
                )
                .unwrap();
        }
        assert_eq!(operation.finish(), None);
    }
    fn paths(&self, n: usize) -> Vec<PathBuf> {
        (0..n)
            .map(|i| {
                let path = self.dir.path().join(format!("import-{i}.wav"));
                let mut bytes = Vec::new();
                bytes.extend(b"RIFF");
                bytes.extend(38_u32.to_le_bytes());
                bytes.extend(b"WAVEfmt ");
                bytes.extend(16_u32.to_le_bytes());
                bytes.extend(1_u16.to_le_bytes());
                bytes.extend(1_u16.to_le_bytes());
                bytes.extend(44100_u32.to_le_bytes());
                bytes.extend(88200_u32.to_le_bytes());
                bytes.extend(2_u16.to_le_bytes());
                bytes.extend(16_u16.to_le_bytes());
                bytes.extend(b"data");
                bytes.extend(2_u32.to_le_bytes());
                bytes.extend(0_i16.to_le_bytes());
                std::fs::write(&path, bytes).unwrap();
                path
            })
            .collect()
    }
    fn import(&self, paths: &[PathBuf]) {
        let operation = Operation::new(&self.core, Kind::Import);
        for slice in paths.chunks(128) {
            operation
                .run_many(
                    json!({"files":slice.len()}),
                    slice.len(),
                    || {
                        self.core.write_then(
                            &NoHost,
                            History::Outside,
                            |w| w.import_files(slice),
                            |_, v| Ok(v),
                        )
                    },
                    |outcomes| {
                        outcomes
                            .iter()
                            .filter_map(|outcome| match outcome {
                                rbl_db::write::ImportOutcome::Added(id) => {
                                    Some(json!({"track":id}))
                                }
                                _ => None,
                            })
                            .collect()
                    },
                )
                .unwrap();
        }
        assert_eq!(operation.finish(), None);
    }
    fn runs(&self) -> Value {
        self.service.call("runs.list", json!({})).unwrap()
    }
}
#[test]
fn registration_and_import_have_distinct_operation_runs() {
    let f = Fixture::new(5);
    f.registration(3);
    f.import(&f.paths(3));
    let runs = f.runs();
    assert_eq!(runs["total"], 2, "{runs}");
    for run in runs["runs"].as_array().unwrap() {
        let shown = f
            .service
            .call("runs.show", json!({"id":run["id"],"limit":1000}))
            .unwrap();
        let entries = shown["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 3);
        let entry = &entries[0];
        assert_eq!(entry["payload"]["records"].as_array().unwrap().len(), 1);
        assert_eq!(entry["reversible"], false);
        assert!(entry["payload"]["not_reversible_reason"]
            .as_str()
            .unwrap()
            .contains("not retained"));
        assert_eq!(
            entry["source"]["kind"],
            if entry["kind"] == "analysis" {
                "tool"
            } else {
                "dj"
            }
        );
    }
    let db = f.core.open_read_only().unwrap();
    let bpm: u32 = db
        .connection()
        .query_row(
            "SELECT BPM FROM djmdContent WHERE ID=?1",
            [fixture::track_id(0)],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(bpm, 12345);
}
#[test]
#[ignore = "explicit fixture-only 2,000-track registration and import timing"]
fn bulk_cost() {
    let mut analysis = Vec::new();
    let mut imports = Vec::new();
    for _ in 0..3 {
        let f = Fixture::new(20_000);
        let paths = f.paths(2_000);
        let start = Instant::now();
        f.registration(2_000);
        analysis.push(start.elapsed().as_secs_f64() * 1000.0);
        let start = Instant::now();
        f.import(&paths);
        imports.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    analysis.sort_by(f64::total_cmp);
    imports.sort_by(f64::total_cmp);
    eprintln!("WBX1411 registration median_ms={} samples={analysis:?}; import median_ms={} samples={imports:?}",analysis[1],imports[1]);
}

#[test]
fn recording_is_bounded_and_duplicate_imports_add_no_run() {
    let f = Fixture::new(260);
    f.registration(257);
    let paths = f.paths(257);
    f.import(&paths);
    let runs = f.runs();
    assert_eq!(runs["total"], 2);
    for run in runs["runs"].as_array().unwrap() {
        let shown = f
            .service
            .call("runs.show", json!({"id":run["id"],"limit":1000}))
            .unwrap();
        let entries = shown["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 257);
        assert!(entries
            .iter()
            .all(|e| e["payload"]["records"].as_array().unwrap().len() == 1));
    }
    f.import(&paths);
    assert_eq!(f.runs()["total"], 2);
    let sidecar = f.dir.path().join("library/wreckordbox-sidecar/sidecar.db");
    let db =
        rusqlite::Connection::open_with_flags(sidecar, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM ledger_intent", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    // Two sets of three 128-id reservations plus three operation intents.
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM ledger_order", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        771
    );
}

#[test]
fn library_protection_refuses_both_bulk_writes_without_a_run() {
    let f = Fixture::new(3);
    f.core.set_protection(true).unwrap();
    for kind in [Kind::Analysis, Kind::Import] {
        let operation = Operation::new(&f.core, kind);
        let paths = f.paths(1);
        let refused = operation.run(
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
        assert!(refused.is_err());
        assert_eq!(operation.finish(), None);
    }
    assert_eq!(f.runs()["total"], 0);
    assert!(
        !f.dir
            .path()
            .join("library/wreckordbox-sidecar/sidecar.db")
            .exists(),
        "refusal must not reserve an intent or sequence id"
    );
    let db = f.core.open_read_only().unwrap();
    assert_ne!(
        db.connection()
            .query_row(
                "SELECT BPM FROM djmdContent WHERE ID=?1",
                [fixture::track_id(0)],
                |row| row.get::<_, u32>(0)
            )
            .unwrap(),
        12345
    );
    assert_eq!(
        db.connection()
            .query_row("SELECT COUNT(*) FROM djmdContent", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        3
    );
}

#[test]
fn querying_an_active_operation_does_not_recover_its_intent() {
    let f = Fixture::new(2);
    let operation = Operation::new(&f.core, Kind::Analysis);
    operation
        .run(
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
                    |_, v| Ok(v),
                )
            },
            |_| vec![json!({"track":fixture::track_id(0)})],
        )
        .unwrap();
    assert_eq!(
        f.runs()["total"],
        0,
        "a query must not convert the active intent into a recovered Run"
    );
    assert_eq!(operation.finish(), None);
    assert_eq!(f.runs()["total"], 1);
}

#[test]
fn finishing_during_a_write_waits_for_its_result() {
    let f = Fixture::new(2);
    let operation = Operation::new(&f.core, Kind::Analysis);
    operation
        .run(
            json!({"track":fixture::track_id(0)}),
            || {
                let result = f.core.write_then(
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
                );
                assert!(operation.finish().unwrap().contains("still finishing"));
                result
            },
            |_| vec![json!({"track":fixture::track_id(0)})],
        )
        .unwrap();
    assert_eq!(f.runs()["total"], 1);
    let runs = f.runs();
    let shown = f
        .service
        .call("runs.show", json!({"id":runs["runs"][0]["id"]}))
        .unwrap();
    assert_eq!(shown["entries"][0]["kind"], "analysis");
}

#[test]
fn unavailable_storage_keeps_the_accepted_write_then_requires_catch_up() {
    let f = Fixture::new(3);
    let unavailable = f.dir.path().join("library/wreckordbox-sidecar/sidecar.db");
    std::fs::create_dir_all(&unavailable).unwrap();
    let write = |operation: &Operation, i| {
        operation.run(
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
            |_| vec![json!({"track":fixture::track_id(i)})],
        )
    };
    let bpm = |i| {
        f.core
            .open_read_only()
            .unwrap()
            .connection()
            .query_row(
                "SELECT BPM FROM djmdContent WHERE ID=?1",
                [fixture::track_id(i)],
                |row| row.get::<_, u32>(0),
            )
            .unwrap()
    };
    let first = Operation::new(&f.core, Kind::Analysis);
    write(&first, 0).unwrap();
    assert!(first.finish().unwrap().contains("accepted"));
    assert_eq!(bpm(0), 12345);
    let before = bpm(1);
    let refused = Operation::new(&f.core, Kind::Analysis);
    assert!(write(&refused, 1).is_err());
    assert_eq!(refused.finish(), None);
    assert_eq!(bpm(1), before, "catch-up refusal precedes the native write");
    assert!(
        f.service.call("runs.list", json!({})).is_err(),
        "an unavailable Sidecar is not an empty Ledger"
    );
    std::fs::remove_dir(&unavailable).unwrap();
    let next = Operation::new(&f.core, Kind::Analysis);
    write(&next, 1).unwrap();
    assert_eq!(next.finish(), None);
    let runs = f.runs();
    assert_eq!(runs["total"], 2);
    let mut tracks = std::collections::BTreeSet::new();
    for run in runs["runs"].as_array().unwrap() {
        let shown = f
            .service
            .call("runs.show", json!({"id":run["id"]}))
            .unwrap();
        assert_eq!(shown["pending_recording"]["total"], 0);
        for entry in shown["entries"].as_array().unwrap() {
            assert_ne!(entry["kind"], "unconfirmed");
            for result in entry["payload"]["records"].as_array().unwrap() {
                tracks.insert(result["track"].as_str().unwrap().to_owned());
            }
        }
    }
    assert_eq!(tracks, [fixture::track_id(0), fixture::track_id(1)].into());
}

#[path = "support/bulk_review.rs"]
mod review;

#[path = "support/bulk_orphans.rs"]
mod orphans;

#[path = "support/player_bulk.rs"]
mod player;

#[test]
fn an_import_keeps_its_callers_source_across_threads_without_dj_facts() {
    struct Capture {
        core: std::sync::Weak<LibraryCore>,
        operation: Arc<std::sync::Mutex<Option<Operation>>>,
    }
    impl rbl_library::Host for Capture {
        fn recover_interrupted_files(&self, _: &rbl_db::LibraryLocation) -> Result<(), String> {
            *self.operation.lock().unwrap() =
                Some(Operation::new(&self.core.upgrade().unwrap(), Kind::Import));
            Ok(())
        }
    }
    for source in [
        json!({"kind":"agent","name":"importer"}),
        json!({"kind":"dj","name":"dj"}),
    ] {
        let captured = Arc::new(std::sync::Mutex::new(None));
        let f = Fixture::with_host(
            3,
            |_| {},
            |core| {
                Arc::new(Capture {
                    core: Arc::downgrade(core),
                    operation: captured.clone(),
                })
            },
        );
        let config = Config {
            runtime_dir: f.dir.path().join("run"),
            idle_exit: None,
            recovery_dir: f.dir.path().join("recovery"),
        };
        let location = f.core.location().unwrap();
        let mut caller =
            wreckordbox_service::Client::connect(&config.socket_path(&location)).unwrap();
        caller.in_run(Some(
            serde_json::from_value(json!({"source":source})).unwrap(),
        ));
        caller
            .call::<Value>("tracks.rate", json!({"id":fixture::track_id(0),"rating":5}))
            .unwrap();
        let operation = captured.lock().unwrap().take().unwrap();
        let core = f.core.clone();
        std::thread::spawn(move || {
            operation
                .run(
                    json!({"track":fixture::track_id(1)}),
                    || {
                        core.recorded_edit(
                            &NoHost,
                            rbl_library::Touched::Tracks,
                            "Imported rating",
                            |w| {
                                let (_, edit) = w.set_rating_with_undo(&fixture::track_id(1), 4)?;
                                Ok(((), rbl_library::history::LibraryEdit::Track(vec![edit])))
                            },
                        )
                    },
                    |_| vec![json!({"track":fixture::track_id(1),"operation":"imported_metadata"})],
                )
                .unwrap();
            assert_eq!(operation.finish(), None);
        })
        .join()
        .unwrap();
        let runs = f.runs();
        assert_eq!(runs["total"], 2, "{runs}");
        let shown = f
            .service
            .call("runs.show", json!({"id":runs["runs"][0]["id"]}))
            .unwrap();
        let entries = shown["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2, "{shown}");
        assert!(
            entries.iter().all(|entry| entry["source"] == source),
            "{shown}"
        );
        assert_eq!(
            f.service
                .call(
                    "facts.of_track",
                    json!({"id":fixture::track_id(1),"all":true})
                )
                .unwrap()["facts"],
            json!([])
        );
    }
}

#[test]
fn generated_import_records_leave_the_writer_gate_and_name_all_three_hundred_rows() {
    let f = Fixture::new(300);
    let operation = Operation::new(&f.core, Kind::Import);
    operation
        .run_generated(
            json!({"operation":"fixture_import"}),
            300,
            || {
                f.core.write_then(
                    &NoHost,
                    History::Outside,
                    |w| {
                        for i in 0..300 {
                            w.set_rating(&fixture::track_id(i), 4)?;
                        }
                        Ok(())
                    },
                    |_, result| Ok(result),
                )
            },
            |(), emit| {
                let core = f.core.clone();
                assert!(
                    std::thread::spawn(move || core.edit_gate().try_lock().is_some())
                        .join()
                        .unwrap(),
                    "record generation runs after releasing the Library gate"
                );
                for i in 0..300 {
                    emit(json!({"track":fixture::track_id(i),"title":"metadata".repeat(128)}));
                }
            },
        )
        .unwrap();
    assert_eq!(operation.finish(), None);
    let runs = f.runs();
    assert_eq!(runs["total"], 1);
    let shown = f
        .service
        .call(
            "runs.show",
            json!({"id":runs["runs"][0]["id"],"limit":1000}),
        )
        .unwrap();
    assert_eq!(shown["total"], 300);
    let tracks: std::collections::BTreeSet<_> = shown["entries"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|e| e["payload"]["records"].as_array().unwrap())
        .map(|r| r["track"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(tracks, (0..300).map(fixture::track_id).collect());
    assert_eq!(
        f.service
            .call(
                "facts.of_track",
                json!({"id":fixture::track_id(1),"all":true})
            )
            .unwrap()["facts"],
        json!([])
    );
}
