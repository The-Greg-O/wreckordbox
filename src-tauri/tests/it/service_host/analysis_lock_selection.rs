use super::*;

fn selection(window: &Window) -> Result<(), rbxport_lib::AppError> {
    run(rbxport_lib::grid::grid_lock_selection(
        window.app.handle().clone(),
        window.state(),
        window.app.state(),
        (0..3).map(track_id).collect(),
        true,
    ))
}
fn selected_flags(window: &Window) -> Vec<(i64, i64)> {
    let db = rbl_db::Library::open(window.location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    (0..3)
        .map(|i| {
            db.connection().query_row(
        "SELECT COALESCE(Analysed,0), COALESCE(rb_local_usn,0) FROM djmdContent WHERE ID=?1",
        [track_id(i)], |r| Ok((r.get(0)?, r.get(1)?)),
    ).unwrap()
        })
        .collect()
}
#[test]
fn selection_lock_is_one_undo_step_for_all_three_tracks() {
    let window = prepared();
    let before = selected_flags(&window);
    let mut client = window.client();
    client.subscribe().unwrap();
    selection(&window).unwrap();
    assert!(selected_flags(&window).iter().all(|v| v.0 & 128 != 0));
    assert_eq!(
        client
            .next_event(Duration::from_secs(2))
            .unwrap()
            .unwrap()
            .event,
        "library_changed"
    );
    assert!(
        client
            .next_event(Duration::from_millis(100))
            .unwrap()
            .is_none(),
        "selection notifications are coalesced"
    );
    let locked = selected_flags(&window);
    selection(&window).unwrap();
    assert_eq!(
        selected_flags(&window),
        locked,
        "a repeated selection writes no revision"
    );
    assert_eq!(entries(&window).len(), 1, "a repeated lock asserts nothing");
    run(commands::undo_edit(
        window.app.handle().clone(),
        window.state(),
    ))
    .unwrap();
    assert_eq!(
        selected_flags(&window)
            .iter()
            .map(|v| v.0)
            .collect::<Vec<_>>(),
        before.iter().map(|v| v.0).collect::<Vec<_>>(),
        "one Undo must unlock all three selected tracks"
    );
    let all = entries(&window);
    assert_eq!(all.len(), 2, "one lock entry and one Undo entry");
    let recorded = all[0]["payload"]["edits"].as_array().unwrap();
    assert_eq!(
        recorded
            .iter()
            .map(|edit| edit["track"].clone())
            .collect::<Vec<_>>(),
        (0..3).map(|i| json!(track_id(i))).collect::<Vec<_>>()
    );
    assert_eq!(all[1]["reverses"], all[0]["id"]);
    assert_eq!(
        window.changes.lock().unwrap().len(),
        2,
        "one refresh per operation"
    );
}
#[test]
fn selection_lock_failure_on_the_second_track_rolls_back_the_whole_selection() {
    let window = prepared();
    let db = rbl_db::Library::open(window.location.clone(), rbl_db::OpenMode::ReadWrite).unwrap();
    db.connection().execute_batch(&format!(
        "CREATE TRIGGER refuse_second_lock BEFORE UPDATE OF Analysed ON djmdContent WHEN OLD.ID='{}' BEGIN SELECT RAISE(ABORT,'fixture second lock failure'); END;", track_id(1)
    )).unwrap();
    let before = selected_flags(&window);
    assert!(selection(&window).is_err());
    assert_eq!(
        selected_flags(&window),
        before,
        "a failed selection must leave every flag and revision unchanged"
    );
    assert!(window.changes.lock().unwrap().is_empty());
    assert!(run(commands::undo_edit(
        window.app.handle().clone(),
        window.state()
    ))
    .is_err());
}

#[test]
fn protection_refuses_the_entire_selection_without_writes() {
    let window = prepared();
    window.protect(true);
    let before = std::fs::read(&window.location.master_db).unwrap();
    let flags = selected_flags(&window);
    assert_eq!(
        selection(&window).unwrap_err().message,
        rbl_library::protection::MESSAGE
    );
    assert_eq!(std::fs::read(&window.location.master_db).unwrap(), before);
    assert_eq!(selected_flags(&window), flags);
    assert!(entries(&window).is_empty());
    assert!(window.changes.lock().unwrap().is_empty());
}

#[test]
fn an_unrelated_flag_change_refuses_the_whole_selection_undo() {
    let window = prepared();
    selection(&window).unwrap();
    let db = rbl_db::Library::open(window.location.clone(), rbl_db::OpenMode::ReadWrite).unwrap();
    db.connection()
        .execute(
            "UPDATE djmdContent SET Analysed=Analysed+256 WHERE ID=?1",
            [track_id(1)],
        )
        .unwrap();
    let before = selected_flags(&window);
    assert!(run(commands::undo_edit(
        window.app.handle().clone(),
        window.state()
    ))
    .is_err());
    assert_eq!(
        selected_flags(&window),
        before,
        "a conflict cannot undo a prefix"
    );
}

#[test]
#[ignore = "crash worker, invoked by selection_crash_leaves_a_bounded_barrier"]
fn crash_after_selection_write_child() {
    let root = PathBuf::from(std::env::var_os("WBX1412_CRASH_ROOT").unwrap());
    let mut location: rbl_db::LibraryLocation =
        serde_json::from_slice(&std::fs::read(root.join("location.json")).unwrap()).unwrap();
    assert!(!location.is_real_install && location.master_db.starts_with(&root));
    location.passphrase = fixture::FIXTURE_PASSPHRASE.to_owned();
    let state = Arc::new(AppState::with_backups(root.join("backups")));
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (index, _) = rbl_index::load(&db).unwrap();
    state.set_library(index, false, db.schema().db_version, 0, location.clone());
    drop(db);
    let app = tauri::test::mock_app();
    app.manage(state);
    app.manage(Arc::new(rbxport_lib::grid::GridEditor::at(
        &root.join("grid"),
    )));
    let config = Config {
        runtime_dir: root.join("run"),
        idle_exit: None,
        recovery_dir: root.join("backups"),
    };
    service_host::host_with(app.handle(), &location, &config).unwrap();
    run(rbxport_lib::grid::grid_lock_selection(
        app.handle().clone(),
        app.state(),
        app.state(),
        (0..3).map(track_id).collect(),
        true,
    ))
    .unwrap();
    panic!("the fault must terminate after the native commit and before its entry");
}

#[test]
fn selection_crash_leaves_a_bounded_barrier() {
    let window = window();
    std::fs::write(
        window.dir.path().join("location.json"),
        serde_json::to_vec(&window.location).unwrap(),
    )
    .unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "service_host::analysis_lock_ledger::selection_tests::crash_after_selection_write_child", "--ignored", "--nocapture"])
        .env("WBX1412_CRASH_ROOT", window.dir.path())
        .env("WRECKORDBOX_LEDGER_FAULTS", "abort_after_write")
        .output().unwrap();
    assert!(!output.status.success(), "the crash worker survived");
    assert!(
        selected_flags(&window).iter().all(|v| v.0 & 128 != 0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let sidecar = rusqlite::Connection::open_with_flags(
        window
            .location
            .master_db
            .parent()
            .unwrap()
            .join("wreckordbox-sidecar/sidecar.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        sidecar
            .query_row("SELECT COUNT(*) FROM ledger", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0,
        "the crash preceded recording"
    );
    drop(sidecar);
    window.host().unwrap();
    window.load();
    let listed = window.cli(&["--json", "projection", "barriers"]).json();
    let barriers = listed["barriers"].as_array().unwrap();
    assert_eq!(barriers.len(), 1, "{listed}");
    assert_eq!(barriers[0]["what"], "unconfirmed");
    assert_eq!(
        barriers[0]["tracks"],
        json!((0..3).map(track_id).collect::<Vec<_>>())
    );
    assert_eq!(entries(&window).len(), 1);
}
