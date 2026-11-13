//! The Stick view (wbx-71): the window lists Sticks, dry-runs a load and
//! loads through the Library service it hosts (`sticks.list`,
//! `sticks.dry_run`, `sticks.load`), never through a bare call: a load
//! needs the DJ's hold (the window's token for exactly this sync) and
//! passes Stick Protection, and the dry run writes nothing.
//!
//! A fixture Library and temporary folders standing in for Sticks; nothing
//! here can reach the installed Library or a mounted Stick.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rbl_db::fixture::{self, playlist_id, Shape};
use rbxport_lib::service_host;
use rbxport_lib::state::AppState;
use rbxport_lib::stick_view;
use serde_json::json;
use tauri::test::MockRuntime;
use tauri::Manager;
use wreckordbox_service::Config;
use wreckordbox_stick_protection::{SPARE_MARKER, STAND_IN_MARKER};

struct Window {
    dir: tempfile::TempDir,
    app: tauri::App<MockRuntime>,
    _sticks: tempfile::TempDir,
    gig: PathBuf,
    spare: PathBuf,
}

/// A silent 8 kHz mono 16-bit WAV file.
fn write_wav(path: &Path, seconds: u32) {
    let rate: u32 = 8_000;
    let data_len = rate * 2 * seconds;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.resize(44 + data_len as usize, 0);
    std::fs::write(path, out).unwrap();
}

/// A Library of three tracks with real audio, all in one playlist, hosted
/// by a mock window; a Gig stick and a Spare stick (temporary folders).
fn hosting_window() -> Window {
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    let location = fixture::build(
        &library,
        Shape {
            tracks: 3,
            playlists: 1,
            tracks_per_playlist: 3,
            history_sessions: 0,
            ..Shape::default()
        },
    )
    .unwrap();
    let audio = dir.path().join("audio");
    std::fs::create_dir_all(&audio).unwrap();
    for index in 0..3 {
        let file = audio.join(format!("Track {index}.wav"));
        write_wav(&file, 2);
        fixture::point_at_audio(&location, index, file.to_str().unwrap(), 2).unwrap();
    }
    let state = AppState::with_backups(dir.path().join("backups"));
    state.core().set_protection(false).unwrap();
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (index, _) = rbl_index::load(&db).unwrap();
    state.set_library(index, false, db.schema().db_version, 0, location.clone());
    let app = tauri::test::mock_app();
    app.manage(Arc::new(state));
    let sticks = tempfile::tempdir().unwrap();
    let gig = sticks.path().join("GIGSTICK");
    let spare = sticks.path().join("SPARE");
    for stick in [&gig, &spare] {
        std::fs::create_dir_all(stick.join("Contents")).unwrap();
        std::fs::write(stick.join("Contents/keep.txt"), b"the DJ's").unwrap();
        std::fs::write(stick.join(STAND_IN_MARKER), b"").unwrap();
    }
    std::fs::write(spare.join(SPARE_MARKER), b"").unwrap();
    let window = Window {
        dir,
        app,
        _sticks: sticks,
        gig,
        spare,
    };
    let config = Config {
        runtime_dir: window.dir.path().join("run"),
        idle_exit: None,
        recovery_dir: window.dir.path().join("backups"),
    };
    service_host::host_with(window.app.handle(), &location, &config).unwrap();
    window
}

impl Window {
    fn state(&self) -> Arc<AppState> {
        Arc::clone(self.app.state::<Arc<AppState>>().inner())
    }

    fn state_dir(&self) -> PathBuf {
        self.state().backup_dir().to_path_buf()
    }

    fn unlock(&self, stick: &Path) {
        let name = stick.file_name().unwrap().to_str().unwrap();
        wreckordbox_stick_protection::unlock(&self.state_dir(), stick, Some(name)).unwrap();
    }

    /// What the window's hold gives: its token for exactly this sync, the
    /// way the hold dialog gets one (`stick_write_plan`, then approve).
    fn held(&self, stick: &Path, playlists: &[String]) -> String {
        let request = serde_json::from_value(json!({
            "destination": stick.to_string_lossy(),
            "operation": { "kind": "sync", "playlists": playlists },
        }))
        .unwrap();
        let plan = rbxport_lib::stick_write_plan::plan(&self.state(), &request)
            .unwrap()
            .plan
            .expect("a Stick write is planned");
        rbxport_lib::stick_approval::approve(&self.state_dir(), &plan).unwrap()
    }
}

fn path(stick: &Path) -> String {
    stick.to_string_lossy().into_owned()
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

fn kind(error: &rbxport_lib::AppError) -> String {
    serde_json::to_value(error.kind)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn a_dry_run_says_what_the_load_will_do_and_writes_nothing() {
    let window = hosting_window();
    let before = bytes(&window.gig);
    let playlists = vec![playlist_id(0)];

    let plan = stick_view::dry_run(&window.state(), &path(&window.gig), &playlists).unwrap();

    assert_eq!(plan["stick"]["name"], "GIGSTICK");
    assert_eq!(plan["stick"]["kind"], "gig");
    assert_eq!(plan["stick"]["protected"], true);
    assert_eq!(plan["changes"]["counts"]["tracks_added"], 3, "{plan}");
    assert_eq!(plan["changes"]["counts"]["playlists_created"], 1, "{plan}");
    assert!(plan["plan"].is_string(), "a plan id: {plan}");
    assert!(plan["space"]["needed_bytes"].as_u64().unwrap() > 0);
    assert_eq!(bytes(&window.gig), before, "a dry run wrote to the Stick");
}

#[test]
fn a_dry_run_of_an_unknown_playlist_says_so() {
    let window = hosting_window();

    let error = stick_view::dry_run(&window.state(), &path(&window.gig), &["no-such".into()])
        .unwrap_err();

    assert_eq!(error.detail.as_deref(), Some("playlist_not_found"), "{error:?}");
}

#[test]
fn a_load_without_the_hold_is_refused_and_writes_nothing() {
    let window = hosting_window();
    window.unlock(&window.gig);
    let playlists = vec![playlist_id(0)];
    let plan = stick_view::dry_run(&window.state(), &path(&window.gig), &playlists).unwrap();
    let plan = plan["plan"].as_str().unwrap().to_owned();
    let before = bytes(&window.gig);

    let error = stick_view::load(&window.state(), &path(&window.gig), &playlists, &plan, None)
        .unwrap_err();
    assert!(error.message.contains("not confirmed"), "{error:?}");

    // A hold given for other playlists does not confirm this load either.
    let other = window.held(&window.gig, &[]);
    let error = stick_view::load(
        &window.state(),
        &path(&window.gig),
        &playlists,
        &plan,
        Some(other),
    )
    .unwrap_err();
    assert!(error.message.contains("different write"), "{error:?}");
    assert_eq!(bytes(&window.gig), before, "a refused load wrote to the Stick");
}

#[test]
fn a_load_on_a_protected_stick_is_refused_even_after_the_hold() {
    let window = hosting_window();
    let playlists = vec![playlist_id(0)];
    let plan = stick_view::dry_run(&window.state(), &path(&window.gig), &playlists).unwrap();
    let plan = plan["plan"].as_str().unwrap().to_owned();
    let token = window.held(&window.gig, &playlists);
    let before = bytes(&window.gig);

    let error = stick_view::load(
        &window.state(),
        &path(&window.gig),
        &playlists,
        &plan,
        Some(token),
    )
    .unwrap_err();

    assert_eq!(kind(&error), "stickProtected", "{error:?}");
    assert!(error.message.contains("GIGSTICK"), "{error:?}");
    assert_eq!(bytes(&window.gig), before, "a protected Stick was written");
}

#[test]
fn a_held_load_on_an_unlocked_stick_writes_what_its_dry_run_said() {
    let window = hosting_window();
    window.unlock(&window.spare);
    let playlists = vec![playlist_id(0)];
    let dry = stick_view::dry_run(&window.state(), &path(&window.spare), &playlists).unwrap();
    assert_eq!(dry["stick"]["kind"], "spare");
    assert_eq!(dry["stick"]["protected"], false);
    let plan = dry["plan"].as_str().unwrap().to_owned();
    let token = window.held(&window.spare, &playlists);

    let load = stick_view::load(
        &window.state(),
        &path(&window.spare),
        &playlists,
        &plan,
        Some(token),
    )
    .unwrap();

    assert_eq!(load["outcome"], "loaded", "{load}");
    assert_eq!(load["written"]["tracks"].as_array().unwrap().len(), 3, "{load}");
    assert_eq!(load["not_written"]["tracks"].as_array().unwrap().len(), 0, "{load}");
    assert!(window.spare.join("PIONEER").is_dir(), "the export is on the Stick");
    // The plan is used up: the same load again is refused.
    let again = window.held(&window.spare, &playlists);
    let error = stick_view::load(
        &window.state(),
        &path(&window.spare),
        &playlists,
        &plan,
        Some(again),
    )
    .unwrap_err();
    assert_eq!(error.detail.as_deref(), Some("stick_load_unconfirmed"), "{error:?}");
}

#[test]
fn a_load_with_no_hosted_service_is_an_error_not_a_bare_write() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(AppState::with_backups(dir.path().join("backups")));

    let error = stick_view::sticks(&state).unwrap_err();

    assert!(error.message.contains("Library service"), "{error:?}");
}
