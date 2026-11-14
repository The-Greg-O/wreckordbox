//! Splitting a track's Stems from the Intake view (wbx-67): the window asks
//! the Library service it hosts, hears the separation's progress, and reads
//! whether a track has current, stale or no Stems without writing anything.
//!
//! A test binary of its own: the separation engine's command comes from the
//! environment, which is the process's. The engine is the separation
//! engine's fake worker (`separation-engine/tests/fake_worker.py`, a real
//! worker over a fake backend); no real separation is ever run. Fixture
//! Libraries in temporary folders; nothing here can reach the installed
//! Library or a Stick.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use rbl_db::fixture::{self, Shape};
use rbxport_lib::intake_stems::{self, STEMS_PROGRESS_EVENT};
use rbxport_lib::service_host;
use rbxport_lib::state::AppState;
use serde_json::{json, Value};
use tauri::test::MockRuntime;
use tauri::{Listener, Manager};
use wreckordbox_service::stems::SEPARATION_COMMAND_ENV;
use wreckordbox_service::Config;

/// The fake engine's log of every separation it was asked for, shared by
/// the whole process (the engine's command is set once).
fn engine_log() -> &'static Path {
    static LOG: OnceLock<PathBuf> = OnceLock::new();
    LOG.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap().keep();
        let log = dir.join("engine.log");
        let fake =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../separation-engine/tests/fake_worker.py");
        std::env::set_var(
            SEPARATION_COMMAND_ENV,
            json!([
                "python3",
                fake.to_str().unwrap(),
                "--log",
                log.to_str().unwrap(),
                "--delay",
                "1"
            ])
            .to_string(),
        );
        log
    })
}

/// How many separations the engine was asked for, of the file `audio`.
fn engine_runs(audio: &Path) -> usize {
    std::fs::read_to_string(engine_log())
        .unwrap_or_default()
        .lines()
        .filter(|line| line.contains(audio.file_name().unwrap().to_str().unwrap()))
        .count()
}

/// A silent 16-bit stereo 44.1 kHz WAV file of `frames` sample frames.
fn write_audio(path: &Path, frames: u32, value: u8) {
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
    out.resize(44 + data_len as usize, value);
    std::fs::write(path, out).unwrap();
}

struct Window {
    dir: tempfile::TempDir,
    app: tauri::App<MockRuntime>,
    /// Two tracks with audio files, then the fixture's three without.
    tracks: Vec<(String, PathBuf)>,
    no_audio: String,
}

impl Window {
    fn state(&self) -> Arc<AppState> {
        Arc::clone(self.app.state::<Arc<AppState>>().inner())
    }

    fn sidecar(&self) -> PathBuf {
        self.dir.path().join("library").join("wreckordbox-sidecar")
    }

    /// Every file under the Sidecar's folder with its length and time.
    fn sidecar_files(&self) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
        fn walk(dir: &Path, out: &mut Vec<(PathBuf, u64, std::time::SystemTime)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let meta = entry.metadata().unwrap();
                if meta.is_dir() {
                    walk(&path, out);
                } else {
                    out.push((path, meta.len(), meta.modified().unwrap()));
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.sidecar(), &mut out);
        out.sort();
        out
    }
}

/// A fixture Library of three tracks plus two imported WAV files, read by
/// a mock app's state as the window reads it, and hosted.
fn hosting_window() -> Window {
    engine_log();
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    let location = fixture::build(
        &library,
        Shape {
            tracks: 3,
            ..Shape::default()
        },
    )
    .unwrap();
    let mut tracks = Vec::new();
    {
        let mut writer =
            rbl_db::write::Writer::open(location.clone(), dir.path().join("backups")).unwrap();
        writer.disable_automatic_backups();
        for name in ["first", "second"] {
            // Unique per test, so the shared engine log tells tests apart.
            let audio = dir.path().join(format!(
                "{name}-{}.wav",
                dir.path().file_name().unwrap().to_str().unwrap()
            ));
            write_audio(&audio, 2_205, 0);
            let id = writer.import_file(&audio).unwrap();
            tracks.push((id.to_string(), audio));
        }
    }
    let state = AppState::with_backups(dir.path().join("backups"));
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (index, _) = rbl_index::load(&db).unwrap();
    state.set_library(index, false, db.schema().db_version, 0, location.clone());
    let app = tauri::test::mock_app();
    app.manage(Arc::new(state));
    let config = Config {
        runtime_dir: dir.path().join("run"),
        idle_exit: None,
        recovery_dir: dir.path().join("backups"),
    };
    service_host::host_with(app.handle(), &location, &config).unwrap();
    Window {
        dir,
        app,
        tracks,
        no_audio: fixture::track_id(1),
    }
}

/// The progress events the window was sent, as they arrived.
fn hear_progress(window: &Window) -> Arc<Mutex<Vec<Value>>> {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&heard);
    window.app.listen_any(STEMS_PROGRESS_EVENT, move |event| {
        sink.lock()
            .unwrap()
            .push(serde_json::from_str(event.payload()).unwrap());
    });
    heard
}

#[test]
fn a_track_never_split_has_no_stems_and_reading_that_writes_nothing() {
    let window = hosting_window();
    let (track, _) = &window.tracks[0];

    let status = intake_stems::status(&window.state(), track).unwrap();

    assert_eq!(status["state"], "none", "{status}");
    assert!(status["separating"].is_null(), "{status}");
    assert!(
        !window.sidecar().exists(),
        "reading a track's Stems made no Sidecar"
    );
}

#[test]
fn a_split_from_the_window_reports_progress_and_then_the_track_has_stems() {
    let window = hosting_window();
    let (track, audio) = window.tracks[0].clone();
    let heard = hear_progress(&window);

    let outcome =
        intake_stems::separate(window.app.handle(), &window.state(), &track, false).unwrap();

    assert_eq!(outcome["ok"], true, "{outcome}");
    assert_eq!(outcome["status"]["state"], "current", "{outcome}");
    assert_eq!(engine_runs(&audio), 1);
    let heard = heard.lock().unwrap().clone();
    assert!(
        !heard.is_empty(),
        "the window heard the separation's progress"
    );
    assert!(heard.iter().all(|event| event["trackId"] == track.as_str()));
    let fractions: Vec<f64> = heard
        .iter()
        .map(|event| event["fraction"].as_f64().unwrap())
        .collect();
    assert!(
        fractions.windows(2).all(|pair| pair[0] <= pair[1]),
        "{fractions:?}"
    );
    // The fake reports every 50 ms for a second; the window is told at most
    // a few times a second.
    assert!(heard.len() <= 8, "progress was coalesced: {}", heard.len());

    let status = intake_stems::status(&window.state(), &track).unwrap();
    assert_eq!(status["state"], "current", "{status}");
    assert!(status["made_at"].as_i64().unwrap() > 0, "{status}");
}

#[test]
fn asking_twice_for_one_track_separates_it_once() {
    let window = hosting_window();
    let (track, audio) = window.tracks[0].clone();
    let state = window.state();

    let handle = window.app.handle().clone();
    let (first_state, first_track) = (Arc::clone(&state), track.clone());
    let first = std::thread::spawn(move || {
        intake_stems::separate(&handle, &first_state, &first_track, false)
    });
    // The first is running: the status says so, to any reader.
    let mut separating = Value::Null;
    for _ in 0..100 {
        separating = intake_stems::status(&state, &track).unwrap()["separating"].clone();
        if !separating.is_null() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(separating, track.as_str());
    let second = intake_stems::separate(window.app.handle(), &state, &track, false).unwrap();
    let first = first.join().unwrap().unwrap();

    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(second["ok"], true, "{second}");
    assert_eq!(engine_runs(&audio), 1, "one separation for both requests");
    // And once there are Stems, asking again uses them.
    let third = intake_stems::separate(window.app.handle(), &state, &track, false).unwrap();
    assert_eq!(third["reused"], true, "{third}");
    assert_eq!(engine_runs(&audio), 1);
}

#[test]
fn current_stems_are_split_again_only_when_asked_to() {
    let window = hosting_window();
    let (track, audio) = window.tracks[0].clone();
    let state = window.state();
    let first = intake_stems::separate(window.app.handle(), &state, &track, false).unwrap();
    let made_at = first["status"]["made_at"].as_i64().unwrap();
    std::thread::sleep(Duration::from_millis(5));

    let reused = intake_stems::separate(window.app.handle(), &state, &track, false).unwrap();
    assert_eq!(reused["reused"], true, "{reused}");
    assert_eq!(engine_runs(&audio), 1);

    let again = intake_stems::separate(window.app.handle(), &state, &track, true).unwrap();
    assert_eq!(again["ok"], true, "{again}");
    assert_eq!(again["reused"], false, "{again}");
    assert_eq!(engine_runs(&audio), 2, "split again when asked to");
    assert_eq!(again["status"]["state"], "current", "{again}");
    assert!(
        again["status"]["made_at"].as_i64().unwrap() > made_at,
        "{again}"
    );
}

#[test]
fn another_track_asked_for_while_one_is_split_is_told_it_is_busy() {
    let window = hosting_window();
    let (first, _) = window.tracks[0].clone();
    let (second, second_audio) = window.tracks[1].clone();
    let state = window.state();
    let handle = window.app.handle().clone();
    let running_state = Arc::clone(&state);
    let running =
        std::thread::spawn(move || intake_stems::separate(&handle, &running_state, &first, false));
    for _ in 0..100 {
        if !intake_stems::status(&state, &second).unwrap()["separating"].is_null() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let busy = intake_stems::separate(window.app.handle(), &state, &second, false).unwrap();

    assert_eq!(busy["ok"], false, "{busy}");
    assert_eq!(busy["error"]["code"], "separation_busy", "{busy}");
    assert_eq!(engine_runs(&second_audio), 0);
    assert_eq!(running.join().unwrap().unwrap()["ok"], true);
}

#[test]
fn a_failed_split_answers_its_reason_and_records_nothing() {
    let window = hosting_window();
    let track = window.no_audio.clone();

    let failed =
        intake_stems::separate(window.app.handle(), &window.state(), &track, false).unwrap();

    assert_eq!(failed["ok"], false, "{failed}");
    assert_eq!(failed["error"]["code"], "track_file_missing", "{failed}");
    assert!(
        failed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("missing"),
        "{failed}"
    );
    let status = intake_stems::status(&window.state(), &track).unwrap();
    assert_eq!(status["state"], "none", "{status}");
    assert!(
        status["unavailable"]["code"] == "track_file_missing",
        "{status}"
    );
}

#[test]
fn stems_of_audio_since_changed_read_as_stale_and_reading_marks_nothing() {
    let window = hosting_window();
    let (track, audio) = window.tracks[0].clone();
    let state = window.state();
    let made = intake_stems::separate(window.app.handle(), &state, &track, false).unwrap();
    assert_eq!(made["ok"], true, "{made}");

    // The audio file is replaced by other audio.
    write_audio(&audio, 2_205, 7);
    let before = window.sidecar_files();
    let status = intake_stems::status(&state, &track).unwrap();
    let again = intake_stems::status(&state, &track).unwrap();

    assert_eq!(status["state"], "stale", "{status}");
    assert_eq!(again["state"], "stale", "{again}");
    assert!(status["made_at"].as_i64().unwrap() > 0, "{status}");
    assert_eq!(window.sidecar_files(), before, "a read wrote nothing");
}

#[test]
fn a_window_that_hosts_no_service_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(AppState::with_backups(dir.path().join("backups")));
    let app = tauri::test::mock_app();

    let error = intake_stems::status(&state, "1").unwrap_err();
    assert!(
        error.message.contains("Library service"),
        "{}",
        error.message
    );
    let error = intake_stems::separate(app.handle(), &state, "1", false).unwrap_err();
    assert!(
        error.message.contains("Library service"),
        "{}",
        error.message
    );
}

// ---------------------------------------------------------------------------
// A Sidecar kept on a Stick (wbx-45.14): a temporary folder carrying the
// stand-in marker stands for the Stick.

impl Window {
    /// Puts the Sidecar's folder on a Stick; unlocked when `unlocked`.
    fn sidecar_on_stick(&self, unlocked: bool) {
        let sidecar = self.sidecar();
        std::fs::create_dir_all(&sidecar).unwrap();
        std::fs::write(sidecar.join(wreckordbox_stick_protection::STAND_IN_MARKER), b"").unwrap();
        if unlocked {
            let name = sidecar.file_name().unwrap().to_str().unwrap().to_owned();
            wreckordbox_stick_protection::unlock(&self.dir.path().join("backups"), &sidecar, Some(&name)).unwrap();
        }
    }
}

#[test]
fn the_windows_split_is_refused_for_a_sidecar_on_a_stick_with_or_without_again() {
    let window = hosting_window();
    window.sidecar_on_stick(true);
    let (track, audio) = window.tracks[0].clone();
    let before = window.sidecar_files();
    for again in [false, true] {
        let answer =
            intake_stems::separate(window.app.handle(), &window.state(), &track, again).unwrap();
        assert_eq!(answer["ok"], false, "again {again}: {answer}");
        assert_eq!(answer["error"]["code"], "sidecar_on_stick", "again {again}: {answer}");
    }
    assert_eq!(engine_runs(&audio), 0, "the engine was never asked");
    assert_eq!(window.sidecar_files(), before, "the Sidecar on the Stick is untouched");
}

#[test]
fn stems_status_reads_a_sidecar_on_a_stick_without_writing_or_being_refused() {
    let window = hosting_window();
    let (track, _) = window.tracks[0].clone();
    // A Sidecar holding this track's Stems, made while it was on this
    // computer; then its folder is a Stick (protected, then unlocked).
    let split = intake_stems::separate(window.app.handle(), &window.state(), &track, false).unwrap();
    assert_eq!(split["ok"], true, "{split}");
    for unlocked in [false, true] {
        window.sidecar_on_stick(unlocked);
        let before = window.sidecar_files();
        let status = intake_stems::status(&window.state(), &track).unwrap();
        assert_eq!(status["unavailable"], Value::Null, "unlocked {unlocked}: {status}");
        assert_eq!(status["state"], "current", "unlocked {unlocked}: {status}");
        assert_eq!(window.sidecar_files(), before, "unlocked {unlocked}: nothing written");
    }
}

#[test]
fn a_split_asked_for_once_the_window_is_closing_is_refused_and_runs_nothing() {
    let window = hosting_window();
    let (track, audio) = window.tracks[0].clone();
    rbxport_lib::shutdown::on_exit(&window.state());
    let error = intake_stems::separate(window.app.handle(), &window.state(), &track, false)
        .expect_err("no service command once closing");
    assert!(error.message.contains("Library service"), "{}", error.message);
    assert_eq!(engine_runs(&audio), 0);
}
