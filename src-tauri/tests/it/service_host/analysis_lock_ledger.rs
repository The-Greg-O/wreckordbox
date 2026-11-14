use super::*;
use serde_json::json;

fn prepared() -> Window {
    let window = window();
    let relative = "/PIONEER/USBANLZ/lock-ledger/ANLZ0000.DAT";
    fixture::set_analysis_path(&window.location, 1, relative).unwrap();
    let dat = rbl_anlz::resolve(&window.location.share_root, relative);
    std::fs::create_dir_all(dat.parent().unwrap()).unwrap();
    let mut builder = rbl_anlz::AnlzBuilder::new();
    builder.path("/fixture.wav").beat_grid(&[rbl_anlz::Beat { time_ms: 0, beat_number: 1, tempo_x100: 12000 }, rbl_anlz::Beat { time_ms: 500, beat_number: 2, tempo_x100: 12000 }]);
    std::fs::write(dat, builder.finish()).unwrap();
    window.app.manage(Arc::new(rbxport_lib::grid::GridEditor::at(&window.dir.path().join("grid"))));
    window.load();
    window.host().unwrap();
    window
}
fn lock(window: &Window, on: bool) -> Result<rbxport_lib::grid::GridStateDto, rbxport_lib::AppError> {
    run(rbxport_lib::grid::grid_lock(window.app.handle().clone(), window.state(), window.app.state(), track_id(1), on))
}
fn native(window: &Window) -> (i64, i64) {
    let db = rbl_db::Library::open(window.location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    db.connection().query_row("SELECT COALESCE(Analysed,0), COALESCE(rb_local_usn,0) FROM djmdContent WHERE ID=?1", [track_id(1)], |r| Ok((r.get(0)?, r.get(1)?))).unwrap()
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
    assert_eq!(entries[0]["payload"]["edits"][0]["after"], (before & 127) | 128);
    assert_eq!(client.next_event(Duration::from_secs(2)).unwrap().unwrap().event, "library_changed");
}
#[test]
fn unchanged_lock_and_protected_lock_do_not_write_or_record() {
    let window = prepared();
    window.protect(true);
    let bytes = std::fs::read(&window.location.master_db).unwrap();
    let before = native(&window);
    assert_eq!(lock(&window, true).unwrap_err().message, rbl_library::protection::MESSAGE);
    assert_eq!(native(&window), before);
    assert_eq!(std::fs::read(&window.location.master_db).unwrap(), bytes);
    assert_eq!(window.cli(&["--json", "runs", "list"]).json()["total"], 0);
    window.protect(false);
    assert!(lock(&window, true).unwrap().locked);
    let before = native(&window);
    let runs = window.cli(&["--json", "runs", "list"]).json();
    assert!(lock(&window, true).unwrap().locked);
    assert_eq!(native(&window), before, "an unchanged lock must not bump the row revision");
    assert_eq!(window.cli(&["--json", "runs", "list"]).json(), runs);
}
