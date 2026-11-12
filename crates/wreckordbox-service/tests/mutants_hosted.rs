//! Mutants and playlist entries through a service a front end hosts on its
//! own Library core, as the window does, while the front end edits the
//! Library at the same moment.
//!
//! The parent's audio is a named pipe, so a test decides when the service
//! finishes reading it: the front end's edit, or another client's command,
//! lands exactly while `mutants.create` is fingerprinting.
//!
//! The Sidecar's folder comes from the environment, which is the
//! process's, so the tests here take turns.
#![cfg(unix)]
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rbl_db::fixture::{self, Shape};
use rbl_db::LibraryLocation;
use rbl_library::{History, LibraryCore, NoHost, Touched};
use serde_json::{json, Value};
use wreckordbox_service::facts::commands as runs;
use wreckordbox_service::mutants::commands;
use wreckordbox_service::reading::commands::PLAYLISTS_TRACKS;
use wreckordbox_service::stems::SIDECAR_DIR_ENV;
use wreckordbox_service::{Client, Config, Event, Service};

static TURNS: Mutex<()> = Mutex::new(());

/// A silent 16-bit stereo 44.1 kHz WAV file of `frames` sample frames.
fn write_audio(path: &Path, frames: u32) {
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

struct Hosted {
    dir: tempfile::TempDir,
    location: LibraryLocation,
    config: Config,
    core: Arc<LibraryCore>,
    service: Arc<Service>,
    events: Receiver<Event>,
    playlist: String,
    /// The fixture's tracks 1 and 2, then a track whose audio is a pipe.
    tracks: [String; 3],
    pipe: PathBuf,
}

/// A fixture Library with a playlist of three tracks, hosted on a core
/// the test holds, as the window holds its own.
fn hosted() -> Hosted {
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
    let pipe = dir.path().join("Night Bus.wav");
    write_audio(&pipe, 2_205);
    let mut writer =
        rbl_db::write::Writer::open(location.clone(), dir.path().join("backups")).unwrap();
    writer.disable_automatic_backups();
    let piped = writer.import_file(&pipe).unwrap();
    let playlist = writer.create_playlist("Friday", "root").unwrap();
    let tracks = [fixture::track_id(1), fixture::track_id(2), piped];
    writer.add_tracks(&playlist, &tracks).unwrap();
    drop(writer);
    std::fs::remove_file(&pipe).unwrap();
    let made = std::process::Command::new("mkfifo")
        .arg(&pipe)
        .status()
        .unwrap();
    assert!(made.success());

    std::env::set_var(SIDECAR_DIR_ENV, dir.path().join("sidecar"));
    let config = Config {
        runtime_dir: dir.path().join("run"),
        idle_exit: None,
        recovery_dir: dir.path().join("recovery"),
    };
    let core = Arc::new(LibraryCore::new(config.recovery_dir.clone()));
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (index, _) = rbl_index::load(&db).unwrap();
    core.set_library(
        &NoHost,
        index,
        false,
        db.schema().db_version,
        0,
        location.clone(),
    );
    let host: Arc<dyn rbl_library::Host + Send + Sync> = Arc::new(NoHost);
    let service = Arc::new(Service::host(&location, &config, &core, &host).unwrap());
    let events = service.events();
    Hosted {
        dir,
        location,
        config,
        core,
        service,
        events,
        playlist,
        tracks,
        pipe,
    }
}

impl Hosted {
    /// An edit the front end makes through its core, as the window's
    /// playlist commands do.
    fn edit(&self, edit: impl FnOnce(&mut rbl_db::write::Writer) -> rbl_db::Result<()>) {
        self.core
            .edit(&NoHost, History::Outside, Touched::Playlists, |writer| {
                edit(writer)
            })
            .unwrap();
    }

    /// The playlist's entry rows in the Library now, in its order.
    fn entry_rows(&self) -> Vec<(String, String)> {
        let library =
            rbl_db::Library::open(self.location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
        let mut rows = library
            .connection()
            .prepare(
                "SELECT ID, ContentID FROM djmdSongPlaylist
                 WHERE PlaylistID = ?1 AND rb_local_deleted = 0 ORDER BY TrackNo, rowid",
            )
            .unwrap();
        let found = rows
            .query_map([&self.playlist], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        found
    }

    /// Runs a command on a thread of its own, as a client would.
    fn call_aside(&self, command: &'static str, args: Value) -> Receiver<Result<Value, String>> {
        let (answer, answered) = mpsc::channel();
        let service = Arc::clone(&self.service);
        std::thread::spawn(move || {
            let _ = answer.send(
                service
                    .call(command, args)
                    .map_err(|error| error.code.clone()),
            );
        });
        answered
    }

    /// The `sidecar_changed` events of kind `mutant` the host was sent.
    fn mutant_events(&self) -> Vec<Value> {
        let mut found = Vec::new();
        while let Ok(event) = self.events.recv_timeout(Duration::from_millis(300)) {
            if event.event == "sidecar_changed" && event.data["kind"] == "mutant" {
                found.push(event.data);
            }
        }
        found
    }

    fn runs(&self) -> i64 {
        rusqlite::Connection::open(self.dir.path().join("sidecar").join("sidecar.db"))
            .unwrap()
            .query_row("SELECT count(*) FROM run", [], |row| row.get(0))
            .unwrap()
    }

    fn listed(&self) -> Value {
        self.service
            .call(commands::MUTANTS_LIST, json!({}))
            .unwrap()["mutants"]
            .clone()
    }
}

/// Holds the service's read of the piped audio: answers when the service
/// has opened the pipe, and lets the read finish when told to.
fn hold_the_audio(pipe: &Path) -> (Receiver<()>, Sender<()>) {
    let (opened, reading) = mpsc::channel();
    let (go, gone) = mpsc::channel::<()>();
    let pipe = pipe.to_owned();
    std::thread::spawn(move || {
        // Opening a pipe to write waits for its reader.
        let first = std::fs::OpenOptions::new().write(true).open(&pipe).unwrap();
        opened.send(()).unwrap();
        let _ = gone.recv();
        drop(first);
        // Every later read ends at once.
        while let Ok(again) = std::fs::OpenOptions::new().write(true).open(&pipe) {
            drop(again);
        }
    });
    (reading, go)
}

fn turn() -> std::sync::MutexGuard<'static, ()> {
    TURNS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[test]
fn a_playlists_tracks_waits_for_an_edit_under_way_and_answers_the_library_it_left() {
    let _turn = turn();
    let hosted = hosted();
    let [first, second, third] = hosted.tracks.clone();

    // The front end is in the middle of an edit.
    let gate = hosted.core.edit_gate().lock();
    let answered = hosted.call_aside(PLAYLISTS_TRACKS, json!({ "id": hosted.playlist }));
    assert!(
        answered.recv_timeout(Duration::from_millis(500)).is_err(),
        "answered while an edit was under way"
    );
    hosted.edit(|writer| {
        writer
            .reorder(
                &hosted.playlist,
                &[third.clone(), first.clone(), second.clone()],
            )
            .map(drop)
    });
    drop(gate);

    let answer = answered
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    let listed: Vec<(String, String)> = answer["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|track| {
            (
                track["entry_id"].as_str().unwrap().to_owned(),
                track["id"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(listed, hosted.entry_rows());
    assert_eq!(listed[0].1, third);
}

#[test]
fn an_entry_removed_while_it_is_being_mutated_is_not_mutated() {
    let _turn = turn();
    let hosted = hosted();
    let entry = hosted.entry_rows()[2].0.clone();
    let (reading, go) = hold_the_audio(&hosted.pipe);

    let answered = hosted.call_aside(
        commands::MUTANTS_CREATE,
        json!({ "entry": entry, "source": { "kind": "agent", "name": "preparer" } }),
    );
    reading.recv_timeout(Duration::from_secs(10)).unwrap();
    // While the service fingerprints the parent, the front end takes the
    // track out of the playlist.
    hosted.edit(|writer| {
        writer
            .remove_tracks(&hosted.playlist, &[hosted.tracks[2].clone()])
            .map(drop)
    });
    go.send(()).unwrap();

    assert_eq!(
        answered.recv_timeout(Duration::from_secs(10)).unwrap(),
        Err("playlist_entry_not_found".to_owned())
    );
    assert_eq!(hosted.listed(), json!([]));
    assert_eq!(hosted.mutant_events(), Vec::<Value>::new());
    assert_eq!(hosted.runs(), 0);
}

#[test]
fn an_entry_moved_while_it_is_being_mutated_is_mutated_where_it_is_now() {
    let _turn = turn();
    let hosted = hosted();
    let entry = hosted.entry_rows()[2].0.clone();
    let (reading, go) = hold_the_audio(&hosted.pipe);

    let answered = hosted.call_aside(
        commands::MUTANTS_CREATE,
        json!({ "entry": entry, "source": { "kind": "dj" } }),
    );
    reading.recv_timeout(Duration::from_secs(10)).unwrap();
    let [first, second, third] = hosted.tracks.clone();
    hosted.edit(|writer| {
        writer
            .reorder(&hosted.playlist, &[third, first, second])
            .map(drop)
    });
    go.send(()).unwrap();

    let made = answered
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    assert_eq!(made["mutant"]["entry_id"], entry.as_str());
    assert_eq!(made["mutant"]["position"], 1);
    assert_eq!(made["run"]["description"].as_str().unwrap().split(' ').nth(2), Some("1"));
    assert_eq!(hosted.listed()[0]["position"], 1);
    assert_eq!(hosted.mutant_events().len(), 1);
}

#[test]
fn a_run_another_client_ends_while_a_mutant_is_being_made_in_it_takes_nothing_more() {
    let _turn = turn();
    let hosted = hosted();
    let entry = hosted.entry_rows()[2].0.clone();
    let mut other = Client::connect(&hosted.config.socket_path(&hosted.location)).unwrap();
    let run: Value = other
        .call(
            runs::RUNS_START,
            json!({ "source": { "kind": "agent", "name": "preparer" }, "description": "Prepare" }),
        )
        .unwrap();
    let run = run["id"].as_i64().unwrap();
    let (reading, go) = hold_the_audio(&hosted.pipe);

    let answered = hosted.call_aside(
        commands::MUTANTS_CREATE,
        json!({ "entry": entry, "run": run }),
    );
    reading.recv_timeout(Duration::from_secs(10)).unwrap();
    let finished: Value = other
        .call(runs::RUNS_FINISH, json!({ "id": run }))
        .unwrap();
    assert!(!finished["finished_at"].is_null(), "{finished}");
    go.send(()).unwrap();

    assert_eq!(
        answered.recv_timeout(Duration::from_secs(10)).unwrap(),
        Err("run_finished".to_owned())
    );
    assert_eq!(hosted.listed(), json!([]));
    assert_eq!(hosted.mutant_events(), Vec::<Value>::new());
}
