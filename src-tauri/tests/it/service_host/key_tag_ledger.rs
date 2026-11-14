use super::*;
use serde_json::json;

fn write_chord_aiff(path: &std::path::Path) {
    let rate = 44_100u32;
    let frames = rate * 20;
    let mut ssnd = vec![0u8; 8];
    for n in 0..frames {
        let t = f64::from(n) / f64::from(rate);
        let v: f64 = [220.0, 261.63, 329.63]
            .iter()
            .map(|f| (2.0 * std::f64::consts::PI * f * t).sin())
            .sum::<f64>()
            / 4.0;
        ssnd.extend_from_slice(&((v * 20_000.0) as i16).to_be_bytes());
    }
    let mut comm = Vec::new();
    comm.extend_from_slice(&1_u16.to_be_bytes());
    comm.extend_from_slice(&frames.to_be_bytes());
    comm.extend_from_slice(&16_u16.to_be_bytes());
    comm.extend_from_slice(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]);
    let mut body = b"AIFF".to_vec();
    for (id, chunk) in [(b"COMM", &comm), (b"SSND", &ssnd)] {
        body.extend_from_slice(id);
        body.extend_from_slice(&(chunk.len() as u32).to_be_bytes());
        body.extend_from_slice(chunk);
    }
    let mut out = b"FORM".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    std::fs::write(path, out).unwrap();
}

fn imported(window: &Window) -> (PathBuf, String) {
    let audio = window.dir.path().join("key.aiff");
    write_chord_aiff(&audio);
    rbl_db::key_tag::write_key(&audio, "Cm").unwrap();
    let report = run(commands::import_files(
        window.app.handle().clone(),
        window.state(),
        vec![audio.display().to_string()],
    ))
    .unwrap();
    (audio, report.tracks[0].id.clone())
}
fn all_entries(window: &Window) -> Vec<serde_json::Value> {
    window.cli(&["--json", "runs", "list"]).json()["runs"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|r| ledger_entries(window, &r["id"]))
        .collect()
}

#[test]
fn an_audio_key_edit_and_its_history_record_the_file_values() {
    let window = window();
    window.load();
    window.host().unwrap();
    let (audio, id) = imported(&window);
    window
        .state()
        .write(|w| w.ensure_detected_key("Fm"))
        .unwrap();
    run(rbxport_lib::details::set_track_field(
        window.app.handle().clone(),
        window.state(),
        vec![id.clone()],
        "key".into(),
        "Fm".into(),
        Some(true),
    ))
    .unwrap();
    assert_eq!(
        rbl_db::key_tag::read_key(&audio).unwrap().as_deref(),
        Some("Fm")
    );
    let entries = all_entries(&window);
    let entry = entries.iter().find(|e| e["kind"] == "key_edit").unwrap();
    let values = json!([{"path":audio, "before":"Cm", "after":"Fm"}]);
    assert_eq!(entry["payload"]["files"], values, "{entry}");
    assert_eq!(entry["source"], json!({"kind":"dj","name":"dj"}));
    assert_eq!(
        entry["reversible"], false,
        "audio Unwind is not implemented"
    );
    run(commands::undo_edit(
        window.app.handle().clone(),
        window.state(),
    ))
    .unwrap();
    assert_eq!(
        rbl_db::key_tag::read_key(&audio).unwrap().as_deref(),
        Some("Cm")
    );
    run(commands::redo_edit(
        window.app.handle().clone(),
        window.state(),
    ))
    .unwrap();
    assert_eq!(
        rbl_db::key_tag::read_key(&audio).unwrap().as_deref(),
        Some("Fm")
    );
    let entries = all_entries(&window);
    for action in ["undo", "redo"] {
        assert!(
            entries.iter().any(|e| e["kind"] == "key_edit"
                && e["action"] == action
                && e["payload"]["files"] == values),
            "{entries:?}"
        );
    }
    let before = std::fs::read(&audio).unwrap();
    let runs = window.cli(&["--json", "runs", "list"]).json();
    window.protect(true);
    assert!(run(rbxport_lib::details::set_track_field(
        window.app.handle().clone(),
        window.state(),
        vec![id],
        "key".into(),
        "Cm".into(),
        Some(true)
    ))
    .is_err());
    assert!(rbxport_lib::key_tag::write_logged(
        window.state().inner(),
        "protected",
        &audio.display().to_string(),
        "Am"
    )
    .is_some());
    assert_eq!(std::fs::read(&audio).unwrap(), before);
    assert_eq!(window.cli(&["--json", "runs", "list"]).json(), runs);
}

#[test]
fn analysis_records_the_audio_tag_in_the_analysis_run() {
    let window = window();
    window.load();
    window.host().unwrap();
    window
        .app
        .manage(Arc::new(rbxport_lib::grid::GridEditor::at(
            &window.dir.path().join("grid"),
        )));
    let (audio, id) = imported(&window);
    let result = run(rbxport_lib::analysis::analyse_track(
        window.app.handle().clone(),
        window.state(),
        window.app.state(),
        id.clone(),
        None,
        Some(rbxport_lib::analysis::AnalysisSettings {
            bpm_grid: false,
            ..Default::default()
        }),
        Some(rbxport_lib::key_tag::KeyNotation::Classic),
        None,
        None,
        None,
    ))
    .unwrap();
    assert_eq!(
        rbl_db::key_tag::read_key(&audio).unwrap().as_deref(),
        Some(result.key.as_str())
    );
    let runs = window.cli(&["--json", "runs", "list"]).json();
    assert_eq!(runs["total"], 2, "one import and one analysis Run");
    let entries = ledger_entries(&window, &runs["runs"][0]["id"]);
    assert!(entries
        .iter()
        .all(|e| e["source"] == json!({"kind":"tool","name":"wreckordbox-analysis"})));
    let records: Vec<_> = entries
        .iter()
        .flat_map(|e| e["payload"]["records"].as_array().into_iter().flatten())
        .collect();
    assert!(
        records.iter().any(|r| r["track"] == id
            && r["audio_key_tag"] == json!({"path":audio,"before":"Cm","after":result.key})),
        "{records:?}"
    );
}

#[test]
fn window_undo_does_not_keep_an_outer_gate_during_audio_preparation() {
    use rbl_library::history::{LibraryEdit, OutsideSteps};
    struct Audio(std::sync::Weak<AppState>);
    struct Staged;
    impl OutsideSteps for Staged {
        fn is_empty(&self) -> bool {
            false
        }
        fn check(&self, _: bool, _: &str) -> Result<(), rbl_db::DbError> {
            Ok(())
        }
        fn step(&self, _: bool, _: &str) -> Result<(), rbl_db::DbError> {
            Ok(())
        }
        fn revert(&self, _: bool) -> (usize, usize) {
            (0, 1)
        }
    }
    impl OutsideSteps for Audio {
        fn is_empty(&self) -> bool {
            false
        }
        fn prepare(
            &self,
            _: bool,
            _: &str,
        ) -> Result<Option<Arc<dyn OutsideSteps>>, rbl_db::DbError> {
            let state = self.0.upgrade().unwrap();
            let free = std::thread::spawn(move || state.core().edit_gate().try_lock().is_some())
                .join()
                .unwrap();
            assert!(
                free,
                "the window must release its outer gate before preparing audio"
            );
            Ok(Some(Arc::new(Staged)))
        }
        fn check(&self, _: bool, _: &str) -> Result<(), rbl_db::DbError> {
            Ok(())
        }
        fn step(&self, _: bool, _: &str) -> Result<(), rbl_db::DbError> {
            Ok(())
        }
        fn revert(&self, _: bool) -> (usize, usize) {
            (0, 1)
        }
    }
    let window = window();
    window.load();
    window.host().unwrap();
    let state = Arc::clone(window.state().inner());
    let id = track_id(1);
    let stars = if window.rating(&id) == 4 { 3 } else { 4 };
    state
        .core()
        .recorded_edit(
            state.as_ref(),
            rbl_library::Touched::Tracks,
            "Audio key",
            |w| {
                let (_, edit) = w.set_rating_with_undo(&id, stars)?;
                Ok((
                    (),
                    LibraryEdit::KeyEdit {
                        tracks: vec![edit],
                        files: Arc::new(Audio(Arc::downgrade(&state))),
                    },
                ))
            },
        )
        .unwrap();
    run(commands::undo_edit(
        window.app.handle().clone(),
        window.state(),
    ))
    .unwrap();
    assert_ne!(window.rating(&id), stars);
}

#[test]
fn an_accepted_audio_tag_survives_recording_failure_and_reads_do_not_replay_it() {
    let window = window();
    window.load();
    window.host().unwrap();
    window
        .app
        .manage(Arc::new(rbxport_lib::grid::GridEditor::at(
            &window.dir.path().join("grid"),
        )));
    let (audio, id) = imported(&window);
    let sidecar = window
        .location
        .master_db
        .parent()
        .unwrap()
        .join("wreckordbox-sidecar/sidecar.db");
    let db = rusqlite::Connection::open(sidecar).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_key_ledger BEFORE INSERT ON ledger BEGIN SELECT RAISE(ABORT,'fixture recording failure'); END;").unwrap();
    let result = run(rbxport_lib::analysis::analyse_track(
        window.app.handle().clone(),
        window.state(),
        window.app.state(),
        id.clone(),
        None,
        Some(rbxport_lib::analysis::AnalysisSettings {
            bpm_grid: false,
            ..Default::default()
        }),
        Some(rbxport_lib::key_tag::KeyNotation::Classic),
        None,
        None,
        None,
    ))
    .unwrap();
    assert!(result.recording_warning.is_some());
    assert_eq!(
        rbl_db::key_tag::read_key(&audio).unwrap().as_deref(),
        Some(result.key.as_str())
    );
    let runs = window.cli(&["--json", "runs", "list"]).json();
    let run_id = runs["runs"][0]["id"].to_string();
    let pending = window.cli(&["--json", "runs", "show", &run_id]).json();
    assert!(
        pending["pending_recording"]["total"].as_u64().unwrap() > 0,
        "{pending}"
    );
    assert_eq!(
        window.cli(&["--json", "runs", "show", &run_id]).json(),
        pending
    );
    let bytes = std::fs::read(&audio).unwrap();
    db.execute_batch("DROP TRIGGER refuse_key_ledger").unwrap();
    assert_eq!(
        window.cli(&["--json", "runs", "show", &run_id]).json(),
        pending,
        "even now a read cannot recover"
    );
    run(commands::set_track_rating(
        window.app.handle().clone(),
        window.state(),
        vec![track_id(0)],
        4,
    ))
    .unwrap();
    let entries = ledger_entries(
        &window,
        &serde_json::Value::from(run_id.parse::<u64>().unwrap()),
    );
    assert!(entries
        .iter()
        .all(|e| e["source"] == json!({"kind":"tool","name":"wreckordbox-analysis"})));
    assert!(
        entries
            .iter()
            .flat_map(|e| e["payload"]["records"].as_array().into_iter().flatten())
            .any(|r| r["audio_key_tag"] == json!({"path":audio,"before":"Cm","after":result.key})),
        "{entries:?}"
    );
    assert_eq!(std::fs::read(&audio).unwrap(), bytes);
}
