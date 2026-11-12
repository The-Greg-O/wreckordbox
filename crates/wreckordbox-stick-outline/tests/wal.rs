//! A Stick whose `exportLibrary.db` rekordbox left in WAL mode: every read
//! sees its committed rows and creates, changes or deletes nothing on the
//! Stick, the database's companion files included.
#![allow(clippy::pedantic, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use rbl_export::device_library::Format;
use wreckordbox_stick_outline::testing::{assert_unchanged, snapshot, stick, wal_checkpointed, wal_pending, without_lock, WAL_RENAMED};
use wreckordbox_stick_outline::{library, outline, read_outline, StickRead};

/// Each fixture, with and without our lock file.
fn sticks() -> Vec<(String, tempfile::TempDir)> {
    let mut out = Vec::new();
    for (name, wal) in [("pending -wal", wal_pending as fn(&Path)), ("checkpointed", wal_checkpointed as fn(&Path))] {
        for lock in [true, false] {
            let dest = tempfile::tempdir().unwrap();
            stick(dest.path());
            wal(dest.path());
            if !lock {
                without_lock(dest.path());
            }
            out.push((format!("{name}, lock {lock}"), dest));
        }
    }
    out
}

fn assert_renamed(names: Vec<String>, what: &str) {
    assert!(names.iter().any(|n| n == WAL_RENAMED), "{what}: committed WAL rows are read: {names:?}");
}

#[test]
fn reading_a_wal_mode_library_creates_or_changes_no_file_on_the_stick() {
    for (case, dest) in sticks() {
        let root = dest.path();
        let before = snapshot(root);

        let one = library(root, Format::OneLibrary).unwrap();
        assert_unchanged(root, &before, &format!("{case}: library()"));
        assert_renamed(one.nodes.iter().map(|n| n.name.clone()).collect(), &case);

        let found = outline(root);
        assert_unchanged(root, &before, &format!("{case}: outline()"));
        assert_renamed(found.libraries[1].nodes.iter().map(|n| n.name.clone()).collect(), &case);
        read_outline(root).unwrap();
        assert_unchanged(root, &before, &format!("{case}: read_outline()"));

        let read = StickRead::begin(root).unwrap();
        for format in [Format::DeviceLibrary, Format::OneLibrary] {
            read.library(format).unwrap();
            assert_unchanged(root, &before, &format!("{case}: {format:?}"));
        }
        read.finish().unwrap();
        assert_unchanged(root, &before, &format!("{case}: finish()"));
    }
}
