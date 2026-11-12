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
    stems
        .with_current_sidecar(|open| {
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
        })
        .ok_or_else(|| ServiceError::new("sidecar_failed", "the Sidecar is busy; try again"))?
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
        match self {
            Self::Open(s) => s.facts(track, "stems"),
            Self::Snapshot(s) => s.facts(track, "stems"),
            Self::Absent => Ok(Vec::new()),
        }
    }
}
