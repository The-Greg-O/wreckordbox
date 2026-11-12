//! The Intake tag panel in a window hosting the Library service (wbx-65):
//! the window reads the panel's description from the service, and every
//! click goes through the service's `intake.set_tags` as one entry of the
//! history the window shares with it: the window's Undo takes back the
//! DJ's last action whichever store it touched (a MyTag, or the DJ's Fact
//! of a dimension that does not reach the deck), and Library Protection
//! refuses every click.
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
    let (refused, _) = window.click(&track, &situation, &[MY_TAG_PEAK]).unwrap_err();
    assert!(refused.contains("Library Protection"), "{refused}");
    assert_eq!(window.set(&track)[&situation], json!([]));
    window.protect(false);

    window.click(&track, &situation, &[MY_TAG_PEAK]).unwrap();
    assert_eq!(window.set(&track)[&situation], json!([MY_TAG_PEAK]));
    // The window's own MyTag edit still works beside it.
    window.put(&track, rbl_db::fixture::MY_TAG_WARM_UP, true).unwrap();

    // Undo, Undo: the window's tick, then the panel's.
    window.undo();
    assert_eq!(window.set(&track)[&situation], json!([MY_TAG_PEAK]));
    window.undo();
    assert_eq!(window.set(&track)[&situation], json!([]));
}

const SCHEMA: &str = r#"
version = 1

[genre]
terms = [
  { id = "house", name = "House" },
  { id = "house-deep", name = "Deep", parent = "house" },
  { id = "techno", name = "Techno" },
]

[dimensions.situation]
terms = [
  { id = "peak", name = "Peak" },
  { id = "warm-up", name = "Warm-up" },
]

[dimensions.vocals]
terms = [{ id = "male", name = "Male" }, { id = "female", name = "Female" }]

[deck]
categories = [{ dimension = "situation", label = "Situation" }]
"#;

impl Window {
    /// The panel's click: `dimension` set to `terms` from the panel as shown.
    fn click(&self, track: &str, dimension: &str, terms: &[&str]) -> Result<Value, (String, String)> {
        let panel = intake_tags::tag_panel(&self.state(), Some(track)).unwrap();
        let version = panel["track"]["versions"][dimension].as_str().unwrap().to_owned();
        let terms: Vec<String> = terms.iter().map(|t| (*t).to_owned()).collect();
        intake_tags::set_tags(&self.state(), track, dimension, &terms, &version, &[], false)
            .map_err(|e| (e.message, e.detail.unwrap_or_default()))
    }

    fn redo(&self) {
        tauri::async_runtime::block_on(rbxport_lib::commands::redo_edit(
            self.app.handle().clone(),
            self.app.state::<Arc<AppState>>(),
        ))
        .unwrap();
    }

    fn rate(&self, track: &str, stars: u8) {
        tauri::async_runtime::block_on(rbxport_lib::commands::set_track_rating(
            self.app.handle().clone(),
            self.app.state::<Arc<AppState>>(),
            vec![track.to_owned()],
            stars,
        ))
        .unwrap();
    }

    fn energy(&self, track: &str) -> Value {
        let page = self
            .state()
            .with_service(|service| service.call("tracks.list", json!({ "limit": 50 })))
            .unwrap()
            .unwrap();
        page["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == track)
            .unwrap()["rating"]
            .clone()
    }

    /// The DJ's live Fact of `kind` on `track`, with its source and Run.
    fn djs(&self, track: &str, kind: &str) -> Option<Value> {
        let facts = self
            .state()
            .with_service(|service| service.call("facts.of_track", json!({ "id": track })))
            .unwrap()
            .unwrap();
        facts["facts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|fact| fact["kind"] == kind && fact["source"]["kind"] == "dj")
            .max_by_key(|fact| fact["id"].as_i64().unwrap())
            .cloned()
    }
}

#[test]
fn undo_takes_back_the_djs_last_action_whichever_store_it_touched() {
    let window = hosting_window(Some(SCHEMA));
    let track = track_id(0);
    let energy_before = window.energy(&track);
    assert_eq!(energy_before, json!(0));

    // Energy (a Library edit), then a tag kept as a Fact.
    window.rate(&track, 4);
    assert_eq!(window.energy(&track), json!(4), "rated");
    window.click(&track, "genre", &["house-deep"]).unwrap();
    assert_eq!(window.energy(&track), json!(4), "after the click");
    assert_eq!(window.djs(&track, "genre").unwrap()["value"], json!(["house-deep"]));

    // Undo: the tag click, not the Energy.
    window.undo();
    assert!(window.djs(&track, "genre").is_none(), "the Fact is withdrawn");
    assert_eq!(window.set(&track)["genre"], json!([]));
    assert_eq!(window.energy(&track), json!(4));

    // A second Undo: the Energy.
    window.undo();
    assert_eq!(window.energy(&track), energy_before);

    // Redo, Redo: Energy, then the tag again, as the DJ's.
    window.redo();
    assert_eq!(window.energy(&track), json!(4));
    window.redo();
    let again = window.djs(&track, "genre").unwrap();
    assert_eq!(again["value"], json!(["house-deep"]));
    assert_eq!(again["source"]["kind"], "dj");
}

#[test]
fn undo_of_a_fact_click_reinstates_the_answer_that_stood_before_with_its_source() {
    let window = hosting_window(Some(SCHEMA));
    let track = track_id(1);
    let before = window
        .state()
        .with_service(|service| {
            service.call(
                "facts.record",
                json!({ "facts": [{ "track": track, "kind": "vocals", "value": "male" }], "source": { "kind": "dj" } }),
            )
        })
        .unwrap()
        .unwrap()["facts"][0]["id"]
        .clone();

    window.click(&track, "vocals", &["female"]).unwrap();
    assert_eq!(window.djs(&track, "vocals").unwrap()["value"], "female");
    window.undo();

    let stood = window.djs(&track, "vocals").unwrap();
    assert_eq!(stood["id"], before, "the same Fact, not a copy");
    assert_eq!(stood["value"], "male");
}

#[test]
fn a_click_is_refused_under_library_protection_and_from_a_stale_panel() {
    let window = hosting_window(Some(SCHEMA));
    let track = track_id(2);

    window.protect(true);
    let (message, code) = window.click(&track, "genre", &["techno"]).unwrap_err();
    assert_eq!(code, "library_protected", "{message}");
    window.protect(false);
    assert!(window.djs(&track, "genre").is_none());

    let panel = intake_tags::tag_panel(&window.state(), Some(&track)).unwrap();
    let old = panel["track"]["versions"]["genre"].as_str().unwrap().to_owned();
    window.click(&track, "genre", &["techno"]).unwrap();
    let error = intake_tags::set_tags(&window.state(), &track, "genre", &["house".to_owned()], &old, &[], false)
        .unwrap_err();
    assert_eq!(error.detail.as_deref(), Some("tag_panel_stale"));
    assert_eq!(window.djs(&track, "genre").unwrap()["value"], json!(["techno"]));
}

#[test]
fn a_tag_click_tells_every_client_of_the_service() {
    let window = hosting_window(Some(SCHEMA));
    let events = window.state().with_service(|service| service.events()).unwrap();

    window.click(&track_id(0), "genre", &["techno"]).unwrap();

    let heard: Vec<String> = std::iter::from_fn(|| events.recv_timeout(std::time::Duration::from_secs(2)).ok())
        .map(|event| event.event)
        .take(2)
        .collect();
    assert!(heard.contains(&"sidecar_changed".to_owned()), "{heard:?}");
}
