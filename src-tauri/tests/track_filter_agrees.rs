//! The Track Filter bar's one filter, sent both ways (wbx-57): the window's
//! list (its `ViewSpec`, opened as `open_view` opens it) and the Library
//! service's `tracks.query` (the CLI's `tracks query`), for every case in
//! `src/lib/shellTrackFilter.cases.json`, whose window and service forms the
//! front end's unit tests prove are what the model sends. Both must narrow a
//! fixture Library to the same tracks in the same order.
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
use rbl_db::write::{TrackField, Writer};
use rbxport_lib::dto::ViewSpecDto;
use rbxport_lib::service_host;
use rbxport_lib::state::{spec_from_wire, AppState};
use serde_json::{json, Value};
use tauri::test::MockRuntime;
use tauri::Manager;
use wreckordbox_service::Config;

const TRACKS: usize = 30;
const KEYS: [&str; 5] = ["8A", "9A", "11B", "8A", "Am"];

struct Window {
    _dir: tempfile::TempDir,
    app: tauri::App<MockRuntime>,
    playlist: String,
}

/// Thirty tracks with every field the bar filters on spread across them,
/// a playlist of tracks 3 to 20, read by a mock app as the window reads it,
/// and hosted.
fn hosting_window() -> Window {
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    let location = fixture::build(
        &library,
        Shape {
            tracks: TRACKS,
            ..Shape::default()
        },
    )
    .unwrap();

    let mut writer = Writer::open(location.clone(), dir.path().join("backups")).unwrap();
    writer.disable_automatic_backups();
    let mut tag = |name: &str| {
        let made = writer.create_my_tag(fixture::MY_TAG_CATEGORY).unwrap();
        writer.rename_my_tag(&made.id, name).unwrap();
        made.id
    };
    let tension = tag("Tension builder");
    let peak = tag("Peak time");
    for name in KEYS {
        writer.ensure_detected_key(name).unwrap();
    }
    for index in 0..TRACKS {
        let id = track_id(index);
        // 120.00, 120.25, 120.50 ...: every quarter, so the half-BPM edges
        // of the whole-BPM buckets are crossed.
        writer
            .set_bpm_x100(&id, 12_000 + 25 * index as u32)
            .unwrap();
        writer
            .set_field(&id, TrackField::Key, KEYS[index % KEYS.len()])
            .unwrap();
        // Not `index % 6`: that ties Energy to the tags set below.
        writer.set_rating(&id, (index / 2 % 6) as u8).unwrap();
        if index % 9 != 0 {
            writer
                .set_color(&id, Some(&(index % 9).to_string()))
                .unwrap();
        }
        let mut tags = Vec::new();
        if index % 2 == 0 {
            tags.push(peak.clone());
        }
        if index % 3 == 0 {
            tags.push(tension.clone());
        }
        if index % 5 == 0 {
            tags.push(fixture::MY_TAG_WARM_UP.to_owned());
        }
        writer.set_my_tags(&id, &tags).unwrap();
    }
    let playlist = writer.create_playlist("Bar test", "root").unwrap();
    let members: Vec<String> = (3..=20).map(track_id).collect();
    writer.add_tracks(&playlist, &members).unwrap();
    drop(writer);

    let state = AppState::with_backups(dir.path().join("backups"));
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (index, _) = rbl_index::load(&db).unwrap();
    state.set_library(index, false, db.schema().db_version, 0, location.clone());
    let app = tauri::test::mock_app();
    app.manage(Arc::new(state));
    let config = Config {
        runtime_dir: dir.path().join("run"),
        idle_exit: None,
        recovery_dir: dir.path().join("backups"),
    };
    service_host::host_with(app.handle(), &location, &config).unwrap();
    Window {
        _dir: dir,
        app,
        playlist,
    }
}

impl Window {
    fn state(&self) -> Arc<AppState> {
        Arc::clone(self.app.state::<Arc<AppState>>().inner())
    }

    /// The track ids the window's list shows for this `ViewSpec`.
    fn listed(&self, spec: Value) -> Vec<String> {
        let dto: ViewSpecDto = serde_json::from_value(spec).unwrap();
        let library = self.state().library().unwrap();
        let view = library.open_view(&spec_from_wire(&library, &dto));
        view.rows
            .iter()
            .map(|&row| library.ids[row as usize].to_string())
            .collect()
    }

    /// The track ids `tracks.query` answers for these arguments.
    fn queried(&self, mut args: Value) -> Vec<String> {
        args["limit"] = json!(1000);
        let page = self
            .state()
            .with_service(|service| service.call("tracks.query", args))
            .unwrap()
            .unwrap();
        page["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|track| track["id"].as_str().unwrap().to_owned())
            .collect()
    }
}

/// The cases file writes a long run of whole BPMs as `"lo..hi"`.
fn expand_bpm(window: &mut Value) {
    // Not by indexing: indexing a missing key would add it as null.
    let Some(values) = window.pointer_mut("/filter/bpm/values") else {
        return;
    };
    if let Some(text) = values.as_str() {
        let (lo, hi) = text.split_once("..").unwrap();
        let (lo, hi): (u32, u32) = (lo.parse().unwrap(), hi.parse().unwrap());
        *values = json!((lo..=hi).collect::<Vec<_>>());
    }
}

#[test]
fn every_case_narrows_the_window_and_the_service_to_the_same_tracks() {
    let window = hosting_window();
    let file: Value =
        serde_json::from_str(include_str!("../../src/lib/shellTrackFilter.cases.json")).unwrap();

    let everything = window.listed(json!({
        "source": { "kind": "collection" }, "sort": "trackNo", "descending": false, "query": ""
    }));
    assert_eq!(everything.len(), TRACKS);

    for case in file["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let in_playlist = case["inPlaylist"].as_bool().unwrap_or(false);
        let mut sent = case["window"].clone();
        expand_bpm(&mut sent);
        let source = if in_playlist {
            json!({ "kind": "playlist", "id": window.playlist })
        } else {
            json!({ "kind": "collection" })
        };
        let mut spec = json!({
            "source": source, "sort": "trackNo", "descending": false, "query": sent["query"],
        });
        if !sent["filter"].is_null() {
            spec["filter"] = sent["filter"].clone();
        }
        let mut args = case["service"].clone();
        if in_playlist {
            args["playlist"] = json!(window.playlist);
        }

        let listed = window.listed(spec);
        let queried = window.queried(args);

        assert_eq!(listed, queried, "{name}");
        // A case that matched everything or nothing would prove little.
        assert!(
            !listed.is_empty() && listed.len() < everything.len(),
            "{name}: {} of {} tracks",
            listed.len(),
            everything.len()
        );
    }
}

#[test]
fn a_bpm_bucket_edge_falls_the_same_way_on_both_sides() {
    let window = hosting_window();
    // Track 10 is 122.50, which the window lists under 123; track 9 is
    // 122.25, under 122.
    let listed = window.listed(json!({
        "source": { "kind": "collection" }, "sort": "trackNo", "descending": false, "query": "",
        "filter": { "bpm": { "values": [123], "tolerancePct": 0, "masterBpmX100": null } }
    }));
    assert_eq!(
        listed,
        [track_id(10), track_id(11), track_id(12), track_id(13)]
    );
    assert_eq!(window.queried(json!({ "bpm": "122.50..123.49" })), listed);
}
