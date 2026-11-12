//! Playlist entries in the window (wbx-59): read with their own identity,
//! and reordered by entry rather than by track, so a track that is in a
//! playlist twice is two entries that move independently, each keeping its
//! identifier (and the Mutant keyed to it).
//!
//! The same command functions the webview invokes, on a mock Tauri app over
//! a fixture Library built in a temporary folder.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::sync::Arc;

use rbl_db::fixture::{self, track_id, Shape};
use rbl_db::write::ROOT;
use rbl_db::{Library as Db, OpenMode};
use rbxport_lib::commands;
use rbxport_lib::dto::{TrackFilterDto, TrackSourceDto, TreeNodeDto, ViewSpecDto};
use rbxport_lib::playlist_entries::{self, PlaylistEntryDto};
use rbxport_lib::state::AppState;
use rbxport_lib::ErrorKind;
use serde_json::json;
use tauri::test::MockRuntime;
use tauri::{AppHandle, Manager, State};
use wreckordbox_sidecar::mutants::NewMutant;
use wreckordbox_sidecar::{Sidecar, Source, SourceKind};

struct Shell {
    dir: tempfile::TempDir,
    app: tauri::App<MockRuntime>,
    location: rbl_db::LibraryLocation,
}

fn shell() -> Shell {
    let dir = tempfile::tempdir().unwrap();
    let location = fixture::build(dir.path(), Shape::default()).expect("build the fixture");
    let state = AppState::with_backups(dir.path().join("backups"));
    state.core().set_protection(false).unwrap();
    let db = Db::open(location.clone(), OpenMode::ReadOnly).expect("open the fixture");
    let (library, _) = rbl_index::load(&db).expect("index the fixture");
    state.set_library(library, false, db.schema().db_version, 0, location.clone());
    let app = tauri::test::mock_app();
    app.manage(Arc::new(state));
    Shell { dir, app, location }
}

fn run<T>(fut: impl std::future::Future<Output = T>) -> T {
    tauri::async_runtime::block_on(fut)
}

impl Shell {
    fn handle(&self) -> AppHandle<MockRuntime> {
        self.app.handle().clone()
    }
    fn state(&self) -> State<'_, Arc<AppState>> {
        self.app.state::<Arc<AppState>>()
    }
    fn node(&self, name: &str) -> TreeNodeDto {
        run(commands::playlist_tree(self.state()))
            .unwrap()
            .into_iter()
            .find(|n| n.name == name)
            .unwrap()
    }
    fn entries(&self, playlist: &str) -> Vec<PlaylistEntryDto> {
        run(playlist_entries::playlist_entries(
            self.state(),
            playlist.to_owned(),
        ))
        .unwrap()
    }
    fn entry_ids(&self, playlist: &str) -> Vec<String> {
        self.entries(playlist)
            .into_iter()
            .map(|e| e.entry_id)
            .collect()
    }
    /// The playlist's tracks as the main list lists them.
    fn listed(&self, playlist: &str) -> Vec<String> {
        let spec = ViewSpecDto {
            source: TrackSourceDto::Playlist {
                id: playlist.to_owned(),
            },
            sort: "trackNo".into(),
            descending: false,
            query: String::new(),
            search_field: rbl_index::SearchField::All,
            filter: TrackFilterDto::default(),
        };
        let handle = run(commands::open_view(self.state(), spec)).unwrap();
        run(commands::fetch_rows(
            self.state(),
            handle.view_id,
            0,
            commands::MAX_ROWS,
            None,
        ))
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect()
    }
    /// Each live entry's `TrackNo`, read from the Library as rekordbox would.
    fn numbers(&self, playlist: &str) -> Vec<(String, i64)> {
        let db = Db::open(self.location.clone(), OpenMode::ReadOnly).unwrap();
        let mut stmt = db
            .connection()
            .prepare("SELECT ID, TrackNo FROM djmdSongPlaylist WHERE PlaylistID = ?1 AND rb_local_deleted = 0 ORDER BY TrackNo, rowid")
            .unwrap();
        stmt.query_map([playlist], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }
    fn reorder(&self, playlist: &str, order: &[String]) -> Result<u32, rbxport_lib::AppError> {
        run(playlist_entries::reorder_playlist_entries(
            self.handle(),
            self.state(),
            playlist.to_owned(),
            order.to_vec(),
        ))
    }
}

/// "Dupes": track 1, track 2, track 1 again: three entries, two of one track.
fn dupes(s: &Shell) -> String {
    run(commands::create_playlist(
        s.handle(),
        s.state(),
        "Dupes".into(),
        ROOT.into(),
        None,
    ))
    .unwrap();
    let id = s.node("Dupes").id;
    run(commands::add_tracks_to_playlist(
        s.handle(),
        s.state(),
        id.clone(),
        vec![track_id(1), track_id(2)],
        None,
        None,
    ))
    .unwrap();
    run(commands::add_tracks_to_playlist(
        s.handle(),
        s.state(),
        id.clone(),
        vec![track_id(1)],
        Some(true),
        None,
    ))
    .unwrap();
    id
}

#[test]
fn entries_are_read_in_order_with_their_own_identity_and_track() {
    let s = shell();
    let id = dupes(&s);
    let entries = s.entries(&id);
    assert_eq!(
        entries
            .iter()
            .map(|e| e.track_id.as_str())
            .collect::<Vec<_>>(),
        [track_id(1), track_id(2), track_id(1)]
    );
    let mut ids: Vec<_> = entries.iter().map(|e| e.entry_id.clone()).collect();
    ids.dedup();
    assert_eq!(ids.len(), 3, "two entries of one track are two entries");
    // The same order the main list lists.
    assert_eq!(
        entries
            .iter()
            .map(|e| e.track_id.clone())
            .collect::<Vec<_>>(),
        s.listed(&id)
    );
    // An intelligent playlist, a folder and an unknown id have no entries.
    assert!(s.entries("no-such-playlist").is_empty());
}

#[test]
fn one_entry_of_a_duplicated_track_moves_alone_keeping_its_mutant_and_undo_and_redo_put_it_back_and_again(
) {
    let s = shell();
    let id = dupes(&s);
    let [first, two, second]: [String; 3] = s.entry_ids(&id).try_into().unwrap();

    // A distinct Mutant on each entry of track 1.
    let sidecar = Sidecar::open(&s.dir.path().join("sidecar")).unwrap();
    let source = Source::new(SourceKind::Dj, "dj");
    let run_id = sidecar.begin_run(&source, "Friday edits").unwrap();
    for (entry, shift) in [(&first, 1), (&second, -2)] {
        sidecar
            .create_mutant(&NewMutant {
                playlist_id: &id,
                entry_id: entry,
                track_id: &track_id(1),
                fingerprint: None,
                edits: json!({ "key_shift": shift }),
                source: &source,
                run: run_id.into(),
            })
            .unwrap();
    }

    // The second entry of track 1 to the top: exactly that entry moves.
    let order = vec![second.clone(), first.clone(), two.clone()];
    s.reorder(&id, &order).unwrap();
    assert_eq!(s.entry_ids(&id), order);
    assert_eq!(s.listed(&id), [track_id(1), track_id(1), track_id(2)]);
    assert_eq!(
        s.numbers(&id),
        [(second.clone(), 1), (first.clone(), 2), (two.clone(), 3)],
        "every entry its own number, not one number for both of track 1"
    );
    // Each Mutant is still on its own entry, now at that entry's new place.
    assert_eq!(
        sidecar.mutant_of_entry(&second).unwrap().unwrap().edits,
        json!({ "key_shift": -2 })
    );
    assert_eq!(
        sidecar.mutant_of_entry(&first).unwrap().unwrap().edits,
        json!({ "key_shift": 1 })
    );

    // One Undo puts back the order and numbers it had; one Redo the new.
    let undone = run(commands::undo_edit(s.handle(), s.state())).unwrap();
    assert_eq!(undone.redo_label.as_deref(), Some("Reorder Playlist"));
    assert_eq!(
        s.entry_ids(&id),
        [first.clone(), two.clone(), second.clone()]
    );
    assert_eq!(s.listed(&id), [track_id(1), track_id(2), track_id(1)]);
    run(commands::redo_edit(s.handle(), s.state())).unwrap();
    assert_eq!(s.entry_ids(&id), order);
    assert_eq!(
        sidecar.mutant_of_entry(&second).unwrap().unwrap().edits,
        json!({ "key_shift": -2 })
    );
}

#[test]
fn an_order_that_is_not_exactly_the_playlists_entries_is_refused_with_nothing_written() {
    let s = shell();
    let id = dupes(&s);
    let entries = s.entry_ids(&id);
    let before = s.numbers(&id);

    // One left out (an entry added since the window read the playlist).
    let error = s.reorder(&id, &entries[..2]).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Conflict);
    // One named twice, and one that is not the playlist's.
    let twice = vec![entries[0].clone(), entries[0].clone(), entries[1].clone()];
    assert_eq!(
        s.reorder(&id, &twice).unwrap_err().kind,
        ErrorKind::Conflict
    );
    let foreign = vec![
        entries[0].clone(),
        entries[1].clone(),
        "not-an-entry".into(),
    ];
    assert_eq!(
        s.reorder(&id, &foreign).unwrap_err().kind,
        ErrorKind::Conflict
    );
    assert_eq!(s.numbers(&id), before);
}

#[test]
fn under_library_protection_a_reorder_is_refused_with_nothing_written() {
    let s = shell();
    let id = dupes(&s);
    let entries = s.entry_ids(&id);
    let before = s.numbers(&id);
    s.state().core().set_protection(true).unwrap();
    let reversed: Vec<String> = entries.iter().rev().cloned().collect();
    let error = s.reorder(&id, &reversed).unwrap_err();
    assert!(
        error.message.contains("Library Protection"),
        "{}",
        error.message
    );
    assert_eq!(s.numbers(&id), before);
}
