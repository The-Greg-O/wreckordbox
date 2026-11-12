//! The window as the host of the Library service: the shell's state on a
//! fixture Library with a mock app in place of the window, and the real
//! `wreckordbox` command run against it.
//!
//! Nothing here can reach the installed Library or the user's runtime
//! directory: the command is given the fixture and a private folder.
#![allow(clippy::pedantic, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use rbl_db::fixture::{self, track_id, Shape};
use rbxport_lib::state::AppState;
use rbxport_lib::{commands, service_host, ErrorKind};
use tauri::test::MockRuntime;
use tauri::{Listener, Manager, State};
use wreckordbox_service::{codes, Client, Config, Service};

/// The `wreckordbox` binary of this build, built once if it is not there.
fn wreckordbox() -> &'static PathBuf {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let built = Command::new(cargo).args(["build", "-p", "wreckordbox"]).status().expect("run cargo");
        assert!(built.success(), "could not build the wreckordbox command");
        // target/<profile>/deps/<this test> -> target/<profile>/wreckordbox
        let deps = std::env::current_exe().unwrap();
        let binary = deps.parent().unwrap().parent().unwrap().join("wreckordbox");
        assert!(binary.is_file(), "no {}", binary.display());
        binary
    })
}

/// What the interface would have been sent, in order.
type Heard<T> = Arc<Mutex<Vec<T>>>;

struct Window {
    dir: tempfile::TempDir,
    app: tauri::App<MockRuntime>,
    location: rbl_db::LibraryLocation,
    /// Every `library:changed` generation the interface would have seen.
    changes: Arc<Mutex<Vec<u32>>>,
    /// Every `service:stopped` message the interface would have shown.
    stopped: Arc<Mutex<Vec<String>>>,
    /// Every `edit-history:changed` the interface would have seen: what the
    /// Edit menu's Undo and Redo items are set from.
    history: Heard<serde_json::Value>,
}

struct Run {
    ok: bool,
    stdout: String,
    stderr: String,
}

impl Run {
    fn json(&self) -> serde_json::Value {
        assert!(self.ok, "{}", self.stderr);
        serde_json::from_str(&self.stdout).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.stdout))
    }
}

/// A fixture Library and a mock app whose state has read it, as
/// `spawn_library_load` reads the real one. Not hosting yet.
fn window() -> Window {
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    let location = fixture::build(&library, Shape { tracks: 5, ..Shape::default() }).unwrap();
    fixture::write_options_json(&dir.path().join("options.json"), location.master_db.to_str().unwrap(), &location.passphrase).unwrap();
    std::fs::create_dir_all(dir.path().join("home")).unwrap();
    let (app, changes, stopped) = app_on(&dir, &location);
    let history: Heard<serde_json::Value> = Arc::default();
    let seen = Arc::clone(&history);
    app.listen("edit-history:changed", move |event| {
        seen.lock().unwrap().push(serde_json::from_str(event.payload()).unwrap());
    });
    // As the app does at start: a change to the history that no command of
    // the window's made is announced to the interface.
    let handle = app.handle().clone();
    app.state::<Arc<AppState>>().on_history_changed(move || {
        let _ = tauri::Emitter::emit(&handle, "edit-history:changed", commands::current_history(handle.state::<Arc<AppState>>().inner()));
    });
    // As the app does at start: every turn of Library Protection reaches
    // the interface.
    service_host::announce_protection(app.handle());
    let window = Window { dir, app, location, changes, stopped, history };
    // As on the machine of a DJ who edits: Library Protection is off.
    window.protect(false);
    window
}

/// A window that is hosting the service and has not read the Library yet,
/// as the app is between starting and showing its tracks.
fn loading_window() -> Window {
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    let location = fixture::build(&library, Shape { tracks: 5, ..Shape::default() }).unwrap();
    fixture::write_options_json(&dir.path().join("options.json"), location.master_db.to_str().unwrap(), &location.passphrase).unwrap();
    std::fs::create_dir_all(dir.path().join("home")).unwrap();
    let app = tauri::test::mock_app();
    app.manage(Arc::new(AppState::with_backups(dir.path().join("backups"))));
    service_host::announce_protection(app.handle());
    let window = Window { dir, app, location, changes: Arc::default(), stopped: Arc::default(), history: Arc::default() };
    window.protect(false);
    window.host().unwrap();
    window
}

fn app_on(
    dir: &tempfile::TempDir,
    location: &rbl_db::LibraryLocation,
) -> (tauri::App<MockRuntime>, Heard<u32>, Heard<String>) {
    let state = AppState::with_backups(dir.path().join("backups"));
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (index, _) = rbl_index::load(&db).unwrap();
    state.set_library(index, false, db.schema().db_version, 0, location.clone());
    let app = tauri::test::mock_app();
    app.manage(Arc::new(state));

    let changes: Arc<Mutex<Vec<u32>>> = Arc::default();
    let seen = Arc::clone(&changes);
    app.listen("library:changed", move |event| {
        seen.lock().unwrap().push(serde_json::from_str::<u32>(event.payload()).unwrap());
    });
    let stopped: Arc<Mutex<Vec<String>>> = Arc::default();
    let told = Arc::clone(&stopped);
    app.listen(service_host::SERVICE_STOPPED, move |event| {
        told.lock().unwrap().push(serde_json::from_str::<String>(event.payload()).unwrap());
    });
    (app, changes, stopped)
}

fn finished(output: std::process::Output) -> Run {
    Run {
        ok: output.status.success(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

fn run<T>(fut: impl std::future::Future<Output = T>) -> T {
    tauri::async_runtime::block_on(fut)
}

/// Waits for `ready`, which another thread brings about.
fn eventually(what: &str, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

impl Window {
    fn config(&self) -> Config {
        Config { runtime_dir: self.dir.path().join("run"), idle_exit: None, recovery_dir: self.dir.path().join("backups") }
    }

    /// Makes the window the host, as the app does before it reads the Library.
    fn host(&self) -> Result<(), wreckordbox_service::ServiceError> {
        service_host::host_with(self.app.handle(), &self.location, &self.config())
    }

    /// Turns Library Protection on or off through the real command the
    /// Preferences window calls.
    fn protect(&self, on: bool) {
        let now = run(service_host::set_library_protection(self.state(), on)).unwrap();
        assert_eq!(now.protect_library, on);
    }

    /// Every Library Protection change from now on that the interface would
    /// have been sent. An earlier change's announcement may still be on its
    /// way: it is older than what stands now, so it is left out.
    fn heard_protection(&self) -> Heard<service_host::ProtectionDto> {
        let heard: Heard<service_host::ProtectionDto> = Arc::default();
        let told = Arc::clone(&heard);
        let standing = service_host::library_protection(self.state()).revision;
        self.app.listen(service_host::PROTECTION_CHANGED, move |event| {
            let now: service_host::ProtectionDto = serde_json::from_str(event.payload()).unwrap();
            if now.revision > standing {
                told.lock().unwrap().push(now);
            }
        });
        heard
    }

    /// Reads the Library, as the app does once it is hosting.
    fn load(&self) {
        let db = rbl_db::Library::open(self.location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
        let (index, _) = rbl_index::load(&db).unwrap();
        self.state().set_library(index, false, db.schema().db_version, 0, self.location.clone());
    }

    fn state(&self) -> State<'_, Arc<AppState>> {
        self.app.state::<Arc<AppState>>()
    }

    fn rating(&self, id: &str) -> u8 {
        let index = self.state().library().unwrap();
        index.rating[index.row_of(id).unwrap() as usize]
    }

    fn generation(&self) -> u32 {
        self.state().summary().3
    }

    fn client(&self) -> Client {
        Client::connect(&self.config().socket_path(&self.location)).unwrap()
    }

    /// The real command, with this window's Library and runtime directory
    /// and nothing else of the machine's.
    fn cli(&self, args: &[&str]) -> Run {
        self.cli_waiting(None, args)
    }

    /// The command, told how many seconds to wait for a window that is
    /// still reading the Library (its own bound when `None`).
    fn cli_waiting(&self, loading_wait_secs: Option<u32>, args: &[&str]) -> Run {
        finished(self.command(loading_wait_secs, args).output().unwrap())
    }

    fn command(&self, loading_wait_secs: Option<u32>, args: &[&str]) -> Command {
        let mut command = Command::new(wreckordbox());
        command
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.dir.path().join("home"))
            .env("RBXPORT_TEST", "1")
            .env("RBXPORT_OPTIONS", self.dir.path().join("options.json"))
            .env("RBXPORT_TEST_FIXTURE", self.dir.path().join("library"))
            .env("WRECKORDBOX_RUNTIME_DIR", self.dir.path().join("run"))
            .env("WRECKORDBOX_IDLE_EXIT_SECS", "30")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .args(args);
        if let Some(secs) = loading_wait_secs {
            command.env("WRECKORDBOX_LOADING_WAIT_SECS", secs.to_string());
        }
        command
    }

    /// The name of a track's Prep colour as the command shows it, `null`
    /// when it has none.
    fn prep_colour(&self, id: &str) -> serde_json::Value {
        self.cli(&["--json", "tracks", "show", id]).json()["prep_colour"].clone()
    }

    fn listed_rating(&self, id: &str) -> u64 {
        let listed = self.cli(&["--json", "tracks", "list"]).json();
        listed["tracks"].as_array().unwrap().iter().find(|track| track["id"] == id).unwrap()["rating"].as_u64().unwrap()
    }
}

#[test]
fn the_command_works_on_the_open_windows_library() {
    let window = window();
    window.host().unwrap();

    let status = window.cli(&["--json", "status"]).json();

    assert_eq!(status["pid"], std::process::id(), "the window's own process answers");
    assert_eq!(status["library"]["tracks"], 5);
    // A read the index does not answer by itself (the My Tag catalogue is
    // read from the database) is served too.
    assert!(window.cli(&["--json", "mytags", "list"]).ok);
}

#[test]
fn unreadable_run_status_does_not_mean_the_service_stopped() {
    for kind in ["corrupt", "newer", "locked"] {
        let window = window();
        window.host().unwrap();
        let dir = window
            .location
            .master_db
            .parent()
            .unwrap()
            .join("wreckordbox-sidecar");
        std::fs::create_dir_all(dir.join("work")).unwrap();
        std::fs::write(dir.join("work/keep"), b"keep").unwrap();
        let lock = if kind == "corrupt" {
            std::fs::write(dir.join("sidecar.db"), b"corrupt fixture").unwrap();
            None
        } else {
            let db = rusqlite::Connection::open(dir.join("sidecar.db")).unwrap();
            db.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT); INSERT INTO meta VALUES ('schema_version', '999');").unwrap();
            if kind == "locked" {
                db.execute_batch("UPDATE meta SET value = '6'; CREATE TABLE run (id INTEGER PRIMARY KEY, description TEXT, finished_at INTEGER); CREATE INDEX run_unfinished ON run (id) WHERE finished_at IS NULL; INSERT INTO run VALUES (1, 'Existing', NULL); BEGIN EXCLUSIVE; UPDATE run SET description = 'Uncommitted';").unwrap();
            }
            Some(db)
        };
        let start = Instant::now();
        let status = tauri::async_runtime::block_on(rbxport_lib::service_status::service_activity(
            window.app.state(),
        ))
        .unwrap();
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "{kind}: status waited behind a writer"
        );
        let status = serde_json::to_value(status).unwrap();
        assert_eq!(status["connected"], true, "{kind}");
        assert_eq!(status["runsUnavailable"], true, "{kind}");
        assert_eq!(status["problem"], serde_json::Value::Null);
        assert!(dir.join("work/keep").exists());
        assert!(window.cli(&["--json", "tracks", "list"]).ok);
        drop(lock);
    }
}

#[test]
fn active_runs_follow_cli_work_in_the_host_and_a_second_client() {
    let window = window();
    let snapshot = || {
        tauri::async_runtime::block_on(rbxport_lib::service_status::service_activity(
            window.app.state(),
        ))
        .unwrap()
    };
    assert!(!snapshot().connected);
    window.host().unwrap();
    let generation = window.generation();
    let active = || {
        let snapshot = snapshot();
        assert!(snapshot.connected);
        assert!(snapshot.problem.is_none());
        serde_json::to_value(snapshot.runs).unwrap()
    };
    assert_eq!(active(), serde_json::json!([]));
    assert!(
        !window
            .location
            .master_db
            .parent()
            .unwrap()
            .join("wreckordbox-sidecar")
            .exists(),
        "reading status must not create a Sidecar"
    );
    let mut client = window.client();
    client.subscribe().unwrap();
    let first = window
        .cli(&[
            "--json",
            "runs",
            "start",
            "--source",
            "agent:tagger",
            "--description",
            "Tag the backlog",
        ])
        .json();
    let second = window
        .cli(&[
            "--json",
            "runs",
            "start",
            "--source",
            "tool:meter",
            "--description",
            "Measure Intensity",
        ])
        .json();
    let expected = serde_json::json!([
        { "id": first["id"], "description": "Tag the backlog" },
        { "id": second["id"], "description": "Measure Intensity" }
    ]);
    assert_eq!(active(), expected);
    assert_eq!(
        client
            .call::<serde_json::Value>("runs.active", serde_json::json!({}))
            .unwrap(),
        expected
    );
    window
        .cli(&["--json", "runs", "finish", &first["id"].to_string()])
        .json();
    assert_eq!(
        active(),
        serde_json::json!([{ "id": second["id"], "description": "Measure Intensity" }])
    );
    window
        .cli(&["--json", "runs", "finish", &second["id"].to_string()])
        .json();
    assert_eq!(active(), serde_json::json!([]));
    assert_eq!(window.generation(), generation);
    assert!(window.changes.lock().unwrap().is_empty());
    window.state().with_service(Service::stop).unwrap();
    eventually("service:stopped", || {
        !window.stopped.lock().unwrap().is_empty()
    });
    let stopped = snapshot();
    assert!(!stopped.connected);
    assert_eq!(
        stopped.problem.as_deref(),
        Some(service_host::STOPPED_MESSAGE)
    );
    assert!(stopped.runs.is_empty());
    window.state().stop_service();
    window.host().unwrap();
    assert!(snapshot().connected);
    assert!(snapshot().problem.is_none());
}

#[test]
fn a_rating_set_from_the_command_shows_in_the_window_as_a_library_change() {
    let window = window();
    window.host().unwrap();
    let id = track_id(1);
    let before = window.generation();

    let rated = window.cli(&["--json", "tracks", "rate", &id, "4"]).json();

    assert_eq!(window.rating(&id), 4, "the window's own index has the edit, with no reload");
    assert_eq!(rated["generation"], window.generation(), "one generation for both");
    assert_eq!(window.generation(), before + 1);
    // The interface is told with the event its own edits send, once.
    eventually("library:changed", || !window.changes.lock().unwrap().is_empty());
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(*window.changes.lock().unwrap(), [before + 1]);
}

#[test]
fn a_mytag_put_on_a_track_from_the_command_shows_in_the_window_as_a_library_change() {
    let window = window();
    window.host().unwrap();
    let id = track_id(1);
    let before = window.generation();

    let assigned = window.cli(&["--json", "mytags", "assign", fixture::MY_TAG_PEAK, &id]).json();

    assert_eq!(assigned["changed"], 1);
    let index = window.state().library().unwrap();
    let carried = index.my_tag_keys(index.row_of(&id).unwrap() as usize);
    assert_eq!(carried, [rbl_index::smart::my_tag_key(fixture::MY_TAG_PEAK)], "the window's own index has the tag, with no reload");
    assert_eq!(assigned["generation"], window.generation(), "one generation for both");
    // The interface is told with the event its own edits send, once.
    eventually("library:changed", || !window.changes.lock().unwrap().is_empty());
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(*window.changes.lock().unwrap(), [window.generation()]);
    assert!(window.generation() > before);
}

#[test]
fn a_rating_set_in_the_window_is_what_the_command_lists_next_and_a_client_hears_of_it() {
    let window = window();
    window.host().unwrap();
    let id = track_id(2);
    let mut watching = window.client();
    watching.subscribe().unwrap();

    run(commands::set_track_rating(window.app.handle().clone(), window.state(), vec![id.clone()], 3)).unwrap();

    assert_eq!(window.listed_rating(&id), 3);
    let heard = watching.next_event(Duration::from_secs(5)).unwrap().unwrap();
    assert_eq!(heard.event, "library_changed");
    assert_eq!(heard.data, serde_json::json!({ "kind": "library", "generation": window.generation() }));
    // The window told its interface itself; it is not told a second time.
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(*window.changes.lock().unwrap(), [window.generation()]);

    // Either side's edit is one more step of the same sequence.
    let rated = window.cli(&["--json", "tracks", "rate", &id, "5"]).json();
    assert_eq!(rated["previous_rating"], 3);
    assert_eq!(rated["generation"], window.generation());
}

#[test]
fn a_service_a_command_started_by_itself_hands_the_library_to_the_window() {
    let window = window();
    let alone = window.cli(&["--json", "status"]).json();
    assert_ne!(alone["pid"], std::process::id());

    window.host().unwrap();

    let hosted = window.cli(&["--json", "status"]).json();
    assert_eq!(hosted["pid"], std::process::id());
    let pid = alone["pid"].as_u64().unwrap();
    eventually("the service that ran by itself to exit", || !std::path::Path::new(&format!("/proc/{pid}")).exists());
    // The next command's edit is the window's.
    let id = track_id(0);
    window.cli(&["tracks", "rate", &id, "2"]).succeeded();
    assert_eq!(window.rating(&id), 2);
}

#[test]
fn a_second_window_on_the_same_library_cannot_write_and_says_so() {
    let first = window();
    first.host().unwrap();
    let (app, _, stopped) = app_on(&first.dir, &first.location);

    let refused = service_host::host_with(app.handle(), &first.location, &first.config()).unwrap_err();

    assert_eq!(refused.code, codes::STOP_REFUSED);
    assert!(stopped.lock().unwrap()[0].contains("Another wreckordbox window"), "{stopped:?}");
    let state = app.state::<Arc<AppState>>();
    let edit = run(commands::set_track_rating(app.handle().clone(), state.clone(), vec![track_id(1)], 5)).unwrap_err();
    assert_eq!(edit.kind, ErrorKind::ReadOnly);
    assert!(edit.message.contains("Another wreckordbox window"), "{}", edit.message);
    assert_eq!(state.service_problem().as_deref(), Some(edit.message.as_str()));
    // The first window is untouched and still the writer.
    assert_eq!(first.rating(&track_id(1)), 0);
    assert_eq!(first.state().service_problem(), None);
    first.cli(&["tracks", "rate", &track_id(1), "1"]).succeeded();
    assert_eq!(first.rating(&track_id(1)), 1);
}

/// wbx-43 review C: a second window on the same Library is not its writer,
/// so it cannot turn Library Protection through its own core (whose edit
/// gate the writer's edits do not take, and whose turns the writer's
/// interface would not hear). It is told why; the switch, the first
/// window's interface and the first window's edits are untouched. Its
/// start-up adoption writes nothing either.
#[test]
fn a_window_that_is_not_the_writer_cannot_turn_library_protection_and_says_so() {
    let first = window();
    first.host().unwrap();
    let (app, _, _) = app_on(&first.dir, &first.location);
    service_host::host_with(app.handle(), &first.location, &first.config()).unwrap_err();
    let second = || app.state::<Arc<AppState>>();
    let heard = first.heard_protection();
    let recorded = service_host::library_protection(first.state());
    assert!(!recorded.protect_library);

    let refused = run(service_host::set_library_protection(second(), true)).unwrap_err();

    assert_eq!(refused.kind, ErrorKind::ReadOnly);
    assert!(refused.message.contains("cannot turn Library Protection"), "{}", refused.message);
    assert!(refused.message.contains("wreckordbox protection"), "{}", refused.message);
    assert_eq!(service_host::library_protection(first.state()), recorded);
    assert_eq!(first.cli(&["--json", "protection"]).json()["protect_library"], false);
    first.cli(&["tracks", "rate", &track_id(1), "2"]).succeeded();
    assert_eq!(first.rating(&track_id(1)), 2);

    // With nothing recorded, its stored preference is not taken up either.
    std::fs::remove_file(first.dir.path().join("backups/library-protection.json")).unwrap();
    let answered = run(service_host::adopt_library_protection(second(), false)).unwrap();
    assert!(answered.protect_library);
    assert!(!first.dir.path().join("backups/library-protection.json").exists());

    // The writer's own switch still turns, and its interface hears only that.
    first.protect(false);
    eventually("library-protection:changed", || !heard.lock().unwrap().is_empty());
    let heard = heard.lock().unwrap().clone();
    assert_eq!(heard.len(), 1, "{heard:?}");
    assert!(!heard[0].protect_library);
}

#[test]
fn the_dj_is_told_when_the_service_stops_and_nothing_more_is_written() {
    let window = window();
    window.host().unwrap();
    let id = track_id(1);

    window.state().with_service(Service::stop).unwrap();

    eventually("service:stopped", || !window.stopped.lock().unwrap().is_empty());
    assert_eq!(*window.stopped.lock().unwrap(), [service_host::STOPPED_MESSAGE]);
    let edit = run(commands::set_track_rating(window.app.handle().clone(), window.state(), vec![id.clone()], 5)).unwrap_err();
    assert_eq!(edit.kind, ErrorKind::ReadOnly);
    assert_eq!(edit.message, service_host::STOPPED_MESSAGE);
    assert_eq!(window.rating(&id), 0);
}

#[test]
fn closing_the_window_lets_go_of_the_library_without_alarm() {
    let window = window();
    window.host().unwrap();

    window.state().stop_service();

    std::thread::sleep(Duration::from_millis(200));
    assert!(window.stopped.lock().unwrap().is_empty());
    // A command run next starts a service of its own.
    let status = window.cli(&["--json", "status"]).json();
    assert_ne!(status["pid"], std::process::id());
    let _ = Command::new("kill").arg(status["pid"].to_string()).status();
}

#[test]
fn a_command_sent_while_the_window_is_still_reading_the_library_waits_and_succeeds() {
    let window = loading_window();
    let id = track_id(1);

    let mut command = window.command(None, &["--json", "tracks", "rate", &id, "4"]).spawn().unwrap();
    // Long enough that the command has been answered "loading" first.
    std::thread::sleep(Duration::from_millis(700));
    assert!(command.try_wait().unwrap().is_none(), "the command did not wait for the Library");
    window.load();
    let rated = finished(command.wait_with_output().unwrap());

    assert_eq!(rated.json()["rating"], 4);
    assert_eq!(window.rating(&id), 4);
}

/// A MyTag edit reads the My Tag catalogue from the database before it
/// edits: that read waits for a loading window too, rather than answering
/// that there is no Library.
#[test]
fn a_mytag_edit_sent_while_the_window_is_still_reading_the_library_waits_and_succeeds() {
    let window = loading_window();
    let id = track_id(1);
    let immediate = window.cli_waiting(Some(0), &["--json", "mytags", "assign", fixture::MY_TAG_PEAK, &id]);
    assert!(!immediate.ok);
    let error: serde_json::Value = serde_json::from_str(&immediate.stderr).unwrap();
    assert_eq!(error["error"]["code"], codes::LIBRARY_LOADING);

    let mut command = window.command(None, &["--json", "mytags", "assign", fixture::MY_TAG_PEAK, &id]).spawn().unwrap();
    std::thread::sleep(Duration::from_millis(700));
    assert!(command.try_wait().unwrap().is_none(), "the command did not wait for the Library");
    window.load();
    let assigned = finished(command.wait_with_output().unwrap());

    assert_eq!(assigned.json()["changed"], 1);
    let index = window.state().library().unwrap();
    assert_eq!(index.my_tag_keys(index.row_of(&id).unwrap() as usize).len(), 1);
}

#[test]
fn a_command_gives_up_with_library_loading_when_the_window_takes_too_long() {
    let window = loading_window();
    wreckordbox();
    let started = Instant::now();

    let run = window.cli_waiting(Some(1), &["--json", "tracks", "list"]);

    assert!(!run.ok);
    let error: serde_json::Value = serde_json::from_str(&run.stderr).unwrap();
    assert_eq!(error["error"]["code"], codes::LIBRARY_LOADING);
    let waited = started.elapsed();
    assert!(waited >= Duration::from_secs(1), "gave up after {waited:?}, before its bound");
    assert!(waited < Duration::from_secs(5), "took {waited:?} to give up on a bound of 1 s");
}

#[test]
fn a_change_from_the_command_is_refused_while_the_windows_library_protection_is_on() {
    let window = window();
    window.host().unwrap();
    window.protect(true);
    let id = track_id(1);
    let before = window.generation();

    let rate = window.cli(&["--json", "tracks", "rate", &id, "4"]);
    let assign = window.cli(&["--json", "mytags", "assign", fixture::MY_TAG_PEAK, &id]);
    let colour = window.cli(&["--json", "intake", "colour", &id, "Green"]);

    for run in [&rate, &assign, &colour] {
        assert!(!run.ok);
        let error: serde_json::Value = serde_json::from_str(&run.stderr).unwrap();
        assert_eq!(error["error"]["code"], "library_protected");
        // The window's own words (`refusal(true)` in `src/lib/menu.ts`).
        assert_eq!(error["error"]["message"], "Editing is locked by Library Protection. Turn it off in Preferences to edit.");
    }
    assert_eq!(window.rating(&id), 0);
    let index = window.state().library().unwrap();
    assert!(index.my_tag_keys(index.row_of(&id).unwrap() as usize).is_empty());
    assert_eq!(window.prep_colour(&id), serde_json::Value::Null);
    assert_eq!(window.generation(), before);
    // Nothing for the window's Undo to take back, either.
    assert!(!commands::current_history(window.state().inner()).can_undo);
    std::thread::sleep(Duration::from_millis(100));
    assert!(window.changes.lock().unwrap().is_empty(), "the interface was told of a change that was refused");
    // Reading is untouched.
    assert_eq!(window.listed_rating(&id), 0);
    assert!(window.cli(&["mytags", "list"]).ok);

    // Off again, the same commands are done.
    window.protect(false);
    window.cli(&["tracks", "rate", &id, "4"]).succeeded();
    window.cli(&["mytags", "assign", fixture::MY_TAG_PEAK, &id]).succeeded();
    window.cli(&["intake", "colour", &id, "Green"]).succeeded();
    assert_eq!(window.rating(&id), 4);
    let index = window.state().library().unwrap();
    assert_eq!(index.my_tag_keys(index.row_of(&id).unwrap() as usize).len(), 1);
    assert_eq!(window.prep_colour(&id), "Green");
}

#[test]
fn protection_turned_off_from_the_command_reaches_the_interface_and_edits_go_through() {
    let window = window();
    window.host().unwrap();
    window.protect(true);
    let heard = window.heard_protection();
    let id = track_id(1);
    assert!(!window.cli(&["tracks", "rate", &id, "4"]).ok);
    let on = service_host::library_protection(window.state());

    let off = window.cli(&["--json", "protection", "off"]).json();

    assert_eq!(off["protect_library"], false);
    assert_eq!(off["previous"], true);
    assert!(off["revision"].as_u64().unwrap() > on.revision);
    // The interface shows what the core recorded, with its revision.
    eventually("library-protection:changed", || !heard.lock().unwrap().is_empty());
    assert_eq!(
        *heard.lock().unwrap(),
        [service_host::ProtectionDto { protect_library: false, revision: off["revision"].as_u64().unwrap() }]
    );
    window.cli(&["tracks", "rate", &id, "4"]).succeeded();
    assert_eq!(window.rating(&id), 4);
}

/// The window's side, end to end: the DJ turns the switch with the real
/// command Preferences calls, and the real `wreckordbox` command obeys it.
#[test]
fn the_switch_the_dj_turns_in_preferences_is_the_one_the_command_obeys() {
    let window = window();
    window.host().unwrap();
    let heard = window.heard_protection();
    let id = track_id(1);

    let on = run(service_host::set_library_protection(window.state(), true)).unwrap();

    assert!(on.protect_library);
    assert_eq!(service_host::library_protection(window.state()), on);
    assert_eq!(window.cli(&["--json", "protection"]).json()["protect_library"], true);
    let refused = window.cli(&["--json", "tracks", "rate", &id, "4"]);
    assert!(!refused.ok);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&refused.stderr).unwrap()["error"]["code"], "library_protected");
    assert_eq!(window.rating(&id), 0);

    let off = run(service_host::set_library_protection(window.state(), false)).unwrap();
    window.cli(&["tracks", "rate", &id, "4"]).succeeded();
    assert_eq!(window.rating(&id), 4);
    eventually("library-protection:changed", || heard.lock().unwrap().len() == 2);
    assert_eq!(*heard.lock().unwrap(), [on, off]);
    assert!(on.revision < off.revision);
}

/// The window's own Rust commands are refused by the core, not only by the
/// interface: an edit, and Undo and Redo of edits made before protection
/// was turned on.
#[test]
fn the_windows_own_edits_and_undo_and_redo_are_refused_while_protection_is_on() {
    let window = window();
    window.host().unwrap();
    let id = track_id(1);
    let app = || window.app.handle().clone();
    run(commands::set_track_rating(app(), window.state(), vec![id.clone()], 3)).unwrap();
    run(commands::set_track_rating(app(), window.state(), vec![id.clone()], 5)).unwrap();
    run(commands::undo_edit(app(), window.state())).unwrap();
    assert_eq!(window.rating(&id), 3);
    window.protect(true);
    let generation = window.generation();

    let edit = run(commands::set_track_rating(app(), window.state(), vec![id.clone()], 1)).unwrap_err();
    let undo = run(commands::undo_edit(app(), window.state())).unwrap_err();
    let redo = run(commands::redo_edit(app(), window.state())).unwrap_err();

    for error in [&edit, &undo, &redo] {
        assert_eq!(error.message, "Editing is locked by Library Protection. Turn it off in Preferences to edit.");
    }
    assert_eq!(window.rating(&id), 3);
    assert_eq!(window.generation(), generation);
    // Both steps are still there for when protection is off.
    let history = commands::current_history(window.state().inner());
    assert!(history.can_undo && history.can_redo);

    window.protect(false);
    run(commands::redo_edit(app(), window.state())).unwrap();
    assert_eq!(window.rating(&id), 5);
}

#[test]
fn on_a_machine_with_nothing_recorded_protection_is_on_until_the_windows_preference_is_taken_up_once() {
    let window = window();
    std::fs::remove_file(window.dir.path().join("backups/library-protection.json")).unwrap();
    window.host().unwrap();
    let id = track_id(1);

    assert!(service_host::library_protection(window.state()).protect_library);
    assert!(!window.cli(&["tracks", "rate", &id, "4"]).ok);

    // The window's first start: its stored preference becomes the switch.
    let adopted = run(service_host::adopt_library_protection(window.state(), false)).unwrap();
    assert!(!adopted.protect_library);
    window.cli(&["tracks", "rate", &id, "4"]).succeeded();

    // A later start with a stale preference changes nothing.
    let again = run(service_host::adopt_library_protection(window.state(), true)).unwrap();
    assert_eq!(again, adopted);
}

#[test]
fn a_restarted_window_keeps_what_was_recorded_whatever_its_preference_says() {
    let window = window();
    window.protect(true);

    // The app again, on the same state folder.
    let restarted = AppState::with_backups(window.dir.path().join("backups"));
    let now = restarted.core().protection();
    assert!(now.protect_library);
    let adopted = restarted.core().adopt_protection(false).unwrap();
    assert!(adopted.protect_library);
}

/// wbx-43 review D, as the window sees it: after a restart the window shows
/// the recorded revision; the record is then damaged, and the DJ's next
/// turn is told to the interface with a revision beyond the one it shows,
/// so the interface applies it rather than dropping it as stale.
#[test]
fn a_turn_after_the_record_was_damaged_reaches_a_window_that_showed_a_higher_revision() {
    let window = window();
    for on in [true, false, true, false, true] {
        window.protect(on);
    }
    // The app again, on the same state folder, hosting.
    let (app, _, _) = app_on(&window.dir, &window.location);
    service_host::announce_protection(app.handle());
    service_host::host_with(app.handle(), &window.location, &window.config()).unwrap();
    let restarted = || app.state::<Arc<AppState>>();
    let shown = service_host::library_protection(restarted());
    assert!(shown.protect_library);
    assert!(shown.revision >= 6, "{shown:?}");
    let heard: Heard<service_host::ProtectionDto> = Arc::default();
    let told = Arc::clone(&heard);
    app.listen(service_host::PROTECTION_CHANGED, move |event| {
        told.lock().unwrap().push(serde_json::from_str(event.payload()).unwrap());
    });
    std::fs::write(window.dir.path().join("backups/library-protection.json"), b"{damaged").unwrap();

    let off = run(service_host::set_library_protection(restarted(), false)).unwrap();

    assert!(!off.protect_library);
    assert!(off.revision > shown.revision, "{off:?} after {shown:?}");
    eventually("library-protection:changed", || !heard.lock().unwrap().is_empty());
    assert_eq!(*heard.lock().unwrap(), [off]);
}

#[test]
fn an_unreadable_record_keeps_protection_on_when_the_window_opens() {
    let window = window();
    std::fs::write(window.dir.path().join("backups/library-protection.json"), b"not json").unwrap();
    window.host().unwrap();

    let adopted = run(service_host::adopt_library_protection(window.state(), false)).unwrap();

    assert!(adopted.protect_library);
    assert!(!window.cli(&["tracks", "rate", &track_id(1), "4"]).ok);
}

#[test]
fn a_switch_that_cannot_be_recorded_is_an_error_and_never_opens_the_library() {
    let window = window();
    window.host().unwrap();
    let id = track_id(1);
    let folder = window.dir.path().join("backups");
    let writable = std::fs::metadata(&folder).unwrap().permissions();
    let mut read_only = writable.clone();
    read_only.set_readonly(true);
    std::fs::set_permissions(&folder, read_only).unwrap();

    let turned = run(service_host::set_library_protection(window.state(), true));
    let now = service_host::library_protection(window.state());
    let rate = window.cli(&["--json", "tracks", "rate", &id, "4"]);
    std::fs::set_permissions(&folder, writable).unwrap();

    let error = turned.unwrap_err();
    assert!(error.message.starts_with("Library Protection could not be saved"), "{}", error.message);
    // On all the same, in the window and for the command.
    assert!(now.protect_library);
    assert!(!rate.ok);
    assert_eq!(window.rating(&id), 0);
}

#[test]
fn a_rating_set_from_the_command_is_undone_and_redone_from_the_window() {
    let window = window();
    window.host().unwrap();
    let id = track_id(1);

    window.cli(&["tracks", "rate", &id, "4"]).succeeded();

    // The Edit menu offers to undo it, as it would the DJ's own rating.
    eventually("edit-history:changed", || !window.history.lock().unwrap().is_empty());
    let offered = window.history.lock().unwrap().last().cloned().unwrap();
    assert_eq!(offered["canUndo"], true, "{offered}");
    assert_eq!(offered["undoLabel"], "Track Edit");
    assert_eq!(offered["canRedo"], false);
    assert_eq!(offered["generation"], window.generation());

    let undone = run(commands::undo_edit(window.app.handle().clone(), window.state())).unwrap();

    assert_eq!(window.rating(&id), 0, "the rating the track had before the command");
    assert_eq!(window.listed_rating(&id), 0, "and the command's next read agrees");
    assert!(!undone.can_undo && undone.can_redo);
    assert_eq!(undone.redo_label.as_deref(), Some("Track Edit"));

    let redone = run(commands::redo_edit(window.app.handle().clone(), window.state())).unwrap();

    assert_eq!(window.rating(&id), 4);
    assert_eq!(window.listed_rating(&id), 4);
    assert!(redone.can_undo && !redone.can_redo);
    // The interface heard of each step: the edit, the undo, the redo.
    let heard = window.history.lock().unwrap().clone();
    let offers: Vec<_> = heard.iter().map(|h| (h["canUndo"].as_bool().unwrap(), h["canRedo"].as_bool().unwrap())).collect();
    assert_eq!(offers, [(true, false), (false, true), (true, false)]);
}

#[test]
fn the_commands_edits_and_the_djs_are_steps_of_one_history() {
    let window = window();
    window.host().unwrap();
    let (djs, agents) = (track_id(1), track_id(2));
    let app = || window.app.handle().clone();

    run(commands::set_track_rating(app(), window.state(), vec![djs.clone()], 3)).unwrap();
    window.cli(&["tracks", "rate", &agents, "5"]).succeeded();

    // Undo takes back the latest edit first, whoever made it.
    run(commands::undo_edit(app(), window.state())).unwrap();
    assert_eq!((window.rating(&djs), window.rating(&agents)), (3, 0));
    let undone = run(commands::undo_edit(app(), window.state())).unwrap();
    assert_eq!((window.rating(&djs), window.rating(&agents)), (0, 0));
    assert!(!undone.can_undo && undone.can_redo);

    // A new edit from the command starts a new branch, as the DJ's would:
    // there is nothing left to redo, and the Edit menu is told.
    window.history.lock().unwrap().clear();
    window.cli(&["tracks", "rate", &agents, "2"]).succeeded();
    eventually("edit-history:changed", || !window.history.lock().unwrap().is_empty());
    let offered = window.history.lock().unwrap().last().cloned().unwrap();
    assert_eq!((offered["canUndo"].clone(), offered["canRedo"].clone()), (true.into(), false.into()));
    let refused = run(commands::redo_edit(app(), window.state())).unwrap_err();
    assert_eq!(refused.kind, ErrorKind::NotFound);
    assert_eq!((window.rating(&djs), window.rating(&agents)), (0, 2));
}

#[test]
fn a_rating_the_track_already_has_leaves_nothing_to_undo() {
    let window = window();
    window.host().unwrap();
    let id = track_id(1);

    window.cli(&["tracks", "rate", &id, "0"]).succeeded();

    let nothing = run(commands::undo_edit(window.app.handle().clone(), window.state())).unwrap_err();
    assert_eq!(nothing.kind, ErrorKind::NotFound);
    assert!(window.history.lock().unwrap().is_empty(), "the Edit menu was not told of an edit that changed nothing");
}

impl Window {
    /// The fixture's My Tags, in the order the command lists them.
    fn my_tags(&self) -> Vec<String> {
        let listed = self.cli(&["--json", "mytags", "list"]).json();
        listed["categories"][0]["tags"].as_array().unwrap().iter().map(|tag| tag["id"].as_str().unwrap().to_owned()).collect()
    }

    fn my_tags_of(&self, track: &str) -> Vec<String> {
        let shown = self.cli(&["--json", "mytags", "show", track]).json();
        shown["my_tags"].as_array().unwrap().iter().map(|tag| tag["id"].as_str().unwrap().to_owned()).collect()
    }

    fn undo(&self) -> Result<rbxport_lib::dto::EditHistoryDto, rbxport_lib::AppError> {
        run(commands::undo_edit(self.app.handle().clone(), self.state()))
    }

    fn redo(&self) -> Result<rbxport_lib::dto::EditHistoryDto, rbxport_lib::AppError> {
        run(commands::redo_edit(self.app.handle().clone(), self.state()))
    }
}

#[test]
fn a_my_tag_put_on_tracks_from_the_command_is_one_undo_in_the_window() {
    let window = window();
    window.host().unwrap();
    let tag = window.my_tags()[0].clone();
    let (one, two) = (track_id(1), track_id(2));
    let before = (window.my_tags_of(&one), window.my_tags_of(&two));
    assert!(!before.0.contains(&tag) && !before.1.contains(&tag));

    window.cli(&["mytags", "assign", &tag, &one, &two]).succeeded();
    assert!(window.my_tags_of(&one).contains(&tag));

    let undone = window.undo().unwrap();

    assert_eq!((window.my_tags_of(&one), window.my_tags_of(&two)), before, "both tracks, in one step");
    assert_eq!(undone.redo_label.as_deref(), Some("Track Edit"));
    window.redo().unwrap();
    assert!(window.my_tags_of(&one).contains(&tag) && window.my_tags_of(&two).contains(&tag));
}

#[test]
fn a_my_tag_moved_from_the_command_is_undone_from_the_window() {
    let window = window();
    window.host().unwrap();
    let before = window.my_tags();
    assert!(before.len() >= 2, "{before:?}");

    window.cli(&["mytags", "move", &before[0], "2"]).succeeded();
    assert_ne!(window.my_tags(), before);

    let undone = window.undo().unwrap();

    assert_eq!(window.my_tags(), before);
    assert_eq!(undone.redo_label.as_deref(), Some("Reorder My Tags"));
}

#[test]
fn a_my_tag_deleted_from_the_command_clears_the_history_as_the_windows_delete_does() {
    let window = window();
    window.host().unwrap();
    let tag = window.my_tags()[0].clone();
    window.cli(&["mytags", "assign", &tag, &track_id(1)]).succeeded();
    window.history.lock().unwrap().clear();

    window.cli(&["mytags", "delete", &tag]).succeeded();

    // An older entry could name the tag that is gone for good.
    eventually("edit-history:changed", || !window.history.lock().unwrap().is_empty());
    let offered = window.history.lock().unwrap().last().cloned().unwrap();
    assert_eq!((offered["canUndo"].clone(), offered["canRedo"].clone()), (false.into(), false.into()));
    assert_eq!(window.undo().unwrap_err().kind, ErrorKind::NotFound);
}

impl Run {
    /// A command that must succeed, whatever it printed.
    fn succeeded(&self) {
        assert!(self.ok, "{}", self.stderr);
    }
}

/// The entries `runs show` lists for a Run.
fn ledger_entries(window: &Window, run: &serde_json::Value) -> Vec<serde_json::Value> {
    window.cli(&["--json", "runs", "show", &run.to_string()]).json()["entries"].as_array().unwrap().clone()
}

#[test]
fn the_djs_edits_in_the_window_and_the_commands_are_ledger_entries_of_their_own_runs() {
    let window = window();
    window.host().unwrap();
    let (djs, agents) = (track_id(1), track_id(2));
    let app = || window.app.handle().clone();

    run(commands::set_track_rating(app(), window.state(), vec![djs.clone()], 3)).unwrap();
    let rated = window.cli(&["--json", "tracks", "rate", &agents, "5", "--source", "agent:tagger"]).json();
    // The DJ takes the agent's rating back, and makes it again: quick undo
    // as before, each step a new entry of the DJ's.
    let undone = run(commands::undo_edit(app(), window.state())).unwrap();
    assert_eq!(window.rating(&agents), 0);
    assert!(undone.can_undo && undone.can_redo);
    let redone = run(commands::redo_edit(app(), window.state())).unwrap();
    assert_eq!(window.rating(&agents), 5);
    assert!(redone.can_undo && !redone.can_redo);

    let runs = window.cli(&["--json", "runs", "list"]).json();
    assert_eq!(runs["total"], 2);
    let (agents_run, windows_run) = (&runs["runs"][0], &runs["runs"][1]);
    assert_eq!(agents_run["id"], rated["run"]["id"]);
    assert_eq!(agents_run["source"], serde_json::json!({"kind": "agent", "name": "tagger"}));
    assert_eq!(windows_run["source"], serde_json::json!({"kind": "dj", "name": "dj"}));
    assert_eq!(windows_run["description"], "Edits in the window");
    assert!(windows_run["finished_at"].is_null(), "the window's Run is open while the DJ edits");
    assert_eq!(windows_run["counts"], serde_json::json!([{"store": "library", "kind": "track", "count": 3}]));

    let agents_entries = ledger_entries(&window, &agents_run["id"]);
    assert_eq!(agents_entries.len(), 1);
    assert_eq!(agents_entries[0]["values"], serde_json::json!([{"subject": format!("track {agents}"), "field": "Rating", "before": 0, "after": 5}]));

    let djs_entries = ledger_entries(&window, &windows_run["id"]);
    let dj = serde_json::json!({"kind": "dj", "name": "dj"});
    assert!(djs_entries.iter().all(|entry| entry["source"] == dj && entry["command"].is_null()));
    assert_eq!(djs_entries[0]["action"], "edit");
    assert_eq!(djs_entries[0]["label"], "Track Edit");
    assert_eq!(djs_entries[0]["values"], serde_json::json!([{"subject": format!("track {djs}"), "field": "Rating", "before": 0, "after": 3}]));
    // The Undo names the agent's entry, with the values as it changed them.
    assert_eq!(djs_entries[1]["action"], "undo");
    assert_eq!(djs_entries[1]["reverses"], agents_entries[0]["id"]);
    assert_eq!(djs_entries[1]["values"][0]["before"], 5);
    assert_eq!(djs_entries[1]["values"][0]["after"], 0);
    assert_eq!(djs_entries[2]["action"], "redo");
    assert_eq!(djs_entries[2]["reverses"], djs_entries[1]["id"]);
    assert_eq!(djs_entries[2]["values"][0]["after"], 5);
}

#[test]
fn the_windows_run_ends_when_the_window_closes() {
    let window = window();
    window.host().unwrap();
    run(commands::set_track_rating(window.app.handle().clone(), window.state(), vec![track_id(1)], 3)).unwrap();

    window.state().stop_service();

    // Read by a service a command starts by itself: the Ledger outlives
    // the window.
    let runs = window.cli(&["--json", "runs", "list"]).json();
    assert_eq!(runs["total"], 1);
    assert_eq!(runs["runs"][0]["description"], "Edits in the window");
    assert_eq!(runs["runs"][0]["outcome"], "wreckordbox stopped");
    assert_eq!(runs["runs"][0]["changes"], 1);
}
