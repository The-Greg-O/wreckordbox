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

/// A DJ import with one accepted write, its intent still open.
fn dj_import(f: &Fixture, track: usize) -> (Operation, i64) {
    let op = Operation::new(&f.core, Kind::Import);
    op.run(
        json!({"track":fixture::track_id(track)}),
        || {
            f.core.write_then(
                &NoHost,
                History::Outside,
                |w| {
                    w.set_analysis(
                        &fixture::track_id(track),
                        &rbl_db::write::AnalysisWrite {
                            bpm_x100: 12800,
                            key: None,
                            analysis_path: "/fixture/ANLZ0000.DAT",
                            length_sec: None,
                        },
                    )
                },
                |_, v| Ok(v),
            )
        },
        |_| vec![json!({"track":fixture::track_id(track)})],
    )
    .unwrap();
    let db =
        rusqlite::Connection::open(f.dir.path().join("library/wreckordbox-sidecar/sidecar.db"))
            .unwrap();
    let intent = db
        .query_row(
            "SELECT id FROM ledger_intent WHERE source_kind='dj'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    (op, intent)
}
fn batches(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter(|e| {
                    e.as_ref()
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with("batch-")
                })
                .count()
        })
        .unwrap_or(0)
}
fn copy_spill(from: &std::path::Path, to: &std::path::Path, manifest: &Value) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
    }
    std::fs::write(to.join("manifest.json"), manifest.to_string()).unwrap();
}
#[test]
fn an_agents_spill_cannot_be_replayed_as_the_djs_unconfirmed_intent() {
    if isolated("an_agents_spill_cannot_be_replayed_as_the_djs_unconfirmed_intent") {
        return;
    }
    let mut o = Orphan::new();
    let (dj, dj_intent) = dj_import(&o.f, 3);
    // The DJ's import crashes: its intent is never settled.
    std::mem::forget(dj);
    o.restart();
    let dj_run: i64 = o
        .db()
        .query_row(
            "SELECT run_id FROM ledger WHERE id=?1 AND kind='unconfirmed' AND source_kind='dj'",
            [dj_intent],
            |r| r.get(0),
        )
        .unwrap();
    let mut supplied = o.manifest.clone();
    supplied["intent"] = json!(dj_intent);
    supplied["run"] = json!(dj_run);
    supplied["source"] = json!({"kind":"dj","name":"dj"});
    let result = o.f.service.call(
        "runs.spills.replay",
        json!({"id":o.id(),"manifest":supplied}),
    );
    assert!(result.is_err(), "the DJ's intent was borrowed: {result:?}");
    assert_eq!(batches(&o.spill), 1, "the agent's spill must be kept");
    let as_dj: i64 = o
        .db()
        .query_row(
            "SELECT COUNT(*) FROM ledger WHERE source_kind='dj' AND kind<>'unconfirmed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(as_dj, 0, "nothing may be recorded as the DJ's");
    o.f.service.stop();
}
#[test]
fn replaying_against_a_live_dj_intent_neither_records_nor_settles_it() {
    if isolated("replaying_against_a_live_dj_intent_neither_records_nor_settles_it") {
        return;
    }
    let o = Orphan::new();
    let (dj, dj_intent) = dj_import(&o.f, 3);
    let mut supplied = o.manifest.clone();
    supplied["intent"] = json!(dj_intent);
    supplied["run"] = Value::Null;
    supplied["source"] = json!({"kind":"dj","name":"dj"});
    let replay = || {
        o.f.service.call(
            "runs.spills.replay",
            json!({"id":o.id(),"manifest":supplied}),
        )
    };
    let bound = replay();
    assert!(
        bound.is_err(),
        "a spill bound to another intent replayed: {bound:?}"
    );
    // A spill from before the binding cannot borrow a live operation's intent either.
    std::fs::remove_file(o.spill.join("intent.json")).unwrap();
    let legacy = replay();
    assert!(legacy.is_err(), "a live intent was borrowed: {legacy:?}");
    let open: i64 = o
        .db()
        .query_row(
            "SELECT COUNT(*) FROM ledger_intent WHERE id=?1",
            [dj_intent],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(open, 1, "the live intent and its Barrier must remain");
    let dj_count = || -> i64 {
        o.db()
            .query_row(
                "SELECT COUNT(*) FROM ledger WHERE source_kind='dj'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(dj_count(), 0);
    assert_eq!(batches(&o.spill), 1);
    assert_eq!(dj.finish(), None);
    assert_eq!(dj_count(), 1, "the DJ's own write is recorded once");
    o.f.service.stop();
}
#[test]
fn a_spill_moved_in_from_another_library_is_refused_kept_and_reported() {
    if isolated("a_spill_moved_in_from_another_library_is_refused_kept_and_reported") {
        return;
    }
    let mut a = Orphan::new();
    let b = Orphan::new();
    let moved = a.spill.with_file_name(b.id());
    copy_spill(&b.spill, &moved, &b.manifest);
    b.f.service.stop();
    a.restart();
    assert!(moved.join("manifest.json").is_file());
    assert_eq!(batches(&moved), 1, "the moved spill must be kept");
    let imports = || -> i64 {
        a.db()
            .query_row("SELECT COUNT(*) FROM ledger WHERE kind='import'", [], |r| {
                r.get(0)
            })
            .unwrap()
    };
    assert_eq!(imports(), 0, "another Library's records were replayed");
    let runs = a.f.service.call("runs.list", json!({})).unwrap();
    let listed = runs["recovery"]["spills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == b.id())
        .cloned()
        .unwrap();
    assert!(
        listed["error"]
            .as_str()
            .is_some_and(|e| e.contains("is not the intent it was accepted under")),
        "the recovery report must name the refusal: {listed}"
    );
    let explicit = a.f.service.call("runs.spills.replay", json!({"id":b.id()}));
    assert!(explicit.is_err(), "{explicit:?}");
    assert_eq!(batches(&moved), 1);
    assert_eq!(imports(), 0);
    a.f.service.stop();
}
#[test]
fn an_intent_already_replayed_refuses_a_second_spill() {
    if isolated("an_intent_already_replayed_refuses_a_second_spill") {
        return;
    }
    let mut o = Orphan::new();
    o.restart();
    let intent = o.manifest["intent"].as_i64().unwrap();
    let run: i64 = o
        .db()
        .query_row(
            "SELECT run_id FROM ledger WHERE id=?1 AND kind='unconfirmed'",
            [intent],
            |r| r.get(0),
        )
        .unwrap();
    let mut original = o.manifest.clone();
    original["run"] = json!(run);
    let copy = o.spill.with_file_name(format!("{}-copy", o.id()));
    copy_spill(&o.spill, &copy, &original);
    std::fs::remove_file(copy.join("manifest.json")).unwrap();
    o.f.service
        .call(
            "runs.spills.replay",
            json!({"id":o.id(),"manifest":original}),
        )
        .unwrap();
    let again = o.f.service.call(
        "runs.spills.replay",
        json!({"id":format!("{}-copy", o.id()),"manifest":original}),
    );
    assert!(again.is_err(), "an intent was replayed twice: {again:?}");
    assert_eq!(batches(&copy), 1);
    o.f.service.stop();
}
#[test]
fn an_old_discard_receipt_naming_an_existing_run_is_refused() {
    if isolated("an_old_discard_receipt_naming_an_existing_run_is_refused") {
        return;
    }
    let o = Orphan::new();
    let run: i64 = o
        .db()
        .query_row(
            "SELECT id FROM run WHERE source_kind='agent' ORDER BY id LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let state = || -> (i64, Option<i64>) {
        o.db()
            .query_row(
                "SELECT (SELECT COUNT(*) FROM ledger WHERE run_id=?1), finished_at FROM run WHERE id=?1",
                [run],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    };
    let before = state();
    std::fs::write(
        o.spill.join("discard.json"),
        json!({"source":{"kind":"dj","name":"dj"},"id":999_999,"run":run,"at":1,
               "payload":{"spill":o.id(),"reason":"An older receipt"}})
        .to_string(),
    )
    .unwrap();
    assert!(
        o.discard().is_err(),
        "an old receipt finished an existing Run"
    );
    assert_eq!(state(), before, "the existing Run must be left as it was");
    assert!(o.spill.join("discard.json").is_file());
    assert_eq!(batches(&o.spill), 1);
    o.f.service.stop();
}
#[test]
fn a_spill_from_before_binding_replays_only_against_its_single_candidate() {
    if isolated("a_spill_from_before_binding_replays_only_against_its_single_candidate") {
        return;
    }
    let mut o = Orphan::new();
    o.restart();
    std::fs::remove_file(o.spill.join("intent.json")).unwrap();
    let intent = o.manifest["intent"].as_i64().unwrap();
    let run: i64 = o
        .db()
        .query_row(
            "SELECT run_id FROM ledger WHERE id=?1 AND kind='unconfirmed'",
            [intent],
            |r| r.get(0),
        )
        .unwrap();
    let mut original = o.manifest.clone();
    original["run"] = json!(run);
    let mut other = original.clone();
    other["intent"] = json!(intent + 1000);
    assert!(o
        .f
        .service
        .call("runs.spills.replay", json!({"id":o.id(),"manifest":other}))
        .is_err());
    o.f.service
        .call(
            "runs.spills.replay",
            json!({"id":o.id(),"manifest":original}),
        )
        .unwrap();
    assert!(!o.spill.exists());
    o.f.service.stop();
}
/// Every file under `dir` with its bytes.
fn tree(dir: &std::path::Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let bytes = std::fs::read(&path).unwrap();
                found.push((path, bytes));
            }
        }
    }
    found.sort();
    found
}
#[test]
fn spill_recovery_writes_nothing_while_the_librarys_stick_is_locked() {
    if isolated("spill_recovery_writes_nothing_while_the_librarys_stick_is_locked") {
        return;
    }
    let mut o = Orphan::new();
    o.restart();
    let intent = o.manifest["intent"].as_i64().unwrap();
    let run: i64 = o
        .db()
        .query_row(
            "SELECT run_id FROM ledger WHERE id=?1 AND kind='unconfirmed'",
            [intent],
            |r| r.get(0),
        )
        .unwrap();
    let mut original = o.manifest.clone();
    original["run"] = json!(run);
    // A damaged manifest: replay would archive it under ledger-bulk-evidence.
    std::fs::write(o.spill.join("manifest.json"), b"{damaged").unwrap();
    // The Library's folder is a Stick, never unlocked.
    let marker =
        o.f.dir
            .path()
            .join("library")
            .join(wreckordbox_stick_protection::STAND_IN_MARKER);
    std::fs::write(&marker, b"").unwrap();
    let recovery = o.f.dir.path().join("recovery/ledger");
    let before = tree(&recovery);
    let entries = || -> i64 {
        o.db()
            .query_row("SELECT COUNT(*) FROM ledger", [], |r| r.get(0))
            .unwrap()
    };
    let recorded = entries();
    let replay = o.f.service.call(
        "runs.spills.replay",
        json!({"id":o.id(),"manifest":original}),
    );
    assert!(replay.is_err(), "{replay:?}");
    let discard = o.discard();
    assert!(discard.is_err(), "{discard:?}");
    assert_eq!(
        tree(&recovery),
        before,
        "nothing may be written on a locked Stick"
    );
    assert_eq!(entries(), recorded);
    // Inspection still answers.
    o.f.service
        .call("runs.spills.show", json!({"id":o.id()}))
        .unwrap();
    // Unlocked by name: the DJ can then deal with the spill on the Stick.
    wreckordbox_stick_protection::unlock(
        &o.f.dir.path().join("recovery"),
        &o.f.dir.path().join("library"),
        Some("library"),
    )
    .unwrap();
    o.f.service
        .call(
            "runs.spills.replay",
            json!({"id":o.id(),"manifest":original}),
        )
        .unwrap();
    assert!(!o.spill.exists());
    o.f.service.stop();
}
#[test]
fn a_bulk_spill_is_not_written_once_the_librarys_stick_locks() {
    if isolated("a_bulk_spill_is_not_written_once_the_librarys_stick_locks") {
        return;
    }
    let f = Fixture::new(2);
    let marker = f
        .dir
        .path()
        .join("library")
        .join(wreckordbox_stick_protection::STAND_IN_MARKER);
    let op = Operation::new(&f.core, Kind::Analysis);
    op.run(
        json!({"track":fixture::track_id(0)}),
        || {
            let written = f.core.write_then(
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
            // The Stick is locked after the native write, before its record.
            std::fs::write(&marker, b"").unwrap();
            written
        },
        |_| vec![json!({"track":fixture::track_id(0)})],
    )
    .unwrap();
    assert!(
        op.finish().is_some(),
        "the unrecorded write must be reported"
    );
    let recovery = f.dir.path().join("recovery/ledger");
    let written: Vec<_> = if recovery.exists() {
        tree(&recovery)
            .into_iter()
            .map(|(path, _)| path)
            .filter(|p| {
                p.components()
                    .any(|c| c.as_os_str().to_string_lossy().starts_with("ledger-bulk"))
            })
            .collect()
    } else {
        Vec::new()
    };
    assert!(
        written.is_empty(),
        "spill files written on a locked Stick: {written:?}"
    );
    std::fs::remove_file(&marker).unwrap();
    f.service.stop();
}
