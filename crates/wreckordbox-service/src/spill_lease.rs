//! An in-process claim, never a mutex held while doing filesystem or SQLite I/O.
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};
static BUSY: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
pub(super) struct Lease(PathBuf);
impl Lease {
    pub(super) fn take(path: &Path) -> Result<Self, String> {
        if super::Faults::from_env().fail_lease {
            return Err("Fixture bulk session lease failure".into());
        }
        if !BUSY
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(path.to_path_buf())
        {
            return Err(format!(
                "Spill {} is still being written or recovered; retry after it finishes",
                path.display()
            ));
        }
        Ok(Self(path.to_path_buf()))
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        BUSY.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.0);
    }
}
