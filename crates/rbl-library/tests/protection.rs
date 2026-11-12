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

/// Review B: first-use adoption decides "nothing was ever recorded" under
/// the file lock, immediately before it writes. Another process that holds
/// the lock and records "on" while the adoption waits for it keeps the
/// last word.
#[test]
fn first_use_adoption_never_overwrites_a_turn_recorded_while_it_waited_for_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let core = Arc::new(core_in(dir.path()));
    let state_dir = dir.path().join("recovery");
    std::fs::create_dir_all(&state_dir).unwrap();
    assert_eq!(core.protection().recorded, Recorded::Never);
    // Another process is part-way through a turn: it holds the lock.
    let other = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(state_dir.join(protection::LOCK_FILE))
        .unwrap();
    other.lock().unwrap();

    let adopting = {
        let core = Arc::clone(&core);
        std::thread::spawn(move || core.adopt_protection(false))
    };
    // The adoption has found nothing recorded and waits for the lock.
    std::thread::sleep(Duration::from_millis(300));
    // The other process records "on" and lets go.
    std::fs::write(protection::file(&state_dir), br#"{"protect_library":true,"revision":7}"#).unwrap();
    other.unlock().unwrap();

    let adopted = adopting.join().unwrap().unwrap();
    assert!(adopted.protect_library);
    let recorded = protection::read(&state_dir);
    assert!(recorded.protect_library);
    assert_eq!(recorded.revision, 7);
    assert!(core.protection().protect_library);
}

/// Review D: a core keeps the highest revision it has ever seen, read or
/// written, and a turn after the record was damaged goes beyond it, so a
/// window that already shows that revision does not drop the turn as stale.
#[test]
fn a_restarted_core_turns_beyond_the_revision_it_read_after_the_record_is_damaged() {
    let dir = tempfile::tempdir().unwrap();
    let state_dir = dir.path().join("recovery");
    for _ in 0..20 {
        protection::store(&state_dir, true).unwrap();
    }
    // The app again: it has only read the record.
    let core = core_in(dir.path());
    assert_eq!(core.protection().revision, 20);
    std::fs::write(protection::file(&state_dir), b"{damaged").unwrap();
    let damaged = core.protection();
    assert!(damaged.protect_library);
    assert_eq!(damaged.recorded, Recorded::Unreadable);
    assert!(damaged.revision >= 20, "{damaged:?}");

    let turned = core.set_protection(false).unwrap();

    assert!(turned.now.revision > 20, "{turned:?}");
    assert_eq!(protection::read(&state_dir).revision, turned.now.revision);
    assert!(!core.protection().protect_library);
}

/// Review D, across processes: a core that never read the record before it
/// was damaged still turns beyond every revision any writer gave out.
#[test]
fn a_core_that_never_read_the_record_still_turns_beyond_every_revision_given_out() {
    let dir = tempfile::tempdir().unwrap();
    let state_dir = dir.path().join("recovery");
    for _ in 0..20 {
        protection::store(&state_dir, true).unwrap();
    }
    std::fs::write(protection::file(&state_dir), b"{damaged").unwrap();
    let core = core_in(dir.path());

    let turned = core.set_protection(false).unwrap();

    assert!(turned.now.revision > 20, "{turned:?}");
}

/// Review A: the one guarded start of an edit that writes the Library's
/// files. It is refused before anything is written while protection is
/// on, and it holds the edit gate while it runs, so protection cannot be
/// turned on half-way through.
#[test]
fn a_file_edit_is_refused_while_protection_is_on_and_holds_the_gate_while_it_runs() {
    let dir = tempfile::tempdir().unwrap();
    let core = Arc::new(core_in(dir.path()));
    core.set_protection(true).unwrap();

    let refusal = core.file_edit().map(|_| ()).unwrap_err();
    assert!(protection::refused(&refusal), "{refusal:?}");

    core.set_protection(false).unwrap();
    let edit = core.file_edit().unwrap();
    let turning = {
        let core = Arc::clone(&core);
        std::thread::spawn(move || core.set_protection(true))
    };
    std::thread::sleep(Duration::from_millis(200));
    // The turn waits for the edit, and the edit still holds the files' lock.
    assert!(!core.protection().protect_library);
    assert!(core.analysis_write().try_lock().is_none());
    drop(edit);
    turning.join().unwrap().unwrap();
    assert!(core.protection().protect_library);
    assert!(core.analysis_write().try_lock().is_some());
}

/// A file edit is refused, as every write is, by a holder that is not the
/// Library's writer.
#[test]
fn a_file_edit_is_refused_by_a_core_that_is_not_the_writer() {
    let dir = tempfile::tempdir().unwrap();
    let core = core_in(dir.path());
    core.set_protection(false).unwrap();
    core.refuse_writes(Some("not the writer".to_owned()));

    let refusal = core.file_edit().map(|_| ()).unwrap_err();

    assert!(matches!(refusal, rbl_db::DbError::WriteRefused(ref why) if why == "not the writer"), "{refusal:?}");
}
