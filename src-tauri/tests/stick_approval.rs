//! Every command that writes to a Stick refuses without an approval token
//! (wbx-45.4, second review), and a source guard keeps it so: a new
//! Stick-writing command cannot be added without consuming a token.
//!
//! Each command is called the way the webview calls it, with no token, on a
//! temporary folder standing for a Stick, and must refuse with a message
//! that asks for a fresh plan, leaving every byte of the Stick as it was.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rbl_db::fixture::{self, playlist_id, track_id, Shape};
use rbl_db::{Library as Db, OpenMode};
use rbxport_lib::state::AppState;
use rbxport_lib::stick_approval::STICK_WRITING_COMMANDS;
use rbxport_lib::{commands, device_library, device_settings};
use tauri::test::MockRuntime;
use tauri::{App, Manager};

struct Rig {
    _dir: tempfile::TempDir,
    app: App<MockRuntime>,
    stick: tempfile::TempDir,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let location = fixture::build(dir.path(), Shape::default()).unwrap();
    let state = AppState::with_backups(dir.path().join("backups"));
    state.core().set_protection(false).unwrap();
    let db = Db::open(location.clone(), OpenMode::ReadOnly).unwrap();
    let (library, _) = rbl_index::load(&db).unwrap();
    state.set_library(library, false, db.schema().db_version, 0, location);
    let app = tauri::test::mock_app();
    app.manage(Arc::new(state));
    let stick = tempfile::tempdir().unwrap();
    // Something on it, so "unchanged" means something.
    std::fs::create_dir_all(stick.path().join("PIONEER/rekordbox")).unwrap();
    std::fs::write(stick.path().join("PIONEER/rekordbox/export.pdb"), b"stand-in").unwrap();
    std::fs::write(stick.path().join("PIONEER/DEVSETTING.DAT"), b"stand-in").unwrap();
    Rig { _dir: dir, app, stick }
}

fn bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() { todo.push(path); } else { out.insert(path.clone(), std::fs::read(&path).unwrap()); }
        }
    }
    out
}

fn refused<T: std::fmt::Debug>(name: &str, result: Result<T, rbxport_lib::AppError>) {
    let error = result.expect_err(name);
    let text = error.to_string();
    assert!(text.contains("was not confirmed") && text.contains("Nothing was written"), "{name}: {text}");
}

#[test]
fn every_stick_writing_command_refuses_without_a_token_and_writes_nothing() {
    let rig = rig();
    let path = rig.stick.path().to_string_lossy().into_owned();
    let before = bytes(rig.stick.path());
    let handle = rig.app.handle().clone();
    let mut called = Vec::new();

    called.push("export_playlist");
    refused("export_playlist", tauri::async_runtime::block_on(commands::export_playlist(handle.clone(), rig.app.state(), playlist_id(0), path.clone(), None, None, None, None)));
    called.push("export_tracks_to_device");
    refused("export_tracks_to_device", tauri::async_runtime::block_on(commands::export_tracks_to_device(handle.clone(), rig.app.state(), vec![track_id(0)], path.clone(), None, None, None)));
    called.push("sync_devices");
    let reports = tauri::async_runtime::block_on(commands::sync_devices(handle.clone(), rig.app.state(), vec![playlist_id(0)], vec![path.clone()], None, None, None, None, None, None, None)).unwrap();
    let report = serde_json::to_value(&reports).unwrap();
    let said = report[0]["error"].as_str().unwrap_or_default();
    assert!(said.contains("was not confirmed"), "sync_devices: {report}");
    called.push("device_playlist_edit");
    let edit = serde_json::from_value(serde_json::json!({ "kind": "create", "parent": "0", "name": "X", "folder": false })).unwrap();
    refused("device_playlist_edit", tauri::async_runtime::block_on(device_library::device_playlist_edit(path.clone(), "deviceLibrary".into(), edit, None)));
    called.push("device_playlist_add_files");
    refused("device_playlist_add_files", tauri::async_runtime::block_on(device_library::device_playlist_add_files(path.clone(), "deviceLibrary".into(), "1".into(), vec!["file:/tmp/x.mp3".into()], None)));
    called.push("save_device_settings");
    let settings = tauri::async_runtime::block_on(device_settings::device_settings(path.clone())).unwrap();
    refused("save_device_settings", tauri::async_runtime::block_on(device_settings::save_device_settings(path.clone(), settings, None)));
    called.push("write_device_defaults");
    let defaults = serde_json::from_value(serde_json::json!({
        "waveformColor": "blue", "waveformPosition": "center", "overviewWaveform": "half", "keyDisplay": "classic",
        "categories": null, "sorts": null, "subColumn": null,
    })).unwrap();
    refused("write_device_defaults", tauri::async_runtime::block_on(device_settings::write_device_defaults(path.clone(), defaults, None)));
    called.push("ensure_device_library");
    refused("ensure_device_library", tauri::async_runtime::block_on(device_settings::ensure_device_library(rig.app.state(), path.clone(), None, None)));
    called.push("format_device");
    refused("format_device", tauri::async_runtime::block_on(commands::format_device(path.clone(), serde_json::from_value(serde_json::json!("fat32")).unwrap(), None)));

    assert_eq!(bytes(rig.stick.path()), before, "not a byte on the Stick changed");
    let mut listed: Vec<&str> = STICK_WRITING_COMMANDS.to_vec();
    listed.sort_unstable();
    called.sort_unstable();
    assert_eq!(called, listed, "this test calls every Stick-writing command");
}

/// Calls inside a Tauri command's body that write to a Stick. A command
/// that calls one must be in `STICK_WRITING_COMMANDS` and consume a token.
const STICK_WRITES: &[&str] = &[
    "write_export_registered", "write_export_with_phase", "export_cancellable", "apply_edit(", "add_files(",
    "write_changes(", "write_dev_defaults(", "rbl_devices::format::format", "ensure_library(", "sync_one(",
    "device_library::edit(", "device::edit(", "create_library_with_root(",
];

/// Each `#[tauri::command]` function's name and body text, from the source.
fn commands_in_source() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("src")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "rs") { continue; }
        let text = std::fs::read_to_string(&path).unwrap();
        let parts: Vec<&str> = text.split("#[tauri::command]").collect();
        for part in parts.iter().skip(1) {
            let Some(at) = part.find("fn ") else { continue };
            let name: String = part[at + 3..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            // The body runs to the next item at the left margin.
            let body_end = part.find("\n}\n").map_or(part.len(), |e| e + 3);
            out.push((name, part[..body_end].to_owned()));
        }
    }
    out
}

#[test]
fn a_stick_writing_command_cannot_be_added_without_consuming_a_token() {
    let commands = commands_in_source();
    assert!(commands.len() > 100, "found the commands: {}", commands.len());
    for (name, body) in &commands {
        let writes = STICK_WRITES.iter().any(|w| body.contains(w));
        let listed = STICK_WRITING_COMMANDS.contains(&name.as_str());
        if writes {
            assert!(listed, "`{name}` writes to a Stick: list it in STICK_WRITING_COMMANDS and consume an approval token");
        }
        if listed {
            assert!(body.contains("stick_approval::consume("), "`{name}` must consume an approval token under the Stick's lock");
            let lock = body.find("stick_approval::lock(").unwrap_or(usize::MAX);
            let consume = body.find("stick_approval::consume(").unwrap_or(usize::MAX);
            assert!(lock < consume, "`{name}` must take the Stick's lock before consuming its token");
        }
    }
    for name in STICK_WRITING_COMMANDS {
        assert!(commands.iter().any(|(n, _)| n == name), "`{name}` is listed but is not a command");
    }
}

fn approve(rig: &Rig, stick: &Path, operation: serde_json::Value) -> rbxport_lib::stick_write_plan::StickWritePlan {
    let request = serde_json::from_value(serde_json::json!({ "destination": stick.to_string_lossy(), "operation": operation })).unwrap();
    rbxport_lib::stick_write_plan::plan(rig.app.state::<Arc<AppState>>().inner(), &request).unwrap()
}

#[test]
fn a_planned_export_runs_with_its_token_once_and_a_changed_stick_needs_a_fresh_plan() {
    let rig = rig();
    let blank = tempfile::tempdir().unwrap();
    let path = blank.path().to_string_lossy().into_owned();
    let handle = rig.app.handle().clone();
    let export = || serde_json::json!({ "kind": "exportPlaylist", "playlist": playlist_id(0) });

    // Planned, then the Stick changes before the write: refused, nothing written.
    let stale = approve(&rig, blank.path(), export()).approval;
    std::fs::create_dir_all(blank.path().join("PIONEER")).unwrap();
    std::fs::write(blank.path().join("PIONEER/DEVSETTING.DAT"), b"put there behind the plan").unwrap();
    let before = bytes(blank.path());
    let error = tauri::async_runtime::block_on(commands::export_playlist(handle.clone(), rig.app.state(), playlist_id(0), path.clone(), None, None, None, stale)).unwrap_err();
    assert!(error.to_string().contains("The Stick changed"), "{error}");
    assert_eq!(bytes(blank.path()), before);

    // A fresh plan's token runs the export, once.
    let token = approve(&rig, blank.path(), export()).approval;
    let written = tauri::async_runtime::block_on(commands::export_playlist(handle.clone(), rig.app.state(), playlist_id(0), path.clone(), None, None, None, token.clone()));
    assert!(written.is_ok(), "{written:?}");
    let held: Vec<String> = rbl_export::Manifest::load(blank.path()).unwrap().playlists.iter().map(|p| p.library_id.to_string()).collect();
    assert_eq!(held, [playlist_id(0)], "exactly the planned playlist is on the Stick");
    let again = tauri::async_runtime::block_on(commands::export_playlist(handle, rig.app.state(), playlist_id(0), path, None, None, None, token)).unwrap_err();
    assert!(again.to_string().contains("already used"), "{again}");
}
