//! Current work without opening a Sidecar writer or scanning finished history.
use crate::{stems::Stems, ServiceError};
use std::path::Path;
use std::time::{Duration, Instant};
pub use wreckordbox_sidecar::ActiveRun;
use wreckordbox_sidecar::{ActiveRunsReader, Sidecar};

pub const RUNS_ACTIVE: &str = "runs.active";

pub(crate) struct RunStatus {
    reader: ActiveRunsReader,
    retry_after: Option<Instant>,
}
impl RunStatus {
    pub(crate) fn new(dir: &Path) -> Self {
        Self {
            reader: ActiveRunsReader::new(dir),
            retry_after: None,
        }
    }
    fn read(&mut self, open: Option<&Sidecar>) -> Option<Vec<ActiveRun>> {
        if self.retry_after.is_some_and(|at| Instant::now() < at) {
            return None;
        }
        let result = open.map_or_else(
            || self.reader.read(),
            |sidecar| sidecar.active_runs().map(Some),
        );
        let runs = result.unwrap_or(None);
        self.retry_after = runs
            .is_none()
            .then(|| Instant::now() + Duration::from_secs(10));
        runs
    }
}

pub(crate) fn active(stems: &Stems) -> Result<serde_json::Value, ServiceError> {
    // Keep the writer closed throughout an immutable read; never wait behind
    // its opening/migration or another status request. Null means unknown.
    let runs = stems
        .with_current_sidecar(|open| {
            stems
                .run_status
                .try_lock()
                .ok()
                .and_then(|mut status| status.read(open))
        })
        .flatten();
    serde_json::to_value(runs)
        .map_err(|error| ServiceError::new(crate::codes::SERVICE_FAILED, error.to_string()))
}
