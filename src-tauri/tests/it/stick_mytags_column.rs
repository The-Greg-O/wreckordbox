//! The MyTags column on a Stick list (wbx-f7y): asked for the `myTags`
//! field, a Stick row carries the names of the Stick's own MyTags, each
//! with its id and category, read through the Stick read that never
//! recovers or writes. Both libraries; the Stick byte for byte unchanged.
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
use tauri::Manager;
use wreckordbox_stick_outline::testing::{assert_unchanged, snapshot, tagged_stick};

fn wire(format: Format) -> &'static str {
    match format {
        Format::DeviceLibrary => "deviceLibrary",
        Format::OneLibrary => "oneLibrary",
    }
}

/// Each listed row's title and its `myTags` field as `fetch_rows` sends it
/// (`None` where the row has no such field).
fn listed(
    stick: &std::path::Path,
    format: Format,
    tags: Value,
    extra: Option<Vec<String>>,
) -> Vec<(String, Option<Value>)> {
    let app = tauri::test::mock_app();
    app.manage(Arc::new(AppState::with_backups(
        stick.with_file_name("backups"),
    )));
    let state = app.state::<Arc<AppState>>();
    let library = wreckordbox_stick_outline::read(stick, |read| read.library(format)).unwrap();
    let playlist = library
        .nodes
        .iter()
        .find(|n| n.name == "Warm Up")
        .unwrap()
        .id;
    let mut spec = json!({
        "source": { "kind": "device", "path": stick.to_string_lossy(), "format": wire(format), "playlist": playlist.to_string() },
        "sort": "trackNo", "descending": false, "query": "",
    });
    if !tags.is_null() {
        spec["filter"] = json!({ "tags": tags });
    }
    let spec: ViewSpecDto = serde_json::from_value(spec).unwrap();
    tauri::async_runtime::block_on(async {
        let handle = commands::open_view(state.clone(), spec).await.unwrap();
        let rows = commands::fetch_rows(state, handle.view_id, 0, handle.len.max(1), extra)
            .await
            .unwrap();
        rows.into_iter()
            .map(|row| {
                let tags = row.extra.as_ref().and_then(|e| e.get("myTags")).cloned();
                (row.title, tags)
            })
            .collect()
    })
}

fn tag(id: &str, name: &str, category: &str) -> Value {
    json!({ "id": id, "name": name, "category": category })
}

#[test]
fn a_stick_row_shows_the_names_of_its_own_mytags() {
    let dir = tempfile::tempdir().unwrap();
    let stick = dir.path().join("STICK");
    std::fs::create_dir_all(&stick).unwrap();
    tagged_stick(&stick);
    let before = snapshot(&stick);
    let wanted = || Some(vec!["color".to_owned(), "myTags".to_owned()]);
    for format in [Format::DeviceLibrary, Format::OneLibrary] {
        // Vibe before Situation (the Stick's order); Situation's own "Dark"
        // is a tag of its own, named with its category.
        let expected = vec![
            (
                "Low Sun".to_owned(),
                Some(json!([
                    tag("11", "Dark", "Vibe"),
                    tag("21", "Peak", "Situation")
                ])),
            ),
            (
                "Paper".to_owned(),
                Some(json!([
                    tag("11", "Dark", "Vibe"),
                    tag("12", "Uplifting", "Vibe")
                ])),
            ),
            (
                "Kiln".to_owned(),
                Some(json!([
                    tag("12", "Uplifting", "Vibe"),
                    tag("22", "Warm-up", "Situation")
                ])),
            ),
            (
                "Odd".to_owned(),
                Some(json!([tag("23", "Dark", "Situation")])),
            ),
            ("Blank".to_owned(), Some(json!([]))),
        ];
        assert_eq!(
            listed(&stick, format, Value::Null, wanted()),
            expected,
            "{format:?}"
        );
        // The same names while the bar narrows the list by a tag.
        assert_eq!(
            listed(
                &stick,
                format,
                json!([{ "category": 0, "tags": ["Dark"] }]),
                wanted()
            ),
            expected[..2].to_vec(),
            "{format:?}"
        );
        // Not asked for, not sent.
        assert!(
            listed(&stick, format, Value::Null, None)
                .iter()
                .all(|(_, tags)| tags.is_none()),
            "{format:?}"
        );
    }
    assert_unchanged(&stick, &before, "showing a Stick list's MyTags");
}
