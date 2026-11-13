//! Intake edits (wbx-63) with the window hosting the Library service: the
//! Prep colour set by the view (by number, whatever the DJ called the
//! labels), a real `wreckordbox` command's edit reaching the window and a
//! second client, and the Ledger entries of the DJ's mixed edits and their
//! Undo.
//!
//! A fixture Library in a temporary folder, a mock app in place of the
//! window, and the command given that fixture and a private runtime folder:
//! nothing here can reach the installed Library.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use rbl_db::fixture::{self, track_id, Shape};
use rbxport_lib::intake_view::{self, IntakeQueueName};
use rbxport_lib::state::AppState;
use rbxport_lib::{commands, details, service_host};
use serde_json::{json, Value};
use tauri::test::MockRuntime;
use tauri::{Listener, Manager, State};
use wreckordbox_service::{Client, Config};

/// The prebuilt CLI. Build it through the heavy queue before the shell tests.
fn wreckordbox() -> &'static PathBuf {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        let deps = std::env::current_exe().unwrap();
        let binary = deps.parent().unwrap().parent().unwrap().join("wreckordbox");
        assert!(
            binary.is_file(),
            "build the CLI first with the heavy queue: cargo build -p wreckordbox"
        );
        binary
    })
}

struct Window {
    dir: tempfile::TempDir,
    app: tauri::App<MockRuntime>,
    location: rbl_db::LibraryLocation,
    /// Every `library:changed` generation the interface would have seen.
    changes: Arc<Mutex<Vec<u32>>>,
}

/// A fixture Library of five tracks whose colour labels are named first,
/// read by a mock app as the window reads it, and hosted.
fn hosting_window(labels: &[(u8, &str)]) -> Window {
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    let location = fixture::build(
        &library,
        Shape {
            tracks: 5,
            ..Shape::default()
        },
    )
    .unwrap();
    {
        let mut writer =
            rbl_db::write::Writer::open(location.clone(), dir.path().join("fixture-backups"))
                .unwrap();
        writer.disable_automatic_backups();
        for (colour, name) in labels {
            writer
                .library()
                .connection()
                .execute(
                    "INSERT INTO djmdColor (ID, ColorCode, SortKey, Commnt, UUID, created_at, updated_at)
                     VALUES (?1, 0, ?1, ?2, 'fixture-colour-' || ?1, '2026-01-01', '2026-01-01')",
                    (colour.to_string(), name),
                )
                .unwrap();
        }
    }
    fixture::write_options_json(
        &dir.path().join("options.json"),
        location.master_db.to_str().unwrap(),
        &location.passphrase,
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("home")).unwrap();
    let state = AppState::with_backups(dir.path().join("backups"));
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (index, _) = rbl_index::load(&db).unwrap();
    state.set_library(index, false, db.schema().db_version, 0, location.clone());
    let app = tauri::test::mock_app();
    app.manage(Arc::new(state));
    let changes: Arc<Mutex<Vec<u32>>> = Arc::default();
    let seen = Arc::clone(&changes);
    app.listen("library:changed", move |event| {
        seen.lock()
            .unwrap()
            .push(serde_json::from_str::<u32>(event.payload()).unwrap());
    });
    let window = Window {
        dir,
        app,
        location,
        changes,
    };
    let protection =
        tauri::async_runtime::block_on(service_host::set_library_protection(window.state(), false))
            .unwrap();
    assert!(!protection.protect_library);
    service_host::host_with(window.app.handle(), &window.location, &window.config()).unwrap();
    window
}

/// The DJ's Prep colours: Clean on Green, Problem on Red.
const DJ_LABELS: &[(u8, &str)] = &[(2, "Problem"), (5, "Clean")];

impl Window {
    fn config(&self) -> Config {
        Config {
            runtime_dir: self.dir.path().join("run"),
            idle_exit: None,
            recovery_dir: self.dir.path().join("backups"),
        }
    }

    fn state(&self) -> State<'_, Arc<AppState>> {
        self.app.state::<Arc<AppState>>()
    }

    fn shared(&self) -> Arc<AppState> {
        Arc::clone(self.state().inner())
    }

    fn client(&self) -> Client {
        Client::connect(&self.config().socket_path(&self.location)).unwrap()
    }

    /// The real command, with this window's Library and runtime folder.
    fn cli(&self, args: &[&str]) -> Value {
        let output = Command::new(wreckordbox())
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
            .arg("--json")
            .args(args)
            .output()
            .unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("not JSON ({e}): {stdout}"))
    }

    fn undo(&self) {
        tauri::async_runtime::block_on(commands::undo_edit(
            self.app.handle().clone(),
            self.state(),
        ))
        .unwrap();
    }

    /// The Intake queue's page as the view reads it.
    fn intake(&self) -> (Vec<Value>, Vec<rbxport_lib::dto::RowDto>) {
        let page = intake_view::queue_page(&self.shared(), IntakeQueueName::Intake, 0, 50).unwrap();
        (
            page.queue.unwrap()["tracks"].as_array().unwrap().clone(),
            page.rows,
        )
    }
}

fn eventually(what: &str, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_view_sets_a_prep_colour_by_number_when_the_djs_labels_are_other_colours_names() {
    // Red is called "Green" and Green is called "Clean".
    let window = hosting_window(&[(2, "Green"), (5, "Clean")]);
    let state = window.shared();
    let colours = intake_view::colours(&state).unwrap();
    assert_eq!(colours["clean"]["id"], 5);

    let clean = intake_view::set_prep_colour(&state, &track_id(1), Some(5)).unwrap();
    assert_eq!(
        clean["prep_colour"],
        json!({ "id": 5, "colour": "Green", "label": "Clean" })
    );
    let labelled_green = intake_view::set_prep_colour(&state, &track_id(2), Some(2)).unwrap();
    assert_eq!(
        labelled_green["prep_colour"],
        json!({ "id": 2, "colour": "Red", "label": "Green" })
    );
    assert_eq!(intake_view::counts(&state).unwrap()["intake"], 3);
}

#[test]
fn a_cli_edit_reaches_the_hosting_window_and_a_second_client_and_the_queue_the_view_reads() {
    let window = hosting_window(DJ_LABELS);
    let id = track_id(1);
    let mut watching = window.client();
    watching.subscribe().unwrap();
    let before = window.changes.lock().unwrap().len();

    window.cli(&["tracks", "rate", &id, "4"]);

    // The window is told (the view re-reads on `library:changed`) ...
    eventually("library:changed in the window", || {
        window.changes.lock().unwrap().len() > before
    });
    // ... and so is another client of the service.
    let heard = watching
        .next_event(Duration::from_secs(5))
        .unwrap()
        .unwrap();
    assert_eq!(heard.event, "library_changed");
    // What the view reads next has the new Energy, with no reload.
    let (tracks, rows) = window.intake();
    let at = tracks
        .iter()
        .position(|track| track["id"] == id.as_str())
        .unwrap();
    assert_eq!(rows[at].rating, 4);
    assert!(!tracks[at]["needs"]
        .as_array()
        .unwrap()
        .contains(&json!("energy")));

    // A Prep colour from the command takes the track out of the queue the view reads.
    window.cli(&["intake", "colour", &id, "Clean"]);
    let heard = watching
        .next_event(Duration::from_secs(5))
        .unwrap()
        .unwrap();
    assert_eq!(heard.event, "library_changed");
    eventually("the second library:changed", || {
        window.changes.lock().unwrap().len() > before + 1
    });
    let (tracks, _) = window.intake();
    assert!(tracks.iter().all(|track| track["id"] != id.as_str()));
}

#[test]
fn a_title_energy_and_prep_colour_edit_from_the_view_are_ledger_entries_and_undo_reverses_each() {
    let window = hosting_window(DJ_LABELS);
    let id = track_id(1);
    let app = || window.app.handle().clone();

    // As the Intake view edits: the title and Energy through the window's
    // own commands, the Prep colour through the service it hosts.
    tauri::async_runtime::block_on(details::set_track_field(
        app(),
        window.state(),
        vec![id.clone()],
        "title".into(),
        "Fixed Title".into(),
        None,
    ))
    .unwrap();
    tauri::async_runtime::block_on(commands::set_track_rating(
        app(),
        window.state(),
        vec![id.clone()],
        3,
    ))
    .unwrap();
    intake_view::set_prep_colour(&window.shared(), &id, Some(5)).unwrap();

    let runs = window.cli(&["runs", "list"]);
    let djs = runs["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|run| run["source"] == json!({"kind": "dj", "name": "dj"}))
        .expect("the window's Run")
        .clone();
    let entries = || {
        window.cli(&["runs", "show", &djs["id"].to_string()])["entries"]
            .as_array()
            .unwrap()
            .clone()
    };
    let edits = entries();
    assert_eq!(edits.len(), 3, "{edits:#?}");
    let subject = format!("track {id}");
    for entry in &edits {
        assert_eq!(entry["action"], "edit");
        assert_eq!(entry["label"], "Track Edit");
        assert_eq!(entry["values"][0]["subject"], subject.as_str());
    }
    assert_eq!(edits[0]["values"][0]["after"], "Fixed Title");
    assert_eq!(edits[1]["values"][0]["after"], 3);
    // The Library stores the colour label as text.
    assert_eq!(edits[2]["values"][0]["after"], "5");

    // Three Undos, newest first: each names the entry it reverses.
    window.undo();
    window.undo();
    window.undo();
    let all = entries();
    assert_eq!(all.len(), 6, "{all:#?}");
    for (undo, edit) in all[3..].iter().zip(edits.iter().rev()) {
        assert_eq!(undo["action"], "undo");
        assert_eq!(undo["reverses"], edit["id"]);
        assert_eq!(undo["values"][0]["before"], edit["values"][0]["after"]);
        assert_eq!(undo["values"][0]["after"], edit["values"][0]["before"]);
    }
    // The track is as it was: in Intake, untitled by the DJ, no Energy.
    let (tracks, rows) = window.intake();
    let at = tracks
        .iter()
        .position(|track| track["id"] == id.as_str())
        .expect("back in Intake");
    assert_eq!(rows[at].rating, 0);
    assert_ne!(rows[at].title, "Fixed Title");
}

#[test]
fn the_views_title_artist_energy_and_prep_colour_edits_are_the_djs_facts_and_undo_withdraws_one() {
    // wbx-19: a hand edit is the DJ's Fact. The view's edits must be too,
    // the Prep colour included though it goes through the hosted service.
    let window = hosting_window(DJ_LABELS);
    let id = track_id(1);
    let app = || window.app.handle().clone();
    for (field, value) in [("title", "Fixed Title"), ("artist", "Fixed Artist")] {
        tauri::async_runtime::block_on(details::set_track_field(
            app(),
            window.state(),
            vec![id.clone()],
            field.into(),
            value.into(),
            None,
        ))
        .unwrap();
    }
    tauri::async_runtime::block_on(commands::set_track_rating(
        app(),
        window.state(),
        vec![id.clone()],
        4,
    ))
    .unwrap();
    // Mark Clean: label 5, which the Library calls Green.
    intake_view::set_prep_colour(&window.shared(), &id, Some(5)).unwrap();

    let standing = |kind: &str| window.cli(&["facts", "standing", &id, kind])["standing"].clone();
    for (kind, value) in [
        ("title", json!("Fixed Title")),
        ("artist", json!("Fixed Artist")),
        ("energy", json!(4)),
        ("prep_colour", json!("Green")),
    ] {
        let fact = standing(kind);
        assert_eq!(fact["value"], value, "{kind}: {fact:#}");
        assert_eq!(fact["source"]["kind"], "dj", "{kind}: {fact:#}");
        assert!(fact["ledger_entry"].is_i64(), "{kind}: {fact:#}");
    }

    // Undo of Mark Clean withdraws its Fact and only that one.
    window.undo();
    assert!(
        standing("prep_colour").is_null(),
        "the colour's Fact is withdrawn"
    );
    assert_eq!(standing("energy")["value"], 4);
    assert_eq!(standing("title")["value"], "Fixed Title");
}
