//! The Intake view's reads (wbx-62): the window asks the Library service it
//! hosts for the Intake and Problems queues, and gets each queued track
//! back as the row its decks load, beside what the track still needs.
//!
//! A fixture Library in a temporary folder and a mock app in place of the
//! window; nothing here can reach the installed Library.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::sync::Arc;

use rbl_db::fixture::{self, track_id, Shape};
use rbxport_lib::intake_view::{self, IntakeQueueName};
use rbxport_lib::service_host;
use rbxport_lib::state::AppState;
use serde_json::json;
use tauri::test::MockRuntime;
use tauri::Manager;
use wreckordbox_service::Config;

struct Window {
    dir: tempfile::TempDir,
    app: tauri::App<MockRuntime>,
    location: rbl_db::LibraryLocation,
}

impl Window {
    fn state(&self) -> Arc<AppState> {
        Arc::clone(self.app.state::<Arc<AppState>>().inner())
    }

    /// Turns Library Protection on or off through the command the
    /// Preferences window calls.
    fn protect(&self, on: bool) {
        let state = self.app.state::<Arc<AppState>>();
        let now = tauri::async_runtime::block_on(service_host::set_library_protection(state, on))
            .unwrap();
        assert_eq!(now.protect_library, on);
    }

    fn config(&self) -> Config {
        Config {
            runtime_dir: self.dir.path().join("run"),
            idle_exit: None,
            recovery_dir: self.dir.path().join("backups"),
        }
    }
}

/// A fixture Library of five tracks, none with a Prep colour, read by a
/// mock app's state as the window reads it, and hosted.
fn hosting_window() -> Window {
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    let location = fixture::build(
        &library,
        Shape {
            tracks: 5,
            ..Shape::default()
        },
    )
    .unwrap();
    let state = AppState::with_backups(dir.path().join("backups"));
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (index, _) = rbl_index::load(&db).unwrap();
    state.set_library(index, false, db.schema().db_version, 0, location.clone());
    let app = tauri::test::mock_app();
    app.manage(Arc::new(state));
    let window = Window { dir, app, location };
    service_host::host_with(window.app.handle(), &window.location, &window.config()).unwrap();
    window
}

#[test]
fn the_intake_queue_lists_every_uncoloured_track_with_its_row_and_needs() {
    let window = hosting_window();

    let page = intake_view::queue_page(&window.state(), IntakeQueueName::Intake, 0, 50).unwrap();

    let queue = page.queue.expect("an Intake queue is always available");
    let tracks = queue["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 5);
    assert_eq!(queue["total"], 5);
    // Each queued track comes with the row a deck loads, in the same order.
    assert_eq!(page.rows.len(), 5);
    for (track, row) in tracks.iter().zip(&page.rows) {
        assert_eq!(track["id"].as_str().unwrap(), row.id);
        assert_eq!(track["title"].as_str().unwrap(), row.title);
        assert!(track["needs"].is_array(), "what it still needs: {track}");
    }
    assert!(page.unavailable.is_none());
}

#[test]
fn the_counts_follow_an_edit_made_through_the_service() {
    let window = hosting_window();
    let state = window.state();
    assert_eq!(intake_view::counts(&state).unwrap()["intake"], 5);
    let colour = || {
        state
            .with_service(|service| {
                service.call(
                    "tracks.set_prep_colour",
                    json!({ "id": track_id(1), "prep_colour": "Green" }),
                )
            })
            .unwrap()
    };

    // Library Protection on: the edit is refused and the queue is unchanged.
    window.protect(true);
    let refused = colour().unwrap_err();
    assert_eq!(refused.code, "library_protected");
    assert_eq!(intake_view::counts(&state).unwrap()["intake"], 5);

    window.protect(false);
    let set = colour().unwrap();
    assert_eq!(set["changed"], true);

    assert_eq!(intake_view::counts(&state).unwrap()["intake"], 4);
    let page = intake_view::queue_page(&state, IntakeQueueName::Intake, 0, 50).unwrap();
    assert!(
        page.rows.iter().all(|row| row.id != track_id(1)),
        "a coloured track left the queue"
    );
}

#[test]
fn problems_without_a_problem_colour_is_an_empty_state_not_an_error() {
    let window = hosting_window();

    let page = intake_view::queue_page(&window.state(), IntakeQueueName::Problems, 0, 50).unwrap();

    assert!(page.queue.is_none());
    assert!(page.rows.is_empty());
    let unavailable = page.unavailable.expect("why Problems cannot be listed");
    assert_eq!(unavailable.code, "problem_colour_not_found");
    assert!(!unavailable.message.is_empty());
    assert!(intake_view::counts(&window.state()).unwrap()["problems"].is_null());
}

#[test]
fn a_window_that_hosts_no_service_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(AppState::with_backups(dir.path().join("backups")));

    let error = intake_view::queue_page(&state, IntakeQueueName::Intake, 0, 50).unwrap_err();
    assert!(
        error.message.contains("Library service"),
        "{}",
        error.message
    );
    assert!(intake_view::counts(&state).is_err());
}

#[test]
fn the_window_offers_the_djs_eight_prep_colours_through_the_service() {
    let window = hosting_window();

    let colours = intake_view::colours(&window.state()).unwrap();

    assert_eq!(colours["colours"].as_array().unwrap().len(), 8);
    assert_eq!(colours["colours"][4]["colour"], "Green");
    // The fixture names no label Clean or Problem.
    assert!(colours["clean"].is_null());
    assert!(colours["problem"].is_null());
}

#[test]
fn a_prep_colour_set_from_the_window_is_one_undoable_edit_refused_under_protection() {
    let window = hosting_window();
    let state = window.state();
    let wanted = track_id(2);

    // Library Protection on: refused, the queue unchanged, nothing to undo.
    window.protect(true);
    let refused = intake_view::set_prep_colour(&state, &wanted, Some(5)).unwrap_err();
    assert!(refused.message.contains("Library Protection"), "{}", refused.message);
    assert_eq!(intake_view::counts(&state).unwrap()["intake"], 5);
    window.protect(false);
    assert!(tauri::async_runtime::block_on(rbxport_lib::commands::undo_edit(
        window.app.handle().clone(),
        window.app.state::<Arc<AppState>>()
    ))
    .is_err());

    let set = intake_view::set_prep_colour(&state, &wanted, Some(5)).unwrap();
    assert_eq!(set["prep_colour"]["colour"], "Green");
    assert_eq!(intake_view::counts(&state).unwrap()["intake"], 4);

    // One Undo in the window takes it back: the track waits in Intake again.
    tauri::async_runtime::block_on(rbxport_lib::commands::undo_edit(
        window.app.handle().clone(),
        window.app.state::<Arc<AppState>>(),
    ))
    .unwrap();
    assert_eq!(intake_view::counts(&state).unwrap()["intake"], 5);
    let page = intake_view::queue_page(&state, IntakeQueueName::Intake, 0, 50).unwrap();
    assert!(page.rows.iter().any(|row| row.id == wanted));

    // Clearing: `None` takes the colour off.
    intake_view::set_prep_colour(&state, &wanted, Some(2)).unwrap();
    let cleared = intake_view::set_prep_colour(&state, &wanted, None).unwrap();
    assert!(cleared["prep_colour"].is_null());
}
