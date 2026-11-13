//! Stick Protection (wbx-45.11) and the DJ's per-operation confirmation
//! (wbx-45.4), held by every command that can write to a Stick.
//!
//! - Every writer is called on a protected Stick (a temporary folder
//!   standing in for one) and refused with `stickProtected`, the Stick's
//!   name and how to unlock it, leaving every byte as it was; the
//!   Stick-only writes even with a confirmed token in hand.
//! - The same writers to an ordinary folder are not refused, and need no
//!   token.
//! - A source guard: every registered command is classified, a Stick
//!   writer or not, and any command that reaches a write primitive without
//!   passing the guard fails the test, through helpers too (with a negative
//!   test of an indirect writer).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rbl_db::fixture::{self, playlist_id, track_id, Shape};
use rbl_db::{Library as Db, OpenMode};
use rbxport_lib::state::AppState;
use rbxport_lib::stick_approval::STICK_WRITING_COMMANDS;
use rbxport_lib::stick_guard::STICK_WRITERS;
use rbxport_lib::{
    commands, details, device_library, device_settings, log_window, stick_add_tracks,
    stick_recovery,
};
use tauri::test::MockRuntime;
use tauri::{App, Manager};
use wreckordbox_stick_protection::{SPARE_MARKER, STAND_IN_MARKER};

struct Rig {
    dir: tempfile::TempDir,
    app: App<MockRuntime>,
    sticks: tempfile::TempDir,
    stick: PathBuf,
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
    app.manage(Arc::new(rbxport_lib::grid::GridEditor::at(
        &dir.path().join("editor"),
    )));
    let sticks = tempfile::tempdir().unwrap();
    let stick = sticks.path().join("GIGSTICK");
    // Something on it, so "unchanged" means something.
    std::fs::create_dir_all(stick.join("Contents")).unwrap();
    std::fs::write(stick.join("Contents/keep.txt"), b"the DJ's").unwrap();
    std::fs::write(stick.join(STAND_IN_MARKER), b"").unwrap();
    Rig {
        dir,
        app,
        sticks,
        stick,
    }
}

impl Rig {
    fn state(&self) -> Arc<AppState> {
        Arc::clone(self.app.state::<Arc<AppState>>().inner())
    }
    fn state_dir(&self) -> PathBuf {
        self.state().backup_dir().to_path_buf()
    }
    fn unlock(&self) {
        wreckordbox_stick_protection::unlock(&self.state_dir(), &self.stick, Some("GIGSTICK"))
            .unwrap();
    }
    fn lock(&self) {
        wreckordbox_stick_protection::lock(&self.state_dir(), &self.stick).unwrap();
    }
    fn path(&self) -> String {
        self.stick.to_string_lossy().into_owned()
    }
    /// A confirmed token for `operation` on the Stick: planned, then
    /// approved, as the window's hold does.
    fn token(&self, operation: &serde_json::Value) -> Option<String> {
        let plan = plan(self, &self.stick, operation);
        plan.plan
            .map(|id| rbxport_lib::stick_approval::approve(&self.state_dir(), &id).unwrap())
    }
}

fn plan(
    rig: &Rig,
    stick: &Path,
    operation: &serde_json::Value,
) -> rbxport_lib::stick_write_plan::StickWritePlan {
    let request = serde_json::from_value(
        serde_json::json!({ "destination": stick.to_string_lossy(), "operation": operation }),
    )
    .unwrap();
    rbxport_lib::stick_write_plan::plan(&rig.state(), &request).unwrap()
}

fn bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                todo.push(path);
            } else {
                out.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    out
}

fn run<T>(future: impl std::future::Future<Output = T>) -> T {
    tauri::async_runtime::block_on(future)
}

/// Refused by Stick Protection: the stable code, the Stick's name, how to unlock it.
fn protected<T: std::fmt::Debug>(name: &str, result: Result<T, rbxport_lib::AppError>) {
    let error = result.expect_err(name);
    assert_eq!(
        serde_json::to_value(error.kind).unwrap(),
        "stickProtected",
        "{name}: {error:?}"
    );
    assert!(
        error.message.contains("GIGSTICK") && error.message.contains("unlock"),
        "{name}: {}",
        error.message
    );
}

/// An MP3 on the Stick, imported into the Library: (file, track id).
fn track_on_stick(rig: &Rig) -> (PathBuf, String) {
    let audio = rig.stick.join("Contents/On Stick.mp3");
    std::fs::create_dir_all(audio.parent().unwrap()).unwrap();
    let mut mp3 = Vec::new();
    for i in 0..40u8 {
        mp3.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x64]);
        mp3.extend(std::iter::repeat_n(i, 413));
    }
    std::fs::write(&audio, &mp3).unwrap();
    rbl_db::key_tag::write_key(&audio, "Am").unwrap();
    let id = run(commands::import_files(
        rig.app.handle().clone(),
        rig.app.state(),
        vec![audio.display().to_string()],
    ))
    .unwrap()
    .tracks[0]
        .id
        .clone();
    rig.state()
        .write(|w| {
            w.ensure_detected_key("Am")?;
            w.ensure_detected_key("Fm")
        })
        .unwrap();
    (audio, id)
}

#[test]
fn every_writer_is_refused_on_a_protected_stick_and_writes_nothing() {
    let rig = rig();
    let path = rig.path();
    let handle = rig.app.handle().clone();
    let (audio, track) = track_on_stick(&rig);
    // While unlocked: the backup folder on the Stick, and a key edit that
    // wrote the file's tag, for Undo and Redo below.
    rig.unlock();
    {
        let access = rbxport_lib::stick_guard::enter(&rig.state_dir(), &rig.stick).unwrap();
        rig.state()
            .set_backup_destination(&access, &rig.stick)
            .unwrap();
    }
    run(details::set_track_field(
        handle.clone(),
        rig.app.state(),
        vec![track.clone()],
        "key".into(),
        "Fm".into(),
        Some(true),
    ))
    .unwrap();
    // Tokens the DJ confirmed, so it is Stick Protection that refuses.
    let tokens: BTreeMap<&str, Option<String>> = [
        ("export_playlist", serde_json::json!({ "kind": "exportPlaylist", "playlist": playlist_id(0) })),
        ("export_tracks_to_device", serde_json::json!({ "kind": "exportTracks", "tracks": [track_id(0)] })),
        ("sync_devices", serde_json::json!({ "kind": "sync", "playlists": [playlist_id(0)] })),
        ("device_playlist_edit", serde_json::json!({ "kind": "playlistEdit", "format": "deviceLibrary", "edit": { "kind": "create", "parent": "0", "name": "X", "folder": false } })),
        ("device_playlist_add_files", serde_json::json!({ "kind": "addFiles", "format": "deviceLibrary", "playlist": "1", "files": ["file:/x.mp3"] })),
        ("save_device_settings", serde_json::json!({ "kind": "saveSettings", "settings": serde_json::to_value(run(device_settings::device_settings(path.clone())).unwrap()).unwrap() })),
        ("ensure_device_library", serde_json::json!({ "kind": "ensureLibrary" })),
        ("format_device", serde_json::json!({ "kind": "format", "layout": "fat32" })),
        ("stick_view_load", serde_json::json!({ "kind": "sync", "playlists": [playlist_id(0)] })),
    ]
    .into_iter()
    .map(|(name, op)| (name, rig.token(&op)))
    .collect();
    rig.lock();
    let before = bytes(&rig.stick);
    let mut called: Vec<&str> = Vec::new();
    let mut call = |name: &'static str| {
        called.push(name);
        tokens.get(name).cloned().flatten()
    };

    let t = call("export_playlist");
    protected(
        "export_playlist",
        run(commands::export_playlist(
            handle.clone(),
            rig.app.state(),
            playlist_id(0),
            path.clone(),
            None,
            None,
            None,
            t,
        )),
    );
    let t = call("export_tracks_to_device");
    protected(
        "export_tracks_to_device",
        run(commands::export_tracks_to_device(
            handle.clone(),
            rig.app.state(),
            vec![track_id(0)],
            path.clone(),
            None,
            None,
            t,
        )),
    );
    let t = call("sync_devices");
    let approvals = t.map(|t| [(path.clone(), t)].into_iter().collect());
    let reports = run(commands::sync_devices(
        handle.clone(),
        rig.app.state(),
        vec![playlist_id(0)],
        vec![path.clone()],
        None,
        None,
        None,
        None,
        None,
        None,
        approvals,
    ))
    .unwrap();
    let said = serde_json::to_value(&reports).unwrap()[0]["error"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(
        said.contains("GIGSTICK is protected"),
        "sync_devices: {said}"
    );
    let t = call("device_playlist_edit");
    let edit = serde_json::from_value(
        serde_json::json!({ "kind": "create", "parent": "0", "name": "X", "folder": false }),
    )
    .unwrap();
    protected(
        "device_playlist_edit",
        run(device_library::device_playlist_edit(
            rig.app.state(),
            path.clone(),
            "deviceLibrary".into(),
            edit,
            t,
        )),
    );
    let t = call("device_playlist_add_files");
    protected(
        "device_playlist_add_files",
        run(device_library::device_playlist_add_files(
            rig.app.state(),
            path.clone(),
            "deviceLibrary".into(),
            "1".into(),
            vec!["file:/x.mp3".into()],
            t,
        )),
    );
    let t = call("device_playlist_add_library_tracks");
    protected(
        "device_playlist_add_library_tracks",
        run(stick_add_tracks::device_playlist_add_library_tracks(
            handle.clone(),
            rig.app.state(),
            path.clone(),
            "deviceLibrary".into(),
            "1".into(),
            vec![track_id(0)],
            false,
            None,
            None,
            t,
        )),
    );
    let t = call("save_device_settings");
    let settings = run(device_settings::device_settings(path.clone())).unwrap();
    protected(
        "save_device_settings",
        run(device_settings::save_device_settings(
            rig.app.state(),
            path.clone(),
            settings,
            t,
        )),
    );
    let t = call("ensure_device_library");
    protected(
        "ensure_device_library",
        run(device_settings::ensure_device_library(
            rig.app.state(),
            path.clone(),
            None,
            t,
        )),
    );
    let t = call("stick_settle");
    protected(
        "stick_settle",
        run(stick_recovery::stick_settle(
            rig.app.state(),
            path.clone(),
            t,
        )),
    );
    let t = call("format_device");
    protected(
        "format_device",
        run(commands::format_device(
            rig.app.state(),
            path.clone(),
            serde_json::from_value(serde_json::json!("fat32")).unwrap(),
            t,
        )),
    );
    // The Stick view's load (wbx-71): refused before the hosted service is
    // asked (this window hosts none), with the hold's token in hand.
    let t = call("stick_view_load");
    protected(
        "stick_view_load",
        run(rbxport_lib::stick_view::stick_view_load(
            rig.app.state(),
            path.clone(),
            vec![playlist_id(0)],
            "plan".into(),
            t,
        )),
    );
    let on_stick = |name: &str| rig.stick.join(name).to_string_lossy().into_owned();
    call("export_playlist_file");
    protected(
        "export_playlist_file",
        run(commands::export_playlist_file(
            rig.app.state(),
            playlist_id(0),
            on_stick("set.m3u8"),
            "m3u8".into(),
        )),
    );
    call("export_xml");
    protected(
        "export_xml",
        run(commands::export_xml(
            rig.app.state(),
            on_stick("New Folder/rekordbox.xml"),
        )),
    );
    call("export_loop_wav");
    protected(
        "export_loop_wav",
        run(commands::export_loop_wav(
            rig.app.state(),
            track_id(0),
            0.0,
            1000.0,
            on_stick("loop.wav"),
        )),
    );
    call("save_log_text");
    protected(
        "save_log_text",
        run(log_window::save_log_text(
            rig.app.state(),
            on_stick("log.txt"),
            "log".into(),
        )),
    );
    call("set_backup_directory");
    protected(
        "set_backup_directory",
        run(commands::set_backup_directory(
            rig.app.state(),
            on_stick("Backups"),
        )),
    );
    call("back_up_library");
    protected(
        "back_up_library",
        run(commands::back_up_library(rig.app.state())),
    );
    call("start_backup");
    protected("start_backup", commands::start_backup(rig.app.state()));
    call("delete_backup");
    protected(
        "delete_backup",
        run(commands::delete_backup(
            rig.app.state(),
            on_stick("Contents/keep.txt"),
        )),
    );
    call("set_track_field");
    let key_before = run(details::track_details(rig.app.state(), track.clone()))
        .unwrap()
        .key;
    protected(
        "set_track_field",
        run(details::set_track_field(
            handle.clone(),
            rig.app.state(),
            vec![track.clone()],
            "key".into(),
            "Am".into(),
            Some(true),
        )),
    );
    assert_eq!(
        run(details::track_details(rig.app.state(), track.clone()))
            .unwrap()
            .key,
        key_before,
        "nor the Library's key"
    );
    call("undo_edit");
    let undo = run(commands::undo_edit(handle.clone(), rig.app.state())).unwrap_err();
    assert!(
        undo.to_string().contains("GIGSTICK is protected"),
        "undo_edit: {undo}"
    );
    call("analyse_track");
    // Analysis writes the key tag through this, after the Library has it.
    rbxport_lib::key_tag::write_logged(&rig.state_dir(), &track, &audio.to_string_lossy(), "Am");
    assert_eq!(bytes(&rig.stick), before, "not a byte on the Stick changed");

    call("redo_edit");
    // Redo of a key edit on the Stick: undone while unlocked, then locked.
    rig.unlock();
    run(commands::undo_edit(handle.clone(), rig.app.state())).unwrap();
    rig.lock();
    let undone = bytes(&rig.stick);
    let redo = run(commands::redo_edit(handle.clone(), rig.app.state())).unwrap_err();
    assert!(
        redo.to_string().contains("GIGSTICK is protected"),
        "redo_edit: {redo}"
    );
    assert_eq!(bytes(&rig.stick), undone, "redo wrote nothing");
    assert_eq!(
        rbl_db::key_tag::read_key(&audio).unwrap().as_deref(),
        Some("Am"),
        "as the unlocked Undo left it"
    );
    let mut listed: Vec<&str> = STICK_WRITERS.to_vec();
    listed.sort_unstable();
    called.sort_unstable();
    assert_eq!(called, listed, "this test calls every Stick writer");
}

#[test]
fn a_spare_stick_unlocked_takes_a_confirmed_export_and_an_unconfirmed_one_is_still_refused() {
    let rig = rig();
    std::fs::write(rig.stick.join(SPARE_MARKER), b"").unwrap();
    wreckordbox_stick_protection::unlock(&rig.state_dir(), &rig.stick, None).unwrap();
    let handle = rig.app.handle().clone();
    let unconfirmed = run(commands::export_playlist(
        handle.clone(),
        rig.app.state(),
        playlist_id(0),
        rig.path(),
        None,
        None,
        None,
        None,
    ))
    .unwrap_err();
    assert!(
        unconfirmed.to_string().contains("was not confirmed"),
        "{unconfirmed}"
    );
    let planned = plan(
        &rig,
        &rig.stick,
        &serde_json::json!({ "kind": "exportPlaylist", "playlist": playlist_id(0) }),
    );
    assert!(
        planned.on_stick && !planned.protected && planned.spare,
        "{planned:?}"
    );
    let plan_id = planned.plan.unwrap();
    let not_yet = run(commands::export_playlist(
        handle.clone(),
        rig.app.state(),
        playlist_id(0),
        rig.path(),
        None,
        None,
        None,
        Some(plan_id.clone()),
    ))
    .unwrap_err();
    assert!(
        not_yet.to_string().contains("not confirmed"),
        "a plan id is not a token: {not_yet}"
    );
    let token = rbxport_lib::stick_write_plan::plan(&rig.state(), &serde_json::from_value(serde_json::json!({ "destination": rig.path(), "operation": { "kind": "exportPlaylist", "playlist": playlist_id(0) } })).unwrap()).unwrap().plan.map(|p| rbxport_lib::stick_approval::approve(&rig.state_dir(), &p).unwrap());
    run(commands::export_playlist(
        handle,
        rig.app.state(),
        playlist_id(0),
        rig.path(),
        None,
        None,
        None,
        token,
    ))
    .unwrap();
    assert!(rbl_export::Manifest::load(&rig.stick).is_some(), "written");
}

#[test]
fn an_ordinary_folder_needs_no_unlock_and_no_token() {
    let rig = rig();
    let folder = rig.dir.path().join("Exports");
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.to_string_lossy().into_owned();
    let handle = rig.app.handle().clone();
    let planned = plan(
        &rig,
        &folder,
        &serde_json::json!({ "kind": "exportPlaylist", "playlist": playlist_id(0) }),
    );
    assert!(!planned.on_stick && planned.plan.is_none(), "{planned:?}");
    run(commands::export_playlist(
        handle.clone(),
        rig.app.state(),
        playlist_id(0),
        path.clone(),
        None,
        None,
        None,
        None,
    ))
    .unwrap();
    assert!(rbl_export::Manifest::load(&folder).is_some(), "exported");
    run(commands::export_xml(
        rig.app.state(),
        folder.join("rekordbox.xml").to_string_lossy().into_owned(),
    ))
    .unwrap();
    run(commands::export_playlist_file(
        rig.app.state(),
        playlist_id(0),
        folder.join("set.m3u8").to_string_lossy().into_owned(),
        "m3u8".into(),
    ))
    .unwrap();
    run(log_window::save_log_text(
        rig.app.state(),
        folder.join("log.txt").to_string_lossy().into_owned(),
        "log".into(),
    ))
    .unwrap();
    let reports = run(commands::sync_devices(
        handle,
        rig.app.state(),
        vec![playlist_id(0)],
        vec![path],
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    ))
    .unwrap();
    assert!(
        serde_json::to_value(&reports).unwrap()[0]["error"].is_null(),
        "{:?}",
        serde_json::to_value(&reports).unwrap()
    );
}

#[test]
fn a_stick_removed_and_put_back_is_protected_again() {
    let rig = rig();
    rig.unlock();
    let moved = rig.sticks.path().join("elsewhere");
    std::fs::rename(&rig.stick, &moved).unwrap();
    std::fs::rename(&moved, &rig.stick).unwrap();
    // Same folder, same inode: a rename is not a re-insert. A new volume is.
    let copy = rig.sticks.path().join("copy");
    std::fs::create_dir(&copy).unwrap();
    for (file, content) in bytes(&rig.stick) {
        let to = copy.join(file.strip_prefix(&rig.stick).unwrap());
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::write(to, content).unwrap();
    }
    std::fs::remove_dir_all(&rig.stick).unwrap();
    std::fs::rename(&copy, &rig.stick).unwrap();
    protected(
        "export_xml",
        run(commands::export_xml(
            rig.app.state(),
            rig.stick.join("x.xml").to_string_lossy().into_owned(),
        )),
    );
}

#[test]
fn a_sync_to_two_sticks_takes_their_locks_in_one_order_and_refuses_the_same_stick_twice() {
    let rig = rig();
    let path = rig.path();
    let twice = run(commands::sync_devices(
        rig.app.handle().clone(),
        rig.app.state(),
        vec![playlist_id(0)],
        vec![path.clone(), format!("{path}/.")],
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    ));
    let error = twice.unwrap_err();
    assert!(error.full_message().contains("more than once"), "{error:?}");
}

/// Functions called by name in `body`, with where, and whether the call is
/// qualified: a method call `.name(` or a path `Type::name(` (followed only
/// when one function of the sources has that name), or a free call `name(`.
fn calls(body: &str) -> Vec<(String, usize, bool)> {
    let mut out = Vec::new();
    let bytes = body.as_bytes();
    for (at, _) in body.match_indices('(') {
        let mut start = at;
        while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
            start -= 1;
        }
        if start == at || bytes[start].is_ascii_digit() {
            continue;
        }
        let qualified = body[..start].ends_with('.') || body[..start].ends_with("::");
        out.push((body[start..at].to_owned(), start, qualified));
    }
    out
}

/// What writes to a Stick: a function reaching one must hold the guard's
/// capability ([`StickAccess`](rbxport_lib::stick_guard::StickAccess)).
const PRIMITIVES: &[&str] = &[
    "write_export_registered(",
    "write_export_with_phase(",
    "export_cancellable(",
    "add_in_place_to_playlist(",
    "device::apply(",
    "write_dev_defaults(",
    "format::format(",
    "ensure_library(",
    "sync_one_device(",
    "stick_outline::settle(",
    "write_range_wav(",
    "swap_key(",
    "write_key(",
    "set_backup_destination(",
    // Backups (`backups::create`, `start`, `delete`) enter the guard inside,
    // after the Library edit gate: followed by the file-write analysis below.
];

/// Any file written, renamed or removed: wherever it is. A registered
/// command reaching one without the guard must be one that writes only the
/// app's own files (`APP_FILE_WRITERS`), decided one by one.
const FILE_PRIMITIVES: &[&str] = &[
    "fs::write(",
    "fs::rename(",
    "fs::copy(",
    "fs::remove_file(",
    "fs::remove_dir(",
    "fs::remove_dir_all(",
    "fs::create_dir(",
    "fs::create_dir_all(",
    "File::create(",
    "File::create_new(",
    "OpenOptions::new().write(",
    "OpenOptions::new().create(",
    "OpenOptions::new().append(",
    "write_atomically(",
    "persist_new(",
    "persist(",
];

/// One function of the app's sources: its name, its text (signature and
/// body), and whether it is a registered command's.
struct Function {
    name: String,
    text: String,
    command: bool,
}

/// Each function in `sources`, free functions and methods alike, split at
/// every `fn `.
fn functions(sources: &[(PathBuf, String)]) -> Vec<Function> {
    let mut out = Vec::new();
    for (_, text) in sources {
        let pieces: Vec<&str> = text.split("fn ").collect();
        for (i, piece) in pieces.iter().enumerate().skip(1) {
            let before = pieces[i - 1];
            // `fn ` as a word: after a space, a newline or the start.
            if !before.is_empty() && !before.ends_with([' ', '\n', '\t']) {
                continue;
            }
            let name: String = piece
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if name.is_empty() {
                continue;
            }
            // `#[tauri::command]` after the previous item ended.
            let tail = before.rfind(['}', ';']).map_or(before, |at| &before[at..]);
            let command = tail.contains("#[tauri::command]");
            out.push(Function {
                name,
                text: (*piece).to_owned(),
                command,
            });
        }
    }
    out
}

/// Where in `text` the guard is held: from each `stick_guard::enter…` bound
/// to a name (not `_`, not inside `drop(`) to that name's `drop(`, or to
/// the end. A guard dropped before a write does not cover it.
fn held(text: &str) -> Vec<std::ops::Range<usize>> {
    held_by(text, &["stick_guard::enter"])
}

/// [`held`] for any of the guards `markers` (the service's
/// `wreckordbox_stick_protection::authorize…`).
fn held_by(text: &str, markers: &[&str]) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let found: Vec<usize> = markers
        .iter()
        .flat_map(|m| text.match_indices(*m).map(|(at, _)| at))
        .collect();
    for at in found {
        let start = text[..at].rfind([';', '{', '}']).map_or(0, |s| s + 1);
        let statement = text[start..at].trim_start();
        if statement.contains("drop(") {
            continue;
        }
        let Some(binding) = statement.strip_prefix("let ") else {
            // `match stick_guard::enter(..) { Ok(access) => .. }`: held in its arms.
            out.push(at..text.len());
            continue;
        };
        let binding = binding.trim_start_matches("mut ");
        let name: String = binding
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() || name == "_" {
            continue;
        }
        let end = text[at..]
            .find(&format!("drop({name})"))
            .map_or(text.len(), |d| at + d);
        out.push(at..end);
    }
    out
}

/// Whether the function takes the guard's capability as a parameter: its
/// caller had to pass the guard to have one.
fn capable(text: &str) -> bool {
    text.split_once('{')
        .is_some_and(|(signature, _)| signature.contains("StickAccess"))
}

/// The places in `text` that write: a primitive, or a call to a writer (a
/// qualified call only when its name is one of `unique`).
fn writes(
    text: &str,
    primitives: &[&str],
    writers: &BTreeMap<String, String>,
    unique: &BTreeSet<String>,
) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = primitives
        .iter()
        .flat_map(|p| text.match_indices(*p).map(|(at, _)| (at, (*p).to_owned())))
        .collect();
    out.extend(
        calls(text)
            .into_iter()
            .filter(|(name, _, qualified)| !qualified || unique.contains(name))
            .filter_map(|(name, at, _)| {
                writers
                    .get(&name)
                    .map(|why| (at, format!("{name}() → {why}")))
            }),
    );
    out
}

/// The names only one function of the sources has.
fn unique_names(functions: &[Function]) -> BTreeSet<String> {
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for f in functions {
        *seen.entry(f.name.as_str()).or_default() += 1;
    }
    seen.into_iter()
        .filter(|(_, n)| *n == 1)
        .map(|(name, _)| name.to_owned())
        .collect()
}

/// The functions that reach a primitive at a place the guard does not
/// cover: a function taking the capability is covered throughout; so is
/// what follows a held guard, until it is dropped.
fn unguarded_writers_of(functions: &[Function], primitives: &[&str]) -> BTreeMap<String, String> {
    unguarded_writers_by(functions, primitives, &["stick_guard::enter"])
}

/// [`unguarded_writers_of`], a write covered by any of the guards `guards`.
fn unguarded_writers_by(
    functions: &[Function],
    primitives: &[&str],
    guards: &[&str],
) -> BTreeMap<String, String> {
    let unique = unique_names(functions);
    let mut writers: BTreeMap<String, String> = BTreeMap::new();
    loop {
        let mut more = Vec::new();
        for f in functions {
            if writers.contains_key(&f.name) || capable(&f.text) {
                continue;
            }
            let cover = held_by(&f.text, guards);
            if let Some((_, why)) = writes(&f.text, primitives, &writers, &unique)
                .into_iter()
                .find(|(at, _)| !cover.iter().any(|r| r.contains(at)))
            {
                more.push((f.name.clone(), why));
            }
        }
        if more.is_empty() {
            return writers;
        }
        writers.extend(more);
    }
}

fn unguarded_writers(functions: &[Function]) -> BTreeMap<String, String> {
    unguarded_writers_of(functions, PRIMITIVES)
}

/// The functions that write a Stick directly without holding the guard or
/// its capability: each must take `StickAccess` (only `stick_guard` makes
/// one) or enter the guard itself.
fn uncapable_helpers(functions: &[Function]) -> Vec<String> {
    functions
        .iter()
        .filter(|f| !capable(&f.text))
        .filter(|f| {
            let cover = held(&f.text);
            writes(&f.text, PRIMITIVES, &BTreeMap::new(), &BTreeSet::new())
                .iter()
                .any(|(at, _)| !cover.iter().any(|r| r.contains(at)))
        })
        .map(|f| f.name.clone())
        .collect()
}

fn sources() -> Vec<(PathBuf, String)> {
    sources_in(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"))
}

/// The sources of a crate of the workspace, by its folder under `crates/`.
fn crate_sources(name: &str) -> Vec<(PathBuf, String)> {
    sources_in(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../crates")
            .join(name)
            .join("src"),
    )
}

/// Every `.rs` file under `root`, test modules at the end of a file cut.
fn sources_in(root: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                todo.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                // Test modules (at the end of a file) write freely to their
                // own temporary folders; other `#[cfg(test)]` items stay.
                let cut = text
                    .match_indices("#[cfg(test)]")
                    .map(|(at, _)| at)
                    .find(|at| {
                        let after = text[*at + "#[cfg(test)]".len()..].trim_start();
                        let after = after.strip_prefix("#[allow(").map_or(after, |rest| {
                            rest.split_once(")]").map_or(rest, |(_, r)| r.trim_start())
                        });
                        after.starts_with("mod ")
                            && after
                                .lines()
                                .next()
                                .is_some_and(|l| l.trim_end().ends_with('{'))
                    })
                    .unwrap_or(text.len());
                let text = text[..cut].to_owned();
                out.push((path, text));
            }
        }
    }
    out
}

/// The commands `lib.rs` registers, by name.
fn registered() -> BTreeSet<String> {
    let lib =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs")).unwrap();
    let start = lib.find("generate_handler![").unwrap() + "generate_handler![".len();
    let end = start + lib[start..].find(']').unwrap();
    lib[start..end]
        .lines()
        .map(|line| line.split("//").next().unwrap_or_default())
        .flat_map(|line| line.split(','))
        .map(|item| {
            item.trim()
                .rsplit("::")
                .next()
                .unwrap_or_default()
                .to_owned()
        })
        .filter(|name| !name.is_empty())
        .collect()
}

/// Registered commands that write files without passing `stick_guard`,
/// each checked to write only the app's own files (its state folder,
/// caches, the Library under Library Protection), never a place the DJ
/// chooses: a new file-writing command fails the guard until it is guarded
/// or added here after that check.
const APP_FILE_WRITERS: &[&str] = &[
    // The Library, its analysis files and caches, under Library Protection
    // (and, on a Stick-classified volume, Stick Protection at the Library's
    // own edit boundary); the app's state folder (stick_unlock's record).
    "add_artwork",
    "add_cue",
    "add_loop",
    "add_to_tag_list",
    "analyse_track",
    "auto_relocate",
    "backup_sizes",
    "clear_artwork",
    "convert_memory_cues_to_hot",
    "create_library",
    "create_my_tag",
    "deck_tempo",
    "delete_cue",
    "device_playlist_import",
    "grid_edit",
    "grid_lock",
    "grid_redo",
    "grid_undo",
    "import_files",
    "import_files_slice",
    "import_folder_playlist",
    "import_itunes",
    "import_itunes_selected",
    "import_usb",
    "import_xml",
    "move_cue",
    "move_hot_cue",
    "record_play",
    // Undo and Redo of a window grid step (wbx-64): the grid editor's
    // analysis files under `file_edit` (Library and Stick Protection, the
    // file journal), and the Library cache re-read when the tempo changed,
    // as `grid_undo`/`grid_redo` do.
    "redo_edit",
    "reload_library",
    "relocate_by_location",
    "rename_my_tag",
    "set_active_loop",
    "set_cue_colour",
    "set_cue_comment",
    "set_track_color",
    "set_track_comment",
    "set_track_field",
    "set_track_rating",
    "stick_unlock",
    "undo_edit",
    "use_default_library",
];

/// Every registered command that cannot write to a Stick: a new command
/// must be added here or to `STICK_WRITERS`, after deciding which it is.
const NOT_STICK_WRITERS: &[&str] = &[
    "add_artwork",
    "add_cue",
    "add_loop",
    "add_playlist_artwork",
    "add_to_tag_list",
    "add_tracks_to_playlist",
    "adopt_library_protection",
    "analysis_stages",
    "app_diagnostics",
    "app_version",
    "audio_devices",
    "auto_relocate",
    "backup_directory",
    "backup_progress",
    "backup_sizes",
    "cancel_backup",
    "cancel_export",
    "clear_artwork",
    "clear_tag_list",
    "collection_removal_mutants",
    "convert_memory_cues_to_hot",
    "create_folder",
    "create_library",
    "create_my_tag",
    "create_playlist",
    "create_smart_playlist",
    "database_drives",
    "deck_clear_loop",
    "deck_key_shift",
    "deck_load",
    "deck_loop_active",
    "deck_master_tempo",
    "deck_metronome",
    "deck_metronome_grid",
    "deck_move",
    "deck_move_grid",
    "deck_move_loop",
    "deck_pause",
    "deck_play",
    "deck_play_after",
    "deck_scrub_begin",
    "deck_scrub_end",
    "deck_scrub_to",
    "deck_seek",
    "deck_set_loop",
    "deck_state",
    "deck_tempo",
    "deck_unload",
    "delete_cue",
    "delete_my_tag",
    "delete_playlist",
    "device_libraries",
    "device_playlist_collection_tracks",
    "device_playlist_duplicates",
    "device_playlist_import",
    "device_playlist_import_plan",
    "device_settings",
    "device_sync_state",
    "disable_read_only",
    "drag_tracks",
    "dropped_file_paths",
    "edit_phrase",
    "eject_device",
    "estimate_sync",
    "explorer_children",
    "explorer_roots",
    "export_progress",
    "fetch_rows",
    "filter_values",
    "find_duplicates",
    "grid_edit",
    "grid_lock",
    "grid_redo",
    "grid_state",
    "grid_undo",
    "import_files",
    "import_files_slice",
    "import_folder_playlist",
    "import_itunes",
    "import_itunes_selected",
    "import_usb",
    "import_xml",
    "intake_colours",
    "intake_counts",
    "intake_queue",
    "intake_set_prep_colour",
    "intake_set_tags",
    "intake_tag_panel",
    "itunes_default_library",
    "itunes_library_at",
    "library_problem",
    "library_protection",
    "library_summary",
    "link_load_track",
    "link_nudge_master",
    "link_peers",
    "link_set_master",
    "link_status",
    "link_take_master_tempo",
    "list_backups",
    "list_devices",
    "list_import_files",
    "log_lines",
    "log_status_message",
    "master_limiter",
    "missing_tracks",
    "move_cue",
    "move_hot_cue",
    "move_playlist",
    "open_backup_directory",
    "open_log",
    "open_log_folder",
    "open_log_window",
    "open_preferences",
    "open_sync_window",
    "open_view",
    "played_tracks",
    "playlist_duplicates",
    "playlist_entries",
    "playlist_tree",
    "preview_play",
    "preview_state",
    "preview_stop",
    "put_my_tag",
    "record_play",
    "reference_stick_settings",
    "rekordbox_browse_settings",
    "reload_library",
    "reload_tags",
    "relocate_by_location",
    "relocate_track",
    "relocation_targets",
    "remember_screen_assets",
    "remove_from_collection",
    "remove_from_history",
    "remove_from_tag_list",
    "remove_missing_tracks",
    "remove_tracks_from_playlist",
    "rename_my_tag",
    "rename_playlist",
    "reorder_playlist",
    "reorder_playlist_entries",
    "reset_play_count",
    "reset_played_tracks",
    "reveal_track",
    "run_edit_command",
    "run_menu_item",
    "script_preferences",
    "script_ready",
    "script_reply",
    "selection_details",
    "selection_my_tags",
    "service_activity",
    "service_problem",
    "set_active_loop",
    "set_audio_config",
    "set_audio_device",
    "set_channel_band",
    "set_channel_kill",
    "set_channel_trim",
    "set_crossfade",
    "set_cue_colour",
    "set_cue_comment",
    "set_eq_curve",
    "set_history_menu",
    "set_library_protection",
    "set_master_level",
    "set_master_limiter",
    "set_menu_enabled",
    "set_menu_labels",
    "set_metronome",
    "set_my_tags",
    "set_smart_rule",
    "set_track_color",
    "set_track_comment",
    "set_track_rating",
    "set_transport_edge",
    "show_window",
    "smart_rule",
    "sort_missing_tracks",
    "sort_playlist_items",
    "start_link_export",
    "startup_milestone",
    "stick_lock",
    "stick_outline",
    "stick_protection",
    "stick_unlock",
    "stick_view_dry_run",
    "stick_view_sticks",
    "stick_write_approve",
    "stick_write_plan",
    "stick_write_revoke",
    "stop_link_export",
    "switch_library",
    "test_eval_result",
    "theme_roles",
    "track_beats",
    "track_cues",
    "track_details",
    "track_lookups",
    "track_missing",
    "track_pcm_waveform",
    "track_phrases",
    "track_playlists",
    "track_vocals",
    "track_waveform",
    "unanalysed_tracks",
    "use_default_library",
    "validate_export_files",
    "view_ids_in_range",
    "write_my_tag_comments",
];

#[test]
fn every_registered_command_is_classified_and_every_stick_writer_passes_the_guard() {
    let registered = registered();
    assert!(
        registered.len() > 200,
        "found the commands: {}",
        registered.len()
    );
    for name in &registered {
        let writer = STICK_WRITERS.contains(&name.as_str());
        let not = NOT_STICK_WRITERS.contains(&name.as_str());
        assert!(writer != not, "`{name}` must be in exactly one of STICK_WRITERS (stick_guard.rs) and NOT_STICK_WRITERS (this test)");
    }
    for name in STICK_WRITERS.iter().chain(NOT_STICK_WRITERS) {
        assert!(
            registered.contains(*name),
            "`{name}` is classified but not registered"
        );
    }
    let sources = sources();
    let functions = functions(&sources);
    let unguarded = unguarded_writers(&functions);
    let commands: BTreeMap<&str, &str> = functions
        .iter()
        .filter(|f| f.command)
        .map(|f| (f.name.as_str(), f.text.as_str()))
        .collect();
    // Every helper that writes a Stick holds the guard or takes its capability.
    let uncapable = uncapable_helpers(&functions);
    assert!(
        uncapable.is_empty(),
        "these write a Stick without taking StickAccess or entering stick_guard: {uncapable:?}"
    );
    // Any other command that writes a file without the guard writes only the
    // app's own files, decided one by one: a new one must be looked at.
    let every_primitive: Vec<&str> = PRIMITIVES.iter().chain(FILE_PRIMITIVES).copied().collect();
    let file_writers = unguarded_writers_of(&functions, &every_primitive);
    let unguarded_files: BTreeMap<&str, &str> = registered
        .iter()
        .filter_map(|name| {
            file_writers
                .get(name)
                .map(|why| (name.as_str(), why.as_str()))
        })
        .collect();
    let decided: BTreeSet<&str> = APP_FILE_WRITERS.iter().copied().collect();
    let found: BTreeSet<&str> = unguarded_files.keys().copied().collect();
    assert_eq!(
        found.difference(&decided).collect::<Vec<_>>(),
        Vec::<&&str>::new(),
        "commands writing files without stick_guard: guard them, or add them to APP_FILE_WRITERS after checking they write only the app's own files: {unguarded_files:#?}"
    );
    assert_eq!(
        decided.difference(&found).collect::<Vec<_>>(),
        Vec::<&&str>::new(),
        "APP_FILE_WRITERS lists commands that no longer write"
    );
    for name in &registered {
        assert!(
            !unguarded.contains_key(name),
            "`{name}` reaches a Stick write without passing stick_guard: {}",
            unguarded[name]
        );
    }
    for name in STICK_WRITERS {
        let body = commands
            .get(name)
            .unwrap_or_else(|| panic!("`{name}` has no #[tauri::command] body"));
        // Guarded inside, after the Library edit gate (key tags; backups),
        // and followed there by the analyses above.
        let indirect = [
            "set_track_field",
            "undo_edit",
            "redo_edit",
            "analyse_track",
            "back_up_library",
            "delete_backup",
        ];
        assert!(
            body.contains("stick_guard::enter") || indirect.contains(name),
            "`{name}` must enter stick_guard before it writes"
        );
    }
    for name in STICK_WRITING_COMMANDS {
        let body = commands[name];
        let guard = body.find("stick_guard::enter").expect(name);
        let consume = body
            .find("stick_approval::consume(")
            .unwrap_or_else(|| panic!("`{name}` must consume its token"));
        assert!(
            guard < consume,
            "`{name}`: Stick Protection, under the lock, before the token"
        );
        assert!(STICK_WRITERS.contains(name), "`{name}`");
    }
}

#[test]
fn the_source_guard_catches_a_writer_behind_a_helper() {
    let source = |text: &str| vec![(PathBuf::from("x.rs"), text.to_owned())];
    let indirect = "#[tauri::command]\npub async fn sneaky(path: String) {\n    helper(&path);\n}\n\nfn helper(path: &str) {\n    deeper(path)\n}\n\nfn deeper(path: &str) {\n    write_range_wav(a, b, c, Path::new(path));\n}\n";
    let found = unguarded_writers(&functions(&source(indirect)));
    assert!(found.contains_key("sneaky"), "{found:?}");
    let guarded = indirect.replace(
        "    helper(&path);",
        "    let access = crate::stick_guard::enter(dir, Path::new(&path))?;\n    helper(&path);",
    );
    let found = unguarded_writers(&functions(&source(&guarded)));
    assert!(!found.contains_key("sneaky"), "{found:?}");
    assert!(
        uncapable_helpers(&functions(&source(&guarded))).contains(&"deeper".to_owned()),
        "a helper writing without the capability"
    );
    let capable = guarded.replace(
        "fn deeper(path: &str)",
        "fn deeper(access: &StickAccess, path: &str)",
    );
    assert!(!uncapable_helpers(&functions(&source(&capable))).contains(&"deeper".to_owned()));
    assert!(functions(&source(indirect))
        .iter()
        .any(|f| f.name == "sneaky" && f.command));
}

/// Round 4, B: a guard dropped before the write, or bound to `_`, covers
/// nothing; a write through a method is followed; a plain `std::fs::write`
/// is a file write the command must be classified for.
#[test]
fn the_source_guard_catches_a_dropped_guard_a_method_and_a_plain_file_write() {
    let source = |text: &str| vec![(PathBuf::from("x.rs"), text.to_owned())];
    let found = |text: &str| unguarded_writers(&functions(&source(text)));
    let dropped = "#[tauri::command]\npub fn early(path: String) {\n    drop(crate::stick_guard::enter(dir, Path::new(&path))?);\n    write_range_wav(a, b, c, Path::new(&path));\n}\n";
    assert!(found(dropped).contains_key("early"), "dropped at once");
    let named = "#[tauri::command]\npub fn early(path: String) {\n    let access = crate::stick_guard::enter(dir, Path::new(&path))?;\n    drop(access);\n    write_range_wav(a, b, c, Path::new(&path));\n}\n";
    assert!(
        found(named).contains_key("early"),
        "dropped by name before the write"
    );
    let underscore = "#[tauri::command]\npub fn early(path: String) {\n    let _ = crate::stick_guard::enter(dir, Path::new(&path))?;\n    write_range_wav(a, b, c, Path::new(&path));\n}\n";
    assert!(found(underscore).contains_key("early"), "bound to _");
    let after = "#[tauri::command]\npub fn late(path: String) {\n    write_range_wav(a, b, c, Path::new(&path));\n    let access = crate::stick_guard::enter(dir, Path::new(&path))?;\n}\n";
    assert!(found(after).contains_key("late"), "a guard after the write");
    let method = "#[tauri::command]\npub fn through(path: String) {\n    Saver.save(&path);\n}\n\nimpl Saver {\n    fn save(&self, path: &str) {\n        write_range_wav(a, b, c, Path::new(path));\n    }\n}\n";
    assert!(found(method).contains_key("through"), "{:?}", found(method));
    let plain = "#[tauri::command]\npub fn plain(destination: String) {\n    std::fs::write(destination, b\"x\").unwrap();\n}\n";
    let every: Vec<&str> = PRIMITIVES.iter().chain(FILE_PRIMITIVES).copied().collect();
    assert!(unguarded_writers_of(&functions(&source(plain)), &every).contains_key("plain"));
}

/// Round 4, D: one lock order everywhere, the Library edit gate before a
/// Stick's execution lock. An export to a Stick waits for the gate without
/// holding the Stick, so the Undo of a key edit on a file on that Stick
/// (which holds the gate, then takes the Stick) can never wait on it in a
/// circle. Controlled: this test holds the gate while both start.
#[test]
fn an_export_and_the_undo_of_a_key_edit_on_that_stick_never_wait_on_each_other() {
    use std::sync::mpsc;
    use std::time::Duration;
    let rig = rig();
    let handle = rig.app.handle().clone();
    let (audio, track) = track_on_stick(&rig);
    rig.unlock();
    run(details::set_track_field(
        handle.clone(),
        rig.app.state(),
        vec![track],
        "key".into(),
        "Fm".into(),
        Some(true),
    ))
    .unwrap();
    let token =
        rig.token(&serde_json::json!({ "kind": "exportPlaylist", "playlist": playlist_id(0) }));
    let state = rig.state();
    let (done, finished) = mpsc::channel();

    let gate = state.core().edit_gate().lock();
    let exporting = {
        let (handle, done, path) = (handle.clone(), done.clone(), rig.path());
        std::thread::spawn(move || {
            let result = run(commands::export_playlist(
                handle.clone(),
                handle.state(),
                playlist_id(0),
                path,
                None,
                None,
                None,
                token,
            ));
            done.send(("export", result.map(|_| ()).map_err(|e| e.to_string())))
                .unwrap();
        })
    };
    std::thread::sleep(Duration::from_millis(500));
    let free = wreckordbox_stick_protection::try_exec_lock(&rig.state_dir(), &rig.stick).unwrap();
    assert!(
        free.is_some(),
        "an export waiting for the Library holds no Stick"
    );
    drop(free);
    let undoing = {
        let (handle, done) = (handle.clone(), done.clone());
        std::thread::spawn(move || {
            let result = run(commands::undo_edit(handle.clone(), handle.state()));
            done.send(("undo", result.map(|_| ()).map_err(|e| e.to_string())))
                .unwrap();
        })
    };
    std::thread::sleep(Duration::from_millis(200));
    drop(gate);

    let mut outcomes = BTreeMap::new();
    for _ in 0..2 {
        let (what, result) = finished
            .recv_timeout(Duration::from_secs(60))
            .expect("both finish: no deadlock");
        outcomes.insert(what, result);
    }
    exporting.join().unwrap();
    undoing.join().unwrap();
    assert_eq!(outcomes["export"], Ok(()), "{outcomes:?}");
    assert_eq!(outcomes["undo"], Ok(()), "{outcomes:?}");
    assert_eq!(
        rbl_db::key_tag::read_key(&audio).unwrap().as_deref(),
        Some("Am"),
        "the Undo put the tag back"
    );
}

// ---------------------------------------------------------------------------
// The shared Stick write crate and the Library service (wbx-70 review, A)

/// What writes to a Stick from outside the window: the export, recovery
/// and settling, the Native stems writer.
const STICK_PRIMITIVES: &[&str] = &[
    "export_cancellable(",
    "export_with_options(",
    "rbl_export::export(",
    "rbl_export::recover(",
    "::settle(",
    "wreckordbox_native_stems::write(",
];

/// The service's guard: Stick Protection's authorization, bound and held.
const SERVICE_GUARDS: &[&str] = &[
    "wreckordbox_stick_protection::authorize(",
    "wreckordbox_stick_protection::authorize_stick(",
];

/// The service commands that write to a Stick. A new command is added here
/// or to `SERVICE_NOT_STICK_WRITERS`, after deciding which it is.
const SERVICE_STICK_WRITERS: &[&str] = &["stems.write", "sticks.load"];

const SERVICE_NOT_STICK_WRITERS: &[&str] = &[
    "decisions.answer", "decisions.create", "decisions.decide", "decisions.get", "decisions.list",
    "facts.kinds", "facts.of_track", "facts.record", "facts.standing",
    "history.list", "history.tracks",
    "intake.colours", "intake.counts", "intake.list", "intake.problems", "intake.set_tags", "intake.tag_panel",
    "intensity.cancel", "intensity.measure", "intensity.show",
    "mutants.clear", "mutants.create", "mutants.edit", "mutants.get", "mutants.list", "mutants.remove",
    "mytags.assign", "mytags.create", "mytags.create_category", "mytags.delete", "mytags.list",
    "mytags.merge", "mytags.move", "mytags.of_track", "mytags.rename", "mytags.reorder",
    "mytags.tracks", "mytags.unassign",
    "playlists.copy", "playlists.delete", "playlists.remove", "playlists.reorder", "playlists.tracks", "playlists.tree",
    "protection.get", "protection.set",
    "runs.active", "runs.finish", "runs.list", "runs.show", "runs.start",
    "schema.check", "schema.convert",
    "stems.separate", "stems.show",
    "sticks.dry_run", "sticks.list",
    "tracks.get", "tracks.list", "tracks.query", "tracks.rate", "tracks.remove_from_collection",
    "tracks.set_prep_colour",
    "status", "subscribe", "stop",
];

/// Every command name the service's sources declare: each `const` naming a
/// `group.command`, and the protocol's own commands.
fn service_commands(sources: &[(PathBuf, String)]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (path, text) in sources {
        for line in text.lines() {
            let line = line.trim();
            let Some(rest) = line
                .strip_prefix("pub const ")
                .or_else(|| line.strip_prefix("pub(crate) const "))
                .or_else(|| line.strip_prefix("const "))
            else {
                continue;
            };
            let Some((_, value)) = rest.split_once(": &str = \"") else {
                continue;
            };
            let Some(value) = value.strip_suffix("\";") else {
                continue;
            };
            let protocol = path.ends_with("protocol.rs")
                && matches!(value, "status" | "subscribe" | "stop");
            if protocol
                || (value.contains('.')
                    && value
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c == '_' || c == '.'))
            {
                out.insert(value.to_owned());
            }
        }
    }
    out
}

/// What writes anything at all: a Stick write, or any file written.
fn every_write() -> Vec<&'static str> {
    STICK_PRIMITIVES
        .iter()
        .chain(PRIMITIVES)
        .chain(FILE_PRIMITIVES)
        .copied()
        .collect()
}

/// The functions of `sources` that contain a write themselves.
fn writing_functions(sources: &[(PathBuf, String)]) -> Vec<(String, String)> {
    let every = every_write();
    functions(sources)
        .into_iter()
        .filter_map(|f| {
            every
                .iter()
                .find(|p| f.text.contains(**p))
                .map(|p| (f.name, (*p).to_owned()))
        })
        .collect()
}

#[test]
fn the_shared_stick_write_crate_never_writes() {
    let sources = crate_sources("wreckordbox-stick-write");
    assert!(sources.len() >= 4, "found the crate's sources");
    let writing = writing_functions(&sources);
    assert!(
        writing.is_empty(),
        "the shared planner, selection and confirmation must only read: {writing:?}"
    );
    // A write added to the planner is caught.
    let (path, plan) = sources
        .iter()
        .find(|(p, _)| p.ends_with("plan.rs"))
        .cloned()
        .unwrap();
    let mutated = plan.replacen(
        "let read_error =",
        "std::fs::write(destination.join(\"x\"), b\"\").ok();\n    let read_error =",
        1,
    );
    assert_ne!(mutated, plan, "the mutation applies");
    let caught = writing_functions(&[(path, mutated)]);
    assert!(
        caught.iter().any(|(name, _)| name == "export_plan_of"),
        "{caught:?}"
    );
}

#[test]
fn every_service_command_is_classified_and_every_stick_write_is_authorized() {
    let sources = crate_sources("wreckordbox-service");
    let declared = service_commands(&sources);
    assert!(declared.len() > 60, "found the commands: {declared:?}");
    for name in &declared {
        let writer = SERVICE_STICK_WRITERS.contains(&name.as_str());
        let not = SERVICE_NOT_STICK_WRITERS.contains(&name.as_str());
        assert!(
            writer != not,
            "service command `{name}` must be in exactly one of SERVICE_STICK_WRITERS and SERVICE_NOT_STICK_WRITERS"
        );
    }
    for name in SERVICE_STICK_WRITERS.iter().chain(SERVICE_NOT_STICK_WRITERS) {
        assert!(
            declared.contains(*name),
            "`{name}` is classified but not declared"
        );
    }
    // No function of the service reaches a Stick write outside a held
    // Stick Protection authorization, however many helpers away.
    let functions = functions(&sources);
    let unguarded = unguarded_writers_by(&functions, STICK_PRIMITIVES, SERVICE_GUARDS);
    assert!(
        unguarded.is_empty(),
        "these reach a Stick write without holding Stick Protection's authorization: {unguarded:#?}"
    );
    // Each writer's handler holds it before writing.
    for (command, handler) in [("sticks.load", "load"), ("stems.write", "write_one")] {
        assert!(
            functions
                .iter()
                .filter(|f| f.name == handler)
                .any(|f| SERVICE_GUARDS.iter().any(|g| f.text.contains(g))),
            "`{command}` ({handler}) must take Stick Protection's authorization"
        );
    }
    // The load with its authorization taken out is caught.
    let (path, text) = sources
        .iter()
        .find(|(p, _)| p.ends_with("stick_load/load.rs"))
        .cloned()
        .unwrap();
    let mutated = text.replacen(
        "wreckordbox_stick_protection::authorize_stick(state_dir, &root)",
        "unauthorized(state_dir, &root)",
        1,
    );
    assert_ne!(mutated, text, "the mutation applies");
    let caught = unguarded_writers_by(
        &self::functions(&[(path, mutated)]),
        STICK_PRIMITIVES,
        SERVICE_GUARDS,
    );
    assert!(caught.contains_key("load"), "{caught:?}");
}
