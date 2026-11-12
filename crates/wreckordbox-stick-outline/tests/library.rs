//! A Stick library read in full for showing it: never a recovery, never a
//! write, whatever a writer does around the read.
//!
//! Every Stick here is written by the export pipeline into a temporary
//! directory; nothing touches a mounted Stick.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::path::Path;

use rbl_export::device_library::{self, Format};
use wreckordbox_stick_outline::testing::{
    interrupt, publish_completely, snapshot, stick, without_lock, JOURNAL, LOCK, NEXT_PLAYLISTS,
    ONE, PDB,
};
use wreckordbox_stick_outline::{library, settle, ReadError, StickRead};

const BOTH: [Format; 2] = [Format::DeviceLibrary, Format::OneLibrary];

fn names(library: &device_library::Library) -> Vec<&str> {
    library.nodes.iter().map(|n| n.name.as_str()).collect()
}

/// Each library read in turn, the Stick's bytes compared after every read.
fn assert_each_read_refused_and_nothing_changed(root: &Path) {
    let before = snapshot(root);
    for format in BOTH {
        let read = library(root, format);
        assert!(
            matches!(read, Err(ReadError::UnfinishedWrite)),
            "{format:?}: {read:?}"
        );
        assert_eq!(
            snapshot(root),
            before,
            "{format:?}: not a byte on the Stick changed"
        );
    }
}

#[test]
fn an_unfinished_write_under_our_lock_is_reported_and_left_alone() {
    let dest = tempfile::tempdir().unwrap();
    stick(dest.path());
    interrupt(dest.path());
    assert!(dest.path().join(LOCK).is_file(), "the export left its lock");
    assert_each_read_refused_and_nothing_changed(dest.path());
}

#[test]
fn an_unfinished_write_with_no_lock_file_is_reported_and_no_lock_is_made() {
    let dest = tempfile::tempdir().unwrap();
    stick(dest.path());
    interrupt(dest.path());
    without_lock(dest.path());
    assert_each_read_refused_and_nothing_changed(dest.path());
    assert!(!dest.path().join(LOCK).exists());
}

#[test]
fn a_settled_stick_reads_exactly_as_the_device_library_reads_it_and_nothing_is_written() {
    for lock in [true, false] {
        let dest = tempfile::tempdir().unwrap();
        stick(dest.path());
        if !lock {
            without_lock(dest.path());
        }
        let before = snapshot(dest.path());
        for format in BOTH {
            let ours = library(dest.path(), format).unwrap();
            assert_eq!(snapshot(dest.path()), before, "{format:?}: nothing written");
            assert_eq!(
                ours,
                device_library::read(dest.path(), format).unwrap(),
                "{format:?}"
            );
        }
    }
}

/// A writer of ours takes the lock and leaves a journal after the read has
/// checked for one, then stops: the read must still answer "unfinished",
/// and must not touch what the writer left.
#[test]
fn a_journal_and_lock_that_appear_after_the_check_are_reported_not_recovered() {
    for lock_before in [true, false] {
        let dest = tempfile::tempdir().unwrap();
        stick(dest.path());
        if !lock_before {
            without_lock(dest.path());
        }
        let read = StickRead::begin(dest.path()).unwrap();
        // The writer, between the check and the read.
        std::fs::write(dest.path().join(LOCK), b"").unwrap();
        interrupt(dest.path());
        let after_writer = snapshot(dest.path());

        for format in BOTH {
            // Whatever is parsed, nothing may be written or settled.
            let _ = read.library(format);
            assert_eq!(snapshot(dest.path()), after_writer, "{format:?}");
        }
        assert!(matches!(read.finish(), Err(ReadError::UnfinishedWrite)));
        assert_eq!(
            snapshot(dest.path()),
            after_writer,
            "the journal is still there, untouched"
        );
    }
}

#[test]
fn a_journal_that_appears_during_a_whole_read_makes_that_read_unfinished() {
    let dest = tempfile::tempdir().unwrap();
    stick(dest.path());
    let read = StickRead::begin(dest.path()).unwrap();
    let parsed = read.library(Format::DeviceLibrary).unwrap();
    assert_eq!(names(&parsed), ["Sets", "Friday", "Warm Up"]);
    interrupt(dest.path());
    let after_writer = snapshot(dest.path());
    assert!(matches!(read.finish(), Err(ReadError::UnfinishedWrite)));
    assert_eq!(snapshot(dest.path()), after_writer);
}

#[cfg(unix)]
#[test]
fn a_stick_path_retargeted_during_a_read_is_refused_and_neither_stick_changes() {
    let (settled, pending, links) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    stick(settled.path());
    stick(pending.path());
    interrupt(pending.path());
    let link = links.path().join("STICK");
    std::os::unix::fs::symlink(settled.path(), &link).unwrap();
    let (a, b) = (snapshot(settled.path()), snapshot(pending.path()));

    let read = StickRead::begin(&link).unwrap();
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(pending.path(), &link).unwrap();
    // The read stays on the Stick it began on.
    assert_eq!(
        names(&read.library(Format::DeviceLibrary).unwrap()),
        ["Sets", "Friday", "Warm Up"]
    );
    assert!(matches!(read.finish(), Err(ReadError::Moved)));

    assert_eq!(snapshot(settled.path()), a);
    assert_eq!(snapshot(pending.path()), b);
    // Through the retargeted path, the pending Stick is reported.
    assert!(matches!(
        library(&link, Format::DeviceLibrary),
        Err(ReadError::UnfinishedWrite)
    ));
    assert_eq!(snapshot(pending.path()), b);
}

#[cfg(unix)]
#[test]
fn a_lease_that_cannot_be_taken_is_an_error_and_nothing_is_read_or_written() {
    use std::os::unix::fs::PermissionsExt;
    let dest = tempfile::tempdir().unwrap();
    stick(dest.path());
    let lock = dest.path().join(LOCK);
    std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::File::open(&lock).is_ok() {
        // Running as root: permissions do not stop the open.
        return;
    }
    for format in BOTH {
        assert!(
            matches!(library(dest.path(), format), Err(ReadError::Io(_))),
            "{format:?}"
        );
    }
    std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o644)).unwrap();
}

#[test]
fn settling_publishes_exactly_the_interrupted_export_and_leaves_it_readable() {
    for lock in [true, false] {
        let dest = tempfile::tempdir().unwrap();
        stick(dest.path());
        let next = interrupt(dest.path());
        if !lock {
            without_lock(dest.path());
        }
        let before = snapshot(dest.path());

        settle(dest.path()).unwrap();

        assert_eq!(std::fs::read(dest.path().join(PDB)).unwrap(), next.pdb);
        assert_eq!(std::fs::read(dest.path().join(ONE)).unwrap(), next.one);
        assert!(!dest.path().join(JOURNAL).exists());
        // Nothing else the Stick held was touched (the lock aside).
        let after = snapshot(dest.path());
        let unchanged =
            |files: &std::collections::BTreeMap<std::path::PathBuf, Option<Vec<u8>>>| {
                files
                    .iter()
                    .filter(|(p, _)| {
                        !p.starts_with(JOURNAL)
                            && p.as_path() != Path::new(PDB)
                            && p.as_path() != Path::new(ONE)
                            && p.as_path() != Path::new(LOCK)
                    })
                    .map(|(p, b)| (p.clone(), b.clone()))
                    .collect::<Vec<_>>()
            };
        assert_eq!(unchanged(&after), unchanged(&before));
        for format in BOTH {
            assert_eq!(
                names(&library(dest.path(), format).unwrap()),
                NEXT_PLAYLISTS,
                "{format:?}"
            );
        }
    }
}

/// The first writer of ours on a Stick with no lock file: it creates the
/// lock, publishes both databases and removes its journal, all between the
/// two libraries' reads. The read mixes two publications and is refused.
#[test]
fn a_publication_completed_during_an_unleased_read_is_refused() {
    let dest = tempfile::tempdir().unwrap();
    stick(dest.path());
    without_lock(dest.path());
    let read = StickRead::begin(dest.path()).unwrap();
    let old = read.library(Format::DeviceLibrary).unwrap();
    assert_eq!(names(&old), ["Sets", "Friday", "Warm Up"]);
    publish_completely(dest.path());
    let new = read.library(Format::OneLibrary).unwrap();
    assert_eq!(names(&new), NEXT_PLAYLISTS);
    assert!(!dest.path().join(JOURNAL).exists());
    assert!(matches!(read.finish(), Err(ReadError::Changed)));
}

/// Another program (rekordbox takes no lock of ours) rewrites a database
/// during a read that holds our lease: refused too.
#[test]
fn a_database_rewritten_during_a_leased_read_is_refused() {
    let dest = tempfile::tempdir().unwrap();
    stick(dest.path());
    assert!(dest.path().join(LOCK).is_file());
    let read = StickRead::begin(dest.path()).unwrap();
    read.library(Format::DeviceLibrary).unwrap();
    let other = tempfile::tempdir().unwrap();
    stick(other.path());
    std::fs::write(
        dest.path().join(ONE),
        std::fs::read(other.path().join(ONE)).unwrap(),
    )
    .unwrap();
    std::fs::write(dest.path().join(ONE), b"rewritten by another program").unwrap();
    assert!(matches!(read.finish(), Err(ReadError::Changed)));
}
