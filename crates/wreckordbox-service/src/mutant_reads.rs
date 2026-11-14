//! Read the Mutant archive without creating, migrating or tidying a Sidecar.
use crate::{protocol::ServiceError, stems::Stems};
use wreckordbox_sidecar::mutants::{Mutant, MutantChange};
use wreckordbox_sidecar::{FactsReader, FactsSnapshot, Sidecar, SidecarError};

pub(super) enum Store<'a> {
    Open(&'a Sidecar),
    Snapshot(&'a FactsReader),
    Absent,
}

pub(super) fn inspect<T>(
    stems: &Stems,
    read: impl FnOnce(Store<'_>) -> Result<T, SidecarError>,
) -> Result<T, ServiceError> {
    stems.with_sidecar_wait(|open| {
        let snapshot;
        let store = if let Some(open) = open {
            Store::Open(open)
        } else {
            snapshot = FactsSnapshot::open(stems.sidecar_dir())
                .map_err(|e| super::mutants::sidecar_error(&e))?;
            match &snapshot {
                FactsSnapshot::Absent => Store::Absent,
                FactsSnapshot::Open(reader) => Store::Snapshot(reader),
                FactsSnapshot::Unknown(why) => {
                    return Err(ServiceError::new("sidecar_failed", why))
                }
            }
        };
        read(store).map_err(|e| super::mutants::sidecar_error(&e))
    })?
}

impl Store<'_> {
    pub(super) fn mutant(&self, id: i64) -> Result<Option<Mutant>, SidecarError> {
        match self {
            Self::Open(s) => s.mutant(id),
            Self::Snapshot(s) => s.mutants().mutant(id),
            Self::Absent => Ok(None),
        }
    }
    pub(super) fn mutant_of_entry(&self, entry: &str) -> Result<Option<Mutant>, SidecarError> {
        match self {
            Self::Open(s) => s.mutant_of_entry(entry),
            Self::Snapshot(s) => s.mutants().mutant_of_entry(entry),
            Self::Absent => Ok(None),
        }
    }
    pub(super) fn mutants(&self, playlist: Option<&str>) -> Result<Vec<Mutant>, SidecarError> {
        match self {
            Self::Open(s) => s.mutants(playlist),
            Self::Snapshot(s) => s.mutants().mutants(playlist),
            Self::Absent => Ok(Vec::new()),
        }
    }
    pub(super) fn mutants_of_track(&self, track: &str) -> Result<Vec<Mutant>, SidecarError> {
        match self {
            Self::Open(s) => s.mutants_of_track(track),
            Self::Snapshot(s) => s.mutants().mutants_of_track(track),
            Self::Absent => Ok(Vec::new()),
        }
    }
    pub(super) fn mutant_changes(&self, id: i64) -> Result<Vec<MutantChange>, SidecarError> {
        match self {
            Self::Open(s) => s.mutant_changes(id),
            Self::Snapshot(s) => s.mutants().mutant_changes(id),
            Self::Absent => Ok(Vec::new()),
        }
    }
    pub(super) fn stem_facts(
        &self,
        track: &str,
    ) -> Result<Vec<wreckordbox_sidecar::Fact>, SidecarError> {
        self.facts(track, "stems")
    }
    pub(super) fn facts(
        &self,
        track: &str,
        kind: &str,
    ) -> Result<Vec<wreckordbox_sidecar::Fact>, SidecarError> {
        match self {
            Self::Open(s) => s.facts(track, kind),
            Self::Snapshot(s) => s.facts(track, kind),
            Self::Absent => Ok(Vec::new()),
        }
    }
}

/// Bounded wait without poisoning unrelated future requests.
pub(crate) fn wait_for_slot<T>(
    slot: &std::sync::Mutex<T>,
    timeout: std::time::Duration,
) -> Result<std::sync::MutexGuard<'_, T>, ServiceError> {
    let until = std::time::Instant::now() + timeout;
    loop {
        match slot.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(std::sync::TryLockError::Poisoned(error)) => return Ok(error.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => {}
        }
        if std::time::Instant::now() >= until {
            return Err(ServiceError::new(
                "sidecar_timeout",
                format!(
                    "timed out after {} ms waiting for the Sidecar slot; retry the command",
                    timeout.as_millis()
                ),
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn occupied_slot_times_out_clearly() {
        let slot = std::sync::Mutex::new(());
        let _held = slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let result = super::wait_for_slot(&slot, std::time::Duration::from_millis(20));
        assert!(
            matches!(result, Err(error) if error.code == "sidecar_timeout" && error.message.contains("20 ms"))
        );
    }
}
