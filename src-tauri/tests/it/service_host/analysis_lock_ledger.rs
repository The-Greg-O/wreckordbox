use super::*;
use serde_json::json;

fn prepared() -> Window {
    let window = window();
    let relative = "/PIONEER/USBANLZ/lock-ledger/ANLZ0000.DAT";
    fixture::set_analysis_path(&window.location, 1, relative).unwrap();
    let dat = rbl_anlz::resolve(&window.location.share_root, relative);
    std::fs::create_dir_all(dat.parent().unwrap()).unwrap();
    let mut builder = rbl_anlz::AnlzBuilder::new();
    builder.path("/fixture.wav").beat_grid(&[
        rbl_anlz::Beat {
            time_ms: 0,
            beat_number: 1,
            tempo_x100: 12000,
        },
        rbl_anlz::Beat {
            time_ms: 500,
            beat_number: 2,
            tempo_x100: 12000,
        },
    ]);
    std::fs::write(dat, builder.finish()).unwrap();
    window
        .app
        .manage(Arc::new(rbxport_lib::grid::GridEditor::at(
            &window.dir.path().join("grid"),
        )));
    window.load();
    window.host().unwrap();
    window
}
fn lock(
    window: &Window,
    on: bool,
) -> Result<rbxport_lib::grid::GridStateDto, rbxport_lib::AppError> {
    run(rbxport_lib::grid::grid_lock(
        window.app.handle().clone(),
        window.state(),
        window.app.state(),
        track_id(1),
        on,
    ))
}
fn native(window: &Window) -> (i64, i64) {
    let db = rbl_db::Library::open(window.location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    db.connection()
        .query_row(
            "SELECT COALESCE(Analysed,0), COALESCE(rb_local_usn,0) FROM djmdContent WHERE ID=?1",
            [track_id(1)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}
#[test]
fn native_lock_records_the_djs_explicit_edit_and_notifies_clients() {
    let window = prepared();
    let before = native(&window).0;
    let mut client = window.client();
    client.subscribe().unwrap();
    assert!(lock(&window, true).unwrap().locked);
    assert_eq!(native(&window).0, (before & 127) | 128);
    let runs = window.cli(&["--json", "runs", "list"]).json();
    assert_eq!(runs["total"], 1, "the accepted native lock needs a Run");
    let entries = ledger_entries(&window, &runs["runs"][0]["id"]);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["label"], "Analysis Lock");
    assert_eq!(entries[0]["source"], json!({"kind":"dj", "name":"dj"}));
    assert_eq!(entries[0]["payload"]["edits"][0]["column"], "Analysed");
    assert_eq!(
        entries[0]["payload"]["edits"][0]["after"],
        (before & 127) | 128
    );
    assert_eq!(
        client
            .next_event(Duration::from_secs(2))
            .unwrap()
            .unwrap()
            .event,
        "library_changed"
    );
}
#[test]
fn unchanged_lock_and_protected_lock_do_not_write_or_record() {
    let window = prepared();
    window.protect(true);
    let bytes = std::fs::read(&window.location.master_db).unwrap();
    let before = native(&window);
    assert_eq!(
        lock(&window, true).unwrap_err().message,
        rbl_library::protection::MESSAGE
    );
    assert_eq!(native(&window), before);
    assert_eq!(std::fs::read(&window.location.master_db).unwrap(), bytes);
    assert_eq!(window.cli(&["--json", "runs", "list"]).json()["total"], 0);
    window.protect(false);
    assert!(lock(&window, true).unwrap().locked);
    let before = native(&window);
    let runs = window.cli(&["--json", "runs", "list"]).json();
    let files = snapshot(window.location.master_db.parent().unwrap());
    let generation = window.generation();
    assert!(lock(&window, true).unwrap().locked);
    assert_eq!(window.generation(), generation);
    assert_eq!(snapshot(window.location.master_db.parent().unwrap()), files);
    assert_eq!(
        native(&window),
        before,
        "an unchanged lock must not bump the row revision"
    );
    assert_eq!(window.cli(&["--json", "runs", "list"]).json(), runs);
}

fn entries(window: &Window) -> Vec<serde_json::Value> {
    window.cli(&["--json", "runs", "list"]).json()["runs"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|r| ledger_entries(window, &r["id"]))
        .collect()
}
fn snapshot(root: &std::path::Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &std::path::Path, out: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
        if !root.exists() {
            return;
        }
        for item in std::fs::read_dir(root).unwrap() {
            let path = item.unwrap().path();
            if path.is_dir() {
                visit(&path, out);
            } else if path.is_file() && !path.to_string_lossy().ends_with("-shm") {
                out.insert(path.clone(), std::fs::read(path).unwrap());
            }
        }
    }
    let mut out = Default::default();
    visit(root, &mut out);
    out
}

#[test]
fn append_failure_keeps_the_lock_and_reads_never_replay_its_record() {
    let window = prepared();
    run(commands::set_track_rating(
        window.app.handle().clone(),
        window.state(),
        vec![track_id(0)],
        4,
    ))
    .unwrap();
    let db = rusqlite::Connection::open(
        window
            .location
            .master_db
            .parent()
            .unwrap()
            .join("wreckordbox-sidecar/sidecar.db"),
    )
    .unwrap();
    db.execute_batch("CREATE TRIGGER refuse_lock_record BEFORE INSERT ON ledger BEGIN SELECT RAISE(ABORT,'fixture recording failure'); END;").unwrap();
    assert!(lock(&window, true).unwrap().locked);
    assert_ne!(native(&window).0 & 128, 0);
    let pending_entries = entries(&window);
    assert!(!pending_entries
        .iter()
        .any(|e| e["label"] == "Analysis Lock"));
    db.execute_batch("DROP TRIGGER refuse_lock_record").unwrap();
    let before = snapshot(window.location.master_db.parent().unwrap());
    let recovery = snapshot(window.state().backup_dir());
    assert_eq!(entries(&window), pending_entries);
    assert_eq!(
        snapshot(window.location.master_db.parent().unwrap()),
        before
    );
    assert_eq!(snapshot(window.state().backup_dir()), recovery);
    run(commands::set_track_rating(
        window.app.handle().clone(),
        window.state(),
        vec![track_id(0)],
        3,
    ))
    .unwrap();
    let all = entries(&window);
    let locks: Vec<_> = all
        .iter()
        .filter(|e| e["label"] == "Analysis Lock")
        .collect();
    assert_eq!(locks.len(), 1);
    assert_eq!(locks[0]["source"], json!({"kind":"dj","name":"dj"}));
    assert_eq!(locks[0]["payload"]["edits"][0]["after"], native(&window).0);
}

#[test]
fn local_cleanup_failure_cannot_lose_the_accepted_native_edit() {
    let window = prepared();
    let editor = window.app.state::<Arc<rbxport_lib::grid::GridEditor>>();
    editor.set_locked(&track_id(1), true).unwrap();
    let path = window.dir.path().join("grid/grid-locks.json");
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(lock(&window, true).is_err());
    assert_ne!(native(&window).0 & 128, 0);
    let all = entries(&window);
    assert_eq!(
        all.iter().filter(|e| e["label"] == "Analysis Lock").count(),
        1
    );
    assert_eq!(window.changes.lock().unwrap().len(), 1);
}

#[test]
fn lock_undo_redo_keeps_flags_and_ledger_links() {
    let window = prepared();
    let before = native(&window).0;
    assert!(lock(&window, true).unwrap().locked);
    run(commands::undo_edit(
        window.app.handle().clone(),
        window.state(),
    ))
    .unwrap();
    assert_eq!(native(&window).0, before);
    run(commands::redo_edit(
        window.app.handle().clone(),
        window.state(),
    ))
    .unwrap();
    assert_eq!(native(&window).0, (before & 127) | 128);
    let all = entries(&window);
    assert_eq!(all.len(), 3);
    assert_eq!(all[1]["action"], "undo");
    assert_eq!(all[1]["reverses"], all[0]["id"]);
    assert_eq!(all[2]["action"], "redo");
    assert_eq!(all[2]["reverses"], all[1]["id"]);
    assert_eq!(all[0]["payload"]["edits"][0]["before"], before);
}

#[test]
fn refresh_failure_keeps_the_native_lock_and_its_exact_record() {
    let window = prepared();
    let db = rbl_db::Library::open(window.location.clone(), rbl_db::OpenMode::ReadWrite).unwrap();
    db.connection()
        .execute(
            "UPDATE djmdContent SET rb_local_deleted=1 WHERE ID=?1",
            [track_id(1)],
        )
        .unwrap();
    let (index, _) = rbl_index::load(&db).unwrap();
    window.state().set_library(
        index,
        false,
        db.schema().db_version,
        0,
        window.location.clone(),
    );
    db.connection()
        .execute(
            "UPDATE djmdContent SET rb_local_deleted=0 WHERE ID=?1",
            [track_id(1)],
        )
        .unwrap();
    assert!(lock(&window, true).is_err());
    assert_ne!(native(&window).0 & 128, 0);
    let all = entries(&window);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0]["label"], "Analysis Lock");
    assert_eq!(all[0]["payload"]["edits"][0]["after"], native(&window).0);
}

#[test]
fn native_locks_in_bulk_work_keep_the_analysis_source_and_one_run() {
    use rbl_library::{
        bulk::{Kind, Operation},
        history::LibraryEdit,
    };
    let window = prepared();
    let operation = Operation::new(window.state().core(), Kind::Analysis);
    for on in (0..130).map(|i| i % 2 == 0) {
        operation.run(
            json!({"track":track_id(1)}),
            || window.state().write(|w| w.set_analysis_lock_with_undo(&track_id(1), on)),
            |edit| edit.iter().map(|edit| json!({
                "track":track_id(1), "analysis_lock":wreckordbox_service::ledger::stored::store(&LibraryEdit::Track(vec![edit.clone()]))
            })).collect(),
        ).unwrap();
    }
    assert!(operation.finish_wait().is_none());
    assert_eq!(native(&window).0 & 128, 0);
    let runs = window.cli(&["--json", "runs", "list"]).json();
    assert_eq!(runs["total"], 1);
    assert_eq!(
        runs["runs"][0]["source"],
        json!({"kind":"tool", "name":"wreckordbox-analysis"})
    );
    let run_id = runs["runs"][0]["id"].to_string();
    let read = window
        .cli(&["--json", "runs", "show", &run_id, "--limit", "200"])
        .json();
    assert_eq!(read["total"], 130);
    for entry in read["entries"].as_array().unwrap() {
        assert_eq!(entry["source"], runs["runs"][0]["source"]);
        assert_eq!(entry["store"], "library");
        assert_eq!(
            entry["payload"]["records"][0]["analysis_lock"]["edits"][0]["column"],
            "Analysed"
        );
    }
}
