//! Mutants follow the window's playlist edits (wbx-80), through a service
//! a front end hosts on its own Library core, as the window does.
//!
//! The window's playlist commands call the core directly (they are not
//! service commands), and its Undo and Redo step the core's history. Each
//! of those edits is recorded at the Ledger's one recording point, and a
//! Mutant's consequence is recorded with it: removing an entry, deleting
//! its playlist or replacing the playlist's contents takes the Mutant off,
//! and an Undo puts it back.
//!
//! The Sidecar's folder comes from the environment, which is the
//! process's, so the tests here take turns.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rbl_db::fixture::{self, Shape};
use rbl_db::LibraryLocation;
use rbl_library::history::LibraryEdit;
use rbl_library::{LibraryCore, NoHost, Touched};
use serde_json::{json, Value};
use wreckordbox_service::mutants::commands;
use wreckordbox_service::reading::commands::PLAYLISTS_TRACKS;
use wreckordbox_service::stems::SIDECAR_DIR_ENV;
use wreckordbox_service::{Config, Event, Service};

static TURNS: Mutex<()> = Mutex::new(());

struct Hosted {
    dir: tempfile::TempDir,
    location: LibraryLocation,
    core: Arc<LibraryCore>,
    service: Arc<Service>,
    events: Receiver<Event>,
    playlist: String,
    tracks: [String; 3],
}

/// A fixture Library with a playlist "Friday" of three tracks, hosted on a
/// core the test holds, as the window holds its own.
fn hosted() -> Hosted {
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    let location = fixture::build(
        &library,
        Shape {
            tracks: 4,
            ..Shape::default()
        },
    )
    .unwrap();
    let mut writer =
        rbl_db::write::Writer::open(location.clone(), dir.path().join("backups")).unwrap();
    writer.disable_automatic_backups();
    let playlist = writer.create_playlist("Friday", "root").unwrap();
    let tracks = [
        fixture::track_id(1),
        fixture::track_id(2),
        fixture::track_id(3),
    ];
    writer.add_tracks(&playlist, &tracks).unwrap();
    drop(writer);

    std::env::set_var(SIDECAR_DIR_ENV, dir.path().join("sidecar"));
    let config = Config {
        runtime_dir: dir.path().join("run"),
        idle_exit: None,
        recovery_dir: dir.path().join("recovery"),
    };
    wreckordbox_service::protection::store(&config.recovery_dir, false).unwrap();
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
        core,
        service,
        events,
        playlist,
        tracks,
    }
}

impl Hosted {
    /// A recorded edit the window makes through its core.
    fn window(
        &self,
        label: &'static str,
        edit: impl FnOnce(&mut rbl_db::write::Writer) -> rbl_db::Result<LibraryEdit>,
    ) {
        self.core
            .recorded_edit(&NoHost, Touched::Playlists, label, |writer| {
                edit(writer).map(|edit| ((), edit))
            })
            .unwrap();
    }

    fn undo(&self) {
        self.core.step_history(&NoHost, true).unwrap();
    }

    fn redo(&self) {
        self.core.step_history(&NoHost, false).unwrap();
    }

    /// The entry identifiers of a playlist, in its order, as a client reads
    /// them.
    fn entries(&self, playlist: &str) -> Vec<String> {
        self.service
            .call(PLAYLISTS_TRACKS, json!({ "id": playlist }))
            .unwrap()["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|track| track["entry_id"].as_str().unwrap().to_owned())
            .collect()
    }

    /// Mutates `entry` with edits, as an agent would.
    fn mutate(&self, entry: &str, edits: Value) -> i64 {
        let made = self
            .service
            .call(
                commands::MUTANTS_CREATE,
                json!({ "entry": entry, "source": { "kind": "agent", "name": "preparer" } }),
            )
            .unwrap();
        let id = made["mutant"]["id"].as_i64().unwrap();
        if edits.as_object().is_some_and(|edits| !edits.is_empty()) {
            let mut args = edits;
            args["id"] = json!(id);
            args["source"] = json!({ "kind": "agent", "name": "preparer" });
            self.service.call(commands::MUTANTS_EDIT, args).unwrap();
        }
        let _ = self.drain();
        id
    }

    fn mutant(&self, id: i64) -> Value {
        self.service
            .call(commands::MUTANTS_GET, json!({ "id": id }))
            .unwrap()
    }

    /// The live Mutants as `(entry, parent)`, as a client lists them.
    fn live(&self) -> Vec<(String, String)> {
        self.service
            .call(commands::MUTANTS_LIST, json!({}))
            .unwrap()["mutants"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| {
                (
                    m["entry_id"].as_str().unwrap().to_owned(),
                    m["parent"]["id"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    /// The `sidecar_changed` events of kind `mutant` sent since the last
    /// call, as `(mutant, action)`.
    fn drain(&self) -> Vec<(i64, String)> {
        let mut found = Vec::new();
        while let Ok(event) = self.events.recv_timeout(Duration::from_millis(300)) {
            if event.event == "sidecar_changed" && event.data["kind"] == "mutant" {
                found.push((
                    event.data["mutant_id"].as_i64().unwrap(),
                    event.data["action"].as_str().unwrap().to_owned(),
                ));
            }
        }
        found
    }

    /// The actions of a Mutant's history, oldest first.
    fn history(&self, id: i64) -> Vec<String> {
        self.mutant(id)["history"]
            .as_array()
            .unwrap()
            .iter()
            .map(|change| change["action"].as_str().unwrap().to_owned())
            .collect()
    }

    /// The Ledger entry ids of the window's Run, and the Run each Mutant
    /// change went in.
    fn sidecar(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.dir.path().join("sidecar").join("sidecar.db")).unwrap()
    }
}

fn turn() -> std::sync::MutexGuard<'static, ()> {
    TURNS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[test]
fn removing_a_mutated_entry_in_the_window_removes_its_mutant_and_undo_brings_it_back() {
    let _turn = turn();
    let hosted = hosted();
    let entries = hosted.entries(&hosted.playlist);
    let mutant = hosted.mutate(&entries[1], json!({}));
    let other = hosted.mutate(&entries[2], json!({}));

    let playlist = hosted.playlist.clone();
    let track = hosted.tracks[1].clone();
    hosted.window("Remove Tracks from Playlist", move |writer| {
        writer
            .remove_tracks_with_undo(&playlist, &[track])
            .map(|(_, edit)| LibraryEdit::RemovePlaylistTracks(edit))
    });

    let gone = hosted.mutant(mutant);
    assert_eq!(gone["mutant"]["removed"], true, "{gone}");
    assert_eq!(hosted.live(), vec![(entries[2].clone(), hosted.tracks[2].clone())]);
    assert_eq!(hosted.mutant(other)["mutant"]["removed"], false);
    assert_eq!(hosted.drain(), vec![(mutant, "detached".to_owned())]);
    // The Mutant's change is in the Run of the Library change that caused
    // it: the window's, by the DJ.
    let (run, source): (i64, String) = hosted
        .sidecar()
        .query_row(
            "SELECT run_id, source_kind FROM mutant_change WHERE mutant_id = ?1 ORDER BY id DESC LIMIT 1",
            [mutant],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let entry_run: i64 = hosted
        .sidecar()
        .query_row(
            "SELECT run_id FROM ledger WHERE kind = 'remove_playlist_tracks'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!((run, source.as_str()), (entry_run, "dj"));

    hosted.undo();
    let back = hosted.mutant(mutant);
    assert_eq!(back["mutant"]["removed"], false, "{back}");
    assert_eq!(back["mutant"]["position"], 2);
    assert_eq!(hosted.drain(), vec![(mutant, "reattached".to_owned())]);

    hosted.redo();
    assert_eq!(hosted.mutant(mutant)["mutant"]["removed"], true);
    hosted.undo();
    assert_eq!(hosted.mutant(mutant)["mutant"]["removed"], false);
    assert_eq!(
        hosted.history(mutant),
        ["created", "detached", "reattached", "detached", "reattached"]
    );
    assert_eq!(hosted.live().len(), 2);
    let _ = hosted.location.clone();
}

#[test]
fn deleting_a_playlist_in_the_window_removes_its_mutants_and_undo_brings_them_back() {
    let _turn = turn();
    let hosted = hosted();
    let entries = hosted.entries(&hosted.playlist);
    let first = hosted.mutate(&entries[0], json!({}));
    let third = hosted.mutate(&entries[2], json!({}));

    let playlist = hosted.playlist.clone();
    hosted.window("Delete Playlist", move |writer| {
        writer
            .delete_playlist_with_undo(&playlist)
            .map(|(_, edit)| LibraryEdit::DeletePlaylist(edit))
    });
    assert!(hosted.live().is_empty());
    let mut heard = hosted.drain();
    heard.sort();
    assert_eq!(
        heard,
        vec![(first, "detached".to_owned()), (third, "detached".to_owned())]
    );

    hosted.undo();
    let mut live = hosted.live();
    live.sort();
    let mut expected = vec![
        (entries[0].clone(), hosted.tracks[0].clone()),
        (entries[2].clone(), hosted.tracks[2].clone()),
    ];
    expected.sort();
    assert_eq!(live, expected);
    assert_eq!(hosted.mutant(third)["mutant"]["position"], 3);

    hosted.redo();
    assert!(hosted.live().is_empty());
    assert_eq!(hosted.history(first), ["created", "detached", "reattached", "detached"]);
}

#[test]
fn a_mutant_removed_on_purpose_does_not_come_back_with_its_entry() {
    let _turn = turn();
    let hosted = hosted();
    let entries = hosted.entries(&hosted.playlist);
    let mutant = hosted.mutate(&entries[1], json!({}));
    hosted
        .service
        .call(
            commands::MUTANTS_REMOVE,
            json!({ "id": mutant, "source": { "kind": "dj" } }),
        )
        .unwrap();

    let playlist = hosted.playlist.clone();
    let track = hosted.tracks[1].clone();
    hosted.window("Remove Tracks from Playlist", move |writer| {
        writer
            .remove_tracks_with_undo(&playlist, &[track])
            .map(|(_, edit)| LibraryEdit::RemovePlaylistTracks(edit))
    });
    hosted.undo();
    assert!(hosted.live().is_empty());
    assert_eq!(hosted.history(mutant), ["created", "removed"]);
}

#[test]
fn a_replacement_import_carries_each_mutant_to_its_tracks_new_entry_and_undo_and_redo_follow() {
    let _turn = turn();
    let hosted = hosted();
    let entries = hosted.entries(&hosted.playlist);
    let kept = hosted.mutate(&entries[0], json!({}));
    let dropped = hosted.mutate(&entries[1], json!({}));

    // An XML import's "Replace existing": the playlist now holds tracks 3,
    // 1 and 4, every entry written afresh.
    let playlist = hosted.playlist.clone();
    let contents = [
        hosted.tracks[2].clone(),
        hosted.tracks[0].clone(),
        fixture::track_id(0),
    ];
    hosted.window("Import", move |writer| {
        let (_, replaced) = writer.set_tracks_with_undo(&playlist, &contents)?;
        Ok(LibraryEdit::ReplacePlaylistContents(vec![replaced.unwrap()]))
    });
    let new_entries = hosted.entries(&hosted.playlist);
    assert!(new_entries.iter().all(|entry| !entries.contains(entry)));

    // Track 1's Mutant is now on track 1's new entry, at position 2; track
    // 2 left the playlist and so did its Mutant.
    let live = hosted.live();
    assert_eq!(live, vec![(new_entries[1].clone(), hosted.tracks[0].clone())]);
    let carried = hosted
        .service
        .call(commands::MUTANTS_GET, json!({ "entry": new_entries[1] }))
        .unwrap();
    assert_eq!(carried["mutant"]["position"], 2);
    assert_eq!(carried["history"][0]["action"], "carried");
    let carried = carried["mutant"]["id"].as_i64().unwrap();
    assert_eq!(hosted.mutant(kept)["mutant"]["removed"], true);
    assert_eq!(hosted.mutant(dropped)["mutant"]["removed"], true);

    hosted.undo();
    let mut live = hosted.live();
    live.sort();
    let mut expected = vec![
        (entries[0].clone(), hosted.tracks[0].clone()),
        (entries[1].clone(), hosted.tracks[1].clone()),
    ];
    expected.sort();
    assert_eq!(live, expected);
    assert_eq!(hosted.mutant(carried)["mutant"]["removed"], true);

    hosted.redo();
    assert_eq!(live_ids(&hosted), vec![carried]);
    hosted.undo();
    let mut back = live_ids(&hosted);
    back.sort();
    assert_eq!(back, vec![kept, dropped]);
}

#[test]
fn reordering_by_entry_or_by_track_leaves_every_mutant_on_its_entry() {
    let _turn = turn();
    let hosted = hosted();
    let entries = hosted.entries(&hosted.playlist);
    let mutant = hosted.mutate(&entries[0], json!({}));

    let playlist = hosted.playlist.clone();
    let order = vec![entries[2].clone(), entries[1].clone(), entries[0].clone()];
    hosted.window("Reorder Playlist", move |writer| {
        rbl_library::entry_order::reorder(writer, &playlist, &order)
            .map(LibraryEdit::ReorderEntries)
    });
    let moved = hosted.mutant(mutant);
    assert_eq!(moved["mutant"]["entry_id"], entries[0].as_str());
    assert_eq!(moved["mutant"]["position"], 3);
    hosted.undo();
    assert_eq!(hosted.mutant(mutant)["mutant"]["position"], 1);
    assert_eq!(hosted.history(mutant), ["created"]);
    assert!(hosted.drain().is_empty());
}

#[test]
fn removing_a_track_from_the_collection_in_the_window_removes_its_mutants() {
    let _turn = turn();
    let hosted = hosted();
    let entries = hosted.entries(&hosted.playlist);
    let mutant = hosted.mutate(&entries[1], json!({}));
    let other = hosted.mutate(&entries[2], json!({}));

    let track = hosted.tracks[1].clone();
    // As the window's Remove from Collection: the write, then the history
    // emptied under the command's name, both under the gate.
    {
        let _gate = hosted.core.edit_gate().lock();
        hosted
            .core
            .write_then(
                &NoHost,
                rbl_library::History::Caller,
                |writer| writer.delete_tracks(&[track]).map(drop),
                |_, ()| Ok(()),
            )
            .unwrap();
        hosted
            .core
            .history()
            .lock()
            .permanent(Some("remove_from_collection"));
    }

    assert_eq!(hosted.mutant(mutant)["mutant"]["removed"], true);
    assert_eq!(hosted.mutant(other)["mutant"]["removed"], false);
}

fn live_ids(hosted: &Hosted) -> Vec<i64> {
    hosted
        .service
        .call(commands::MUTANTS_LIST, json!({}))
        .unwrap()["mutants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_i64().unwrap())
        .collect()
}
