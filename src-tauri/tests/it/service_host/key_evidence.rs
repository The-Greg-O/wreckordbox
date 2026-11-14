//! Real key publication over frozen version-1 evidence, through the hosting shell.
use super::*;
use serde_json::{json, Value};
#[path = "../../../../crates/wreckordbox-sidecar/tests/fingerprint_support/mod.rs"]
mod fixture;
#[path = "../../../../crates/wreckordbox-sidecar/tests/fingerprint_support/store.rs"]
mod store;

fn setup(format: &str) -> (Window, PathBuf, String, String, PathBuf) {
    let window = window();
    let audio = fixture::write(window.dir.path(), format);
    let (id, playlist) = window
        .state()
        .write(|w| {
            let id = w.import_file(&audio)?;
            let playlist = w.create_playlist("Evidence fixture", "root")?;
            w.add_tracks(&playlist, std::slice::from_ref(&id))?;
            w.ensure_detected_key("Fm")?;
            Ok((id, playlist))
        })
        .unwrap();
    let sidecar = window
        .location
        .master_db
        .parent()
        .unwrap()
        .join("wreckordbox-sidecar");
    store::seed(&sidecar, &[(format, id.clone())]);
    window.load();
    window.host().unwrap();
    let stick = window.dir.path().join("SPARE");
    std::fs::create_dir(&stick).unwrap();
    std::fs::write(
        stick.join(wreckordbox_stick_protection::STAND_IN_MARKER),
        b"",
    )
    .unwrap();
    std::fs::write(stick.join(wreckordbox_stick_protection::SPARE_MARKER), b"").unwrap();
    (window, audio, id, playlist, stick)
}
fn edit(window: &Window, id: &str) {
    run(rbxport_lib::details::set_track_field(
        window.app.handle().clone(),
        window.state(),
        vec![id.into()],
        "key".into(),
        "Fm".into(),
        Some(true),
    ))
    .unwrap();
}
fn shown(window: &Window, id: &str) -> Value {
    window.cli(&["--json", "stems", "show", id]).json()
}
fn preview(window: &Window, playlist: &str, stick: &std::path::Path) -> Value {
    window
        .cli(&[
            "--json",
            "stick",
            "dry-run",
            stick.to_str().unwrap(),
            playlist,
        ])
        .json()
}
fn current(format: &str) {
    let (window, audio, id, _, _) = setup(format);
    let original = shown(&window, &id);
    assert!(!original["stems"].is_null());
    assert_eq!(
        original["stems"]["fingerprint"],
        store::golden()[format]["a1"]
    );
    edit(&window, &id);
    assert_eq!(
        rbl_db::key_tag::read_key(&audio).unwrap().as_deref(),
        Some("Fm")
    );
    let after = shown(&window, &id);
    assert!(
        !after["stems"].is_null(),
        "{format}: the verified key writer must carry existing Stems: {after}"
    );
    assert!(after["stems"]["fingerprint"]
        .as_str()
        .unwrap()
        .starts_with("a1:"));
    assert!(!window.cli(&["--json", "intensity", "show", &id]).json()["intensity"].is_null());
    if format != "aiff" {
        assert_eq!(after["stems"], original["stems"]);
        assert!(carries(&window).is_empty());
        // Valid changed samples, not corruption that merely forces f1 fallback.
        static ENCODING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _encoder = ENCODING.lock().unwrap();
        let changed = window
            .dir
            .path()
            .join(format!("changed.{}", fixture::extension(format)));
        let codec = if format == "flac" {
            "flac"
        } else {
            "libmp3lame"
        };
        let output = Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-v",
                "error",
                "-threads",
                "1",
                "-filter_threads",
                "1",
                "-i",
            ])
            .arg(&audio)
            .args(["-af", "volume=0.5", "-c:a", codec, "-threads", "1"])
            .arg(&changed)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        std::fs::rename(changed, &audio).unwrap();
        assert!(wreckordbox_sidecar::fingerprint_file(&audio)
            .unwrap()
            .as_str()
            .starts_with("a2:"));
        assert!(shown(&window, &id)["stems"].is_null());
        assert!(window.cli(&["--json", "intensity", "show", &id]).json()["intensity"].is_null());
    }
}
#[test]
fn aiff_key_publication_keeps_legacy_stems_and_measurements_current() {
    current("aiff");
}
#[test]
fn mp3_key_publication_keeps_legacy_evidence_unchanged() {
    current("mp3-cbr");
}
#[test]
fn flac_key_publication_keeps_legacy_evidence_unchanged() {
    current("flac");
}
#[test]
fn aiff_key_publication_keeps_stems_in_the_stick_preview() {
    let (window, _, id, playlist, stick) = setup("aiff");
    assert_eq!(
        preview(&window, &playlist, &stick)["changes"]["native_stems"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    edit(&window, &id);
    let dry = preview(&window, &playlist, &stick);
    assert_eq!(
        dry["changes"]["native_stems"].as_array().unwrap().len(),
        1,
        "{dry}"
    );
}
#[test]
fn a_changed_aiff_sample_is_stale_even_after_a_key_edit() {
    let (window, audio, id, _, _) = setup("aiff");
    edit(&window, &id);
    let mut bytes = std::fs::read(&audio).unwrap();
    let ssnd = bytes.windows(4).position(|b| b == b"SSND").unwrap();
    bytes[ssnd + 16] ^= 1;
    std::fs::write(&audio, bytes).unwrap();
    assert!(shown(&window, &id)["stems"].is_null());
    assert!(window.cli(&["--json", "intensity", "show", &id]).json()["intensity"].is_null());
    assert_eq!(
        window
            .client()
            .call::<Value>("stems.status", json!({"id":id}))
            .unwrap()["state"],
        "stale"
    );
}

fn carries(window: &Window) -> Vec<Value> {
    window.cli(&["--json", "runs", "list"]).json()["runs"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|r| {
            window
                .cli(&[
                    "--json",
                    "runs",
                    "show",
                    &r["id"].to_string(),
                    "--limit",
                    "1000",
                ])
                .json()["entries"]
                .as_array()
                .unwrap()
                .clone()
        })
        .filter(|e| e["kind"] == "key_evidence_carry")
        .collect()
}
#[test]
fn key_history_carries_back_only_verified_legacy_evidence_with_writer_attribution() {
    let (window, audio, id, _, _) = setup("aiff");
    let original = shown(&window, &id)["stems"]["fact_id"].clone();
    edit(&window, &id);
    let forward = shown(&window, &id)["stems"]["fact_id"].clone();
    assert_ne!(forward, original);
    for undo in [true, false] {
        if undo {
            run(commands::undo_edit(
                window.app.handle().clone(),
                window.state(),
            ))
            .unwrap();
        } else {
            run(commands::redo_edit(
                window.app.handle().clone(),
                window.state(),
            ))
            .unwrap();
        }
        let now = shown(&window, &id);
        assert!(!now["stems"].is_null(), "{now}");
        assert_eq!(
            now["stems"]["fingerprint"],
            wreckordbox_sidecar::fingerprint_file(&audio)
                .unwrap()
                .legacy()
                .as_str()
        );
    }
    let entries = carries(&window);
    assert_eq!(entries.len(), 3, "{entries:?}");
    for entry in &entries {
        assert_eq!(
            entry["source"],
            json!({"kind":"tool","name":"wreckordbox-key-tag"})
        );
        assert_eq!(entry["payload"]["evidence_version"], 1);
        let linked = entry["payload"]["facts"].as_array().unwrap();
        assert_eq!(linked.len(), 2);
        assert!(linked
            .iter()
            .all(|f| f["track"] == id && f["from_fact"] != f["to_fact"]));
    }
    let root = window
        .location
        .master_db
        .parent()
        .unwrap()
        .join("wreckordbox-sidecar");
    let old = rusqlite::Connection::open_with_flags(
        root.join("sidecar.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let fingerprint: String = old
        .query_row(
            "SELECT fingerprint FROM fact WHERE id=?1",
            [original.as_i64().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(fingerprint, store::golden()["aiff"]["a1"].as_str().unwrap());
    let snapshot = store::snapshot(&root);
    for _ in 0..2 {
        shown(&window, &id);
        window
            .cli(&["--json", "facts", "show", &id, "--check-audio"])
            .json();
        window.cli(&["--json", "intensity", "show", &id]).json();
        carries(&window);
    }
    assert_eq!(
        store::snapshot(&root),
        snapshot,
        "reads never carry or repair evidence"
    );
}
#[test]
fn failed_carry_recording_keeps_the_key_and_no_copies_or_empty_writer_run() {
    let (window, audio, id, _, _) = setup("aiff");
    let root = window
        .location
        .master_db
        .parent()
        .unwrap()
        .join("wreckordbox-sidecar");
    let db = rusqlite::Connection::open(root.join("sidecar.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_key_evidence BEFORE INSERT ON ledger WHEN NEW.kind='key_evidence_carry' BEGIN SELECT RAISE(ABORT, 'fixture ledger refusal'); END;").unwrap();
    let before: i64 = db
        .query_row("SELECT COUNT(*) FROM fact", [], |r| r.get(0))
        .unwrap();
    let result = run(rbxport_lib::details::set_track_field(
        window.app.handle().clone(),
        window.state(),
        vec![id.clone()],
        "key".into(),
        "Fm".into(),
        Some(true),
    ));
    assert!(result
        .unwrap_err()
        .full_message()
        .contains("evidence was not fully carried"));
    assert_eq!(
        rbl_db::key_tag::read_key(&audio).unwrap().as_deref(),
        Some("Fm")
    );
    assert!(shown(&window, &id)["stems"].is_null());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM fact", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before
    );
    assert!(carries(&window).is_empty());
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM run WHERE source_name='wreckordbox-key-tag'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(window
        .state()
        .core()
        .history()
        .lock()
        .undo_label()
        .is_some());
}
#[test]
fn protection_refuses_key_and_carry_without_writing() {
    let (window, audio, id, _, _) = setup("aiff");
    window.protect(true);
    let root = window.location.master_db.parent().unwrap();
    let before = store::snapshot(root);
    let bytes = std::fs::read(&audio).unwrap();
    assert!(run(rbxport_lib::details::set_track_field(
        window.app.handle().clone(),
        window.state(),
        vec![id.clone()],
        "key".into(),
        "Fm".into(),
        Some(true)
    ))
    .is_err());
    assert_eq!(std::fs::read(audio).unwrap(), bytes);
    assert_eq!(store::snapshot(root), before);
    assert!(carries(&window).is_empty());
}

#[test]
fn an_external_key_write_and_preexisting_audio_change_are_never_carried() {
    let (window, audio, id, _, _) = setup("aiff");
    rbl_db::key_tag::write_key(&audio, "Cm").unwrap();
    assert!(shown(&window, &id)["stems"].is_null());
    assert!(carries(&window).is_empty());
    // Even the next verified app rewrite cannot bless evidence already stale.
    edit(&window, &id);
    assert!(shown(&window, &id)["stems"].is_null());
    assert!(carries(&window).is_empty());
}
#[test]
fn analysis_key_publication_uses_one_tool_run_and_carries_existing_evidence() {
    let (window, audio, id, _, _) = setup("aiff");
    let operation =
        rbl_library::bulk::Operation::new(window.state().core(), rbl_library::bulk::Kind::Analysis);
    let result = operation.run(
        json!({"track":id}),
        || {
            let (written, warning) = rbxport_lib::key_tag::write_analysis_key_tag(
                window.state().inner(),
                &id,
                audio.to_str().unwrap(),
                "Fm",
            );
            assert!(warning.is_none(), "{warning:?}");
            assert!(written.is_some());
            Ok::<_, String>(())
        },
        |_| vec![json!({"track":id,"kind":"audio_key_tag"})],
    );
    result.unwrap();
    assert!(operation.finish().is_none());
    assert!(!shown(&window, &id)["stems"].is_null());
    let entries = carries(&window);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["source"]["name"], "wreckordbox-key-tag");
    let run = window
        .cli(&["--json", "runs", "show", &entries[0]["run"].to_string()])
        .json();
    assert_eq!(run["run"]["source"]["name"], "wreckordbox-analysis");
    assert!(run["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["kind"] == "analysis"));
}

#[test]
fn many_measurements_carry_in_bounded_batches_in_one_writer_run() {
    let (window, _, id, _, _) = setup("aiff");
    let db = rusqlite::Connection::open(
        window
            .location
            .master_db
            .parent()
            .unwrap()
            .join("wreckordbox-sidecar/sidecar.db"),
    )
    .unwrap();
    for i in 0..130 {
        db.execute("INSERT INTO fact(track_id,fingerprint,kind,value,source_kind,source_name,confidence,run_id,recorded_at) SELECT ?1,fingerprint,kind,value,'tool',?2,confidence,run_id,recorded_at FROM fact WHERE id=7", rusqlite::params![id,format!("fixture-{i}")]).unwrap();
    }
    edit(&window, &id);
    let entries = carries(&window);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["run"], entries[1]["run"]);
    let batches: Vec<_> = entries
        .iter()
        .map(|e| e["payload"]["facts"].as_array().unwrap().len())
        .collect();
    assert!(batches.iter().all(|n| *n <= 128));
    assert_eq!(batches.iter().sum::<usize>(), 132);
    let remaining: i64 = db.query_row("SELECT COUNT(*) FROM attached_fact WHERE effective_track_id=?1 AND superseded_at IS NULL", [&id], |r|r.get(0)).unwrap();
    assert_eq!(remaining, 132);
}

#[test]
fn a_completed_undo_with_failed_carry_announces_its_history_and_native_change() {
    let (window, audio, id, _, _) = setup("aiff");
    edit(&window, &id);
    let db = rusqlite::Connection::open(
        window
            .location
            .master_db
            .parent()
            .unwrap()
            .join("wreckordbox-sidecar/sidecar.db"),
    )
    .unwrap();
    db.execute_batch("CREATE TRIGGER refuse_key_evidence BEFORE INSERT ON ledger WHEN NEW.kind='key_evidence_carry' BEGIN SELECT RAISE(ABORT, 'fixture ledger refusal'); END;").unwrap();
    let changes = window.changes.lock().unwrap().len();
    let history = window.history.lock().unwrap().len();
    let error = run(commands::undo_edit(
        window.app.handle().clone(),
        window.state(),
    ))
    .unwrap_err();
    assert!(error
        .full_message()
        .contains("evidence was not fully carried"));
    assert_ne!(
        rbl_db::key_tag::read_key(&audio).unwrap().as_deref(),
        Some("Fm")
    );
    assert!(window
        .state()
        .core()
        .history()
        .lock()
        .redo_label()
        .is_some());
    assert_eq!(
        window.changes.lock().unwrap().len(),
        changes + 1,
        "accepted Undo must refresh the window even when carry failed"
    );
    assert_eq!(
        window.history.lock().unwrap().len(),
        history + 1,
        "the menu must offer Redo of the accepted Undo"
    );
    assert!(shown(&window, &id)["stems"].is_null());
}
