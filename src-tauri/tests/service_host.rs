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
    let window = Window { dir, app, location, changes, stopped };
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
    let window = Window { dir, app, location, changes: Arc::default(), stopped: Arc::default() };
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

    /// Turns Library Protection on or off, as the window records it for
    /// the service: a file in the folder the two share.
    fn protect(&self, on: bool) {
        let state_dir = self.dir.path().join("backups");
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::write(state_dir.join("library-protection.json"), format!("{{\"protect_library\":{on}}}")).unwrap();
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

    for run in [&rate, &assign] {
        assert!(!run.ok);
        let error: serde_json::Value = serde_json::from_str(&run.stderr).unwrap();
        assert_eq!(error["error"]["code"], "library_protected");
        // The window's own words (`refusal(true)` in `src/lib/menu.ts`).
        assert_eq!(error["error"]["message"], "Editing is locked by Library Protection. Turn it off in Preferences to edit.");
    }
    assert_eq!(window.rating(&id), 0);
    let index = window.state().library().unwrap();
    assert!(index.my_tag_keys(index.row_of(&id).unwrap() as usize).is_empty());
    assert_eq!(window.generation(), before);
    std::thread::sleep(Duration::from_millis(100));
    assert!(window.changes.lock().unwrap().is_empty(), "the interface was told of a change that was refused");
    // Reading is untouched.
    assert_eq!(window.listed_rating(&id), 0);
    assert!(window.cli(&["mytags", "list"]).ok);

    // Off again, the same commands are done.
    window.protect(false);
    window.cli(&["tracks", "rate", &id, "4"]).succeeded();
    window.cli(&["mytags", "assign", fixture::MY_TAG_PEAK, &id]).succeeded();
    assert_eq!(window.rating(&id), 4);
}

#[test]
fn protection_turned_off_from_the_command_reaches_the_interface_and_edits_go_through() {
    let window = window();
    window.host().unwrap();
    window.protect(true);
    let heard: Heard<bool> = Arc::default();
    let told = Arc::clone(&heard);
    window.app.listen("library-protection:changed", move |event| {
        told.lock().unwrap().push(serde_json::from_str::<bool>(event.payload()).unwrap());
    });
    let id = track_id(1);
    assert!(!window.cli(&["tracks", "rate", &id, "4"]).ok);

    let off = window.cli(&["--json", "protection", "off"]).json();

    assert_eq!(off, serde_json::json!({ "protect_library": false, "previous": true }));
    // The interface makes it its own preference, so the status bar and the
    // window's own edits agree with the command.
    eventually("library-protection:changed", || !heard.lock().unwrap().is_empty());
    assert_eq!(*heard.lock().unwrap(), [false]);
    window.cli(&["tracks", "rate", &id, "4"]).succeeded();
    assert_eq!(window.rating(&id), 4);
}

#[test]
fn the_switch_the_dj_turns_in_preferences_is_the_one_the_command_obeys() {
    let window = window();
    window.host().unwrap();
    let id = track_id(1);

    // What the interface calls when the preference changes.
    service_host::record_library_protection(window.state(), true).unwrap();

    assert_eq!(service_host::library_protection(window.state()), Some(true));
    assert_eq!(window.cli(&["--json", "protection"]).json()["protect_library"], true);
    assert!(!window.cli(&["tracks", "rate", &id, "4"]).ok);
    assert_eq!(window.rating(&id), 0);

    service_host::record_library_protection(window.state(), false).unwrap();
    window.cli(&["tracks", "rate", &id, "4"]).succeeded();
    assert_eq!(window.rating(&id), 4);
}

#[test]
fn a_window_on_a_machine_with_nothing_recorded_is_told_so() {
    let window = window();
    std::fs::remove_file(window.dir.path().join("backups/library-protection.json")).unwrap();

    assert_eq!(service_host::library_protection(window.state()), None);
}

impl Run {
    /// A command that must succeed, whatever it printed.
    fn succeeded(&self) {
        assert!(self.ok, "{}", self.stderr);
    }
}
