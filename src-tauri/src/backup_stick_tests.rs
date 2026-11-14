//! A backup to a Stick that is locked or replaced while it copies
//! (wbx-45.14). The backup's own progress report is the hold: the test acts
//! from inside it, at a known file, on the backup's thread. Temporary
//! folders stand in for Sticks.

use std::fs;
use std::path::{Path, PathBuf};

use super::create_with_progress;
use crate::state::AppState;

fn fixture() -> (tempfile::TempDir, AppState, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let location = rbl_db::fixture::build(dir.path(), rbl_db::fixture::Shape::default()).unwrap();
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (library, _) = rbl_index::load(&db).unwrap();
    let state = AppState::unprotected(dir.path().join("backups"));
    state.set_library(library, false, db.schema().db_version, 0, location);
    let stick = dir.path().join("sticks/GIG");
    fs::create_dir_all(&stick).unwrap();
    crate::stick_guard::testing::unlock(&state, &stick);
    state
        .set_backup_destination(&crate::stick_guard::testing::ordinary(), &stick)
        .unwrap();
    (dir, state, stick)
}

fn walk(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                out.push((path.clone(), Vec::new()));
                todo.push(path);
            } else {
                out.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// Runs a backup and, at the first file it reports copying after the
/// database (an analysis file, mid-copy), calls `then`; answers the
/// backup's result and what the Stick held right after `then`.
fn backup_interrupted(
    state: &AppState,
    stick: &Path,
    then: impl Fn(),
) -> (crate::AppResult<String>, Vec<(PathBuf, Vec<u8>)>) {
    let mut at = None;
    let result = create_with_progress(state, &mut |phase, done, _, item| {
        if at.is_none() && phase == "copying" && done > 0 {
            then();
            at = Some((item.to_owned(), walk(stick)));
        }
        Ok(())
    });
    let (item, held) = at.expect("the backup reached copying");
    assert!(!item.is_empty());
    (result, held)
}

#[test]
fn a_backup_to_a_stick_locked_mid_copy_stops_at_once_and_says_what_it_wrote() {
    let (_dir, state, stick) = fixture();
    let state_dir = state.backup_dir().to_path_buf();
    let (result, held) = backup_interrupted(&state, &stick, || {
        // A lock that does not wait for the backup's execution lock (the
        // backup's own thread, as an eject is): only the record changes.
        wreckordbox_stick_protection::lock(&state_dir, &stick).unwrap();
    });

    let error = result.expect_err("the backup stops");
    assert_eq!(
        error.kind,
        crate::error::ErrorKind::StickProtected,
        "{error:?}"
    );
    assert_eq!(
        walk(&stick),
        held,
        "nothing more was written to the Stick, nothing removed"
    );
    assert!(
        error.message.contains("stopped after copying"),
        "{}",
        error.message
    );
    assert!(
        error.message.contains(".partial-"),
        "names the unfinished copy left on it: {}",
        error.message
    );
    assert!(
        !fs::read_dir(&stick).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("rbexport-")),
        "no backup was named"
    );
}

#[test]
fn a_backup_to_a_stick_replaced_mid_copy_writes_nothing_to_the_new_one() {
    let (dir, state, stick) = fixture();
    let old = dir.path().join("sticks/GIG-taken-out");
    let (result, held) = backup_interrupted(&state, &stick, || {
        // Taken out, and another Stick put in at the same place.
        fs::rename(&stick, &old).unwrap();
        fs::create_dir_all(&stick).unwrap();
        fs::write(
            stick.join(wreckordbox_stick_protection::STAND_IN_MARKER),
            b"",
        )
        .unwrap();
    });

    let error = result.expect_err("the backup stops");
    assert_eq!(error.kind, crate::error::ErrorKind::NotFound, "{error:?}");
    assert_eq!(
        walk(&stick),
        held,
        "the Stick put in its place is left as it was"
    );
    assert!(
        error.message.contains("stopped after copying"),
        "{}",
        error.message
    );
}
