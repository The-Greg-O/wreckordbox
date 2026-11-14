//! Each Stick load's identity, the service's own (wbx-45.14, from the
//! wbx-72 review).
//!
//! A caller may name its load (`operation`, the window's report id), but
//! that name is only a label: the service gives every load a unique id of
//! its own (`load`), which every progress event and the answer carry. A
//! label is refused (`stick_load_duplicate`, before anything is read or
//! written) while another load of this service carries it, so two loads
//! running together can never share one, and a client that hears an event
//! with its label and a different `load` knows it is not its own load's
//! (an earlier load's late event).

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::protocol::ServiceError;

/// The labels of the loads running in one service, and the next id.
#[derive(Default)]
pub(crate) struct LoadIds {
    running: Mutex<BTreeSet<String>>,
    next: AtomicU64,
}

/// One load's identity, held until its answer is made: its label is free
/// again when this is dropped.
pub(crate) struct LoadId<'a> {
    pub(crate) id: String,
    label: Option<String>,
    ids: &'a LoadIds,
}

impl LoadIds {
    fn running(&self) -> std::sync::MutexGuard<'_, BTreeSet<String>> {
        self.running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A new load labelled `label` (none: unlabelled): its unique id, or
    /// refused when a load running now has that label.
    pub(crate) fn begin(&self, label: Option<&str>) -> Result<LoadId<'_>, ServiceError> {
        if let Some(label) = label {
            if !self.running().insert(label.to_owned()) {
                return Err(ServiceError::new(
                    super::codes::STICK_LOAD_DUPLICATE,
                    format!(
                        "another load running now is named \"{label}\"; a load's name must be its own. Nothing was written"
                    ),
                ));
            }
        }
        let n = self.next.fetch_add(1, Ordering::Relaxed);
        let started = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        Ok(LoadId {
            id: format!("load-{:x}-{started:x}-{n}", std::process::id()),
            label: label.map(str::to_owned),
            ids: self,
        })
    }
}

impl Drop for LoadId<'_> {
    fn drop(&mut self) {
        if let Some(label) = &self.label {
            self.ids.running().remove(label);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn every_load_has_an_id_of_its_own_and_a_running_label_is_not_shared() {
        let ids = LoadIds::default();
        let first = ids.begin(Some("op")).unwrap();
        let unlabelled = ids.begin(None).unwrap();
        assert_ne!(first.id, unlabelled.id);
        let again = ids
            .begin(Some("op"))
            .err()
            .expect("refused while the first runs");
        assert_eq!(again.code, super::super::codes::STICK_LOAD_DUPLICATE);
        let first_id = first.id.clone();
        drop(first);
        let later = ids.begin(Some("op")).unwrap();
        assert_ne!(later.id, first_id, "a label used again names another load");
    }
}
