use super::*;

fn isolated(name: &str) -> bool {
    if std::env::var_os("WBX146_SPILL_REVIEW").is_some() {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &format!("spill_review::{name}"), "--nocapture"])
        .env("WBX146_SPILL_REVIEW", "1")
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
    true
}
struct Orphan {
    f: Fixture,
    spill: PathBuf,
    manifest: Value,
}
impl Orphan {
    fn new() -> Self {
        Self::sized(3, Kind::Import)
    }
    fn sized(tracks: usize, kind: Kind) -> Self {
        struct Capture {
            core: std::sync::Weak<LibraryCore>,
            op: Arc<std::sync::Mutex<Option<Operation>>>,
            kind: Kind,
        }
        impl rbl_library::Host for Capture {
            fn recover_interrupted_files(&self, _: &rbl_db::LibraryLocation) -> Result<(), String> {
                *self.op.lock().unwrap() =
                    Some(Operation::new(&self.core.upgrade().unwrap(), self.kind));
                Ok(())
            }
        }
        let op = Arc::new(std::sync::Mutex::new(None));
        let f = Fixture::with_host(
            tracks + 1,
            |_| {},
            |core| {
                Arc::new(Capture {
                    core: Arc::downgrade(core),
                    op: op.clone(),
                    kind,
                })
            },
        );
        let config = Config {
            runtime_dir: f.dir.path().join("run"),
            recovery_dir: f.dir.path().join("recovery"),
            idle_exit: None,
        };
        let mut client =
            wreckordbox_service::Client::connect(&config.socket_path(&f.core.location().unwrap()))
                .unwrap();
        client.in_run(Some(
            serde_json::from_value(json!({"source":{"kind":"agent","name":"importer"}})).unwrap(),
        ));
        std::env::set_var("WRECKORDBOX_LEDGER_FAULTS", "fail_append=999");
        client
            .call::<Value>(
                "tracks.rate",
                json!({"id":fixture::track_id(tracks),"rating":5}),
            )
            .unwrap();
        std::env::remove_var("WRECKORDBOX_LEDGER_FAULTS");
        let op = op.lock().unwrap().take().unwrap();
        op.run_many(json!({"tracks":(0..tracks).map(fixture::track_id).collect::<Vec<_>>()}),tracks, || {
            f.core.write_then(&NoHost, History::Outside, |w| {
                for i in 0..tracks { w.set_analysis(&fixture::track_id(i), &rbl_db::write::AnalysisWrite { bpm_x100:12345,key:None,analysis_path:"/fixture/ANLZ0000.DAT",length_sec:None })?; }
                Ok(())
            }, |_,v|Ok(v))
        }, |_| (0..tracks).map(|i|json!({"track":fixture::track_id(i),"target":{"track":fixture::track_id(i),"field":"bpm"}})).collect()).unwrap();
        assert!(op.finish().unwrap().contains("accepted"));
        drop(op);
        let root = std::fs::read_dir(f.dir.path().join("recovery/ledger"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let spill = std::fs::read_dir(root.join("ledger-bulk"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let manifest =
            serde_json::from_slice(&std::fs::read(spill.join("manifest.json")).unwrap()).unwrap();
        std::fs::remove_file(spill.join("manifest.json")).unwrap();
        Self { f, spill, manifest }
    }
    fn id(&self) -> String {
        self.spill.file_name().unwrap().to_str().unwrap().into()
    }
    fn restart(&mut self) {
        self.f.service.stop();
        self.f.service.stopped().wait();
        self.f.service = Service::start(
            self.f.core.location().unwrap(),
            &Config {
                runtime_dir: self.f.dir.path().join("run"),
                recovery_dir: self.f.dir.path().join("recovery"),
                idle_exit: None,
            },
        )
        .unwrap();
    }
    fn discard(&self) -> Result<Value, wreckordbox_service::protocol::ServiceError> {
        self.f.service.call("runs.spills.discard",json!({"id":self.id(),"source":{"kind":"dj"},"reason":"Fixture DJ explicitly discards these records"}))
    }
    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(
            self.f
                .dir
                .path()
                .join("library/wreckordbox-sidecar/sidecar.db"),
        )
        .unwrap()
    }
}
#[test]
fn replay_refuses_changed_source_run_or_kind_after_restart() {
    if isolated("replay_refuses_changed_source_run_or_kind_after_restart") {
        return;
    }
    let mut o = Orphan::new();
    o.restart();
    let intent = o.manifest["intent"].as_i64().unwrap();
    let recorded_run: i64 = o
        .db()
        .query_row(
            "SELECT run_id FROM ledger WHERE id=?1 AND kind='unconfirmed'",
            [intent],
            |r| r.get(0),
        )
        .unwrap();
    let mut original = o.manifest.clone();
    original["run"] = json!(recorded_run);
    for (field, value) in [
        ("source", json!({"kind":"dj","name":"dj"})),
        ("run", Value::Null),
        ("kind", json!("link")),
        ("intent", Value::Null),
    ] {
        let mut supplied = original.clone();
        supplied[field] = value;
        if field == "source" {
            supplied["run"] = Value::Null;
        }
        let result = o.f.service.call(
            "runs.spills.replay",
            json!({"id":o.id(),"manifest":supplied}),
        );
        assert!(result.is_err(), "changed {field} was accepted: {result:?}");
        assert!(!o.spill.join("manifest.json").exists());
    }
    o.f.service
        .call(
            "runs.spills.replay",
            json!({"id":o.id(),"manifest":original}),
        )
        .unwrap();
    let count:i64=o.db().query_row("SELECT COUNT(*) FROM ledger WHERE kind='import' AND source_kind='agent' AND source_name='importer' AND run_id=?1",[recorded_run],|r|r.get(0)).unwrap();
    assert_eq!(count, 3);
    o.f.service.stop();
}
#[test]
fn discard_names_every_target_and_leaves_the_barrier_after_restart() {
    if isolated("discard_names_every_target_and_leaves_the_barrier_after_restart") {
        return;
    }
    let mut o = Orphan::new();
    let before = o.f.service.call("projection.barriers", json!({})).unwrap();
    assert!(!before["barriers"].as_array().unwrap().is_empty());
    let result = o.discard().unwrap();
    let shown =
        o.f.service
            .call("runs.show", json!({"id":result["run"],"limit":1000}))
            .unwrap();
    let text = shown["entries"].to_string();
    for i in 0..3 {
        assert!(
            text.contains(&fixture::track_id(i)),
            "discard must name track {}: {shown}",
            fixture::track_id(i)
        );
    }
    o.restart();
    let barriers = o.f.service.call("projection.barriers", json!({})).unwrap();
    assert!(
        !barriers["barriers"].as_array().unwrap().is_empty(),
        "discard must not clear the Barrier"
    );
    o.f.service
        .call("protection.set", json!({"protect_library":false}))
        .unwrap();
    std::fs::write(
        o.f.dir
            .path()
            .join("library/wreckordbox-sidecar/projection-policy.toml"),
        "[energy]\napply = 0.5\n",
    )
    .unwrap();
    for i in 0..3 {
        o.f.service.call("facts.record",json!({"source":{"kind":"agent","name":"tagger"},"facts":[{"track":fixture::track_id(i),"kind":"energy","value":5,"confidence":0.99}]})).unwrap();
    }
    let plan = o.f.service.call("projection.plan", json!({})).unwrap();
    for i in 0..3 {
        assert!(
            plan["kept"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["track"] == fixture::track_id(i)
                    && v["field"] == "energy"
                    && v["reason"] == "provenance_incomplete"),
            "{plan}"
        );
    }
    let before = o.f.service.call("tracks.list", json!({})).unwrap();
    o.f.service.call("projection.apply", json!({})).unwrap();
    assert_eq!(o.f.service.call("tracks.list", json!({})).unwrap(), before);
    o.f.service.stop();
}
#[test]
fn rejected_discard_receipt_creates_no_empty_dj_run() {
    if isolated("rejected_discard_receipt_creates_no_empty_dj_run") {
        return;
    }
    let o = Orphan::new();
    let db = o.db();
    let count = || {
        db.query_row("SELECT COUNT(*) FROM run", [], |r| r.get::<_, i64>(0))
            .unwrap()
    };
    let before = count();
    db.execute_batch("CREATE TRIGGER refuse_discard BEFORE INSERT ON ledger WHEN NEW.kind='ledger_spill_discard' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(o.discard().is_err());
    assert!(o.spill.join("discard.json").is_file());
    assert_eq!(
        count(),
        before,
        "no empty DJ Run may precede its discard receipt"
    );
    db.execute_batch("DROP TRIGGER refuse_discard").unwrap();
    o.discard().unwrap();
    assert_eq!(count(), before + 1);
    o.f.service.stop();
}
#[test]
fn replay_preserves_the_corrupt_manifest_bytes() {
    if isolated("replay_preserves_the_corrupt_manifest_bytes") {
        return;
    }
    let o = Orphan::new();
    let corrupt = b"{original truncated manifest";
    std::fs::write(o.spill.join("manifest.json"), corrupt).unwrap();
    // Refuse the entry so inspection can still see all recovery artifacts.
    o.db().execute_batch("CREATE TRIGGER refuse_import BEFORE INSERT ON ledger WHEN NEW.kind='import' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(o
        .f
        .service
        .call(
            "runs.spills.replay",
            json!({"id":o.id(),"manifest":o.manifest})
        )
        .is_err());
    let archived = o
        .spill
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("ledger-bulk-evidence")
        .join(o.id())
        .join("manifest.json");
    assert_eq!(
        std::fs::read(archived).unwrap_or_default(),
        corrupt,
        "recovery overwrote the corrupt bytes"
    );
    o.db().execute_batch("DROP TRIGGER refuse_import").unwrap();
    o.f.service
        .call("runs.spills.replay", json!({"id":o.id()}))
        .unwrap();
    assert!(!o.spill.exists());
    let archived = o
        .spill
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("ledger-bulk-evidence")
        .join(o.id())
        .join("manifest.json");
    assert_eq!(std::fs::read(archived).unwrap(), corrupt);
    o.f.service.stop();
}

#[test]
fn a_failed_session_lease_is_reported_before_native_writes() {
    if isolated("a_failed_session_lease_is_reported_before_native_writes") {
        return;
    }
    let f = Fixture::new(1);
    std::env::set_var("WRECKORDBOX_LEDGER_FAULTS", "fail_lease");
    let operation = Operation::new(&f.core, Kind::Analysis);
    std::env::remove_var("WRECKORDBOX_LEDGER_FAULTS");
    let called = std::cell::Cell::new(false);
    let result = operation.run(
        json!({"track":fixture::track_id(0)}),
        || {
            f.core.write_then(
                &NoHost,
                History::Outside,
                |w| {
                    called.set(true);
                    w.set_rating(&fixture::track_id(0), 5)
                },
                |_, v| Ok(v),
            )
        },
        |_| vec![json!({"track":fixture::track_id(0)})],
    );
    assert!(
        result.is_err(),
        "a lost lease must not silently accept a write: {result:?}"
    );
    assert!(!called.get(), "refused before entering the native writer");
    assert!(result.unwrap_err().to_string().contains("lease"));
    assert!(operation.finish().unwrap().contains("lease"));
    assert_eq!(f.runs()["total"], 0);
    f.service.stop();
}

#[test]
fn supplied_manifest_must_match_the_open_intents_kind_and_run() {
    if isolated("supplied_manifest_must_match_the_open_intents_kind_and_run") {
        return;
    }
    let o = Orphan::sized(3, Kind::Analysis);
    for (field, value) in [("kind", json!("import")), ("run", Value::Null)] {
        let mut supplied = o.manifest.clone();
        supplied[field] = value;
        assert!(
            o.f.service
                .call(
                    "runs.spills.replay",
                    json!({"id":o.id(),"manifest":supplied})
                )
                .is_err(),
            "changed {field} must be refused"
        );
        assert!(!o.spill.join("manifest.json").exists());
    }
    o.f.service
        .call(
            "runs.spills.replay",
            json!({"id":o.id(),"manifest":o.manifest}),
        )
        .unwrap();
    o.f.service.stop();
}

#[test]
fn bounded_discard_retries_one_run_and_names_all_129_accepted_tracks() {
    if isolated("bounded_discard_retries_one_run_and_names_all_129_accepted_tracks") {
        return;
    }
    let o = Orphan::sized(129, Kind::Import);
    let db = o.db();
    let count = || {
        db.query_row("SELECT COUNT(*) FROM run", [], |r| r.get::<_, i64>(0))
            .unwrap()
    };
    let before = count();
    db.execute_batch("CREATE TRIGGER refuse_second_discard BEFORE INSERT ON ledger WHEN NEW.kind='ledger_spill_discard' AND NEW.part_of IS NOT NULL BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(o.discard().is_err());
    assert_eq!(count(), before + 1);
    assert!(
        o.spill.join("batch-00000000000000000001.json").exists(),
        "no deletion before the complete receipt is recorded"
    );
    db.execute_batch("DROP TRIGGER refuse_second_discard")
        .unwrap();
    let result = o.discard().unwrap();
    assert_eq!(count(), before + 1);
    let shown =
        o.f.service
            .call("runs.show", json!({"id":result["run"]}))
            .unwrap();
    assert_eq!(shown["total"], 2);
    let entries = shown["entries"].as_array().unwrap();
    assert_eq!(entries[1]["part_of"], entries[0]["id"]);
    let tracks: std::collections::BTreeSet<_> = entries
        .iter()
        .flat_map(|e| {
            let records = e["payload"]["discarded"]["pending"]["records"]
                .as_array()
                .unwrap();
            assert!(records.len() <= 128);
            records
                .iter()
                .map(|r| r["result"]["track"].as_str().unwrap().to_owned())
        })
        .collect();
    assert_eq!(tracks, (0..129).map(fixture::track_id).collect());
    assert!(!o.spill.exists());
    o.f.service.stop();
}

#[test]
fn unreadable_accepted_results_are_kept_when_their_targets_cannot_be_recorded() {
    if isolated("unreadable_accepted_results_are_kept_when_their_targets_cannot_be_recorded") {
        return;
    }
    let o = Orphan::new();
    let batch = o.spill.join("batch-00000000000000000001.json");
    let bytes = b"[{truncated accepted result";
    std::fs::write(&batch, bytes).unwrap();
    let result = o.discard();
    assert!(
        result.is_err(),
        "cannot name the discarded targets: {result:?}"
    );
    assert_eq!(std::fs::read(&batch).unwrap(), bytes);
    assert!(!o.spill.join("discard.json").exists());
    o.f.service.stop();
}
