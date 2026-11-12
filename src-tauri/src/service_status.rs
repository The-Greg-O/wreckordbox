//! The status line reads through the hosted service, never opening the
//! Library or Sidecar itself. Work on a blocking thread, off the UI thread.
use std::sync::Arc;

use serde::Serialize;
use wreckordbox_service::run_status::{ActiveRun, RUNS_ACTIVE};

use crate::{state::AppState, AppError, AppResult};

#[derive(Debug, Serialize)]
pub struct ServiceActivity {
    pub connected: bool,
    pub problem: Option<String>,
    pub runs: Vec<ActiveRun>,
}

fn read(state: &AppState) -> AppResult<ServiceActivity> {
    if let Some(problem) = state.service_problem() {
        return Ok(ServiceActivity {
            connected: false,
            problem: Some(problem),
            runs: Vec::new(),
        });
    }
    match state.with_service(|service| service.call(RUNS_ACTIVE, serde_json::json!({}))) {
        None => Ok(ServiceActivity {
            connected: false,
            problem: None,
            runs: Vec::new(),
        }),
        Some(Ok(runs)) => Ok(ServiceActivity {
            connected: true,
            problem: None,
            runs: serde_json::from_value(runs)
                .map_err(|error| AppError::internal(error.to_string()))?,
        }),
        Some(Err(error)) => Err(AppError::internal(error.to_string())),
    }
}

#[tauri::command]
pub async fn service_activity(
    state: tauri::State<'_, Arc<AppState>>,
) -> AppResult<ServiceActivity> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || read(&state))
        .await
        .map_err(|error| AppError::internal(error.to_string()))?
}
