//! Fingerprints for Stick previews only (wbx-cne).
//!
//! A Spare-stick dry run needs the fingerprint of each selected track that
//! has Stem Facts, to tell current Stems from stale; reading the audio in
//! full on every preview is the cost this saves. A fingerprint read here is
//! kept, in this process's memory and nowhere else, with the file's identity
//! when it was read: device, inode, size, modification time and change time
//! (to the nanosecond). It is used again only while all five are the same.
//! The change time cannot be set back by anyone rewriting the file, so audio
//! changed behind a restored size and modification time is read again.
//!
//! Kept in memory for the service's life, not in the Sidecar's cache table
//! or the app's state: a dry run writes nothing at all, and filling a stored
//! cache would be a write. The cost is one read of each track with Stems
//! per service start.
//!
//! Never for a load. A load plans again under the Stick's lock with a fresh
//! fingerprint (`stems::fingerprint`); a wrong entry here can at worst make
//! a preview differ from the load, which the load's bound check refuses.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use wreckordbox_sidecar::Fingerprint;

/// What must be unchanged for a kept fingerprint to be used again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

/// The file's identity now; `None` when it is not a regular file, or where
/// the platform gives no inode and change time (nothing is kept there).
fn identity(path: &Path) -> Option<Identity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(path)
            .ok()
            .filter(std::fs::Metadata::is_file)?;
        Some(Identity {
            device: meta.dev(),
            inode: meta.ino(),
            size: meta.len(),
            modified: (meta.mtime(), meta.mtime_nsec()),
            changed: (meta.ctime(), meta.ctime_nsec()),
        })
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// More than any one selection needs; past it everything is forgotten.
const LIMIT: usize = 65_536;

type Kept = HashMap<PathBuf, (Identity, Fingerprint)>;

fn kept() -> MutexGuard<'static, Kept> {
    static KEPT: OnceLock<Mutex<Kept>> = OnceLock::new();
    KEPT.get_or_init(Default::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// The fingerprint of the audio at `path`, for a preview: the kept one while
/// the file's identity is unchanged, otherwise read now (and kept, if the
/// file did not change while it was read). `None` when it cannot be read.
pub(crate) fn for_preview(path: &Path) -> Option<Fingerprint> {
    with_reader(path, |path| {
        wreckordbox_sidecar::fingerprint_file(path).ok()
    })
}

fn with_reader(
    path: &Path,
    read: impl FnOnce(&Path) -> Option<Fingerprint>,
) -> Option<Fingerprint> {
    let Some(before) = identity(path) else {
        kept().remove(path);
        return read(path);
    };
    if let Some((then, fingerprint)) = kept().get(path) {
        if *then == before {
            return Some(fingerprint.clone());
        }
    }
    let fingerprint = read(path);
    let mut kept = kept();
    match (&fingerprint, identity(path)) {
        (Some(fingerprint), Some(after)) if after == before => {
            if kept.len() >= LIMIT {
                kept.clear();
            }
            kept.insert(path.to_path_buf(), (after, fingerprint.clone()));
        }
        _ => {
            kept.remove(path);
        }
    }
    fingerprint
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use std::cell::Cell;

    fn counted<'a>(
        reads: &'a Cell<u32>,
        answer: &'a str,
    ) -> impl FnOnce(&Path) -> Option<Fingerprint> + 'a {
        move |_| {
            reads.set(reads.get() + 1);
            Some(Fingerprint::from_stored(answer.to_owned()))
        }
    }

    #[test]
    fn an_unchanged_file_is_read_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.wav");
        std::fs::write(&path, b"audio").unwrap();
        let reads = Cell::new(0);
        let first = with_reader(&path, counted(&reads, "a1:one"));
        let second = with_reader(&path, counted(&reads, "a1:two"));
        assert_eq!(reads.get(), 1);
        assert_eq!(first, second);
    }

    #[test]
    fn a_rewrite_keeping_size_and_mtime_is_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.wav");
        std::fs::write(&path, b"audio").unwrap();
        let reads = Cell::new(0);
        with_reader(&path, counted(&reads, "a1:one"));
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        // The change time moves on even when it is the same instant's tick.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, b"other").unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
        let again = with_reader(&path, counted(&reads, "a1:two"));
        assert_eq!(reads.get(), 2);
        assert_eq!(again.unwrap().as_str(), "a1:two");
    }

    #[test]
    fn another_file_moved_into_the_path_is_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.wav");
        std::fs::write(&path, b"audio").unwrap();
        let reads = Cell::new(0);
        with_reader(&path, counted(&reads, "a1:one"));
        let other = dir.path().join("b.wav");
        std::fs::write(&other, b"audio").unwrap();
        std::fs::rename(&other, &path).unwrap();
        with_reader(&path, counted(&reads, "a1:two"));
        assert_eq!(reads.get(), 2);
    }

    #[test]
    fn a_failed_read_is_not_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.wav");
        std::fs::write(&path, b"audio").unwrap();
        let reads = Cell::new(0);
        assert_eq!(with_reader(&path, |_| None), None);
        with_reader(&path, counted(&reads, "a1:one"));
        assert_eq!(reads.get(), 1);
    }
}
