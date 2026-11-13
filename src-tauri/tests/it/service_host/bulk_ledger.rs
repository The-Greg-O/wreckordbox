use super::*;
use rbxport_lib::bulk_writes::{begin_ledger_operation, finish_ledger_operation};
use serde_json::json;

fn wav(path: &std::path::Path) {
    let rate = 22_050_u32;
    let frames = rate * 4;
    let data: Vec<u8> = (0..frames)
        .flat_map(|i| {
            let since = i % (rate / 2);
            let sample: i16 = if since < 400 {
                20_000 - (since * 50) as i16
            } else {
                0
            };
            sample.to_le_bytes()
        })
        .collect();
    let mut bytes = Vec::new();
    bytes.extend(b"RIFF");
    bytes.extend((36 + data.len() as u32).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16_u32.to_le_bytes());
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend(rate.to_le_bytes());
    bytes.extend((rate * 2).to_le_bytes());
    bytes.extend(2_u16.to_le_bytes());
    bytes.extend(16_u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend((data.len() as u32).to_le_bytes());
    bytes.extend(data);
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn imported_slices_and_analysis_are_operations_visible_to_the_real_cli() {
    let window = window();
    window.load();
    window.host().unwrap();
    window
        .app
        .manage(Arc::new(rbxport_lib::grid::GridEditor::at(
            &window.dir.path().join("grid"),
        )));
    let mut client = window.client();
    client.subscribe().unwrap();
    window.changes.lock().unwrap().clear();
    let operation = run(begin_ledger_operation(window.state(), "import".into())).unwrap();
    let mut ids = Vec::new();
    for i in 0..2 {
        let path = window.dir.path().join(format!("bulk-{i}.wav"));
        wav(&path);
        let result = run(commands::import_files_slice(
            window.app.handle().clone(),
            window.state(),
            vec![path.to_string_lossy().into_owned()],
            Some(operation.clone()),
        ))
        .unwrap();
        assert_eq!(result.imported, 1);
        ids.push(result.tracks[0].id.clone());
    }
    assert!(
        window.changes.lock().unwrap().is_empty(),
        "slices do not refresh the Library individually"
    );
    assert_eq!(
        run(finish_ledger_operation(window.state(), operation)).unwrap(),
        None
    );
    run(commands::reload_library(
        window.app.handle().clone(),
        window.state(),
    ))
    .unwrap();
    assert_eq!(window.changes.lock().unwrap().len(), 1);
    let event = client.next_event(Duration::from_secs(2)).unwrap().unwrap();
    assert_eq!(event.event, "library_changed");
    assert!(client
        .next_event(Duration::from_millis(100))
        .unwrap()
        .is_none());
    let runs = window.cli(&["--json", "runs", "list"]).json();
    assert_eq!(runs["total"], 1);
    let entries = ledger_entries(&window, &runs["runs"][0]["id"]);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["kind"], "import");
    assert_eq!(entries[0]["source"]["kind"], "dj");
    assert_eq!(
        entries[0]["payload"]["records"].as_array().unwrap().len(),
        1
    );
    let operation = run(begin_ledger_operation(window.state(), "analysis".into())).unwrap();
    for id in &ids {
        let result = run(rbxport_lib::analysis::analyse_track(
            window.app.handle().clone(),
            window.state(),
            window.app.state(),
            id.clone(),
            Some("rbxport".into()),
            None,
            None,
            None,
            None,
            Some(operation.clone()),
        ))
        .unwrap();
        assert!(!result.analysis_path.is_empty());
        assert!(rbl_anlz::resolve(&window.location.share_root, &result.analysis_path).is_file());
    }
    assert_eq!(
        run(finish_ledger_operation(window.state(), operation)).unwrap(),
        None
    );
    let runs = window.cli(&["--json", "runs", "list"]).json();
    assert_eq!(runs["total"], 2);
    let entries = ledger_entries(&window, &runs["runs"][0]["id"]);
    assert_eq!(
        entries
            .iter()
            .flat_map(|e| e["payload"]["records"].as_array().into_iter().flatten())
            .filter(|r| r.get("after").is_some())
            .count(),
        2
    );
    assert_eq!(
        entries[0]["source"],
        json!({"kind":"tool","name":"wreckordbox-analysis"})
    );
    assert_eq!(
        entries[0]["payload"]["records"].as_array().unwrap().len(),
        1
    );
    window.protect(true);
    let operation = run(begin_ledger_operation(window.state(), "import".into())).unwrap();
    let path = window.dir.path().join("refused.wav");
    wav(&path);
    assert!(run(commands::import_files_slice(
        window.app.handle().clone(),
        window.state(),
        vec![path.to_string_lossy().into_owned()],
        Some(operation.clone())
    ))
    .is_err());
    assert_eq!(
        run(finish_ledger_operation(window.state(), operation)).unwrap(),
        None
    );
    assert_eq!(window.cli(&["--json", "runs", "list"]).json()["total"], 2);
}

#[test]
fn forty_abandoned_operation_tokens_do_not_wedge_later_imports() {
    let window = window();
    window.load();
    window.host().unwrap();
    let mut expired = String::new();
    for i in 0..40 {
        let token = run(begin_ledger_operation(window.state(), "import".into())).unwrap();
        if i == 0 {
            expired = token;
        }
    }
    let late_path = window.dir.path().join("expired-token.wav");
    wav(&late_path);
    let late = run(commands::import_files_slice(
        window.app.handle().clone(),
        window.state(),
        vec![late_path.display().to_string()],
        Some(expired),
    ))
    .unwrap();
    assert_eq!(late.imported, 1);
    assert!(late
        .recording_warning
        .as_deref()
        .is_some_and(|w| w.contains("own Run")));
    let token = run(begin_ledger_operation(window.state(), "import".into())).unwrap();
    let path = window.dir.path().join("after-abandon.wav");
    wav(&path);
    let report = run(commands::import_files_slice(
        window.app.handle().clone(),
        window.state(),
        vec![path.to_string_lossy().into_owned()],
        Some(token.clone()),
    ))
    .unwrap();
    assert_eq!(report.imported, 1);
    assert_eq!(
        run(finish_ledger_operation(window.state(), token)).unwrap(),
        None
    );
    assert_eq!(window.cli(&["--json", "runs", "list"]).json()["total"], 2);
}

#[test]
fn cli_names_accepted_imports_waiting_on_disk_without_replaying_them() {
    if std::env::var_os("WBX1411_CLI_SPILL_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "bulk_ledger::cli_names_accepted_imports_waiting_on_disk_without_replaying_them",
                "--nocapture",
            ])
            .env("WBX1411_CLI_SPILL_CHILD", "1")
            .env("WRECKORDBOX_LEDGER_FAULTS", "fail_append=999")
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
    let window = window();
    window.load();
    window.host().unwrap();
    let path = window.dir.path().join("pending.wav");
    wav(&path);
    let imported = run(commands::import_files(
        window.app.handle().clone(),
        window.state(),
        vec![path.display().to_string()],
    ))
    .unwrap();
    assert_eq!(imported.imported, 1);
    assert!(imported.recording_warning.is_some());
    let runs = window.cli(&["--json", "runs", "list"]).json();
    let id = runs["runs"][0]["id"].to_string();
    let shown = window.cli(&["--json", "runs", "show", &id]).json();
    assert_eq!(shown["pending_recording"]["total"], 1);
    assert_eq!(
        shown["pending_recording"]["records"][0]["result"]["track"],
        imported.tracks[0].id
    );
    let text = window.cli(&["runs", "show", &id]);
    assert!(text.stdout.contains(&imported.tracks[0].id));
    assert!(text.stdout.contains("waiting for Ledger recording"));
    assert_eq!(window.cli(&["--json", "runs", "show", &id]).json(), shown);
}
