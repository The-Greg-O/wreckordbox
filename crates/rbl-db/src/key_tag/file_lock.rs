//! Serialize validation/replacement of one canonical path, never unrelated audio.
//! The registry lock is held only for lookup, with no filesystem I/O under it.
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, Weak},
};

static FILES: LazyLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn for_path(path: &Path) -> Arc<Mutex<()>> {
    let mut files = FILES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    files.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = files.get(path).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    files.insert(path.to_path_buf(), Arc::downgrade(&lock));
    lock
}
