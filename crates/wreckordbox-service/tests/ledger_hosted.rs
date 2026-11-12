//! The Ledger of a service a front end hosts on its own Library core: an
//! edit made through the core by the front end, and a command the front
//! end asks of the service, are the DJ's, in the window's Run, which ends
//! when the DJ pauses; a client's edit is its own Run, and the Ledger
//! stops recording the core when the service stops.
//!
//! One test, in a test binary of its own: the Sidecar's folder and the
//! pause come from the environment, which is the process's.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::sync::Arc;

use rbl_db::fixture::{self, Shape};
use rbl_library::history::{LibraryEdit, TRACK_EDIT};
use rbl_library::{LibraryCore, NoHost, Touched};
use serde_json::{json, Value};
use wreckordbox_service::ledger::{commands, RunArg, WINDOW_PAUSE_ENV, WINDOW_RUN};
use wreckordbox_service::stems::SIDECAR_DIR_ENV;
use wreckordbox_service::{Client, Config, Service};

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

#[test]
fn the_front_ends_edits_are_the_djs_in_the_windows_run_and_a_clients_are_its_own() {
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
    std::env::set_var(SIDECAR_DIR_ENV, dir.path().join("sidecar"));
    let config = Config {
        runtime_dir: dir.path().join("run"),
        idle_exit: None,
        recovery_dir: dir.path().join("recovery"),
    };
    let core = Arc::new(LibraryCore::new(config.recovery_dir.clone()));
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (index, _) = rbl_index::load(&db).unwrap();
    core.set_library(
        &NoHost,
        index,
        false,
        db.schema().db_version,
        0,
        location.clone(),
    );
    let host: Arc<dyn rbl_library::Host + Send + Sync> = Arc::new(NoHost);
    let (first, second, third) = (
        fixture::track_id(1),
        fixture::track_id(2),
        fixture::track_id(3),
    );

    // Before a service has the core, nothing records and nothing refuses.
    rate(&core, &first, 1);

    let service = Service::host(&location, &config, &core, &host).unwrap();
    let runs = || service.call(commands::RUNS_LIST, json!({})).unwrap();
    let entries = |run: &Value| -> Vec<Value> {
        service
            .call(commands::RUNS_SHOW, json!({ "id": run }))
            .unwrap()["entries"]
            .as_array()
            .unwrap()
            .clone()
    };
    assert_eq!(runs()["total"], 0);

    // The front end edits through the core, and asks the service for an
    // edit: both the DJ's, in one Run.
    rate(&core, &first, 2);
    service
        .call("tracks.rate", json!({ "id": second, "rating": 3 }))
        .unwrap();
    // A client's edit, with nothing said about who it is by.
    let mut client = Client::connect(&config.socket_path(&location)).unwrap();
    client.rate_track(&third, 4).unwrap();
    let clients = client.last_run().unwrap().clone();
    // And one where the client says it is the DJ's, in a Run it describes.
    client.in_run(Some(RunArg {
        description: Some("At the command line".to_owned()),
        source: Some(serde_json::from_value(json!({"kind": "dj"})).unwrap()),
        ..RunArg::default()
    }));
    client.rate_track(&third, 5).unwrap();
    assert_eq!(
        client.last_run().unwrap().description,
        "At the command line"
    );

    let listed = runs();
    assert_eq!(listed["total"], 3);
    let windows = &listed["runs"][2];
    assert_eq!(windows["description"], WINDOW_RUN);
    assert_eq!(windows["source"], json!({"kind": "dj", "name": "dj"}));
    assert!(windows["finished_at"].is_null());
    let made = entries(&windows["id"]);
    assert_eq!(made.len(), 2);
    assert!(made[0]["command"].is_null(), "an edit through the core");
    assert_eq!(made[0]["values"][0]["before"], 1);
    assert_eq!(made[0]["values"][0]["after"], 2);
    assert_eq!(made[1]["command"]["name"], "tracks.rate");
    assert!(made
        .iter()
        .all(|entry| entry["source"] == json!({"kind": "dj", "name": "dj"})));
    assert_eq!(listed["runs"][1]["id"], clients.id.0);
    assert_eq!(
        listed["runs"][1]["source"],
        json!({"kind": "tool", "name": "wreckordbox"})
    );
    assert_eq!(
        listed["runs"][0]["source"],
        json!({"kind": "dj", "name": "dj"})
    );

    // The service stops: the window's Run ends, and the core, which the
    // front end keeps, is no longer recorded or refused.
    service.stop();
    rate(&core, &first, 5);

    // With no pause allowed, each edit of the front end's is a Run.
    std::env::set_var(WINDOW_PAUSE_ENV, "0");
    let service = Service::host(&location, &config, &core, &host).unwrap();
    rate(&core, &first, 3);
    rate(&core, &first, 4);
    let listed = service.call(commands::RUNS_LIST, json!({})).unwrap();
    assert_eq!(listed["total"], 5);
    assert_eq!(listed["runs"][4]["outcome"], "wreckordbox stopped");
    assert_eq!(listed["runs"][4]["changes"], 2);
    assert_eq!(listed["runs"][1]["outcome"], "the DJ paused");
    assert!(listed["runs"][0]["finished_at"].is_null());
    assert_eq!(listed["runs"][0]["changes"], 1);
}
