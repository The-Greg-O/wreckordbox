//! Stick Protection hardening (wbx-45.14): a Stick locked or replaced in
//! the middle of a write that began while it was unlocked.
//!
//! Every race here is controlled by the shell's test holds
//! (`rbxport_lib::test_holds`): the writer is held at a named moment, the
//! test does what the race needs (locks the Stick, edits the Library), then
//! lets it go. No sleeps. Temporary folders stand in for Sticks.

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
