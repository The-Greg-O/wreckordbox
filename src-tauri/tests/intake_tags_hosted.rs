//! The Intake tag panel in a window hosting the Library service (wbx-65):
//! the window reads the panel's description from the service, toggles a
//! deck-facing tag with its existing MyTag edit (one undoable step, refused
//! under Library Protection), and sets a vocabulary dimension as the DJ's
//! Fact.
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

use rbl_db::fixture::{self, track_id, Shape, MY_TAG_PEAK};
use rbxport_lib::state::AppState;
use rbxport_lib::{details, intake_tags, service_host};
use serde_json::{json, Value};
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

    fn protect(&self, on: bool) {
        let state = self.app.state::<Arc<AppState>>();
        tauri::async_runtime::block_on(service_host::set_library_protection(state, on)).unwrap();
    }

    fn sidecar_dir(&self) -> std::path::PathBuf {
        self.location
            .master_db
            .parent()
            .unwrap()
            .join("wreckordbox-sidecar")
    }

    /// The window's own My Tag tick, as the panel sends it.
    fn put(&self, track: &str, tag: &str, on: bool) -> Result<(), String> {
        tauri::async_runtime::block_on(details::put_my_tag(
            self.app.handle().clone(),
            self.app.state::<Arc<AppState>>(),
            vec![track.to_owned()],
            tag.to_owned(),
            on,
            None,
        ))
        .map(|_| ())
        .map_err(|e| e.message)
    }

    fn undo(&self) {
        tauri::async_runtime::block_on(rbxport_lib::commands::undo_edit(
            self.app.handle().clone(),
            self.app.state::<Arc<AppState>>(),
        ))
        .unwrap();
    }

    fn set(&self, track: &str) -> Value {
        intake_tags::tag_panel(&self.state(), Some(track)).unwrap()["track"]["set"].clone()
    }
}

fn hosting_window(schema: Option<&str>) -> Window {
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    let location = fixture::build(
        &library,
        Shape {
            tracks: 3,
            ..Shape::default()
        },
    )
    .unwrap();
    if let Some(text) = schema {
        let sidecar = location
            .master_db
            .parent()
            .unwrap()
            .join("wreckordbox-sidecar");
        std::fs::create_dir_all(&sidecar).unwrap();
        std::fs::write(sidecar.join("tag-schema.toml"), text).unwrap();
    }
    let state = AppState::with_backups(dir.path().join("backups"));
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (index, _) = rbl_index::load(&db).unwrap();
    state.set_library(index, false, db.schema().db_version, 0, location.clone());
    let app = tauri::test::mock_app();
    app.manage(Arc::new(state));
    let window = Window { dir, app, location };
    let config = Config {
        runtime_dir: window.dir.path().join("run"),
        idle_exit: None,
        recovery_dir: window.dir.path().join("backups"),
    };
    service_host::host_with(window.app.handle(), &window.location, &config).unwrap();
    // As on the machine of a DJ who edits.
    window.protect(false);
    window
}

#[test]
fn a_deck_tag_toggled_from_the_panel_is_one_undoable_mytag_edit_refused_under_protection() {
    let window = hosting_window(None);
    let track = track_id(1);
    let panel = intake_tags::tag_panel(&window.state(), Some(&track)).unwrap();
    let situation = panel["dimensions"][0]["id"].as_str().unwrap().to_owned();
    assert_eq!(panel["dimensions"][0]["label"], "Situation");
    assert_eq!(panel["dimensions"][0]["deck"], true);
    assert_eq!(window.set(&track)[&situation], json!([]));

    // Library Protection on: refused, and the track carries nothing.
    window.protect(true);
    let refused = window.put(&track, MY_TAG_PEAK, true).unwrap_err();
    assert!(refused.contains("Library Protection"), "{refused}");
    assert_eq!(window.set(&track)[&situation], json!([]));
    window.protect(false);

    window.put(&track, MY_TAG_PEAK, true).unwrap();
    assert_eq!(window.set(&track)[&situation], json!([MY_TAG_PEAK]));

    // One Undo takes the tick back.
    window.undo();
    assert_eq!(window.set(&track)[&situation], json!([]));
}

const SCHEMA: &str = r#"
version = 1

[genre]
terms = [
  { id = "house", name = "House" },
  { id = "house-deep", name = "Deep", parent = "house" },
]

[dimensions.situation]
terms = [
  { id = "peak", name = "Peak" },
  { id = "warm-up", name = "Warm-up" },
]

[deck]
categories = [{ dimension = "situation", label = "Situation" }]
"#;

#[test]
fn a_vocabulary_tag_is_set_and_cleared_as_the_djs_fact() {
    let window = hosting_window(Some(SCHEMA));
    let track = track_id(0);
    let panel = intake_tags::tag_panel(&window.state(), Some(&track)).unwrap();
    assert_eq!(panel["source"], "tag_schema");
    assert_eq!(panel["dimensions"][1]["id"], "genre");
    assert_eq!(panel["dimensions"][1]["edit"], "fact");

    intake_tags::set_tag_fact(&window.state(), &track, "genre", &["house-deep".to_owned()])
        .unwrap();
    assert_eq!(window.set(&track)["genre"], json!(["house-deep"]));

    intake_tags::set_tag_fact(&window.state(), &track, "genre", &[]).unwrap();
    assert_eq!(window.set(&track)["genre"], json!([]));

    // A term the schema does not have is refused, and nothing changes.
    let refused =
        intake_tags::set_tag_fact(&window.state(), &track, "genre", &["polka".to_owned()])
            .unwrap_err();
    assert!(!refused.message.is_empty());
    assert_eq!(window.set(&track)["genre"], json!([]));
    assert!(window.sidecar_dir().exists());
}

#[test]
fn a_deck_tag_ticked_from_the_panel_is_recorded_as_the_djs_fact() {
    let window = hosting_window(Some(SCHEMA));
    let track = track_id(2);

    window.put(&track, MY_TAG_PEAK, true).unwrap();

    assert_eq!(window.set(&track)["situation"], json!(["peak"]));
    let facts = window
        .state()
        .with_service(|service| service.call("facts.of_track", json!({ "id": track })))
        .unwrap()
        .unwrap();
    let situation: Vec<&Value> = facts["facts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|fact| fact["kind"] == "situation")
        .collect();
    assert_eq!(situation.len(), 1, "{facts}");
    assert_eq!(situation[0]["value"], json!(["peak"]));
    assert_eq!(situation[0]["source"]["kind"], "dj");
}
