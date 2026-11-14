//! Closing the window latches shutdown (wbx-45.14, from the wbx-71
//! review): once it is closing no new core write starts, an edit queued
//! behind another is refused, and the window cannot host the service again.
//! The race is held at the moment the window has closed and not yet let
//! the service go (`test_holds::SHUTDOWN_CLOSED`); no sleeps.

use super::*;

use rbxport_lib::test_holds;

fn rate(window: &Window, id: &str, stars: u8) -> Result<(), rbxport_lib::AppError> {
    run(commands::set_track_rating(
        window.app.handle().clone(),
        window.state(),
        vec![id.to_owned()],
        stars,
    ))
    .map(|_| ())
}

#[test]
fn an_edit_queued_while_the_window_closes_is_refused_and_writes_nothing() {
    let window = window();
    window.host().unwrap();
    let id = track_id(1);
    let state = Arc::clone(window.state().inner());

    // An edit under way holds the edit gate; another is queued behind it.
    let gate = state.core().edit_gate().lock();
    let held = test_holds::arm(test_holds::SHUTDOWN_CLOSED, state.backup_dir());
    let stopping = {
        let state = Arc::clone(&state);
        // The final `Exit` of the app's run loop.
        std::thread::spawn(move || rbxport_lib::shutdown::on_exit(&state))
    };
    held.reached();
    let queued = {
        let handle = window.app.handle().clone();
        let id = id.clone();
        std::thread::spawn(move || {
            run(commands::set_track_rating(
                handle.clone(),
                handle.state(),
                vec![id],
                5,
            ))
            .map(|_| ())
        })
    };
    held.release();
    drop(gate);

    stopping.join().unwrap();
    let refused = queued
        .join()
        .unwrap()
        .expect_err("refused: the window is closing");
    assert_eq!(
        refused.message,
        rbxport_lib::shutdown::CLOSING,
        "{refused:?}"
    );
    assert_eq!(window.rating(&id), 0, "nothing was written");
    let db = rbl_db::Library::open(window.location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (index, _) = rbl_index::load(&db).unwrap();
    assert_eq!(
        index.rating[index.row_of(&id).unwrap() as usize],
        0,
        "nor in the Library on disk"
    );
    assert!(
        window.stopped.lock().unwrap().is_empty(),
        "closing is not reported as a failure"
    );
}

#[test]
fn a_closed_window_never_hosts_the_service_again_and_stays_unwritable() {
    let window = window();
    window.host().unwrap();
    let id = track_id(1);

    rbxport_lib::shutdown::on_exit(window.state().inner());
    let again = window.host().expect_err("not hosted again once closing");
    assert_eq!(again.code, wreckordbox_service::codes::SERVICE_NOT_RUNNING);
    assert!(window.state().service_caller().is_none());
    let refused = rate(&window, &id, 4).expect_err("still refused");
    assert_eq!(refused.message, rbxport_lib::shutdown::CLOSING);
    assert_eq!(window.rating(&id), 0);
    // A command run next starts a service of its own: the window let go.
    let status = window.cli(&["--json", "status"]).json();
    assert_ne!(status["pid"], std::process::id());
    let _ = Command::new("kill").arg(status["pid"].to_string()).status();
}

/// A Stick beside the window's Library, unlocked by the DJ, and a confirmed
/// token for exporting the first playlist to it.
fn unlocked_stick(window: &Window) -> (std::path::PathBuf, Option<String>) {
    let stick = window.dir.path().join("sticks/GIGSTICK");
    std::fs::create_dir_all(&stick).unwrap();
    std::fs::write(
        stick.join(wreckordbox_stick_protection::STAND_IN_MARKER),
        b"",
    )
    .unwrap();
    let state_dir = window.state().backup_dir().to_path_buf();
    wreckordbox_stick_protection::unlock(&state_dir, &stick, Some("GIGSTICK")).unwrap();
    let request = serde_json::from_value(serde_json::json!({
        "destination": stick.to_string_lossy(),
        "operation": { "kind": "exportPlaylist", "playlist": fixture::playlist_id(0) },
    }))
    .unwrap();
    let plan = rbxport_lib::stick_write_plan::plan(window.state().inner(), &request).unwrap();
    let token = plan
        .plan
        .map(|id| rbxport_lib::stick_approval::approve(&state_dir, &id).unwrap());
    assert!(token.is_some());
    (stick, token)
}

fn export(
    window: &Window,
    stick: &std::path::Path,
    token: Option<String>,
) -> std::thread::JoinHandle<Result<(), String>> {
    let (handle, path) = (
        window.app.handle().clone(),
        stick.to_string_lossy().into_owned(),
    );
    std::thread::spawn(move || {
        run(commands::export_playlist(
            handle.clone(),
            handle.state(),
            fixture::playlist_id(0),
            path,
            None,
            None,
            None,
            token,
        ))
        .map(|_| ())
        .map_err(|e| e.message)
    })
}

fn files(root: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                todo.push(path);
            } else {
                out.push((path.clone(), std::fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// wbx-45.14 round 1 (d): a confirmed export queued behind the Library
/// while the window closes is refused at the latch and writes nothing.
#[test]
fn an_export_queued_while_the_window_closes_is_refused_and_writes_nothing() {
    let window = window();
    window.host().unwrap();
    let (stick, token) = unlocked_stick(&window);
    let before = files(&stick);
    let state = Arc::clone(window.state().inner());

    let gate = state.core().edit_gate().lock();
    let exporting = export(&window, &stick, token);
    let held = test_holds::arm(test_holds::SHUTDOWN_CLOSED, state.backup_dir());
    let closing = {
        let state = Arc::clone(&state);
        std::thread::spawn(move || rbxport_lib::shutdown::on_exit(&state))
    };
    held.reached();
    held.release();
    drop(gate);

    let refused = exporting
        .join()
        .unwrap()
        .expect_err("refused: the window is closing");
    closing.join().unwrap();
    assert_eq!(refused, rbxport_lib::shutdown::CLOSING);
    assert_eq!(files(&stick), before, "nothing was written to the Stick");
}

/// wbx-45.14 round 1 (d): an export admitted just before the window closes
/// is waited for: closing does not finish until its publication has.
#[test]
fn an_export_admitted_before_the_window_closes_is_drained_before_exit() {
    let window = window();
    window.host().unwrap();
    let (stick, token) = unlocked_stick(&window);
    let state = Arc::clone(window.state().inner());

    let writing = test_holds::arm(test_holds::EXPORT_WRITING, state.backup_dir());
    let exporting = export(&window, &stick, token);
    writing.reached();
    let awaiting = test_holds::arm(test_holds::SHUTDOWN_AWAITING_WRITERS, state.backup_dir());
    let closing = {
        let state = Arc::clone(&state);
        std::thread::spawn(move || rbxport_lib::shutdown::on_exit(&state))
    };
    // Closing has latched and is waiting for the export it admitted.
    awaiting.reached();
    awaiting.release();
    writing.release();

    closing.join().unwrap();
    // Closing is over: the export had finished, published in full.
    assert!(
        !stick.join(".rbxport-publication").exists(),
        "no unfinished write left"
    );
    let library = wreckordbox_stick_outline::library(
        &stick,
        rbl_export::device_library::Format::DeviceLibrary,
    )
    .unwrap();
    assert!(!library.nodes.is_empty(), "the playlist is on the Stick");
    assert_eq!(exporting.join().unwrap(), Ok(()));
}
