//! Which Sticks are mounted, what kind each is, whether it is protected,
//! when it was last loaded and how much room it has.
//!
//! The mounted volumes are the ones the window offers (`rbl_devices::list`),
//! each read without recovering or writing anything (`rbl_devices::inspect`,
//! the path table in `stick-recovery.md`). A volume counts when Stick
//! Protection classifies it as a Stick. In test mode only the stand-ins the
//! test names (`RB_LITE_FAKE_VOLUMES`) are looked at, never the machine's
//! real mounts, and the files [`TEST_FREE_BYTES_FILE`] and
//! [`TEST_CLUSTER_BYTES_FILE`] in the state folder, read each time, replace
//! every Stick's free space and cluster size.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// In the state folder, in test mode only: every Stick's free space.
pub const TEST_FREE_BYTES_FILE: &str = "test-stick-free-bytes";
/// In the state folder, in test mode only: every Stick's cluster size.
pub const TEST_CLUSTER_BYTES_FILE: &str = "test-stick-cluster-bytes";

/// A mounted Stick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stick {
    /// Its root: what `sticks.dry_run` and `sticks.load` take.
    pub path: PathBuf,
    /// Its name, as the DJ types it to unlock a Gig stick.
    pub name: String,
    /// `spare` when it carries the Spare stick marker, else `gig`.
    pub kind: String,
    /// Stick Protection: nothing is written to it until the DJ unlocks it.
    pub protected: bool,
    /// An earlier write did not finish; nothing is loaded until it is
    /// settled in the window.
    pub unfinished_write: bool,
    /// When wreckordbox last loaded it, as the Stick's own record says;
    /// `null` when it never has (or another program wrote its library).
    pub last_loaded: Option<String>,
    /// Free space in bytes; `null` when it cannot be read.
    pub free_bytes: Option<u64>,
    /// Size in bytes; `null` when it cannot be read.
    pub total_bytes: Option<u64>,
    /// What its exported library holds now; `null` when it holds none.
    pub tracks: Option<usize>,
    pub playlists: Option<usize>,
}

/// What `sticks.list` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sticks {
    pub sticks: Vec<Stick>,
}

pub(crate) fn testing() -> bool {
    std::env::var_os(rbl_devices::TEST_MODE).is_some_and(|v| v == "1")
}

/// Every mounted Stick, in path order.
pub(crate) fn list(state_dir: &Path) -> Vec<Stick> {
    // In test mode, the stand-ins named or nothing: a test never reads a
    // real mount.
    if testing() && rbl_devices::fake_volumes().is_none() {
        return Vec::new();
    }
    let mut sticks: Vec<Stick> = rbl_devices::list()
        .into_iter()
        .filter_map(|device| {
            let wreckordbox_stick_protection::Volume::Stick { root } =
                wreckordbox_stick_protection::classify(&device.mount_point)
            else {
                return None;
            };
            // A folder inside a Stick is not a Stick of its own.
            if device.mount_point.canonicalize().ok()? != root {
                return None;
            }
            let mut stick = describe(state_dir, &root);
            if device.total_bytes > 0 && !testing() {
                stick.free_bytes = Some(device.free_bytes);
                stick.total_bytes = Some(device.total_bytes);
            }
            Some(stick)
        })
        .collect();
    sticks.sort_by(|a, b| a.path.cmp(&b.path));
    sticks.dedup_by(|a, b| a.path == b.path);
    sticks
}

/// What the Stick whose root is `root` says about itself, read without
/// writing or recovering anything.
pub(crate) fn describe(state_dir: &Path, root: &Path) -> Stick {
    let status = wreckordbox_stick_protection::status(state_dir, root);
    let export = rbl_devices::inspect(root);
    Stick {
        path: root.to_path_buf(),
        name: status.name,
        kind: if status.spare { "spare" } else { "gig" }.to_owned(),
        protected: status.protected,
        unfinished_write: rbl_export::recovery_pending(root),
        last_loaded: export
            .as_ref()
            .filter(|e| e.ours)
            .map(|e| e.written.clone())
            .filter(|w| !w.is_empty()),
        free_bytes: free_bytes(state_dir, root),
        total_bytes: None,
        tracks: export.as_ref().map(|e| e.tracks),
        playlists: export.as_ref().map(|e| e.playlists),
    }
}

/// A number a test left in the state folder, in test mode only.
fn test_number(state_dir: &Path, file: &str) -> Option<u64> {
    testing()
        .then(|| {
            std::fs::read_to_string(state_dir.join(file))
                .ok()?
                .trim()
                .parse()
                .ok()
        })
        .flatten()
}

/// The Stick's free space now.
pub(crate) fn free_bytes(state_dir: &Path, root: &Path) -> Option<u64> {
    test_number(state_dir, TEST_FREE_BYTES_FILE)
        .or_else(|| rbl_export::device_import::free_space(root).ok())
}

/// The size of the clusters the Stick's file system stores files in: a
/// file, a folder, takes whole ones.
pub(crate) fn cluster_bytes(state_dir: &Path, root: &Path) -> u64 {
    test_number(state_dir, TEST_CLUSTER_BYTES_FILE)
        .unwrap_or_else(|| rbl_export::plan::allocation_unit(root))
        .max(1)
}
