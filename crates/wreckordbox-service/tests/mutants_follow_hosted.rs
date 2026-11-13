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
use rbl_library::ledger::Preparation;
use rbl_library::{History, LibraryCore, NoHost, Touched};
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
            self.edit(id, edits);
        }
        let _ = self.drain();
        id
    }

    /// Changes a Mutant's edits, as an agent would.
    fn edit(&self, id: i64, edits: Value) {
        self.service
            .call(
                commands::MUTANTS_EDIT,
                json!({ "id": id, "edits": edits, "source": { "kind": "agent", "name": "preparer" } }),
            )
            .unwrap();
    }

    /// A Mutant's edits as they stand.
    fn edits_of(&self, id: i64) -> Value {
        self.mutant(id)["mutant"]["edits"].clone()
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
    let mutant = hosted.mutate(&entries[1], json!({ "key_shift": 2 }));
    let other = hosted.mutate(&entries[2], json!({ "key_shift": -3 }));
    let edits = hosted.edits_of(mutant);
    assert_eq!(edits["key_shift"], 2);

    let playlist = hosted.playlist.clone();
    let track = hosted.tracks[1].clone();
    hosted.window("Remove Tracks from Playlist", move |writer| {
        writer
            .remove_tracks_with_undo(&playlist, &[track])
            .map(|(_, edit)| LibraryEdit::RemovePlaylistTracks(edit))
    });

    let gone = hosted.mutant(mutant);
    assert_eq!(gone["mutant"]["removed"], true, "{gone}");
    assert_eq!(
        hosted.live(),
        vec![(entries[2].clone(), hosted.tracks[2].clone())]
    );
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
        [
            "created",
            "edited",
            "detached",
            "reattached",
            "detached",
            "reattached"
        ]
    );
    assert_eq!(hosted.live().len(), 2);
    assert_eq!(hosted.edits_of(mutant), edits);
    assert_eq!(hosted.edits_of(other)["key_shift"], -3);
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
        vec![
            (first, "detached".to_owned()),
            (third, "detached".to_owned())
        ]
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
    assert_eq!(
        hosted.history(first),
        ["created", "detached", "reattached", "detached"]
    );
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

/// The window's route for an import's Replace existing: the write declared
/// as one that replaces entries, then the history entry, under one gate.
fn window_replaces(hosted: &Hosted, contents: Vec<String>) {
    let _gate = hosted.core.edit_gate().lock();
    let playlist = hosted.playlist.clone();
    let preparation = Preparation {
        replaces_entries: true,
        ..Preparation::default()
    };
    let replaced = hosted
        .core
        .write_then_prepared(
            &NoHost,
            History::Caller,
            &preparation,
            |writer| writer.set_tracks_with_undo(&playlist, &contents),
            |db, (_, replaced)| {
                hosted
                    .core
                    .refresh_after_edit(&NoHost, db, Touched::Playlists)?;
                Ok(replaced)
            },
        )
        .unwrap()
        .expect("the playlist changed");
    hosted.core.history().lock().record(
        LibraryEdit::ReplacePlaylistContents(vec![replaced]),
        "Import",
    );
}

/// Track 1 again at the end of the playlist: entries 1, 2, 3, 1.
fn with_track_one_twice(hosted: &Hosted) -> Vec<String> {
    let playlist = hosted.playlist.clone();
    let track = hosted.tracks[0].clone();
    hosted.window("Add Tracks to Playlist", move |writer| {
        writer
            .add_tracks_with_undo(&playlist, &[track], true)
            .map(|(_, addition)| LibraryEdit::AddPlaylistTracks {
                addition,
                transaction: None,
            })
    });
    let entries = hosted.entries(&hosted.playlist);
    assert_eq!(entries.len(), 4);
    entries
}

#[test]
fn a_replacement_import_carries_a_mutant_with_its_edits_and_undo_and_redo_keep_both_versions() {
    let _turn = turn();
    let hosted = hosted();
    let entries = with_track_one_twice(&hosted);
    let first = hosted.mutate(&entries[0], json!({ "key_shift": 1 }));
    let second = hosted.mutate(&entries[1], json!({ "key_shift": -2 }));
    let again = hosted.mutate(&entries[3], json!({ "tempo": 1.04 }));
    let (first_edits, second_edits, again_edits) = (
        hosted.edits_of(first),
        hosted.edits_of(second),
        hosted.edits_of(again),
    );

    // Tracks 3, 1 and a new one, every entry written afresh; track 1 once.
    window_replaces(
        &hosted,
        vec![
            hosted.tracks[2].clone(),
            hosted.tracks[0].clone(),
            fixture::track_id(0),
        ],
    );
    let new_entries = hosted.entries(&hosted.playlist);
    assert!(new_entries.iter().all(|entry| !entries.contains(entry)));
    // Track 1's first Mutant, with its own edits, on track 1's new entry.
    let live = live_ids(&hosted);
    assert_eq!(live.len(), 1);
    let carried = live[0];
    let shown = hosted.mutant(carried);
    assert_eq!(shown["mutant"]["entry_id"], new_entries[1].as_str());
    assert_eq!(shown["mutant"]["edits"], first_edits);
    assert_eq!(shown["history"][0]["action"], "carried");
    for gone in [first, second, again] {
        assert_eq!(hosted.mutant(gone)["mutant"]["removed"], true);
    }

    // The carried Mutant and the original are two versions from now on.
    hosted.edit(carried, json!({ "key_shift": 3 }));
    let carried_edits = hosted.edits_of(carried);
    assert_eq!(carried_edits["key_shift"], 3);
    assert_eq!(hosted.edits_of(first), first_edits);

    hosted.undo();
    let mut back = live_ids(&hosted);
    back.sort_unstable();
    assert_eq!(back, vec![first, second, again]);
    assert_eq!(hosted.edits_of(first), first_edits);
    assert_eq!(hosted.edits_of(second), second_edits);
    assert_eq!(hosted.edits_of(again), again_edits);
    assert_eq!(
        hosted.mutant(again)["mutant"]["entry_id"],
        entries[3].as_str()
    );

    hosted.redo();
    assert_eq!(live_ids(&hosted), vec![carried]);
    assert_eq!(hosted.edits_of(carried), carried_edits);
    hosted.undo();
    assert_eq!(live_ids(&hosted).len(), 3);
    assert_eq!(hosted.edits_of(first), first_edits);
}

#[test]
fn reordering_by_track_or_by_entry_with_a_track_twice_keeps_every_mutant_and_its_edits() {
    let _turn = turn();
    let hosted = hosted();
    let entries = with_track_one_twice(&hosted);
    let first = hosted.mutate(&entries[0], json!({ "key_shift": 1 }));
    let again = hosted.mutate(&entries[3], json!({ "tempo": 1.04 }));
    let (first_edits, again_edits) = (hosted.edits_of(first), hosted.edits_of(again));
    let _ = hosted.drain();

    // The window's older reorder, by track: both entries of track 1 move
    // together.
    let playlist = hosted.playlist.clone();
    let order = vec![
        hosted.tracks[2].clone(),
        hosted.tracks[0].clone(),
        hosted.tracks[1].clone(),
    ];
    hosted.window("Reorder Playlist", move |writer| {
        writer
            .reorder_with_undo(&playlist, &order)
            .map(|(_, edit)| LibraryEdit::ReorderPlaylist(edit))
    });
    // And by entry.
    let now = hosted.entries(&hosted.playlist);
    let mut by_entry = now.clone();
    by_entry.reverse();
    let playlist = hosted.playlist.clone();
    hosted.window("Reorder Playlist", move |writer| {
        rbl_library::entry_order::reorder(writer, &playlist, &by_entry)
            .map(LibraryEdit::ReorderEntries)
    });
    for step in [true, false, true, true, false] {
        if step {
            hosted.undo()
        } else {
            hosted.redo()
        }
        for (id, entry, edits) in [
            (first, &entries[0], &first_edits),
            (again, &entries[3], &again_edits),
        ] {
            let shown = hosted.mutant(id);
            assert_eq!(shown["mutant"]["removed"], false);
            assert_eq!(shown["mutant"]["entry_id"], entry.as_str());
            assert_eq!(&shown["mutant"]["edits"], edits);
        }
    }
    assert_eq!(hosted.history(first), ["created", "edited"]);
    assert!(hosted.drain().is_empty());
}

#[test]
fn a_copy_is_one_undo_step_and_its_mutants_are_versions_of_their_own() {
    use wreckordbox_service::playlist_edits::commands as edits;
    let _turn = turn();
    let hosted = hosted();
    let entries = with_track_one_twice(&hosted);
    let first = hosted.mutate(&entries[0], json!({ "key_shift": 1 }));
    let again = hosted.mutate(&entries[3], json!({ "tempo": 1.04 }));
    let (first_edits, again_edits) = (hosted.edits_of(first), hosted.edits_of(again));

    let copied = hosted
        .service
        .call(
            edits::PLAYLISTS_COPY,
            json!({ "playlist": hosted.playlist, "name": "Friday again" }),
        )
        .unwrap();
    let copy = copied["playlist"].as_str().unwrap().to_owned();
    let copy_entries = hosted.entries(&copy);
    assert_eq!(copy_entries.len(), 4);
    let copies: Vec<i64> = copied["mutants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_i64().unwrap())
        .collect();
    assert_eq!(copies.len(), 2);
    let (copy_first, copy_again) = (copies[0], copies[1]);
    assert_eq!(
        hosted.mutant(copy_first)["mutant"]["entry_id"],
        copy_entries[0].as_str()
    );
    assert_eq!(hosted.edits_of(copy_first), first_edits);
    assert_eq!(
        hosted.mutant(copy_again)["mutant"]["entry_id"],
        copy_entries[3].as_str()
    );
    assert_eq!(hosted.edits_of(copy_again), again_edits);
    assert_eq!(hosted.history(copy_first), ["copied"]);

    // Each is edited on its own.
    hosted.edit(copy_first, json!({ "key_shift": 4 }));
    hosted.edit(first, json!({ "key_shift": -1 }));
    let (copy_first_edits, first_now) = (hosted.edits_of(copy_first), hosted.edits_of(first));
    assert_eq!(copy_first_edits["key_shift"], 4);
    assert_eq!(first_now["key_shift"], -1);

    // One Undo takes the whole copy back, Mutants and all.
    hosted.undo();
    assert_eq!(
        hosted
            .service
            .call(PLAYLISTS_TRACKS, json!({ "id": copy }))
            .unwrap_err()
            .code,
        "playlist_not_found"
    );
    assert_eq!(hosted.mutant(copy_first)["mutant"]["removed"], true);
    assert_eq!(hosted.mutant(copy_again)["mutant"]["removed"], true);
    assert_eq!(hosted.edits_of(first), first_now);
    hosted.redo();
    assert_eq!(hosted.entries(&copy), copy_entries);
    assert_eq!(hosted.mutant(copy_first)["mutant"]["removed"], false);
    assert_eq!(hosted.edits_of(copy_first), copy_first_edits);
    assert_eq!(hosted.edits_of(copy_again), again_edits);
    assert_eq!(hosted.edits_of(first), first_now);
}

#[test]
fn a_mutant_change_that_cannot_be_written_keeps_the_entry_waiting_instead_of_dropping_it() {
    let _turn = turn();
    let hosted = hosted();
    let entries = hosted.entries(&hosted.playlist);
    let mutant = hosted.mutate(&entries[1], json!({ "key_shift": 2 }));
    // The Sidecar refuses to record a Mutant leaving its entry.
    hosted
        .sidecar()
        .execute_batch(
            "CREATE TRIGGER refuse_detach BEFORE INSERT ON mutant_change
             WHEN NEW.action = 'detached' BEGIN SELECT RAISE(ABORT, 'refused for the test'); END;",
        )
        .unwrap();

    let playlist = hosted.playlist.clone();
    let track = hosted.tracks[1].clone();
    hosted.window("Remove Tracks from Playlist", move |writer| {
        writer
            .remove_tracks_with_undo(&playlist, &[track])
            .map(|(_, edit)| LibraryEdit::RemovePlaylistTracks(edit))
    });
    // The change is in the Library; its entry and the Mutant's consequence
    // wait together, neither recorded without the other.
    let entries_of = |kind: &str| -> i64 {
        hosted
            .sidecar()
            .query_row("SELECT count(*) FROM ledger WHERE kind = ?1", [kind], |r| {
                r.get(0)
            })
            .unwrap()
    };
    assert_eq!(entries_of("remove_playlist_tracks"), 0);
    assert_eq!(hosted.mutant(mutant)["mutant"]["removed"], false);
    assert!(hosted
        .dir
        .path()
        .join("sidecar")
        .join("ledger-spill.jsonl")
        .is_file());
    // Nothing more is changed while it waits.
    let playlist = hosted.playlist.clone();
    let track = hosted.tracks[2].clone();
    let refused = hosted.core.recorded_edit(
        &NoHost,
        Touched::Playlists,
        "Remove Tracks from Playlist",
        |writer| {
            writer
                .remove_tracks_with_undo(&playlist, &[track])
                .map(|(_, edit)| ((), LibraryEdit::RemovePlaylistTracks(edit)))
        },
    );
    assert!(refused.is_err());

    hosted
        .sidecar()
        .execute_batch("DROP TRIGGER refuse_detach")
        .unwrap();
    let playlist = hosted.playlist.clone();
    let track = hosted.tracks[2].clone();
    hosted.window("Remove Tracks from Playlist", move |writer| {
        writer
            .remove_tracks_with_undo(&playlist, &[track])
            .map(|(_, edit)| LibraryEdit::RemovePlaylistTracks(edit))
    });
    assert_eq!(entries_of("remove_playlist_tracks"), 2);
    assert_eq!(hosted.mutant(mutant)["mutant"]["removed"], true);
    assert_eq!(hosted.history(mutant), ["created", "edited", "detached"]);
    let noted: i64 = hosted
        .sidecar()
        .query_row(
            "SELECT count(*) FROM ledger WHERE payload LIKE '%mutants_not_followed%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(noted, 0);
}

#[test]
fn a_copy_whose_entry_had_to_wait_gets_the_edits_the_source_had_when_it_was_copied() {
    use wreckordbox_service::playlist_edits::commands as edits;
    let _turn = turn();
    std::env::set_var("WRECKORDBOX_LEDGER_FAULTS", "fail_append=1");
    let hosted = hosted();
    std::env::remove_var("WRECKORDBOX_LEDGER_FAULTS");
    let entries = hosted.entries(&hosted.playlist);
    let source = hosted.mutate(&entries[1], json!({ "key_shift": 1 }));
    let at_copy = hosted.edits_of(source);

    let copied = hosted
        .service
        .call(
            edits::PLAYLISTS_COPY,
            json!({ "playlist": hosted.playlist }),
        )
        .unwrap();
    let copy = copied["playlist"].as_str().unwrap().to_owned();
    // The entry was refused, so the copy's Mutant waits with it.
    assert!(copied["mutants"].as_array().unwrap().is_empty());
    // The source changes before the waiting entry is written.
    hosted.edit(source, json!({ "key_shift": 5 }));

    let playlist = hosted.playlist.clone();
    let track = hosted.tracks[2].clone();
    hosted.window("Remove Tracks from Playlist", move |writer| {
        writer
            .remove_tracks_with_undo(&playlist, &[track])
            .map(|(_, edit)| LibraryEdit::RemovePlaylistTracks(edit))
    });
    let copies = hosted
        .service
        .call(commands::MUTANTS_LIST, json!({ "playlist": copy }))
        .unwrap()["mutants"]
        .clone();
    assert_eq!(copies.as_array().unwrap().len(), 1, "{copies}");
    assert_eq!(copies[0]["edits"], at_copy);
    assert_eq!(hosted.edits_of(source)["key_shift"], 5);
}

#[test]
fn a_playlist_edits_change_is_told_to_the_host_even_when_its_answer_fails() {
    use wreckordbox_service::playlist_edits::commands as edits;
    let _turn = turn();
    let hosted = hosted();
    let entries = hosted.entries(&hosted.playlist);
    hosted.mutate(&entries[1], json!({}));
    let _ = hosted.events.try_iter().count();
    std::env::set_var("WRECKORDBOX_EDIT_FAULTS", "fail_answer");
    let failed = hosted.service.call(
        edits::PLAYLISTS_REMOVE,
        json!({ "playlist": hosted.playlist, "tracks": [hosted.tracks[1]] }),
    );
    std::env::remove_var("WRECKORDBOX_EDIT_FAULTS");
    assert!(failed.is_err());
    assert_eq!(hosted.entries(&hosted.playlist).len(), 2);
    let mut told = false;
    while let Ok(event) = hosted.events.recv_timeout(Duration::from_millis(300)) {
        told |= event.event == "library_changed";
    }
    assert!(told, "the host was not told of the change");
}

/// Hosts a service again on what a stopped one left.
fn host_again(
    dir: &std::path::Path,
    location: &LibraryLocation,
    core: &Arc<LibraryCore>,
) -> Service {
    let config = Config {
        runtime_dir: dir.join("run"),
        idle_exit: None,
        recovery_dir: dir.join("recovery"),
    };
    let host: Arc<dyn rbl_library::Host + Send + Sync> = Arc::new(NoHost);
    Service::host(location, &config, core, &host).unwrap()
}

/// An intent left open, as by a process that died after its Library write.
fn leave_an_open_intent(sidecar: &rusqlite::Connection) {
    sidecar
        .execute_batch(
            "INSERT INTO ledger_order DEFAULT VALUES;
             INSERT INTO ledger_intent (id, run_id, at, source_kind, source_name, description, command)
             VALUES (last_insert_rowid(), NULL, 0, 'dj', 'dj', 'Edits in the window', NULL);",
        )
        .unwrap();
}

#[test]
fn recovery_never_takes_a_failed_read_for_a_removed_entry_and_checks_the_playlist_and_parent_too() {
    let _turn = turn();
    let hosted = hosted();
    let entries = hosted.entries(&hosted.playlist);
    let removed = hosted.mutate(&entries[0], json!({ "key_shift": 1 }));
    let parent_gone = hosted.mutate(&entries[1], json!({ "key_shift": 2 }));
    let kept = hosted.mutate(&entries[2], json!({ "key_shift": 3 }));
    let Hosted {
        dir,
        location,
        core,
        service,
        playlist,
        tracks,
        ..
    } = hosted;
    service.stop();
    drop(service);

    // What a write that died before its entry left: the first entry out of
    // the playlist, the second's track out of the collection (its entry
    // row still there), and an open intent.
    {
        let mut writer =
            rbl_db::write::Writer::open(location.clone(), dir.path().join("backups")).unwrap();
        writer.disable_automatic_backups();
        writer
            .remove_tracks(&playlist, &[tracks[0].clone()])
            .unwrap();
        writer
            .library()
            .connection()
            .execute(
                "UPDATE djmdContent SET rb_local_deleted = 1 WHERE ID = ?1",
                [&tracks[1]],
            )
            .unwrap();
    }
    let sidecar_db = dir.path().join("sidecar").join("sidecar.db");
    leave_an_open_intent(&rusqlite::Connection::open(&sidecar_db).unwrap());
    let open_intents = || -> i64 {
        rusqlite::Connection::open(&sidecar_db)
            .unwrap()
            .query_row("SELECT count(*) FROM ledger_intent", [], |r| r.get(0))
            .unwrap()
    };

    // The Library cannot be read when the next service recovers.
    use std::os::unix::fs::PermissionsExt;
    let master = location.master_db.clone();
    let readable = std::fs::metadata(&master).unwrap().permissions();
    std::fs::set_permissions(&master, std::fs::Permissions::from_mode(0o000)).unwrap();
    let service = host_again(dir.path(), &location, &core);
    // Read from the Sidecar itself: the Library may be unreadable.
    let live = |_: &Service| -> Vec<i64> {
        let db = rusqlite::Connection::open(&sidecar_db).unwrap();
        let mut rows = db
            .prepare("SELECT id FROM mutant WHERE removed_at IS NULL ORDER BY id")
            .unwrap();
        let found = rows
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        found
    };
    let mut still = live(&service);
    still.sort_unstable();
    assert_eq!(
        still,
        vec![removed, parent_gone, kept],
        "nothing is assumed gone"
    );
    assert_eq!(open_intents(), 1, "the change stays to be recovered");
    service.stop();
    drop(service);

    // Readable again: recovered, by the entry, its playlist and its track.
    std::fs::set_permissions(&master, readable).unwrap();
    let service = host_again(dir.path(), &location, &core);
    assert_eq!(live(&service), vec![kept]);
    assert_eq!(open_intents(), 0);
}

#[test]
fn five_thousand_mutated_entries_are_reordered_removed_and_put_back_in_bounded_time() {
    use std::time::Instant;
    use wreckordbox_service::playlist_edits::commands as edits;
    let _turn = turn();
    const N: usize = 5_000;
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    let location = fixture::build(
        &library,
        Shape {
            tracks: N,
            ..Shape::default()
        },
    )
    .unwrap();
    let tracks: Vec<String> = (0..N).map(fixture::track_id).collect();
    let playlist = {
        let mut writer =
            rbl_db::write::Writer::open(location.clone(), dir.path().join("backups")).unwrap();
        writer.disable_automatic_backups();
        let playlist = writer.create_playlist("Long", "root").unwrap();
        writer.add_tracks(&playlist, &tracks).unwrap();
        playlist
    };
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
    let service = Service::host(&location, &config, &core, &host).unwrap();

    let entries: Vec<String> = service
        .call(PLAYLISTS_TRACKS, json!({ "id": playlist, "limit": N }))
        .unwrap()["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["entry_id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(entries.len(), N);
    let made = service
        .call(
            commands::MUTANTS_CREATE,
            json!({ "entry": entries[0], "source": { "kind": "agent", "name": "load" } }),
        )
        .unwrap();
    let run = made["run"]["id"].as_i64().unwrap();
    {
        let mut sidecar =
            rusqlite::Connection::open(dir.path().join("sidecar").join("sidecar.db")).unwrap();
        let tx = sidecar.transaction().unwrap();
        for (entry, track) in entries.iter().zip(&tracks).skip(1) {
            tx.execute(
                "INSERT INTO mutant (playlist_id, entry_id, track_id, fingerprint, created_at, created_run)
                 VALUES (?1, ?2, ?3, NULL, 0, ?4)",
                rusqlite::params![playlist, entry, track, run],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO mutant_change (mutant_id, action, edits, fingerprint, source_kind, source_name, run_id, recorded_at)
                 VALUES (last_insert_rowid(), 'created', '{\"key_shift\":1}', NULL, 'agent', 'load', ?1, 0)",
                [run],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }

    let timed = |what: &str, work: &mut dyn FnMut()| {
        let started = Instant::now();
        work();
        let took = started.elapsed();
        eprintln!("{what}: {} ms", took.as_millis());
        assert!(took.as_secs() < 30, "{what} took {took:?}");
    };
    timed("mutants.list of 5000", &mut || {
        let listed = service
            .call(commands::MUTANTS_LIST, json!({ "playlist": playlist }))
            .unwrap();
        assert_eq!(listed["total"], N);
    });
    let mut reversed = entries.clone();
    reversed.reverse();
    timed("playlists.reorder of 5000 mutated entries", &mut || {
        let answer = service
            .call(
                edits::PLAYLISTS_REORDER,
                json!({ "playlist": playlist, "entries": reversed }),
            )
            .unwrap();
        assert_eq!(answer["mutants"].as_array().unwrap().len(), N);
        assert_eq!(answer["mutants"][0]["position"], N);
    });
    let half: Vec<String> = tracks[..N / 2].to_vec();
    timed("playlists.remove of 2500 mutated entries", &mut || {
        let answer = service
            .call(
                edits::PLAYLISTS_REMOVE,
                json!({ "playlist": playlist, "tracks": half }),
            )
            .unwrap();
        assert_eq!(answer["mutants"].as_array().unwrap().len(), N / 2);
    });
    timed("window Undo of the removal", &mut || {
        core.step_history(&NoHost, true).unwrap();
    });
    assert_eq!(
        service
            .call(commands::MUTANTS_LIST, json!({ "playlist": playlist }))
            .unwrap()["total"],
        N
    );
}

/// A core and a service hosted on a Library already on disk, as a window
/// that starts again finds it.
fn host_fresh(dir: &std::path::Path, location: &LibraryLocation) -> (Arc<LibraryCore>, Service) {
    let recovery = dir.join("recovery");
    let core = Arc::new(LibraryCore::new(recovery));
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
    let service = host_again(dir, location, &core);
    (core, service)
}

const CHILD_DIR: &str = "WBX80_CHILD_DIR";
const CHILD_KEY: &str = "WBX80_CHILD_KEY";
const CHILD_PLAYLIST: &str = "WBX80_CHILD_PLAYLIST";

/// Run by the next test in a process of its own: the window's Replace
/// existing, in a service told to die once the Library is written.
#[test]
#[ignore = "run by a_replacement_whose_process_died_after_the_write_still_carries_its_mutants"]
fn replace_and_die() {
    let Ok(dir) = std::env::var(CHILD_DIR) else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let location = LibraryLocation {
        master_db: dir.join("library").join("master.db"),
        share_root: dir.join("library").join("share"),
        passphrase: std::env::var(CHILD_KEY).unwrap(),
        is_real_install: false,
    };
    std::env::set_var(SIDECAR_DIR_ENV, dir.join("sidecar"));
    let (core, service) = host_fresh(&dir, &location);
    let playlist = std::env::var(CHILD_PLAYLIST).unwrap();
    let contents = vec![
        fixture::track_id(3),
        fixture::track_id(1),
        fixture::track_id(0),
    ];
    let _gate = core.edit_gate().lock();
    let preparation = Preparation {
        replaces_entries: true,
        ..Preparation::default()
    };
    let _ = core.write_then_prepared(
        &NoHost,
        History::Caller,
        &preparation,
        |writer| writer.set_tracks_with_undo(&playlist, &contents),
        |_, replaced| Ok(replaced),
    );
    drop(service);
    unreachable!("the service should have died after the write");
}

#[test]
fn a_replacement_whose_process_died_after_the_write_still_carries_its_mutants() {
    let _turn = turn();
    let hosted = hosted();
    let entries = hosted.entries(&hosted.playlist);
    let first = hosted.mutate(&entries[0], json!({ "key_shift": 1 }));
    let second = hosted.mutate(&entries[1], json!({ "key_shift": -2 }));
    let first_edits = hosted.edits_of(first);
    let Hosted {
        dir,
        location,
        service,
        playlist,
        ..
    } = hosted;
    service.stop();
    drop(service);

    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "replace_and_die", "--ignored", "--nocapture"])
        .env(CHILD_DIR, dir.path())
        .env(CHILD_KEY, &location.passphrase)
        .env(CHILD_PLAYLIST, &playlist)
        .env("WRECKORDBOX_LEDGER_FAULTS", "abort_after_write")
        .output()
        .unwrap();
    assert!(
        !child.status.success(),
        "the child did not die: {}",
        String::from_utf8_lossy(&child.stdout)
    );

    // The replacement is in the Library and its entry was never written.
    let (_core, service) = host_fresh(dir.path(), &location);
    let listed = service
        .call(PLAYLISTS_TRACKS, json!({ "id": playlist }))
        .unwrap()["tracks"]
        .clone();
    let new_entries: Vec<String> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["entry_id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(new_entries.len(), 3);
    assert!(new_entries.iter().all(|entry| !entries.contains(entry)));

    // Recovered from the plan: track 1's Mutant on its new entry with its
    // edits, track 2's gone with its entry.
    let live = service.call(commands::MUTANTS_LIST, json!({})).unwrap()["mutants"].clone();
    assert_eq!(live.as_array().unwrap().len(), 1, "{live}");
    assert_eq!(live[0]["entry_id"], new_entries[1].as_str());
    assert_eq!(live[0]["edits"], first_edits);
    let carried = live[0]["id"].as_i64().unwrap();
    let shown = service
        .call(commands::MUTANTS_GET, json!({ "id": carried }))
        .unwrap();
    assert_eq!(shown["history"][0]["action"], "carried");
    for gone in [first, second] {
        let shown = service
            .call(commands::MUTANTS_GET, json!({ "id": gone }))
            .unwrap();
        assert_eq!(shown["mutant"]["removed"], true);
    }
    // Recovered once.
    service.stop();
    drop(service);
    let (_core, service) = host_fresh(dir.path(), &location);
    let live = service.call(commands::MUTANTS_LIST, json!({})).unwrap()["mutants"].clone();
    assert_eq!(live.as_array().unwrap().len(), 1);
}

#[path = "support/bulk_follow.rs"]
mod bulk_follow;
