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
use rbxport_lib::dto::ViewSpecDto;
use rbxport_lib::filter_playlist::{self, SaveFilterDto};
use rbxport_lib::state::spec_from_wire;
use rbxport_lib::state::AppState;
use rbxport_lib::{AppError, ErrorKind};
use serde_json::{json, Value};
use tauri::test::MockRuntime;
use tauri::Manager;

type App = tauri::App<MockRuntime>;

use crate::track_filter_agrees::{hosting_window, Window};

/// The track ids the window's list shows for this `ViewSpec`.
fn listed(app: &App, spec: Value) -> Vec<String> {
    let dto: ViewSpecDto = serde_json::from_value(spec).unwrap();
    let library = app.state::<Arc<AppState>>().library().unwrap();
    let view = library.open_view(&spec_from_wire(&library, &dto));
    view.rows
        .iter()
        .map(|&row| library.ids[row as usize].to_string())
        .collect()
}

/// What the Track Filter shows on All Tracks for this list filter.
fn filtered(app: &App, filter: Value) -> Vec<String> {
    let mut spec = json!({
        "source": { "kind": "collection" }, "sort": "trackNo", "descending": false, "query": ""
    });
    if !filter.is_null() {
        spec["filter"] = filter;
    }
    listed(app, spec)
}

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
    app: &App,
    name: &str,
    filter: Value,
) -> Result<filter_playlist::SavedFilterPlaylistDto, AppError> {
    let filter: SaveFilterDto = serde_json::from_value(filter).unwrap();
    run(filter_playlist::save_filter_playlist(
        app.handle().clone(),
        app.state::<Arc<AppState>>(),
        name.to_owned(),
        filter,
    ))
}

/// The Library's playlists and folders by name, as the tree lists them.
fn tree_names(app: &App) -> Vec<String> {
    run(commands::playlist_tree(app.state::<Arc<AppState>>()))
        .unwrap()
        .into_iter()
        .map(|node| node.name)
        .collect()
}

fn tree_node(app: &App, id: &str) -> Option<rbxport_lib::dto::TreeNodeDto> {
    run(commands::playlist_tree(app.state::<Arc<AppState>>()))
        .unwrap()
        .into_iter()
        .find(|node| node.id == id)
}

fn playlist_tracks(app: &App, id: &str) -> Vec<String> {
    listed(
        app,
        json!({
            "source": { "kind": "playlist", "id": id }, "sort": "trackNo", "descending": false, "query": ""
        }),
    )
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
        let before = tree_names(&window.app);
        match (outcome(name), save(&window.app, name, request_for(case))) {
            (Ok(()), Ok(made)) => {
                let node = tree_node(&window.app, &made.id)
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
                // The filter's own list, as the bar shows it on All Tracks.
                assert_eq!(
                    case["window"]["query"], "",
                    "{name}: a saved case has no text"
                );
                let shown = filtered(&window.app, case["window"]["filter"].clone());
                let tracks = playlist_tracks(&window.app, &made.id);
                assert_eq!(
                    tracks, shown,
                    "{name}: the playlist lists what the filter showed"
                );
                assert_eq!(tracks, expected, "{name}: the playlist's tracks");
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
                assert_eq!(
                    tree_names(&window.app),
                    before,
                    "{name}: nothing was written"
                );
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
    let made = save(&window.app, "Every 8A", json!({ "keys": ["8A"] })).unwrap();
    assert_eq!(playlist_tracks(&window.app, &made.id), shown);
    assert!(shown.len() > 3, "{shown:?}");

    // Two Prep colours alone, likewise.
    let shown = window.listed(json!({
        "source": { "kind": "collection" }, "sort": "trackNo", "descending": false, "query": "",
        "filter": { "colors": ["Green", "Red"] }
    }));
    let made = save(
        &window.app,
        "Green or Red",
        json!({ "prep_colours": ["Green", "Red"] }),
    )
    .unwrap();
    assert_eq!(playlist_tracks(&window.app, &made.id), shown);
}

#[test]
fn a_key_spelled_more_than_one_way_cannot_join_another_field_and_the_reason_names_the_spellings() {
    let window = window();
    let error = save(
        &window.app,
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
    let before = tree_names(&window.app);
    let error = save(&window.app, "Nothing", json!({})).unwrap_err();
    assert!(
        error.message.contains("Nothing is picked"),
        "{}",
        error.message
    );
    let error = save(
        &window.app,
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
    let error = save(&window.app, "   ", json!({ "prep_colours": ["Green"] })).unwrap_err();
    assert!(error.message.contains("needs a name"), "{}", error.message);
    assert_eq!(tree_names(&window.app), before);
}

#[test]
fn saving_is_one_undo_step_recorded_in_the_ledger_as_the_djs() {
    let window = window();
    let filter = json!({ "energies": ["3..5"], "my_tag_picks": [{ "category": "Situation", "tags": ["Peak time"], "any": false }] });
    let made = save(&window.app, "Peak 3+", filter).unwrap();
    let listed = playlist_tracks(&window.app, &made.id);
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
    assert!(tree_node(&window.app, &made.id).is_none());
    assert!(!undone.can_undo, "one step, nothing more");
    run(commands::redo_edit(
        window.app.handle().clone(),
        window.app.state::<Arc<AppState>>(),
    ))
    .unwrap();
    assert_eq!(
        tree_node(&window.app, &made.id).map(|node| node.name),
        Some("Peak 3+".to_owned())
    );
    assert_eq!(playlist_tracks(&window.app, &made.id), listed);
}

#[test]
fn under_library_protection_a_save_is_refused_with_nothing_written() {
    let window = window();
    window.state().core().set_protection(true).unwrap();
    let before = tree_names(&window.app);
    let error = save(
        &window.app,
        "Protected",
        json!({ "prep_colours": ["Green"] }),
    )
    .unwrap_err();
    assert!(
        error.message.contains("Library Protection"),
        "{}",
        error.message
    );
    assert_eq!(tree_names(&window.app), before);
    assert!(commands::current_history(&window.state())
        .undo_label
        .is_none());
}

// ------------------------------------------------- a Library of its own

/// A window over eight fixture tracks with keys, tempos and a MyTag set
/// for the cases the shared fixture cannot hold: keys stored in spellings
/// the writer would not make (`mystery` beside `Mystery`, `8A ` with a
/// trailing space, as an imported Library can hold them), tempos exactly
/// at and just past a range's bounds, and a MyTag id above `i32::MAX`.
///
/// Track i (id `track_id(i)`): 0 `Mystery`, 1 `mystery`, 2 `8A `, 3 `8A`,
/// the rest `5A`; BPM 120.49, 120.50, 122.49, 122.50 for 4 to 7, 130.00
/// otherwise; the MyTag "Big id" (id 3000000000) on 0 and 4.
struct Own {
    _dir: tempfile::TempDir,
    app: App,
}

const BIG_TAG_ID: &str = "3000000000";

fn own_window() -> Own {
    use rbl_db::fixture::{self, track_id, Shape};
    use rbl_db::write::{TrackField, Writer};

    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    let location = fixture::build(
        &library,
        Shape {
            tracks: 8,
            ..Shape::default()
        },
    )
    .unwrap();
    let mut writer = Writer::open(location.clone(), dir.path().join("backups")).unwrap();
    writer.disable_automatic_backups();
    let keys = [
        "Mystery",
        "KEY-LOWER",
        "KEY-SPACED",
        "8A",
        "5A",
        "5A",
        "5A",
        "5A",
    ];
    for name in ["Mystery", "KEY-LOWER", "KEY-SPACED", "8A", "5A"] {
        writer.ensure_detected_key(name).unwrap();
    }
    let tempos = [
        13_000, 13_000, 13_000, 13_000, 12_049, 12_050, 12_249, 12_250,
    ];
    for i in 0..8 {
        let id = track_id(i);
        writer.set_field(&id, TrackField::Key, keys[i]).unwrap();
        writer.set_bpm_x100(&id, tempos[i]).unwrap();
    }
    let tag = writer.create_my_tag(fixture::MY_TAG_CATEGORY).unwrap();
    writer.rename_my_tag(&tag.id, "Big id").unwrap();
    for i in [0, 4] {
        writer
            .set_my_tags(&track_id(i), std::slice::from_ref(&tag.id))
            .unwrap();
    }
    writer
        .library()
        .connection()
        .execute_batch(&format!(
            "UPDATE djmdKey SET ScaleName = 'mystery' WHERE ScaleName = 'KEY-LOWER';
             UPDATE djmdKey SET ScaleName = '8A ' WHERE ScaleName = 'KEY-SPACED';
             UPDATE djmdMyTag SET ID = '{BIG_TAG_ID}' WHERE ID = '{old}';
             UPDATE djmdSongMyTag SET MyTagID = '{BIG_TAG_ID}' WHERE MyTagID = '{old}';",
            old = tag.id
        ))
        .unwrap();
    drop(writer);

    let state = AppState::with_backups(dir.path().join("backups"));
    state.core().set_protection(false).unwrap();
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (index, _) = rbl_index::load(&db).unwrap();
    state.set_library(index, false, db.schema().db_version, 0, location);
    let app = tauri::test::mock_app();
    app.manage(Arc::new(state));
    Own { _dir: dir, app }
}

fn ids(indices: &[usize]) -> Vec<String> {
    indices
        .iter()
        .map(|&i| rbl_db::fixture::track_id(i))
        .collect()
}

/// A key filter the rule cannot keep exactly is refused, naming the key;
/// the filter's own list is what the playlist would have had to match.
fn refused_naming_key(own: &Own, identity: &str, shown: &[usize]) {
    let filter = filtered(&own.app, json!({ "keyIdentities": [identity] }));
    assert_eq!(filter, ids(shown), "the filter's own list for {identity:?}");
    let before = tree_names(&own.app);
    match save(&own.app, "Key", json!({ "keys": [identity] })) {
        Ok(made) => panic!(
            "saved {identity:?}: the playlist lists {:?}, the filter showed {filter:?}",
            playlist_tracks(&own.app, &made.id)
        ),
        Err(error) => {
            assert_eq!(error.kind, ErrorKind::Malformed);
            assert!(
                error
                    .message
                    .contains(&format!("the key {identity} exactly")),
                "{}",
                error.message
            );
        }
    }
    assert_eq!(tree_names(&own.app), before, "nothing was written");
}

#[test]
fn a_key_whose_spellings_differ_only_in_case_is_refused_because_the_rule_would_admit_both() {
    // The filter matches a name off the wheel exactly: `Mystery` alone.
    // The rule compares names whatever their case: it would add `mystery`.
    let own = own_window();
    refused_naming_key(&own, "Mystery", &[0]);
}

#[test]
fn a_key_stored_with_a_trailing_space_is_refused_because_the_rule_would_leave_it_out() {
    // The filter takes `8A ` as 8A; the rule compares the stored name as it
    // is, space and all, so `8A` would leave that track out.
    let own = own_window();
    refused_naming_key(&own, "8A", &[2, 3]);
}

#[test]
fn a_key_spelled_one_way_is_saved_and_lists_exactly_what_the_filter_showed() {
    let own = own_window();
    let shown = filtered(&own.app, json!({ "keyIdentities": ["5A"] }));
    assert_eq!(shown, ids(&[4, 5, 6, 7]));
    let made = save(&own.app, "5A", json!({ "keys": ["5A"] })).unwrap();
    assert_eq!(playlist_tracks(&own.app, &made.id), shown);
}

#[test]
fn tracks_exactly_at_both_bpm_bounds_are_in_and_those_just_past_them_are_not() {
    let own = own_window();
    let shown = filtered(
        &own.app,
        json!({ "bpm": { "values": [], "tolerancePct": 0, "masterBpmX100": null, "rangesX100": [[12050, 12249]] } }),
    );
    assert_eq!(shown, ids(&[5, 6]));
    let made = save(
        &own.app,
        "121 to 122",
        json!({ "bpm_ranges": ["120.50..122.49"] }),
    )
    .unwrap();
    assert_eq!(playlist_tracks(&own.app, &made.id), shown);
}

#[test]
fn a_mytag_whose_id_is_above_the_signed_32_bit_range_is_saved_and_lists_its_tracks() {
    let own = own_window();
    let library = own.app.state::<Arc<AppState>>().library().unwrap();
    let (at, category) = library
        .my_tags()
        .iter()
        .enumerate()
        .find(|(_, c)| c.tags.iter().any(|t| t == "Big id"))
        .map(|(at, c)| (at, c.name.clone()))
        .unwrap();
    drop(library);
    let shown = filtered(
        &own.app,
        json!({ "tags": [{ "category": at, "any": false, "tags": ["Big id"] }] }),
    );
    assert_eq!(shown, ids(&[0, 4]));
    let made = save(
        &own.app,
        "Big",
        json!({ "my_tag_picks": [{ "category": category, "tags": ["Big id"], "any": false }] }),
    )
    .unwrap();
    assert_eq!(playlist_tracks(&own.app, &made.id), shown);
}

#[test]
fn a_bpm_range_is_written_byte_for_byte_in_the_form_rekordbox_writes() {
    // `rules_are_written_byte_for_byte_as_rekordbox_writes_them`
    // (rbl-index): rekordbox 7.2.14's own BPM 120 to 126.5 rule.
    let own = own_window();
    let library = own.app.state::<Arc<AppState>>().library().unwrap();
    let filter: SaveFilterDto =
        serde_json::from_value(json!({ "bpm_ranges": ["120.00..126.50"] })).unwrap();
    let rule = filter_playlist::rule_for(&library, &filter).unwrap();
    assert_eq!(
        rule.to_xml(3_213_900_474),
        r#"<NODE Id="-1081066822" LogicalOperator="1" AutomaticUpdate="0"><CONDITION PropertyName="bpm" Operator="5" ValueUnit="0" ValueLeft="12000" ValueRight="12650"/></NODE>"#
    );
}

#[test]
fn names_are_kept_as_given_a_name_saved_twice_is_two_playlists_and_undo_takes_only_the_second() {
    let own = own_window();
    let filter = || json!({ "keys": ["5A"] });
    let first = save(&own.app, "Twice", filter()).unwrap();
    let second = save(&own.app, "Twice", filter()).unwrap();
    assert_ne!(first.id, second.id);
    run(commands::undo_edit(
        own.app.handle().clone(),
        own.app.state::<Arc<AppState>>(),
    ))
    .unwrap();
    assert!(
        tree_node(&own.app, &second.id).is_none(),
        "the second is gone"
    );
    assert_eq!(
        tree_node(&own.app, &first.id).map(|n| n.name),
        Some("Twice".to_owned())
    );
    assert_eq!(playlist_tracks(&own.app, &first.id), ids(&[4, 5, 6, 7]));

    let long = "Long name ".repeat(60);
    let made = save(&own.app, &long, filter()).unwrap();
    assert_eq!(
        tree_node(&own.app, &made.id).map(|n| n.name),
        Some(long.trim().to_owned())
    );
    let made = save(&own.app, "Sets/2026\\Peak", filter()).unwrap();
    assert_eq!(
        tree_node(&own.app, &made.id).map(|n| n.name),
        Some("Sets/2026\\Peak".to_owned())
    );

    let before = tree_names(&own.app);
    let error = save(&own.app, "", filter()).unwrap_err();
    assert!(error.message.contains("needs a name"), "{}", error.message);
    assert_eq!(tree_names(&own.app), before);
}
