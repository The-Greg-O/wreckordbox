//! Saving the Track Filter bar's filter as an intelligent playlist (wbx-58),
//! on the Track Filter's own fixture and cases (`track_filter_agrees`): each
//! case's filter is either saved as an intelligent playlist that lists
//! exactly the tracks the filter showed on All Tracks, or refused with a
//! reason that names what an intelligent playlist cannot hold, with nothing
//! written. Saving is one Undo step, recorded in the Ledger as the DJ's, and
//! refused under Library Protection.
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

use rbxport_lib::commands;
use rbxport_lib::filter_playlist::{self, SaveFilterDto};
use rbxport_lib::state::AppState;
use rbxport_lib::{AppError, ErrorKind};
use serde_json::{json, Value};
use tauri::Manager;

use crate::track_filter_agrees::{hosting_window, Window};

fn run<T>(fut: impl std::future::Future<Output = T>) -> T {
    tauri::async_runtime::block_on(fut)
}

/// The fixture's window, as the DJ who edits has it: Library Protection off.
fn window() -> Window {
    let window = hosting_window();
    window.state().core().set_protection(false).unwrap();
    window
}

fn save(
    window: &Window,
    name: &str,
    filter: Value,
) -> Result<filter_playlist::SavedFilterPlaylistDto, AppError> {
    let filter: SaveFilterDto = serde_json::from_value(filter).unwrap();
    run(filter_playlist::save_filter_playlist(
        window.app.handle().clone(),
        window.app.state::<Arc<AppState>>(),
        name.to_owned(),
        filter,
    ))
}

/// The Library's playlists and folders by name, as the tree lists them.
fn tree_names(window: &Window) -> Vec<String> {
    run(commands::playlist_tree(window.app.state::<Arc<AppState>>()))
        .unwrap()
        .into_iter()
        .map(|node| node.name)
        .collect()
}

fn tree_node(window: &Window, id: &str) -> Option<rbxport_lib::dto::TreeNodeDto> {
    run(commands::playlist_tree(window.app.state::<Arc<AppState>>()))
        .unwrap()
        .into_iter()
        .find(|node| node.id == id)
}

fn playlist_tracks(window: &Window, id: &str) -> Vec<String> {
    window.listed(json!({
        "source": { "kind": "playlist", "id": id }, "sort": "trackNo", "descending": false, "query": ""
    }))
}

/// What the window sends for a case: the service form of the filter, with
/// the text as `search`, the open list when it is not All Tracks, and
/// which of Key match and BPM ±6% are switched on.
fn request_for(case: &Value) -> Value {
    let mut request = case["service"].clone();
    if case["inPlaylist"].as_bool().unwrap_or(false) {
        request["within"] = json!("Bar test");
    } else if case["inIntelligent"].as_bool().unwrap_or(false) {
        request["within"] = json!("House");
    }
    let model = &case["filter"];
    let mut follows = Vec::new();
    if model["keyMatch"].as_bool().unwrap_or(false) {
        follows.push("key");
    }
    if model["bpmMatch"].as_bool().unwrap_or(false) {
        follows.push("bpm");
    }
    if !follows.is_empty() {
        request["follows"] = json!(follows);
    }
    request
}

/// What each case comes to: saved, or refused with words that say why.
fn outcome(name: &str) -> Result<(), &'static str> {
    match name {
        "the design's example: two Situation tags, Energy 3 to 5"
        | "Energy 1 to 5 is every track that has one"
        | "unrated and no Prep colour, each a choice of its own"
        | "BPM edges: whole 121 to 122 is 120.50 to 122.49"
        | "BPM open below, up to 120 (up to 120.49)"
        | "All within both categories" => Ok(()),
        "two keys by identity and two Prep colours" => {
            Err("2 keys (8A, 11B) and 2 Prep colours (Green, Red)")
        }
        "unrated together with Energy 5" => Err("Energy 5 or unrated"),
        "a key off the wheel exactly, and every spelling of 8A" => Err("2 keys (Mystery, 8A)"),
        "Any within one category, All across two" => {
            Err("Situation: any of Warm-up, Tension builder")
        }
        "Key match and BPM ±6% against a loaded 9A at 123.00" => {
            Err("Key match and BPM ±6% follow the loaded track")
        }
        "Key match with keys picked: both must hold" => Err("Key match follows the loaded track"),
        "within a playlist: a key and a tag" => Err("cannot keep this filter to \"Bar test\""),
        "within an intelligent playlist: Energy and a BPM range" => {
            Err("cannot keep this filter to \"House\"")
        }
        "text with fields" => Err("cannot search for text, so it cannot keep \"track 01\""),
        other => panic!("no outcome for the case {other:?}: add one"),
    }
}

#[test]
fn every_case_saves_a_playlist_listing_the_filters_tracks_or_is_refused_with_the_reason() {
    let window = window();
    let file: Value =
        serde_json::from_str(include_str!("../../../src/lib/shellTrackFilter.cases.json")).unwrap();

    let mut saved = 0;
    for case in file["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let before = tree_names(&window);
        match (outcome(name), save(&window, name, request_for(case))) {
            (Ok(()), Ok(made)) => {
                let node = tree_node(&window, &made.id)
                    .unwrap_or_else(|| panic!("{name}: not in the tree"));
                assert_eq!(
                    (node.name.as_str(), node.kind),
                    (name, "smartPlaylist"),
                    "{name}"
                );
                let expected: Vec<String> = case["expected"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|id| id.to_string())
                    .collect();
                assert!(
                    !expected.is_empty(),
                    "{name}: a case that lists nothing proves little"
                );
                assert_eq!(
                    playlist_tracks(&window, &made.id),
                    expected,
                    "{name}: the playlist's tracks"
                );
                saved += 1;
            }
            (Err(reason), Err(error)) => {
                assert_eq!(
                    error.kind,
                    ErrorKind::Malformed,
                    "{name}: {}",
                    error.message
                );
                assert!(
                    error.message.contains(reason),
                    "{name}: {:?} does not say {reason:?}",
                    error.message
                );
                assert_eq!(tree_names(&window), before, "{name}: nothing was written");
            }
            (Ok(()), Err(error)) => panic!("{name}: refused: {}", error.message),
            (Err(reason), Ok(_)) => panic!("{name}: saved, but should be refused for {reason:?}"),
        }
    }
    assert_eq!(saved, 6);
}

#[test]
fn a_field_of_one_choice_alone_is_saved_as_any_of_its_spellings() {
    let window = window();
    // Every spelling of 8A (8A, Am, Open Key 1m) and nothing else: one
    // "any of" rule. The filter's own list is the reference.
    let shown = window.listed(json!({
        "source": { "kind": "collection" }, "sort": "trackNo", "descending": false, "query": "",
        "filter": { "keyIdentities": ["8A"] }
    }));
    let made = save(&window, "Every 8A", json!({ "keys": ["8A"] })).unwrap();
    assert_eq!(playlist_tracks(&window, &made.id), shown);
    assert!(shown.len() > 3, "{shown:?}");

    // Two Prep colours alone, likewise.
    let shown = window.listed(json!({
        "source": { "kind": "collection" }, "sort": "trackNo", "descending": false, "query": "",
        "filter": { "colors": ["Green", "Red"] }
    }));
    let made = save(
        &window,
        "Green or Red",
        json!({ "prep_colours": ["Green", "Red"] }),
    )
    .unwrap();
    assert_eq!(playlist_tracks(&window, &made.id), shown);
}

#[test]
fn a_key_spelled_more_than_one_way_cannot_join_another_field_and_the_reason_names_the_spellings() {
    let window = window();
    let error = save(
        &window,
        "8A Green",
        json!({ "keys": ["8A"], "prep_colours": ["Green"] }),
    )
    .unwrap_err();
    assert!(
        error
            .message
            .contains("the key 8A, spelled 3 ways in the Library (8A, Am, 1m)"),
        "{}",
        error.message
    );
}

#[test]
fn an_empty_filter_and_a_tag_no_longer_in_the_library_are_refused() {
    let window = window();
    let before = tree_names(&window);
    let error = save(&window, "Nothing", json!({})).unwrap_err();
    assert!(
        error.message.contains("Nothing is picked"),
        "{}",
        error.message
    );
    let error = save(
        &window,
        "Gone",
        json!({ "my_tag_picks": [{ "category": "Situation", "tags": ["Afterhours"], "any": false }] }),
    )
    .unwrap_err();
    assert!(
        error
            .message
            .contains("Situation: Afterhours is no longer in the Library"),
        "{}",
        error.message
    );
    let error = save(&window, "   ", json!({ "prep_colours": ["Green"] })).unwrap_err();
    assert!(error.message.contains("needs a name"), "{}", error.message);
    assert_eq!(tree_names(&window), before);
}

#[test]
fn saving_is_one_undo_step_recorded_in_the_ledger_as_the_djs() {
    let window = window();
    let filter = json!({ "energies": ["3..5"], "my_tag_picks": [{ "category": "Situation", "tags": ["Peak time"], "any": false }] });
    let made = save(&window, "Peak 3+", filter).unwrap();
    let listed = playlist_tracks(&window, &made.id);
    assert!(!listed.is_empty());
    let history = commands::current_history(&window.state());
    assert_eq!(
        history.undo_label.as_deref(),
        Some("Save Filter as Intelligent Playlist")
    );
    assert!(made.generation > 0);

    // The Ledger: one entry of the DJ's, in the window's Run.
    let runs = window
        .state()
        .with_service(|s| s.call("runs.list", json!({})))
        .unwrap()
        .unwrap();
    let djs = runs["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|run| run["source"] == json!({"kind": "dj", "name": "dj"}))
        .expect("the window's Run")
        .clone();
    let shown = window
        .state()
        .with_service(|s| s.call("runs.show", json!({ "id": djs["id"] })))
        .unwrap()
        .unwrap();
    let entries = shown["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{shown:#}");
    assert_eq!(entries[0]["action"], "edit");
    assert_eq!(entries[0]["label"], "Save Filter as Intelligent Playlist");
    assert_eq!(entries[0]["source"], json!({"kind": "dj", "name": "dj"}));

    // One Undo takes the playlist away; Redo brings it back, listing the same.
    let undone = run(commands::undo_edit(
        window.app.handle().clone(),
        window.app.state::<Arc<AppState>>(),
    ))
    .unwrap();
    assert_eq!(
        undone.redo_label.as_deref(),
        Some("Save Filter as Intelligent Playlist")
    );
    assert!(tree_node(&window, &made.id).is_none());
    assert!(!undone.can_undo, "one step, nothing more");
    run(commands::redo_edit(
        window.app.handle().clone(),
        window.app.state::<Arc<AppState>>(),
    ))
    .unwrap();
    assert_eq!(
        tree_node(&window, &made.id).map(|node| node.name),
        Some("Peak 3+".to_owned())
    );
    assert_eq!(playlist_tracks(&window, &made.id), listed);
}

#[test]
fn under_library_protection_a_save_is_refused_with_nothing_written() {
    let window = window();
    window.state().core().set_protection(true).unwrap();
    let before = tree_names(&window);
    let error = save(&window, "Protected", json!({ "prep_colours": ["Green"] })).unwrap_err();
    assert!(
        error.message.contains("Library Protection"),
        "{}",
        error.message
    );
    assert_eq!(tree_names(&window), before);
    assert!(commands::current_history(&window.state())
        .undo_label
        .is_none());
}
