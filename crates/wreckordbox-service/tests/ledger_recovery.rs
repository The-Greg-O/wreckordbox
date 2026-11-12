//! The Ledger of a service a front end hosts, when an entry cannot be
//! written at once and when the service stops in the middle of an edit:
//! an Undo and a Redo still name exactly the change they step, and an
//! edit the window is making as the service stops is recorded.
//!
//! One test, in a test binary of its own: the Sidecar's folder and the
//! injected failure come from the environment, which is the process's.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::sync::{Arc, Barrier};
use std::time::Duration;

use rbl_db::fixture::{self, Shape};
use rbl_library::history::{LibraryEdit, TRACK_EDIT};
use rbl_library::{History, LibraryCore, NoHost, Touched};
use serde_json::{json, Value};
use wreckordbox_service::ledger::{commands, FAULTS_ENV};
use wreckordbox_service::stems::SIDECAR_DIR_ENV;
use wreckordbox_service::{Config, Service};

fn rate(core: &LibraryCore, track: &str, stars: u8) {
    core.recorded_edit(
        &NoHost,
        Touched::Metadata(vec![track.to_owned()]),
        TRACK_EDIT,
        |writer| {
            let (_, recorded) = writer.set_rating_with_undo(track, stars)?;
            Ok(((), LibraryEdit::Track(vec![recorded])))
        },
    )
    .unwrap();
}

fn rating_of(core: &LibraryCore, track: &str) -> u8 {
    let library = core.library().unwrap();
    library.rating[library.row_of(track).unwrap() as usize]
}

#[test]
fn undo_and_redo_name_the_change_they_step_across_a_kept_entry_and_a_stop_waits_for_the_windows_edit()
{
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    let location = fixture::build(
        &library,
        Shape {
            tracks: 4,
            ..Shape::default()
        },
    )
    .unwrap();
    let sidecar_dir = dir.path().join("sidecar");
    std::env::set_var(SIDECAR_DIR_ENV, &sidecar_dir);
    std::env::set_var("RBXPORT_TEST", "1");
    // The first entry is refused by the Sidecar.
    std::env::set_var(FAULTS_ENV, "fail_append=1");
    let config = Config {
        runtime_dir: dir.path().join("run"),
        idle_exit: None,
        recovery_dir: dir.path().join("recovery"),
    };
    let core = Arc::new(LibraryCore::new(config.recovery_dir.clone()));
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (index, _) = rbl_index::load(&db).unwrap();
    core.set_library(&NoHost, index, false, db.schema().db_version, 0, location.clone());
    let host: Arc<dyn rbl_library::Host + Send + Sync> = Arc::new(NoHost);
    let track = fixture::track_id(1);

    let service = Service::host(&location, &config, &core, &host).unwrap();
    rate(&core, &track, 4);
    assert!(sidecar_dir.join("ledger-spill.jsonl").is_file(), "the entry was kept");
    core.step_history(&NoHost, true).unwrap();
    core.step_history(&NoHost, false).unwrap();
    assert_eq!(rating_of(&core, &track), 4);

    let listed = service.call(commands::RUNS_LIST, json!({})).unwrap();
    let run = &listed["runs"][0]["id"];
    let entries: Vec<Value> = service
        .call(commands::RUNS_SHOW, json!({ "id": run }))
        .unwrap()["entries"]
        .as_array()
        .unwrap()
        .clone();
    let steps: Vec<(&str, Option<i64>)> = entries
        .iter()
        .map(|entry| (entry["action"].as_str().unwrap(), entry["reverses"].as_i64()))
        .collect();
    let ids: Vec<i64> = entries.iter().map(|entry| entry["id"].as_i64().unwrap()).collect();
    assert_eq!(
        steps,
        vec![("edit", None), ("undo", Some(ids[0])), ("redo", Some(ids[1]))],
        "{entries:#?}"
    );
    assert_eq!(entries[0]["values"][0]["after"], 4);
    assert!(!sidecar_dir.join("ledger-spill.jsonl").exists());

    // The window is part way through an edit, holding the Library's edit
    // gate, when the service is told to stop.
    let started = Arc::new(Barrier::new(2));
    let editing = {
        let (core, started, track) = (Arc::clone(&core), Arc::clone(&started), track.clone());
        std::thread::spawn(move || {
            let _gate = core.edit_gate().lock();
            started.wait();
            let recorded = core
                .write_then(
                    &NoHost,
                    History::Caller,
                    |writer| writer.set_rating_with_undo(&track, 2).map(|(_, recorded)| recorded),
                    |_, recorded| Ok(recorded),
                )
                .unwrap();
            std::thread::sleep(Duration::from_millis(400));
            core.history()
                .lock()
                .record(LibraryEdit::Track(vec![recorded]), TRACK_EDIT);
        })
    };
    started.wait();
    std::thread::sleep(Duration::from_millis(100));
    service.stop();
    editing.join().unwrap();

    // Read with nothing running: the edit is there, before the window's
    // Run ended.
    let sidecar = wreckordbox_sidecar::Sidecar::open(&sidecar_dir).unwrap();
    let (runs, _) = sidecar.runs(10, 0).unwrap();
    let window = &runs[0].run;
    assert_eq!(window.outcome.as_deref(), Some("wreckordbox stopped"));
    let (entries, _) = sidecar.run_entries(window.id, 50, 0).unwrap();
    let last = entries.last().unwrap();
    assert_eq!(last.action, wreckordbox_sidecar::ledger::Action::Edit);
    assert_eq!(last.payload.as_ref().unwrap()["edits"][0]["after"], 2);
    assert!(sidecar.open_intents().unwrap().is_empty());
}
