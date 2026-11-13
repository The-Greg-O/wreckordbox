//! The Track Filter bar's one filter, sent both ways (wbx-57): the window's
//! list (its `ViewSpec`, opened as `open_view` opens it) and the Library
//! service's `tracks.query` (the CLI's `tracks query`), for every case in
//! `src/lib/shellTrackFilter.cases.json`, whose window and service forms the
//! front end's unit tests prove are what the model sends. Both must list
//! exactly the case's `expected` tracks, worked out from the fixture's
//! definition below, in the same order.
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

/// The fixture, track `i` of 0 to 29 (id 10000 + i):
///
/// - BPM 120.00 + 0.25 i (so the half-BPM edges of whole BPMs are crossed);
/// - key `8A, 9A, 11B, 8A, Am` by i mod 5, except 28 is Open Key `1m` and
///   29 is `Mystery`, a name off the wheel;
/// - Energy (i / 2) mod 6, 0 being none;
/// - Prep colour i mod 9, 0 being none (1 Pink ... 8 Purple);
/// - Situation: Peak time when i is even, Tension builder when i mod 3 is 0,
///   Warm-up when i mod 5 is 0; Mood: Dark when i mod 4 is 0, Dreamy when
///   i mod 7 is 0;
/// - genre "Deep House" when i mod 3 is 1;
/// - the playlist "Bar test" holds tracks 3 to 20; the intelligent playlist
///   "House" is genre contains "house".
const TRACKS: usize = 30;
const KEYS: [&str; 5] = ["8A", "9A", "11B", "8A", "Am"];
const MOOD: &str = "700101";

fn key_of(index: usize) -> &'static str {
    match index {
        28 => "1m",
        29 => "Mystery",
        _ => KEYS[index % KEYS.len()],
    }
}

struct Window {
    _dir: tempfile::TempDir,
    app: tauri::App<MockRuntime>,
    playlist: String,
    intelligent: String,
}

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
    // A second category, written as the fixture builder writes its one (the
    // writer cannot make categories: rekordbox fixes them).
    writer
        .library()
        .connection()
        .execute_batch(&format!(
            "INSERT INTO djmdMyTag
                (ID, Seq, Name, Attribute, ParentID, UUID,
                 rb_data_status, rb_local_data_status, rb_local_deleted,
                 rb_local_synced, usn, rb_local_usn, created_at, updated_at)
             SELECT '{MOOD}', 2, 'Mood', 1, 'root', 'fixture-mytag-{MOOD}',
                    0, 0, 0, 0, NULL, rb_local_usn, created_at, updated_at
             FROM djmdMyTag WHERE ID = '{}'",
            fixture::MY_TAG_CATEGORY
        ))
        .unwrap();
    let mut tag = |category: &str, name: &str| {
        let made = writer.create_my_tag(category).unwrap();
        writer.rename_my_tag(&made.id, name).unwrap();
        made.id
    };
    let peak = tag(fixture::MY_TAG_CATEGORY, "Peak time");
    let tension = tag(fixture::MY_TAG_CATEGORY, "Tension builder");
    let dark = tag(MOOD, "Dark");
    let dreamy = tag(MOOD, "Dreamy");
    let warm_up = fixture::MY_TAG_WARM_UP.to_owned();
    for name in ["8A", "9A", "11B", "Am", "1m", "Mystery"] {
        writer.ensure_detected_key(name).unwrap();
    }
    for index in 0..TRACKS {
        let id = track_id(index);
        writer
            .set_bpm_x100(&id, 12_000 + 25 * index as u32)
            .unwrap();
        writer
            .set_field(&id, TrackField::Key, key_of(index))
            .unwrap();
        writer.set_rating(&id, (index / 2 % 6) as u8).unwrap();
        let colour = (index % 9 != 0).then(|| (index % 9).to_string());
        writer.set_color(&id, colour.as_deref()).unwrap();
        if index % 3 == 1 {
            writer
                .set_field(&id, TrackField::Genre, "Deep House")
                .unwrap();
        }
        let tags: Vec<String> = [
            (index % 2 == 0, &peak),
            (index % 3 == 0, &tension),
            (index % 5 == 0, &warm_up),
            (index % 4 == 0, &dark),
            (index % 7 == 0, &dreamy),
        ]
        .into_iter()
        .filter(|(carries, _)| *carries)
        .map(|(_, tag)| tag.clone())
        .collect();
        writer.set_my_tags(&id, &tags).unwrap();
    }
    let playlist = writer.create_playlist("Bar test", "root").unwrap();
    let members: Vec<String> = (3..=20).map(track_id).collect();
    writer.add_tracks(&playlist, &members).unwrap();
    let intelligent = writer
        .create_smart_playlist("House", "root", |node| {
            format!(
                "<NODE Id=\"{node}\" LogicalOperator=\"1\" AutomaticUpdate=\"0\">\
                 <CONDITION PropertyName=\"genre\" Operator=\"8\" ValueUnit=\"\" \
                 ValueLeft=\"house\" ValueRight=\"\"/></NODE>"
            )
        })
        .unwrap();
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
        intelligent,
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

#[test]
fn every_case_lists_exactly_the_expected_tracks_in_the_window_and_the_service() {
    let window = hosting_window();
    let file: Value =
        serde_json::from_str(include_str!("../../../src/lib/shellTrackFilter.cases.json")).unwrap();

    let everything = window.listed(json!({
        "source": { "kind": "collection" }, "sort": "trackNo", "descending": false, "query": ""
    }));
    assert_eq!(everything.len(), TRACKS);

    let mut checked = 0;
    for case in file["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let playlist = if case["inPlaylist"].as_bool().unwrap_or(false) {
            Some(&window.playlist)
        } else if case["inIntelligent"].as_bool().unwrap_or(false) {
            Some(&window.intelligent)
        } else {
            None
        };
        let source = match playlist {
            Some(id) => json!({ "kind": "playlist", "id": id }),
            None => json!({ "kind": "collection" }),
        };
        let sent = &case["window"];
        let mut spec = json!({
            "source": source, "sort": "trackNo", "descending": false, "query": sent["query"],
        });
        if !sent["filter"].is_null() {
            spec["filter"] = sent["filter"].clone();
        }
        let mut args = case["service"].clone();
        if let Some(playlist) = playlist {
            args["playlist"] = json!(playlist);
        }

        let listed = window.listed(spec);
        let queried = window.queried(args);

        assert_eq!(listed, queried, "{name}: window and service differ");
        match case["expected"].as_array() {
            Some(expected) => {
                let expected: Vec<String> = expected.iter().map(|id| id.to_string()).collect();
                assert_eq!(listed, expected, "{name}");
                checked += 1;
            }
            None => assert!(
                !listed.is_empty() && listed.len() < everything.len(),
                "{name}: {} of {} tracks",
                listed.len(),
                everything.len()
            ),
        }
    }
    assert!(checked >= 14, "only {checked} cases had expected tracks");
}

#[test]
fn a_bpm_bucket_edge_falls_the_same_way_on_both_sides() {
    let window = hosting_window();
    // Track 10 is 122.50, which the list rounds to 123; track 9 is 122.25,
    // which it rounds to 122.
    let listed = window.listed(json!({
        "source": { "kind": "collection" }, "sort": "trackNo", "descending": false, "query": "",
        "filter": { "bpm": { "values": [], "tolerancePct": 0, "masterBpmX100": null, "rangesX100": [[12250, 12349]] } }
    }));
    assert_eq!(
        listed,
        [track_id(10), track_id(11), track_id(12), track_id(13)]
    );
    assert_eq!(
        window.queried(json!({ "bpm_ranges": ["122.50..123.49"] })),
        listed
    );
}
