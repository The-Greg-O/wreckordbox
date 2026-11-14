//! The Stick view (wbx-71): the window lists Sticks, dry-runs a load and
//! loads through the Library service it hosts (`sticks.list`,
//! `sticks.dry_run`, `sticks.load`), never through a bare call: a load
//! needs the DJ's hold, which approves exactly the service plan the view
//! showed (its id and its bound result), passes Stick Protection, and the
//! dry run writes nothing. A hosted load never holds the window's hosting
//! lock, so closing the window reaches the load and stops it where it
//! safely can, with its report.
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
    location: rbl_db::LibraryLocation,
    audio: Vec<PathBuf>,
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
            playlists: 2,
            tracks_per_playlist: 3,
            history_sessions: 0,
            ..Shape::default()
        },
    )
    .unwrap();
    let folder = dir.path().join("audio");
    std::fs::create_dir_all(&folder).unwrap();
    let mut audio = Vec::new();
    for index in 0..3 {
        let file = folder.join(format!("Track {index}.wav"));
        write_wav(&file, 2);
        fixture::point_at_audio(&location, index, file.to_str().unwrap(), 2).unwrap();
        audio.push(file);
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
        location: location.clone(),
        audio,
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

    /// What the window's hold gives: the DJ confirmed the plan the view
    /// showed, so the window approves that plan id (`stick_write_approve`).
    fn held(&self, plan: &str) -> String {
        rbxport_lib::stick_approval::approve(&self.state_dir(), plan).unwrap()
    }

    /// A dry run's plan id.
    fn plan(&self, stick: &Path, playlists: &[String]) -> String {
        let dry = stick_view::dry_run(&self.state(), &path(stick), playlists).unwrap();
        dry["plan"].as_str().unwrap().to_owned()
    }
}

/// The window's load command, as the Stick view calls it.
fn load(
    window: &Window,
    stick: &str,
    playlists: &[String],
    plan: &str,
    approval: Option<&str>,
) -> Result<serde_json::Value, rbxport_lib::AppError> {
    load_from(window.app.handle(), stick, playlists, plan, approval)
}

fn load_from(
    handle: &tauri::AppHandle<MockRuntime>,
    stick: &str,
    playlists: &[String],
    plan: &str,
    approval: Option<&str>,
) -> Result<serde_json::Value, rbxport_lib::AppError> {
    tauri::async_runtime::block_on(stick_view::stick_view_load(
        handle.state(),
        stick.to_owned(),
        playlists.to_vec(),
        plan.to_owned(),
        approval.map(str::to_owned),
        None,
    ))
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

    let error =
        stick_view::dry_run(&window.state(), &path(&window.gig), &["no-such".into()]).unwrap_err();

    assert_eq!(
        error.detail.as_deref(),
        Some("playlist_not_found"),
        "{error:?}"
    );
}

#[test]
fn a_load_without_the_hold_is_refused_and_writes_nothing() {
    let window = hosting_window();
    window.unlock(&window.gig);
    let playlists = vec![playlist_id(0)];
    let plan = window.plan(&window.gig, &playlists);
    let before = bytes(&window.gig);

    let error = load(&window, &path(&window.gig), &playlists, &plan, None).unwrap_err();
    assert!(error.message.contains("not confirmed"), "{error:?}");

    // A hold given for another load's plan does not confirm this one.
    let other_plan = window.plan(&window.gig, &[playlist_id(1)]);
    let other = window.held(&other_plan);
    let error = load(
        &window,
        &path(&window.gig),
        &playlists,
        &plan,
        Some(other.as_str()),
    )
    .unwrap_err();
    assert_eq!(
        error.detail.as_deref(),
        Some("stick_load_unconfirmed"),
        "{error:?}"
    );
    assert_eq!(
        bytes(&window.gig),
        before,
        "a refused load wrote to the Stick"
    );
}

/// Four synthetic Stems for the track whose audio is `audio`, recorded in
/// the Library's Sidecar (beside `master.db`), as a separation would.
fn record_stems(window: &Window, track: &str, audio: &Path) {
    use wreckordbox_sidecar::stems::{NewStems, Role};
    use wreckordbox_sidecar::{Sidecar, Source, SourceKind};
    let frames = 2 * 44_100;
    let sidecar = Sidecar::open(&wreckordbox_sidecar::default_dir(
        &window.location.master_db,
    ))
    .unwrap();
    let source = Source::new(SourceKind::Tool, "separation-engine");
    let run = sidecar
        .begin_run(&source, "Synthetic Stems for a test")
        .unwrap();
    let workspace = sidecar.workspace(run).unwrap();
    let files: Vec<(Role, PathBuf)> = Role::ALL
        .into_iter()
        .zip([880.0, 660.0, 110.0, 220.0])
        .map(|(role, hertz)| {
            let path = workspace.join(format!("{role}.wav"));
            write_tone(&path, frames, hertz);
            (role, path)
        })
        .collect();
    let fingerprint = wreckordbox_sidecar::fingerprint_file(audio).unwrap();
    sidecar
        .record_stems(&NewStems {
            track_id: track,
            fingerprint: &fingerprint,
            parent_samples: u64::from(frames),
            sample_rate: 44_100,
            files: &files,
            source: &source,
            produced_by: json!({"engine": "test"}),
            run,
        })
        .unwrap();
    sidecar.finish_run(run, "recorded").unwrap();
}

fn write_tone(path: &Path, frames: u32, hertz: f64) {
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
    for frame in 0..frames {
        let t = f64::from(frame) / 44_100.0;
        let sample = ((t * hertz * std::f64::consts::TAU).sin() * 12_000.0) as i16;
        out.extend_from_slice(&sample.to_le_bytes());
        out.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}

#[test]
fn an_earlier_hold_does_not_confirm_a_fresh_plan_with_other_native_stems() {
    let window = hosting_window();
    window.unlock(&window.spare);
    let playlists = vec![playlist_id(0)];
    // The DJ is shown a load with no Native stems and holds for it.
    let first = stick_view::dry_run(&window.state(), &path(&window.spare), &playlists).unwrap();
    assert_eq!(first["changes"]["counts"]["native_stems"], 0, "{first}");
    let held = window.held(first["plan"].as_str().unwrap());
    // Then Stems arrive for a track (no Library edit) and a fresh dry run,
    // for the same playlists, has another result: one Native stems set.
    record_stems(&window, &fixture::track_id(0), &window.audio[0]);
    let fresh = stick_view::dry_run(&window.state(), &path(&window.spare), &playlists).unwrap();
    assert_eq!(fresh["changes"]["counts"]["native_stems"], 1, "{fresh}");
    let before = bytes(&window.spare);

    // The fresh plan's id with the earlier hold: refused, nothing written.
    let error = load(
        &window,
        &path(&window.spare),
        &playlists,
        fresh["plan"].as_str().unwrap(),
        Some(held.as_str()),
    )
    .unwrap_err();

    assert_eq!(
        error.detail.as_deref(),
        Some("stick_load_unconfirmed"),
        "{error:?}"
    );
    assert_eq!(
        bytes(&window.spare),
        before,
        "an unconfirmed plan was written"
    );
}

#[test]
fn a_hold_for_a_plan_whose_result_changed_is_refused() {
    let window = hosting_window();
    window.unlock(&window.spare);
    let playlists = vec![playlist_id(0)];
    let plan = window.plan(&window.spare, &playlists);
    let held = window.held(&plan);
    record_stems(&window, &fixture::track_id(0), &window.audio[0]);
    let before = bytes(&window.spare);

    let error = load(
        &window,
        &path(&window.spare),
        &playlists,
        &plan,
        Some(held.as_str()),
    )
    .unwrap_err();

    assert_eq!(
        error.detail.as_deref(),
        Some("stick_load_changed"),
        "{error:?}"
    );
    assert!(error.message.contains("native_stems"), "{error:?}");
    assert_eq!(bytes(&window.spare), before, "a changed plan was written");
}

#[test]
fn a_load_on_a_protected_stick_is_refused_even_after_the_hold() {
    let window = hosting_window();
    let playlists = vec![playlist_id(0)];
    let plan = window.plan(&window.gig, &playlists);
    let token = window.held(&plan);
    let before = bytes(&window.gig);

    let error = load(
        &window,
        &path(&window.gig),
        &playlists,
        &plan,
        Some(token.as_str()),
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
    let token = window.held(&plan);

    let loaded = load(
        &window,
        &path(&window.spare),
        &playlists,
        &plan,
        Some(token.as_str()),
    )
    .unwrap();

    assert_eq!(loaded["outcome"], "loaded", "{loaded}");
    assert_eq!(
        loaded["written"]["tracks"].as_array().unwrap().len(),
        3,
        "{loaded}"
    );
    assert_eq!(
        loaded["not_written"]["tracks"].as_array().unwrap().len(),
        0,
        "{loaded}"
    );
    assert!(
        window.spare.join("PIONEER").is_dir(),
        "the export is on the Stick"
    );
    // The hold is used up: the same load again is refused.
    let error = load(
        &window,
        &path(&window.spare),
        &playlists,
        &plan,
        Some(token.as_str()),
    )
    .unwrap_err();
    assert_eq!(
        error.detail.as_deref(),
        Some("stick_load_unconfirmed"),
        "{error:?}"
    );
}

#[test]
fn an_authorized_load_with_no_hosted_service_is_refused_and_writes_nothing() {
    let window = hosting_window();
    window.unlock(&window.spare);
    let playlists = vec![playlist_id(0)];
    let plan = window.plan(&window.spare, &playlists);
    let token = window.held(&plan);
    let before = bytes(&window.spare);
    // The window no longer hosts the service (another window took over).
    window.state().stop_service();

    let error = load(
        &window,
        &path(&window.spare),
        &playlists,
        &plan,
        Some(token.as_str()),
    )
    .unwrap_err();

    assert!(error.message.contains("Library service"), "{error:?}");
    assert_eq!(bytes(&window.spare), before, "a load ran with no service");
    assert!(!window.spare.join("PIONEER").exists());
}

#[test]
fn closing_the_window_stops_a_hosted_load_where_it_safely_can_and_waits_for_its_report() {
    use std::sync::mpsc;
    use std::time::Duration;
    let window = hosting_window();
    window.unlock(&window.spare);
    let playlists = vec![playlist_id(0)];
    let plan = window.plan(&window.spare, &playlists);
    let token = window.held(&plan);
    let state_dir = window.state_dir();
    let pause = state_dir.join(wreckordbox_service::stick_load::TEST_PAUSE_AT_FILE);
    std::fs::write(&pause, "copying").unwrap();

    let loading = {
        let handle = window.app.handle().clone();
        let stick = path(&window.spare);
        std::thread::spawn(move || {
            load_from(&handle, &stick, &playlists, &plan, Some(token.as_str()))
        })
    };
    let paused = state_dir.join(wreckordbox_service::stick_load::TEST_PAUSED_FILE);
    for _ in 0..3000 {
        if paused.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if !paused.exists() {
        let _ = std::fs::remove_file(&pause);
        panic!("the load did not reach copying: {:?}", loading.join());
    }

    // The window closes: stopping the service must reach the running load.
    let closing = {
        let state = window.state();
        std::thread::spawn(move || state.stop_service())
    };
    let (told, heard) = mpsc::channel();
    {
        let state = window.state();
        std::thread::spawn(move || {
            // Each look takes the hosting lock: it must be free while the load runs.
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while state.with_service(|_| ()).is_some() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            let _ = told.send(state.with_service(|_| ()).is_none());
        });
    }
    let reached = heard.recv_timeout(Duration::from_secs(5));
    // Let the paused load go whatever happened, so nothing hangs.
    let _ = std::fs::remove_file(&pause);
    assert_eq!(
        reached,
        Ok(true),
        "stopping the service waited on the hosting lock while the load ran"
    );

    let report = loading.join().unwrap().unwrap();
    closing.join().unwrap();
    assert_eq!(report["outcome"], "failed", "{report}");
    assert!(
        report["error"]["message"]
            .as_str()
            .unwrap()
            .contains("stopped"),
        "{report}"
    );
    assert_eq!(report["written"]["tracks"], json!([]), "{report}");
    assert!(
        !window.spare.join(".rbxport-publication").exists(),
        "no unfinished write is left"
    );
}

// ---- Closing the window during a load (wbx-71 verification review) -----

/// A load started on its own thread and paused at `stage` (the service's
/// test hook); returns the thread and the pause file that lets it go.
fn paused_load(
    window: &Window,
    stick: &Path,
    stage: &str,
) -> (
    std::thread::JoinHandle<Result<serde_json::Value, rbxport_lib::AppError>>,
    PathBuf,
) {
    use std::time::Duration;
    let playlists = vec![playlist_id(0)];
    let plan = window.plan(stick, &playlists);
    let token = window.held(&plan);
    let state_dir = window.state_dir();
    let pause = state_dir.join(wreckordbox_service::stick_load::TEST_PAUSE_AT_FILE);
    std::fs::write(&pause, stage).unwrap();
    let handle = window.app.handle().clone();
    let path = path(stick);
    let loading = std::thread::spawn(move || {
        load_from(&handle, &path, &playlists, &plan, Some(token.as_str()))
    });
    let paused = state_dir.join(wreckordbox_service::stick_load::TEST_PAUSED_FILE);
    for _ in 0..3000 {
        if paused.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if !paused.exists() {
        let _ = std::fs::remove_file(&pause);
        panic!("the load did not reach {stage}: {:?}", loading.join());
    }
    (loading, pause)
}

/// Whether `thread` is still running after a moment.
fn still_running<T>(thread: &std::thread::JoinHandle<T>) -> bool {
    std::thread::sleep(std::time::Duration::from_millis(600));
    !thread.is_finished()
}

#[test]
fn a_second_exit_request_during_publishing_is_held_and_the_final_exit_waits_for_the_load() {
    use rbxport_lib::shutdown;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let window = hosting_window();
    window.unlock(&window.spare);
    let (loading, pause) = paused_load(&window, &window.spare, "publishing");
    let state = window.state();
    let exits = Arc::new(AtomicUsize::new(0));
    // When the app is let exit, the load must be over and its report kept.
    let reports_at_exit = Arc::new(parking_lot::Mutex::new(None));
    let exit = {
        let exits = Arc::clone(&exits);
        let reports_at_exit = Arc::clone(&reports_at_exit);
        let state_dir = window.state_dir();
        move || {
            *reports_at_exit.lock() =
                Some(rbxport_lib::stick_load_reports::kept_reports(&state_dir).len());
            exits.fetch_add(1, Ordering::SeqCst);
        }
    };

    let first = shutdown::exit_requested(&state, true, exit.clone());
    let second = shutdown::exit_requested(&state, true, exit);
    // The final Exit (a restart's, or one the OS forces) cannot be prevented: it waits.
    let final_exit = {
        let state = Arc::clone(&state);
        std::thread::spawn(move || shutdown::on_exit(&state))
    };
    let waited = still_running(&final_exit);
    let _ = std::fs::remove_file(&pause);

    assert!(first, "the first exit request is held while the load runs");
    assert!(second, "a second exit request is held too");
    assert!(
        waited,
        "the final Exit returned while the load was still publishing"
    );
    let report = loading.join().unwrap().unwrap();
    final_exit.join().unwrap();
    for _ in 0..500 {
        if exits.load(Ordering::SeqCst) > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        exits.load(Ordering::SeqCst),
        1,
        "the app exits once, when drained"
    );
    assert_eq!(
        *reports_at_exit.lock(),
        Some(1),
        "the report was kept before the exit"
    );
    assert!(report["outcome"].is_string(), "{report}");
    assert!(!window.spare.join(".rbxport-publication").exists());
}

#[test]
fn a_restart_waits_for_a_publishing_load_and_keeps_its_report() {
    use rbxport_lib::shutdown;
    let window = hosting_window();
    window.unlock(&window.spare);
    let (loading, pause) = paused_load(&window, &window.spare, "publishing");
    let state = window.state();
    let state_dir = window.state_dir();
    let restarted = Arc::new(parking_lot::Mutex::new(None));

    let restarting = {
        let state = Arc::clone(&state);
        let restarted = Arc::clone(&restarted);
        std::thread::spawn(move || {
            shutdown::drain_then(&state, || {
                *restarted.lock() =
                    Some(rbxport_lib::stick_load_reports::kept_reports(&state_dir).len());
            });
        })
    };
    let waited = still_running(&restarting);
    let _ = std::fs::remove_file(&pause);

    assert!(
        waited,
        "the restart went ahead while the load was publishing"
    );
    loading.join().unwrap().unwrap();
    restarting.join().unwrap();
    assert_eq!(
        *restarted.lock(),
        Some(1),
        "restarted once, with the report kept"
    );
}

/// Closes the window (exit asked, drained, exited) during a load paused at
/// `stage`; answers the reports a reopened window finds.
fn close_during(window: &Window, stick: &Path, stage: &str) -> Vec<serde_json::Value> {
    use rbxport_lib::shutdown;
    let (loading, pause) = paused_load(window, stick, stage);
    let state = window.state();
    let (exited, heard) = std::sync::mpsc::channel();
    assert!(shutdown::exit_requested(&state, true, move || {
        let _ = exited.send(());
    }));
    let _ = std::fs::remove_file(&pause);
    let _ = loading.join().unwrap();
    heard
        .recv_timeout(std::time::Duration::from_secs(60))
        .expect("the app exits once the load is over");
    shutdown::on_exit(&state);
    // Reopened: a new window over the same state folder.
    let reopened = AppState::with_backups(window.state_dir());
    rbxport_lib::stick_load_reports::kept_reports(reopened.backup_dir())
}

#[test]
fn closing_during_copying_keeps_the_full_report_for_the_next_start() {
    let window = hosting_window();
    window.unlock(&window.spare);

    let reports = close_during(&window, &window.spare, "copying");

    assert_eq!(reports.len(), 1, "{reports:?}");
    let saved = &reports[0];
    assert!(
        saved["id"].as_str().is_some_and(|id| !id.is_empty()),
        "{saved}"
    );
    assert_eq!(saved["stick"]["path"], path(&window.spare), "{saved}");
    assert!(saved["stick"]["volume"].is_string(), "{saved}");
    let report = &saved["report"];
    assert_eq!(report["outcome"], "failed", "{saved}");
    assert!(
        report["error"]["message"]
            .as_str()
            .unwrap()
            .contains("stopped"),
        "{saved}"
    );
    assert_eq!(
        report["not_written"]["tracks"].as_array().unwrap().len(),
        3,
        "{saved}"
    );
    assert_eq!(report["written"]["tracks"], json!([]), "{saved}");
    // Kept off the Stick and out of the Library.
    assert!(!bytes(&window.spare)
        .keys()
        .any(|p| p.to_string_lossy().contains("report")));

    // Dismissed, it is gone for good.
    let id = saved["id"].as_str().unwrap();
    rbxport_lib::stick_load_reports::dismiss_report(&window.state_dir(), id).unwrap();
    assert!(rbxport_lib::stick_load_reports::kept_reports(&window.state_dir()).is_empty());
}

#[test]
fn closing_after_publication_before_native_stems_finish_keeps_every_stems_outcome() {
    let window = hosting_window();
    window.unlock(&window.spare);
    // Two Native stems sets: the window closes while the first is written,
    // after the audio was published; the second is never begun.
    record_stems(&window, &fixture::track_id(0), &window.audio[0]);
    record_stems(&window, &fixture::track_id(1), &window.audio[1]);

    let reports = close_during(&window, &window.spare, "native_stems_verified");

    assert_eq!(reports.len(), 1, "{reports:?}");
    let report = &reports[0]["report"];
    assert_ne!(report["outcome"], "loaded", "{report}");
    assert_eq!(
        report["written"]["tracks"].as_array().unwrap().len(),
        3,
        "{report}"
    );
    let stems = report["native_stems"].as_array().unwrap();
    assert_eq!(
        stems.len(),
        2,
        "every Native stems outcome is kept: {report}"
    );
    let unwritten: Vec<_> = stems.iter().filter(|s| s["outcome"] != "written").collect();
    assert_eq!(unwritten.len(), 1, "{report}");
    assert!(
        unwritten[0]["reason"]
            .as_str()
            .is_some_and(|r| r.contains("stopped")),
        "{report}"
    );
}
