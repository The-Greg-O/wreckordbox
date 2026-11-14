//! MyTags on a Stick playlist (wbx-45.12): the Track Filter bar's tag picks
//! narrow a Stick's playlist by the Stick's own MyTags, picked by category
//! (its position among the Stick's categories, as the bar's choices list
//! them), All or Any within a category and every category together, the
//! way the Library's are applied; the bar's choices are the Stick's own
//! categories and tags. Both of the Stick's libraries; a Stick exported
//! into a temporary folder, byte for byte unchanged by every read.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::sync::Arc;

use rbl_export::device_library::Format;
use rbxport_lib::commands;
use rbxport_lib::dto::ViewSpecDto;
use rbxport_lib::state::AppState;
use serde_json::{json, Value};
use tauri::test::MockRuntime;
use tauri::Manager;
use wreckordbox_stick_outline::testing::{assert_unchanged, snapshot, tagged_stick};

struct Window {
    _dir: tempfile::TempDir,
    stick: std::path::PathBuf,
    app: tauri::App<MockRuntime>,
}

fn window() -> Window {
    let dir = tempfile::tempdir().unwrap();
    let stick = dir.path().join("STICK");
    std::fs::create_dir_all(&stick).unwrap();
    tagged_stick(&stick);
    let app = tauri::test::mock_app();
    app.manage(Arc::new(AppState::with_backups(dir.path().join("backups"))));
    Window {
        _dir: dir,
        stick,
        app,
    }
}

fn wire(format: Format) -> &'static str {
    match format {
        Format::DeviceLibrary => "deviceLibrary",
        Format::OneLibrary => "oneLibrary",
    }
}

impl Window {
    /// "Warm Up"'s id in this library, read the way the tree reads it.
    fn playlist(&self, format: Format) -> String {
        let library =
            wreckordbox_stick_outline::read(&self.stick, |read| read.library(format)).unwrap();
        library
            .nodes
            .iter()
            .find(|n| n.name == "Warm Up")
            .map(|n| n.id.to_string())
            .unwrap()
    }

    fn spec(&self, format: Format, tags: Value, query: &str) -> ViewSpecDto {
        let mut spec = json!({
            "source": { "kind": "device", "path": self.stick.to_string_lossy(), "format": wire(format), "playlist": self.playlist(format) },
            "sort": "trackNo", "descending": false, "query": query,
        });
        if !tags.is_null() {
            spec["filter"] = json!({ "tags": tags });
        }
        serde_json::from_value(spec).unwrap()
    }

    fn listed(&self, format: Format, tags: Value, query: &str) -> Vec<String> {
        let state = self.app.state::<Arc<AppState>>();
        tauri::async_runtime::block_on(async {
            let handle = commands::open_view(state.clone(), self.spec(format, tags, query))
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
fn a_stick_playlist_is_narrowed_by_its_own_mytags() {
    let w = window();
    let before = snapshot(&w.stick);
    // Vibe is category 0 and Situation 1 on this Stick (by Seq).
    for format in [Format::DeviceLibrary, Format::OneLibrary] {
        let listed = |tags: Value, query: &str| w.listed(format, tags, query);
        assert_eq!(
            listed(Value::Null, ""),
            ["Low Sun", "Paper", "Kiln", "Odd", "Blank"],
            "{format:?}"
        );
        // One tag; "Elsewhere" carries it too but is not in the playlist.
        assert_eq!(
            listed(json!([{ "category": 0, "tags": ["Dark"] }]), ""),
            ["Low Sun", "Paper"],
            "{format:?}"
        );
        // All within a category, and Any.
        assert_eq!(
            listed(
                json!([{ "category": 0, "tags": ["Dark", "Uplifting"] }]),
                ""
            ),
            ["Paper"],
            "{format:?}"
        );
        assert_eq!(
            listed(
                json!([{ "category": 0, "any": true, "tags": ["Dark", "Uplifting"] }]),
                ""
            ),
            ["Low Sun", "Paper", "Kiln"],
            "{format:?}"
        );
        // Every category together.
        assert_eq!(
            listed(
                json!([{ "category": 0, "any": true, "tags": ["Dark", "Uplifting"] }, { "category": 1, "tags": ["Peak"] }]),
                ""
            ),
            ["Low Sun"],
            "{format:?}"
        );
        // A name is a tag of its own category only: Situation's "Dark".
        assert_eq!(
            listed(json!([{ "category": 1, "tags": ["Dark"] }]), ""),
            ["Odd"],
            "{format:?}"
        );
        // A name the category does not hold, or a category past the
        // Stick's, admits nothing, as on the Library; a column with nothing
        // picked constrains nothing.
        assert_eq!(
            listed(json!([{ "category": 0, "tags": ["Nope"] }]), ""),
            Vec::<String>::new(),
            "{format:?}"
        );
        assert_eq!(
            listed(json!([{ "category": 5, "tags": ["Dark"] }]), ""),
            Vec::<String>::new(),
            "{format:?}"
        );
        assert_eq!(
            listed(json!([{ "category": 0, "tags": [] }]), "").len(),
            5,
            "{format:?}"
        );
        // Tags and text together.
        assert_eq!(
            listed(json!([{ "category": 0, "tags": ["Dark"] }]), "paper"),
            ["Paper"],
            "{format:?}"
        );
    }
    assert_unchanged(&w.stick, &before, "filtering a Stick playlist by MyTags");
}

#[test]
fn the_bar_offers_the_sticks_own_categories_and_tags() {
    let w = window();
    let before = snapshot(&w.stick);
    for format in [Format::DeviceLibrary, Format::OneLibrary] {
        let state = w.app.state::<Arc<AppState>>();
        let values = tauri::async_runtime::block_on(commands::filter_values(
            state,
            w.spec(format, Value::Null, ""),
        ))
        .unwrap();
        let values = serde_json::to_value(values).unwrap();
        assert_eq!(
            values["tags"],
            json!([
                { "name": "Vibe", "tags": ["Dark", "Uplifting"] },
                { "name": "Situation", "tags": ["Peak", "Warm-up", "Dark"] },
            ]),
            "{format:?}"
        );
    }
    assert_unchanged(&w.stick, &before, "reading a Stick's MyTag choices");
}
