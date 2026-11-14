//! Stick Protection hardening (wbx-45.14): a Stick locked or replaced in
//! the middle of a write that began while it was unlocked.
//!
//! Every race here is controlled by the shell's test holds
//! (`rbxport_lib::test_holds`): the writer is held at a named moment, the
//! test does what the race needs (locks the Stick, edits the Library), then
//! lets it go. No sleeps. Temporary folders stand in for Sticks.

#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rbl_db::fixture::{self, Shape};
use rbl_db::{Library as Db, LibraryLocation, OpenMode};
use rbxport_lib::state::AppState;
use rbxport_lib::{commands, details, key_tag, test_holds};
use tauri::test::MockRuntime;
use tauri::{App, Manager};
use wreckordbox_stick_protection::STAND_IN_MARKER;

pub(crate) struct Rig {
    pub(crate) dir: tempfile::TempDir,
    pub(crate) app: App<MockRuntime>,
    pub(crate) location: LibraryLocation,
    _sticks: tempfile::TempDir,
    pub(crate) stick: PathBuf,
}

pub(crate) fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let location = fixture::build(dir.path(), Shape::default()).unwrap();
    let state = AppState::with_backups(dir.path().join("backups"));
    state.core().set_protection(false).unwrap();
    let db = Db::open(location.clone(), OpenMode::ReadOnly).unwrap();
    let (library, _) = rbl_index::load(&db).unwrap();
    state.set_library(library, false, db.schema().db_version, 0, location.clone());
    let app = tauri::test::mock_app();
    app.manage(Arc::new(state));
    app.manage(Arc::new(rbxport_lib::grid::GridEditor::at(
        &dir.path().join("editor"),
    )));
    let sticks = tempfile::tempdir().unwrap();
    let stick = sticks.path().join("GIGSTICK");
    std::fs::create_dir_all(stick.join("Contents")).unwrap();
    std::fs::write(stick.join(STAND_IN_MARKER), b"").unwrap();
    Rig {
        dir,
        app,
        location,
        _sticks: sticks,
        stick,
    }
}

impl Rig {
    pub(crate) fn state(&self) -> Arc<AppState> {
        Arc::clone(self.app.state::<Arc<AppState>>().inner())
    }
    pub(crate) fn state_dir(&self) -> PathBuf {
        self.state().backup_dir().to_path_buf()
    }
    pub(crate) fn unlock(&self) {
        wreckordbox_stick_protection::unlock(&self.state_dir(), &self.stick, Some("GIGSTICK"))
            .unwrap();
    }
    pub(crate) fn protected(&self) -> bool {
        wreckordbox_stick_protection::status(&self.state_dir(), &self.stick).protected
    }
}

pub(crate) fn run<T>(future: impl std::future::Future<Output = T>) -> T {
    tauri::async_runtime::block_on(future)
}

/// The DJ locks the Stick now, from another client. A lock waits for a
/// writer holding the Stick's execution lock; when nobody holds it, the
/// lock takes effect at once (taken here, before the writer is let go).
/// Answers the thread of a lock that has to wait.
pub(crate) fn lock_now(rig: &Rig) -> Option<std::thread::JoinHandle<()>> {
    let state_dir = rig.state_dir();
    match wreckordbox_stick_protection::try_exec_lock(&state_dir, &rig.stick).unwrap() {
        Some(free) => {
            wreckordbox_stick_protection::lock(&state_dir, &rig.stick).unwrap();
            drop(free);
            None
        }
        None => {
            let stick = rig.stick.clone();
            Some(std::thread::spawn(move || {
                wreckordbox_stick_protection::lock(&state_dir, &stick).unwrap();
            }))
        }
    }
}

/// An MP3 on the Stick holding the key `Am`, imported: (file, track id).
fn track_on_stick(rig: &Rig) -> (PathBuf, String) {
    let audio = rig.stick.join("Contents/On Stick.mp3");
    let mut mp3 = Vec::new();
    for i in 0..40u8 {
        mp3.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x64]);
        mp3.extend(std::iter::repeat_n(i, 413));
    }
    std::fs::write(&audio, &mp3).unwrap();
    rbl_db::key_tag::write_key(&audio, "Am").unwrap();
    let id = run(commands::import_files(
        rig.app.handle().clone(),
        rig.app.state(),
        vec![audio.display().to_string()],
    ))
    .unwrap()
    .tracks[0]
        .id
        .clone();
    rig.state()
        .write(|w| {
            w.ensure_detected_key("Am")?;
            w.ensure_detected_key("Fm")?;
            w.ensure_detected_key("Gm")
        })
        .unwrap();
    (audio, id)
}

fn library_key(rig: &Rig, id: &str) -> String {
    run(details::track_details(rig.app.state(), id.to_owned()))
        .unwrap()
        .key
}

fn tag(audio: &Path) -> Option<String> {
    rbl_db::key_tag::read_key(audio).unwrap()
}

fn set_key(rig: &Rig, id: &str, key: &str) -> Result<(), String> {
    run(details::set_track_field(
        rig.app.handle().clone(),
        rig.app.state(),
        vec![id.to_owned()],
        "key".into(),
        key.into(),
        Some(true),
    ))
    .map(|_| ())
    .map_err(|e| e.message)
}

#[test]
fn locking_the_stick_while_a_key_edit_writes_waits_and_the_tag_agrees_with_the_library() {
    let rig = rig();
    let (audio, id) = track_on_stick(&rig);
    rig.unlock();

    let held = test_holds::arm(key_tag::LIBRARY_KEY_WRITTEN, &rig.state_dir());
    let editing = {
        let (handle, id) = (rig.app.handle().clone(), id.clone());
        std::thread::spawn(move || {
            run(details::set_track_field(
                handle.clone(),
                handle.state(),
                vec![id],
                "key".into(),
                "Fm".into(),
                Some(true),
            ))
            .map(|_| ())
            .map_err(|e| e.message)
        })
    };
    // The Library holds Fm; the file still Am. The DJ locks the Stick.
    held.reached();
    let locking = lock_now(&rig);
    held.release();

    assert_eq!(editing.join().unwrap(), Ok(()));
    if let Some(locking) = locking {
        locking.join().unwrap();
    }
    assert!(
        rig.protected(),
        "the lock took effect once the edit was done"
    );
    assert_eq!(library_key(&rig, &id), "Fm");
    assert_eq!(
        tag(&audio).as_deref(),
        Some("Fm"),
        "the file's tag agrees with the Library"
    );
}

#[test]
fn locking_the_stick_during_an_undo_whose_library_step_fails_still_puts_the_tag_back() {
    let rig = rig();
    let (audio, id) = track_on_stick(&rig);
    rig.unlock();
    set_key(&rig, &id, "Fm").unwrap();
    assert_eq!(tag(&audio).as_deref(), Some("Fm"));

    let held = test_holds::arm(key_tag::TAGS_STEPPED, &rig.state_dir());
    let undoing = {
        let handle = rig.app.handle().clone();
        std::thread::spawn(move || {
            run(commands::undo_edit(handle.clone(), handle.state()))
                .map(|_| ())
                .map_err(|e| e.message)
        })
    };
    // The Undo has put the file back to Am and not yet the Library. Another
    // program changes the track's key meanwhile, so the Library's step
    // fails and the files must be put back; and the DJ locks the Stick.
    held.reached();
    assert_eq!(tag(&audio).as_deref(), Some("Am"));
    {
        let mut other =
            rbl_db::write::Writer::open(rig.location.clone(), rig.dir.path().join("other-backups"))
                .unwrap();
        other.disable_automatic_backups();
        other
            .set_field(&id, rbl_db::write::TrackField::Key, "Gm")
            .unwrap();
    }
    let locking = lock_now(&rig);
    held.release();

    let undone = undoing.join().unwrap();
    if let Some(locking) = locking {
        locking.join().unwrap();
    }
    let error = undone.expect_err("the Undo is refused: the key was changed elsewhere");
    assert!(!error.contains("could not be changed back"), "{error}");
    assert!(rig.protected());
    assert_eq!(
        tag(&audio).as_deref(),
        Some("Fm"),
        "the file is as it was before the Undo"
    );
}

/// A confirmed token for exporting `playlist` to the rig's Stick: planned,
/// then approved, as the window's hold does.
fn export_token(rig: &Rig, playlist: &str) -> Option<String> {
    let request = serde_json::from_value(serde_json::json!({
        "destination": rig.stick.to_string_lossy(),
        "operation": { "kind": "exportPlaylist", "playlist": playlist },
    }))
    .unwrap();
    let plan = rbxport_lib::stick_write_plan::plan(&rig.state(), &request).unwrap();
    plan.plan
        .map(|id| rbxport_lib::stick_approval::approve(&rig.state_dir(), &id).unwrap())
}

fn stick_tag_names(stick: &Path) -> Vec<String> {
    let tags = wreckordbox_stick_outline::read(stick, |read| {
        read.my_tags(rbl_export::device_library::Format::OneLibrary)
    })
    .unwrap();
    tags.categories
        .iter()
        .flat_map(|c| c.tags.iter().map(|t| t.name.clone()))
        .collect()
}

/// wbx-45.4 review, C4: the export checks its confirmation against the
/// Library's generation and reads what it exports under one hold of the
/// edit gate, so a My Tag edit arriving in between waits, and the Stick
/// gets exactly the Library that was confirmed.
#[test]
fn a_my_tag_edit_arriving_during_an_export_plan_waits_and_is_not_exported() {
    let rig = rig();
    rig.unlock();
    let token = export_token(&rig, &fixture::playlist_id(0));
    assert!(token.is_some());

    let held = test_holds::arm(test_holds::EXPORT_CONFIRMED, &rig.state_dir());
    let exporting = {
        let (handle, path) = (
            rig.app.handle().clone(),
            rig.stick.to_string_lossy().into_owned(),
        );
        std::thread::spawn(move || {
            run(commands::export_playlist(
                handle.clone(),
                handle.state(),
                fixture::playlist_id(0),
                path,
                None,
                None,
                None,
                token,
            ))
            .map(|_| ())
            .map_err(|e| e.message)
        })
    };
    held.reached();
    // The DJ renames a My Tag now, from the window or an agent. When the
    // Library is free it is renamed at once, before the export reads it.
    let rename = |state: &AppState| {
        state
            .write(|w| w.rename_my_tag(fixture::MY_TAG_PEAK, "Peak Hour"))
            .map(|_| ())
    };
    let state = rig.state();
    let renaming = match state.core().edit_gate().try_lock() {
        Some(free) => {
            drop(free);
            rename(&state).unwrap();
            None
        }
        None => {
            let state = Arc::clone(&state);
            Some(std::thread::spawn(move || rename(&state).unwrap()))
        }
    };
    held.release();

    assert_eq!(exporting.join().unwrap(), Ok(()));
    if let Some(renaming) = renaming {
        renaming.join().unwrap();
    }
    let on_stick = stick_tag_names(&rig.stick);
    assert!(
        on_stick.contains(&"Peak".to_owned()),
        "the confirmed Library was exported: {on_stick:?}"
    );
    assert!(
        !on_stick.contains(&"Peak Hour".to_owned()),
        "the later edit was not: {on_stick:?}"
    );
}

/// wbx-45.14 round 1 (a): the Stick is replaced (taken out, another put in
/// at the same place) after the Library has the new key and before the tag
/// is written. The edit fails, says why, and the Library is put back, so
/// the file on the original Stick and the Library still agree.
#[test]
fn replacing_the_stick_while_a_key_edit_writes_fails_the_edit_and_both_stores_agree() {
    let rig = rig();
    let (audio, id) = track_on_stick(&rig);
    rig.unlock();

    let held = test_holds::arm(key_tag::LIBRARY_KEY_WRITTEN, &rig.state_dir());
    let editing = {
        let (handle, id) = (rig.app.handle().clone(), id.clone());
        std::thread::spawn(move || {
            run(details::set_track_field(
                handle.clone(),
                handle.state(),
                vec![id],
                "key".into(),
                "Fm".into(),
                Some(true),
            ))
            .map(|_| ())
            .map_err(|e| e.message)
        })
    };
    held.reached();
    let original = rig.stick.with_file_name("GIGSTICK-taken-out");
    std::fs::rename(&rig.stick, &original).unwrap();
    std::fs::create_dir_all(rig.stick.join("Contents")).unwrap();
    std::fs::write(rig.stick.join(STAND_IN_MARKER), b"").unwrap();
    held.release();

    let error = editing
        .join()
        .unwrap()
        .expect_err("the edit fails: its Stick was replaced");
    assert!(error.contains("GIGSTICK"), "names the Stick: {error}");
    assert_eq!(
        library_key(&rig, &id),
        "Am",
        "the Library's key was put back"
    );
    let moved = original.join(audio.strip_prefix(&rig.stick).unwrap());
    assert_eq!(
        tag(&moved).as_deref(),
        Some("Am"),
        "the file on the original Stick keeps its key"
    );
    assert_eq!(
        std::fs::read_dir(rig.stick.join("Contents"))
            .unwrap()
            .count(),
        0,
        "nothing was written to the Stick put in its place"
    );
}
