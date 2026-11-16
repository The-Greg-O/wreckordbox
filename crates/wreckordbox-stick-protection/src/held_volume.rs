//! One path classified again and again by a holder that keeps a file open
//! there (wbx-8yh: the Sidecar's write admission, asked at every commit).
//!
//! An open file stays on the filesystem it was opened on, so the volume a
//! held path is on is read from the mount table once, at the first ask,
//! not at every commit. Unlike [`classify`](crate::classify), a mount table
//! that cannot be read fails **closed**: the path is taken to be on a
//! Stick (its own folder standing for the Stick's root, which is then never
//! unlocked), so nothing is written on a guess. The test-mode stand-in
//! marker is still looked for at every ask.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::{facts_in, is_stick, resolve, test_mode, Volume, VolumeFacts, STAND_IN_MARKER};

type ReadTable = Box<dyn Fn() -> std::io::Result<String> + Send + Sync>;

/// What the mount table said of the path, once read.
enum Read {
    /// The table could not be read.
    Unreadable,
    /// The volume holding the path, if a mount was found.
    Found(Option<VolumeFacts>),
}

/// A path whose volume is decided once.
pub struct HeldVolume {
    resolved: Option<PathBuf>,
    facts: OnceLock<Read>,
    read_table: ReadTable,
}

impl std::fmt::Debug for HeldVolume {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeldVolume")
            .field("resolved", &self.resolved)
            .finish_non_exhaustive()
    }
}

impl HeldVolume {
    /// For `path`, read against this machine's mount table.
    #[must_use]
    pub fn new(path: &Path) -> Self {
        let at = resolve(path);
        Self::reading(
            path,
            Box::new(move || match &at {
                Some(at) => crate::mount_table(at),
                None => Err(std::io::Error::other("the path cannot be resolved")),
            }),
        )
    }

    fn reading(path: &Path, read_table: ReadTable) -> Self {
        Self {
            resolved: resolve(path),
            facts: OnceLock::new(),
            read_table,
        }
    }

    /// Whether the path is on a Stick, and which.
    #[must_use]
    pub fn classify(&self) -> Volume {
        let Some(resolved) = &self.resolved else {
            return Volume::Ordinary;
        };
        if test_mode() {
            if let Some(root) = resolved
                .ancestors()
                .find(|dir| dir.join(STAND_IN_MARKER).is_file())
            {
                return Volume::Stick {
                    root: root.to_path_buf(),
                };
            }
        }
        let facts = self.facts.get_or_init(|| {
            (self.read_table)().map_or(Read::Unreadable, |table| {
                Read::Found(facts_in(&table, resolved))
            })
        });
        match facts {
            // Fail closed: nothing tells this is not a Stick.
            Read::Unreadable => Volume::Stick {
                root: resolved.clone(),
            },
            Read::Found(Some(facts)) if is_stick(facts) => Volume::Stick {
                root: facts.root.clone(),
            },
            Read::Found(_) => Volume::Ordinary,
        }
    }

    /// The Stick's root, if the path is on one.
    #[must_use]
    pub fn stick_root(&self) -> Option<PathBuf> {
        match self.classify() {
            Volume::Stick { root } => Some(root),
            Volume::Ordinary => None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn a_mount_table_that_cannot_be_read_counts_as_a_stick() {
        let dir = tempfile::tempdir().unwrap();
        let held = HeldVolume::reading(
            dir.path(),
            Box::new(|| Err(std::io::Error::other("no /proc here"))),
        );
        assert_eq!(
            held.stick_root(),
            Some(dir.path().canonicalize().unwrap()),
            "fails closed"
        );
    }

    /// What was decided while the volume could not be looked at holds for
    /// the holder's life: protected, never "ordinary".
    #[test]
    fn a_volume_that_could_not_be_looked_at_stays_a_stick_for_this_holder() {
        let held = HeldVolume::reading(
            Path::new("/Volumes/wbx-no-such-volume/Sidecar/facts.db"),
            Box::new(|| Ok("0 0 unknown / /Volumes/wbx-no-such-volume - - - -\n".to_owned())),
        );
        for _ in 0..2 {
            assert_eq!(
                held.stick_root(),
                Some(PathBuf::from("/Volumes/wbx-no-such-volume")),
                "its root cannot be read, and nothing says it is fixed"
            );
        }
    }

    #[test]
    fn the_mount_table_is_read_once_however_often_it_is_asked() {
        let dir = tempfile::tempdir().unwrap();
        let reads = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&reads);
        let held = HeldVolume::reading(
            dir.path(),
            Box::new(move || {
                counted.fetch_add(1, Ordering::Relaxed);
                Ok("22 1 0:20 / / rw - tmpfs tmpfs rw\n".to_owned())
            }),
        );
        for _ in 0..5 {
            assert_eq!(held.stick_root(), None, "an ordinary tmpfs");
        }
        assert_eq!(reads.load(Ordering::Relaxed), 1);
    }
}
