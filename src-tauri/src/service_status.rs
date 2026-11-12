//! Read through the hosted service, off the UI thread. Unreadable Run data
//! is independent of whether the Library service can accept commands.
use crate::{state::AppState, AppError, AppResult};
use serde::Serialize;
use std::sync::Arc;
use wreckordbox_service::run_status::{ActiveRun, RUNS_ACTIVE};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceActivity {
    pub connected: bool,
    pub problem: Option<String>,
    pub runs: Vec<ActiveRun>,
    pub runs_unavailable: bool,
}

fn read(state: &AppState) -> ServiceActivity {
    if let Some(problem) = state.service_problem() {
        return ServiceActivity {
            connected: false,
            problem: Some(problem),
            runs: Vec::new(),
            runs_unavailable: false,
        };
    }
    match state.with_service(|service| service.call(RUNS_ACTIVE, serde_json::json!({}))) {
        None => ServiceActivity {
            connected: false,
            problem: None,
            runs: Vec::new(),
            runs_unavailable: false,
        },
        Some(result) => {
            let runs = result.ok().and_then(|value| {
                serde_json::from_value::<Option<Vec<ActiveRun>>>(value)
                    .ok()
                    .flatten()
            });
            ServiceActivity {
                connected: true,
                problem: None,
                runs_unavailable: runs.is_none(),
                runs: runs.unwrap_or_default(),
            }
        }
    }
}

#[tauri::command]
pub async fn service_activity(
    state: tauri::State<'_, Arc<AppState>>,
) -> AppResult<ServiceActivity> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || read(&state))
        .await
        .map_err(|error| AppError::internal(error.to_string()))
}
