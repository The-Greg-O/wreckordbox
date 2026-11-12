//! Read-only current work. Unlike a paginated Ledger, this includes every
//! unfinished Run regardless of how much work has finished since it began.
pub use wreckordbox_sidecar::ActiveRun;

use crate::{stems::Stems, ServiceError};

pub const RUNS_ACTIVE: &str = "runs.active";

pub(crate) fn active(stems: &Stems) -> Result<serde_json::Value, ServiceError> {
    let runs = stems.sidecar_if_made()?.map_or_else(
        || Ok(Vec::new()),
        |sidecar| {
            sidecar.active_runs().map_err(|error| {
                ServiceError::new(crate::stems::codes::SIDECAR_FAILED, error.to_string())
            })
        },
    )?;
    serde_json::to_value(runs)
        .map_err(|error| ServiceError::new(crate::codes::SERVICE_FAILED, error.to_string()))
}
