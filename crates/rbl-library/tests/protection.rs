//! Library Protection at the Library core: every write the core makes, a
//! window's own edits and its Undo and Redo included, is refused while it
//! is on, checked under the gate that serialises edits. The switch is kept
//! in a file in the core's recovery folder, which is its only source of
//! truth. Throwaway fixture Library, no window.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rbl_library::history::{LibraryEdit, StepError, TRACK_EDIT};
use rbl_library::protection::{self, Recorded};
use rbl_library::{History, LibraryCore, NoHost, Touched};

/// A core over a fresh fixture Library in `dir`, with nothing recorded
/// about Library Protection.
fn core_in(dir: &std::path::Path) -> LibraryCore {
    let location = rbl_db::fixture::build(dir, rbl_db::fixture::Shape::default()).unwrap();
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (library, _) = rbl_index::load(&db).unwrap();
    let core = LibraryCore::new(dir.join("recovery"));
    core.set_library(&NoHost, library, false, db.schema().db_version, 0, location);
    core
}

fn rating_of(core: &LibraryCore, track: &str) -> u8 {
    let library = core.library().unwrap();
    library.rating[library.row_of(track).unwrap() as usize]
}

fn rate(core: &LibraryCore, track: &str, stars: u8) -> Result<(), rbl_db::DbError> {
    core.recorded_edit(
        &NoHost,
        Touched::Metadata(vec![track.to_owned()]),
        TRACK_EDIT,
        |w| {
            let (_, recorded) = w.set_rating_with_undo(track, stars)?;
            Ok(((), LibraryEdit::Track(vec![recorded])))
        },
    )
    .map(|_| ())
}

fn refused(result: &Result<impl std::fmt::Debug, rbl_db::DbError>) -> bool {
    matches!(result, Err(error) if protection::refused(error))
}

#[test]
fn every_write_undo_and_redo_included_is_refused_while_protection_is_on() {
    let dir = tempfile::tempdir().unwrap();
    let core = core_in(dir.path());
    let track = rbl_db::fixture::track_id(1);
    core.set_protection(false).unwrap();
    rate(&core, &track, 4).unwrap();
    core.step_history(&NoHost, true).unwrap();
    assert_eq!(rating_of(&core, &track), 0);

    core.set_protection(true).unwrap();
    let generation = core.generation();

    assert!(refused(&rate(&core, &track, 2)));
    assert!(refused(&core.edit(
        &NoHost,
        History::Outside,
        Touched::Playlists,
        |_| Ok(())
    )));
    let redo = core.step_history(&NoHost, false);
    assert!(
        matches!(&redo, Err(StepError::Write(error)) if protection::refused(error)),
        "{redo:?}"
    );
    assert_eq!(rating_of(&core, &track), 0);
    assert_eq!(core.generation(), generation);
    // The refused Redo is still there to run once protection is off.
    assert_eq!(core.history().lock().redo_label(), Some(TRACK_EDIT));

    core.set_protection(false).unwrap();
    core.step_history(&NoHost, false).unwrap();
    assert_eq!(rating_of(&core, &track), 4);
}

#[test]
fn with_nothing_recorded_protection_is_on_and_the_windows_preference_is_taken_up_once() {
    let dir = tempfile::tempdir().unwrap();
    let core = core_in(dir.path());
    let track = rbl_db::fixture::track_id(1);

    let first = core.protection();
    assert!(first.protect_library);
    assert_eq!(first.recorded, Recorded::Never);
    assert!(refused(&rate(&core, &track, 4)));

    // The window's stored preference, on its first start since the file.
    let adopted = core.adopt_protection(false).unwrap();
    assert!(!adopted.protect_library);
    assert_eq!(adopted.recorded, Recorded::Set);
    rate(&core, &track, 4).unwrap();

    // Never again: a stale preference does not overwrite the file.
    let again = core.adopt_protection(true).unwrap();
    assert!(!again.protect_library);
    assert_eq!(again.revision, adopted.revision);
}

#[test]
fn an_unreadable_file_is_on_and_not_replaced_by_the_windows_preference() {
    let dir = tempfile::tempdir().unwrap();
    let core = core_in(dir.path());
    let track = rbl_db::fixture::track_id(1);
    std::fs::create_dir_all(dir.path().join("recovery")).unwrap();
    std::fs::write(
        protection::file(&dir.path().join("recovery")),
        b"{\"protect_lib",
    )
    .unwrap();

    assert!(core.protection().protect_library);
    assert_eq!(core.protection().recorded, Recorded::Unreadable);
    let adopted = core.adopt_protection(false).unwrap();
    assert!(adopted.protect_library);
    assert!(refused(&rate(&core, &track, 4)));

    // Only an explicit change turns it off.
    let off = core.set_protection(false).unwrap();
    assert!(!off.now.protect_library);
    assert!(off.previous);
    rate(&core, &track, 4).unwrap();
}

#[test]
fn every_transition_has_a_higher_revision_and_is_heard_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let core = core_in(dir.path());
    let heard = Arc::new(Mutex::new(Vec::new()));
    let told = Arc::clone(&heard);
    core.watch_protection(move |state| {
        told.lock()
            .unwrap()
            .push((state.protect_library, state.revision));
        true
    });

    let a = core.set_protection(true).unwrap().now;
    let b = core.set_protection(false).unwrap().now;
    let c = core.set_protection(true).unwrap().now;

    assert!(a.revision < b.revision && b.revision < c.revision);
    assert_eq!(
        *heard.lock().unwrap(),
        [(true, a.revision), (false, b.revision), (true, c.revision)]
    );
    assert_eq!(core.protection().revision, c.revision);
}

/// Writers in one process and another core on the same folder, at once:
/// each with its own temporary file, one after another, so the file always
/// says what the last of them was told it says.
#[test]
fn writers_at_once_never_share_a_temporary_file_or_lose_the_last_word() {
    let dir = tempfile::tempdir().unwrap();
    let core = Arc::new(core_in(dir.path()));
    let other = Arc::new(LibraryCore::new(dir.path().join("recovery")));
    let answers = Arc::new(Mutex::new(Vec::new()));
    let writers: Vec<_> = (0..8)
        .map(|n| {
            let core = if n % 2 == 0 {
                Arc::clone(&core)
            } else {
                Arc::clone(&other)
            };
            let answers = Arc::clone(&answers);
            std::thread::spawn(move || {
                for i in 0..25 {
                    let on = (n + i) % 2 == 0;
                    let state = core.set_protection(on).unwrap().now;
                    answers
                        .lock()
                        .unwrap()
                        .push((state.revision, state.protect_library));
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }

    let answers = answers.lock().unwrap();
    let mut revisions: Vec<u64> = answers.iter().map(|(revision, _)| *revision).collect();
    revisions.sort_unstable();
    revisions.dedup();
    assert_eq!(
        revisions.len(),
        answers.len(),
        "two writers were given one revision"
    );
    let last = answers.iter().max().unwrap();
    let recorded = protection::read(&dir.path().join("recovery"));
    assert_eq!((recorded.revision, recorded.protect_library), *last);
    let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("recovery"))
        .unwrap()
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == "tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn a_change_that_cannot_be_recorded_is_an_error_and_never_leaves_edits_open() {
    let dir = tempfile::tempdir().unwrap();
    let core = core_in(dir.path());
    let track = rbl_db::fixture::track_id(1);
    let recovery = dir.path().join("recovery");
    // Something in the way of the file: it can be neither read nor replaced.
    std::fs::create_dir_all(protection::file(&recovery)).unwrap();

    assert!(core.set_protection(false).is_err());
    assert!(core.protection().protect_library);
    assert!(refused(&rate(&core, &track, 4)));

    // Recorded off, then turned on where it cannot be recorded: on all the
    // same, in this process, until a change can be recorded.
    std::fs::remove_dir(protection::file(&recovery)).unwrap();
    core.set_protection(false).unwrap();
    let before = core.protection().revision;
    // The folder can be read (the file still says off) but not written.
    let writable = std::fs::metadata(&recovery).unwrap().permissions();
    let mut read_only = writable.clone();
    read_only.set_readonly(true);
    std::fs::set_permissions(&recovery, read_only).unwrap();

    let turned = core.set_protection(true);
    let now = core.protection();
    let rated = rate(&core, &track, 4);
    std::fs::set_permissions(&recovery, writable).unwrap();

    assert!(turned.is_err());
    assert!(now.protect_library);
    assert_eq!(now.recorded, Recorded::Unsaved);
    assert!(now.revision > before);
    assert!(refused(&rated));
    assert_eq!(rating_of(&core, &track), 0);
    // Recorded once it can be: the file says on, and so does the core.
    let on = core.set_protection(true).unwrap();
    assert!(on.now.protect_library);
    assert_eq!(protection::read(&recovery).recorded, Recorded::Set);
}

/// An edit already waiting for the gate when protection is turned on is
/// refused: the check is made under the gate, not before it.
#[test]
fn an_edit_queued_behind_the_gate_is_refused_when_protection_was_turned_on_first() {
    let dir = tempfile::tempdir().unwrap();
    let core = Arc::new(core_in(dir.path()));
    let track = rbl_db::fixture::track_id(1);
    core.set_protection(false).unwrap();

    let gate = core.edit_gate().lock();
    let queued = {
        let core = Arc::clone(&core);
        let track = track.clone();
        std::thread::spawn(move || rate(&core, &track, 4))
    };
    std::thread::sleep(Duration::from_millis(200));
    core.set_protection(true).unwrap();
    drop(gate);

    assert!(refused(&queued.join().unwrap()));
    assert_eq!(rating_of(&core, &track), 0);
}
