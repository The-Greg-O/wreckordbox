//! The Track Filter bar over a Stick's playlist (wbx-57): the list a Stick
//! playlist opens is narrowed by the Stick's own key, Energy, Prep colour
//! and BPM, and the bar's choices come from the Stick, through the reads
//! that cannot recover or write a Stick. A Stick exported into a temporary
//! folder; nothing here can reach a mounted Stick.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::path::Path;
use std::sync::Arc;

use rbl_export::{export, SourcePlaylist, SourceTrack};
use rbxport_lib::commands;
use rbxport_lib::dto::ViewSpecDto;
use rbxport_lib::state::AppState;
use serde_json::{json, Value};
use tauri::test::MockRuntime;
use tauri::Manager;
use wreckordbox_stick_outline::testing::{assert_unchanged, snapshot};

/// `(title, key, bpm x100, stars, colour id)`.
const TRACKS: [(&str, &str, u32, u8, u8); 6] = [
    ("Low Sun", "Am", 12350, 5, 5),
    ("Paper", "8A", 12849, 3, 0),
    ("Kiln", "Em", 12850, 0, 2),
    ("Odd", "Mystery", 12000, 4, 5),
    ("Blank", "", 0, 4, 5),
    ("Elsewhere", "Am", 12400, 5, 5),
];

/// Exports a Stick with a playlist "Bar" of the first five tracks; the
/// sixth is on the Stick but not in the playlist.
fn build_stick(dest: &Path) {
    let src = tempfile::tempdir().unwrap();
    let tracks: Vec<SourceTrack> = TRACKS
        .iter()
        .enumerate()
        .map(|(at, (title, key, bpm, stars, colour))| {
            let id = at as u64 + 1;
            let path = src.path().join(format!("source-{id}.mp3"));
            std::fs::write(&path, vec![id as u8; 2048]).unwrap();
            SourceTrack {
                id,
                source_path: path,
                title: (*title).into(),
                artist: "Stick".into(),
                key: (*key).into(),
                bpm_x100: *bpm,
                rating: *stars,
                color_id: *colour,
                ..SourceTrack::default()
            }
        })
        .collect();
    let playlists = vec![SourcePlaylist {
        id: 11,
        name: "Bar".into(),
        track_indices: vec![0, 1, 2, 3, 4],
        ..Default::default()
    }];
    export(dest, &tracks, &playlists).unwrap();
}

struct Window {
    _dir: tempfile::TempDir,
    stick: std::path::PathBuf,
    app: tauri::App<MockRuntime>,
    playlist: String,
}

fn window() -> Window {
    let dir = tempfile::tempdir().unwrap();
    let stick = dir.path().join("STICK");
    std::fs::create_dir_all(&stick).unwrap();
    build_stick(&stick);
    let app = tauri::test::mock_app();
    app.manage(Arc::new(AppState::with_backups(dir.path().join("backups"))));
    // The playlist's id in the Stick's own library, read the way the tree
    // reads it.
    let library = wreckordbox_stick_outline::read(&stick, |read| {
        read.library(rbl_export::device_library::Format::DeviceLibrary)
    })
    .unwrap();
    let playlist = library
        .nodes
        .iter()
        .find(|node| node.name == "Bar")
        .map(|node| node.id.to_string())
        .unwrap();
    Window {
        _dir: dir,
        stick,
        app,
        playlist,
    }
}

impl Window {
    fn spec(&self, filter: Value, query: &str) -> ViewSpecDto {
        let mut spec = json!({
            "source": { "kind": "device", "path": self.stick.to_string_lossy(), "format": "deviceLibrary", "playlist": self.playlist },
            "sort": "trackNo", "descending": false, "query": query,
        });
        if !filter.is_null() {
            spec["filter"] = filter;
        }
        serde_json::from_value(spec).unwrap()
    }

    /// The titles the playlist lists under this filter, in order.
    fn listed(&self, filter: Value, query: &str) -> Vec<String> {
        let state = self.app.state::<Arc<AppState>>();
        tauri::async_runtime::block_on(async {
            let handle = commands::open_view(state.clone(), self.spec(filter, query))
                .await
                .unwrap();
            let rows = commands::fetch_rows(state, handle.view_id, 0, handle.len.max(1), None)
                .await
                .unwrap();
            assert_eq!(rows.len() as u32, handle.len);
            rows.into_iter().map(|row| row.title).collect()
        })
    }
}

#[test]
fn a_stick_playlist_is_narrowed_by_the_sticks_own_fields() {
    let w = window();
    let before = snapshot(&w.stick);

    assert_eq!(
        w.listed(Value::Null, ""),
        ["Low Sun", "Paper", "Kiln", "Odd", "Blank"]
    );
    // Energy 5 only: not every track, and not the one outside the playlist.
    assert_eq!(w.listed(json!({ "ratings": [5] }), ""), ["Low Sun"]);
    // Unrated is its own choice.
    assert_eq!(w.listed(json!({ "ratings": [0] }), ""), ["Kiln"]);
    // A key by identity: both spellings of 8A; an unknown key exactly.
    assert_eq!(
        w.listed(json!({ "keyIdentities": ["8A"] }), ""),
        ["Low Sun", "Paper"]
    );
    assert_eq!(
        w.listed(json!({ "keyIdentities": ["Mystery"] }), ""),
        ["Odd"]
    );
    // No Prep colour, and Green.
    assert_eq!(w.listed(json!({ "colors": ["none"] }), ""), ["Paper"]);
    assert_eq!(
        w.listed(json!({ "colors": ["Green"] }), ""),
        ["Low Sun", "Odd", "Blank"]
    );
    // The whole-BPM bucket 124 (123.50 to 124.49) and its edges.
    assert_eq!(w.listed(json!({ "bpm": { "values": [], "tolerancePct": 0, "masterBpmX100": null, "rangesX100": [[12350, 12449]] } }), ""), ["Low Sun"]);
    assert_eq!(w.listed(json!({ "bpm": { "values": [], "tolerancePct": 0, "masterBpmX100": null, "rangesX100": [[12450, 12849]] } }), ""), ["Paper"]);
    // Fields and text together.
    assert_eq!(w.listed(json!({ "colors": ["Green"] }), "odd"), ["Odd"]);

    assert_unchanged(&w.stick, &before, "filtering a Stick playlist");
}

#[test]
fn the_choices_on_a_stick_playlist_come_from_the_stick() {
    let w = window();
    let state = w.app.state::<Arc<AppState>>();
    let values =
        tauri::async_runtime::block_on(commands::filter_values(state, w.spec(Value::Null, "")))
            .unwrap();
    let values = serde_json::to_value(values).unwrap();
    let keys: Vec<&str> = values["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["value"].as_str().unwrap())
        .collect();
    // As stored on the Stick, in wheel order; a blank key is not offered,
    // and "Elsewhere" is not in the playlist (its Am is counted once, from
    // "Low Sun").
    assert_eq!(keys, ["8A", "Am", "Em", "Mystery"]);
    assert_eq!(values["keys"][1]["count"], 1);
    // A Stick's library as read carries no MyTags.
    assert_eq!(values["tags"], json!([]));
}
