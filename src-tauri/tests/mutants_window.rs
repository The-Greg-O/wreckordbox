//! Mutants and the window's own playlist routes (wbx-80): a folder drop's
//! Replace existing carries the replaced playlist's Mutants to the new
//! one, and Undo and Redo follow; Remove from Collection names the Mutants
//! that would go before the DJ confirms.
//!
//! The window's real command functions on a mock app hosting the Library
//! service on a fixture Library in a temporary folder.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::path::Path;
use std::sync::Arc;

use rbl_db::fixture::{self, track_id, Shape};
use rbl_db::write::ROOT;
use rbxport_lib::state::AppState;
use rbxport_lib::{commands, mutant_window, service_host};
use serde_json::{json, Value};
use tauri::test::MockRuntime;
use tauri::{Manager, State};
use wreckordbox_service::mutants::commands as mutants;
use wreckordbox_service::reading::commands::PLAYLISTS_TRACKS;
use wreckordbox_service::Config;

fn write_wav(path: &Path) {
    let frames: u32 = 44_100;
    let data_len = frames * 4;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&44_100_u32.to_le_bytes());
    out.extend_from_slice(&(44_100_u32 * 4).to_le_bytes());
    out.extend_from_slice(&4_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.resize(44 + data_len as usize, 0);
    std::fs::write(path, out).unwrap();
}

struct Window {
    dir: tempfile::TempDir,
    app: tauri::App<MockRuntime>,
    /// The playlist "Warm Up": fixture track 1, then the dropped file.
    playlist: String,
    /// The track the folder's file already is.
    dropped: String,
}

fn run<T>(fut: impl std::future::Future<Output = T>) -> T {
    tauri::async_runtime::block_on(fut)
}

fn window() -> Window {
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    let location = fixture::build(&library, Shape { tracks: 3, ..Shape::default() }).unwrap();
    let folder = dir.path().join("Warm Up");
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("a.wav");
    write_wav(&file);
    let (playlist, dropped) = {
        let mut writer =
            rbl_db::write::Writer::open(location.clone(), dir.path().join("fixture-backups")).unwrap();
        writer.disable_automatic_backups();
        let dropped = writer.import_file(&file).unwrap();
        let playlist = writer.create_playlist("Warm Up", ROOT).unwrap();
        writer.add_tracks(&playlist, &[track_id(1), dropped.clone()]).unwrap();
        (playlist, dropped)
    };
    std::fs::create_dir_all(dir.path().join("home")).unwrap();
    let state = AppState::with_backups(dir.path().join("backups"));
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (index, _) = rbl_index::load(&db).unwrap();
    state.set_library(index, false, db.schema().db_version, 0, location.clone());
    let app = tauri::test::mock_app();
    app.manage(Arc::new(state));
    let window = Window { dir, app, playlist, dropped };
    run(service_host::set_library_protection(window.state(), false)).unwrap();
    let config = Config {
        runtime_dir: window.dir.path().join("run"),
        idle_exit: None,
        recovery_dir: window.dir.path().join("backups"),
    };
    service_host::host_with(window.app.handle(), &location, &config).unwrap();
    window
}

impl Window {
    fn state(&self) -> State<'_, Arc<AppState>> {
        self.app.state::<Arc<AppState>>()
    }

    fn call(&self, command: &str, args: Value) -> Value {
        self.state()
            .with_service(|service| service.call(command, args))
            .expect("the window hosts the service")
            .unwrap()
    }

    fn entries(&self, playlist: &str) -> Vec<String> {
        self.call(PLAYLISTS_TRACKS, json!({ "id": playlist }))["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["entry_id"].as_str().unwrap().to_owned())
            .collect()
    }

    fn mutate(&self, entry: &str, edits: Value) -> (i64, Value) {
        let id = self.call(
            mutants::MUTANTS_CREATE,
            json!({ "entry": entry, "source": { "kind": "agent", "name": "preparer" } }),
        )["mutant"]["id"]
            .as_i64()
            .unwrap();
        self.call(
            mutants::MUTANTS_EDIT,
            json!({ "id": id, "edits": edits, "source": { "kind": "agent", "name": "preparer" } }),
        );
        (id, self.mutant(id)["edits"].clone())
    }

    fn mutant(&self, id: i64) -> Value {
        self.call(mutants::MUTANTS_GET, json!({ "id": id }))["mutant"].clone()
    }

    fn live(&self) -> Vec<Value> {
        self.call(mutants::MUTANTS_LIST, json!({}))["mutants"]
            .as_array()
            .unwrap()
            .clone()
    }
}

#[test]
fn a_folder_drop_replace_existing_carries_the_playlists_mutants_and_undo_and_redo_follow() {
    let w = window();
    let entries = w.entries(&w.playlist);
    let (kept, kept_edits) = w.mutate(&entries[1], json!({ "key_shift": 2 }));
    let (dropped_track, _) = w.mutate(&entries[0], json!({ "key_shift": -1 }));

    // The DJ drops the folder "Warm Up" on the tree and chooses Replace.
    let path = w.dir.path().join("Warm Up").display().to_string();
    let asked = run(commands::import_folder_playlist(
        w.app.handle().clone(), w.state(), path.clone(), ROOT.into(), None, Some(0), None,
    ))
    .unwrap();
    assert_eq!(asked.conflict.as_deref(), Some(w.playlist.as_str()));
    let replaced = run(commands::import_folder_playlist(
        w.app.handle().clone(), w.state(), path, ROOT.into(), Some(w.playlist.clone()), asked.at, None,
    ))
    .unwrap();
    let made = replaced.playlist.unwrap();
    let new_entries = w.entries(&made);
    assert_eq!(new_entries.len(), 1);

    // The dropped file's Mutant is on the new playlist's entry of it, with
    // its edits; track 1 is not in the new playlist, so its Mutant went.
    let live = w.live();
    assert_eq!(live.len(), 1, "{live:?}");
    assert_eq!(live[0]["playlist"]["id"], made.as_str());
    assert_eq!(live[0]["entry_id"], new_entries[0].as_str());
    assert_eq!(live[0]["parent"]["id"], w.dropped.as_str());
    assert_eq!(live[0]["edits"], kept_edits);
    let carried = live[0]["id"].as_i64().unwrap();
    assert_eq!(w.mutant(kept)["removed"], true);
    assert_eq!(w.mutant(dropped_track)["removed"], true);

    run(commands::undo_edit(w.app.handle().clone(), w.state())).unwrap();
    let mut back: Vec<i64> = w.live().iter().map(|m| m["id"].as_i64().unwrap()).collect();
    back.sort_unstable();
    assert_eq!(back, vec![kept, dropped_track]);
    assert_eq!(w.mutant(kept)["edits"], kept_edits);
    assert_eq!(w.mutant(kept)["position"], 2);
    assert_eq!(w.mutant(carried)["removed"], true);

    run(commands::redo_edit(w.app.handle().clone(), w.state())).unwrap();
    let live: Vec<i64> = w.live().iter().map(|m| m["id"].as_i64().unwrap()).collect();
    assert_eq!(live, vec![carried]);
    assert_eq!(w.mutant(carried)["edits"], kept_edits);
}

#[test]
fn remove_from_collection_names_the_mutants_that_would_go_before_anything_is_removed() {
    let w = window();
    let entries = w.entries(&w.playlist);
    let (id, _) = w.mutate(&entries[1], json!({ "key_shift": 1 }));
    let (other, _) = w.mutate(&entries[0], json!({ "key_shift": 2 }));

    let named = run(mutant_window::collection_removal_mutants(w.state(), vec![w.dropped.clone()])).unwrap();
    assert_eq!(named.len(), 1);
    assert_eq!(named[0].id, id);
    assert_eq!(named[0].playlist_name, "Warm Up");
    assert_eq!(named[0].position, Some(2));
    assert_eq!(named[0].track_id, w.dropped);
    assert!(!named[0].description.is_empty());
    // Asking removed nothing.
    assert_eq!(w.live().len(), 2);
    assert_eq!(w.entries(&w.playlist).len(), 2);

    // Confirmed: the window's own command removes the track, and its
    // Mutant goes with it.
    run(commands::remove_from_collection(w.app.handle().clone(), w.state(), vec![w.dropped.clone()])).unwrap();
    let live: Vec<i64> = w.live().iter().map(|m| m["id"].as_i64().unwrap()).collect();
    assert_eq!(live, vec![other]);
    assert_eq!(w.mutant(id)["removed"], true);
    let none = run(mutant_window::collection_removal_mutants(w.state(), vec![track_id(2)])).unwrap();
    assert!(none.is_empty());
}
