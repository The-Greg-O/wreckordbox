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
