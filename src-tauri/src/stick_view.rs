//! The Stick view's reads and its load (wbx-71), through the Library
//! service this window hosts (wbx-70: `sticks.list`, `sticks.dry_run`,
//! `sticks.load`, see `stick_load` in `wreckordbox-service`).
//!
//! - Listing and the dry run only read: the service's reads of a Stick never
//!   recover an unfinished write or write anything (`rbl_devices::inspect`,
//!   the non-recovering planner). A dry run answers the service's plan id.
//! - A load is a Stick write like the window's others: Stick Protection is
//!   checked under the Stick's lock (`stick_guard`), and the DJ's hold is
//!   required: the window's token for exactly this sync of these playlists
//!   (`stick_write_plan` -> hold -> `stick_write_approve`, as every Stick
//!   write in the window stages it) is used up before the service is asked.
//!   The service then checks protection again under its own hold of the
//!   Stick's lock, and its plan id binds the load to the dry run the DJ was
//!   shown (anything changed since is refused, `stick_load_changed`).
//!
//! The window's lock and the token are let go before the service runs the
//! load, because the service takes the same lock itself.

use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};
use wreckordbox_service::stick_load::commands;
use wreckordbox_service::ServiceError;

use crate::commands::blocking;
use crate::error::{AppError, AppResult, ErrorKind};
use crate::state::AppState;

/// Runs one Stick command on the service this window hosts, through a
/// handle taken under the hosting lock and called with it let go, so a
/// load never keeps the window from stopping the service (which asks the
/// load to stop where it safely can and waits for its report).
fn ask(state: &AppState, command: &str, args: Value) -> AppResult<Value> {
    let caller = state.service_caller().ok_or_else(|| {
        AppError::new(
            ErrorKind::NotFound,
            "The Library service is not running in this window, so the Stick cannot be read or loaded. Nothing was written.",
        )
    })?;
    caller.call(command, args).map_err(|error| failed(&error))
}

/// The service's refusal in the window's words, its code kept as the detail.
fn failed(error: &ServiceError) -> AppError {
    let kind = match error.code.as_str() {
        "stick_protected" => ErrorKind::StickProtected,
        "stick_unfinished_write" => ErrorKind::UnfinishedWrite,
        "stick_not_found" | "playlist_not_found" | "not_a_stick" => ErrorKind::NotFound,
        "write_refused" => ErrorKind::ReadOnly,
        "stick_load_unconfirmed"
        | "stick_load_changed"
        | "stick_too_small"
        | "stick_load_incomplete" => ErrorKind::Conflict,
        "bad_request" => ErrorKind::Malformed,
        _ => ErrorKind::Internal,
    };
    AppError::new(kind, error.message.clone()).with_detail(error.code.clone())
}

/// The mounted Sticks (`sticks.list`).
pub fn sticks(state: &AppState) -> AppResult<Value> {
    ask(state, commands::STICKS_LIST, json!({}))
}

/// What loading `playlists` onto `stick` would do now (`sticks.dry_run`).
/// Writes nothing.
pub fn dry_run(state: &AppState, stick: &str, playlists: &[String]) -> AppResult<Value> {
    ask(
        state,
        commands::STICKS_DRY_RUN,
        json!({ "stick": stick, "playlists": playlists }),
    )
}

/// The mounted Sticks, for the Stick view.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a command is handed its state by value"
)]
pub async fn stick_view_sticks(state: tauri::State<'_, Arc<AppState>>) -> AppResult<Value> {
    let state = Arc::clone(&state);
    blocking("stick_view_sticks", move || sticks(&state)).await
}

/// What a load would do (the Stick view's summary and full list).
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a command is handed its state by value"
)]
pub async fn stick_view_dry_run(
    state: tauri::State<'_, Arc<AppState>>,
    stick: String,
    playlists: Vec<String>,
) -> AppResult<Value> {
    let state = Arc::clone(&state);
    blocking("stick_view_dry_run", move || {
        dry_run(&state, &stick, &playlists)
    })
    .await
}

/// The load the DJ held for: `plan` is the dry run's plan id the view
/// showed, `approval` the token the window's hold exchanged that plan for
/// (`stick_write_approve(plan)`), `operation` the window's id for this load
/// (one is made when absent). Stick Protection is checked here first, under
/// the Stick's lock; the token is used up by the service under its own hold
/// of that lock, and only for exactly `plan` and the result it was approved
/// for. The window's hosting lock is not held while the load runs.
///
/// Whatever the service answers (its report, or why it refused or failed)
/// is kept in the app's state folder ([`crate::stick_load_reports`]) under
/// `operation` before this lets the window's shutdown complete, so a load
/// cut short by closing the window is reported at the next start.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a command is handed its state by value"
)]
pub async fn stick_view_load(
    state: tauri::State<'_, Arc<AppState>>,
    stick: String,
    playlists: Vec<String>,
    plan: String,
    approval: Option<String>,
    operation: Option<String>,
) -> AppResult<Value> {
    let state = Arc::clone(&state);
    blocking("stick_view_load", move || {
        let root = Path::new(&stick);
        // Refused here, before anything else, on a protected Stick.
        drop(crate::stick_guard::enter_stick(state.backup_dir(), root)?);
        let Some(approval) = approval.filter(|token| !token.is_empty()) else {
            return Err(AppError::new(
                ErrorKind::ReadOnly,
                "This write to the Stick was not confirmed. Nothing was written: hold to confirm the load shown.",
            ));
        };
        let operation = operation
            .filter(|id| crate::stick_load_reports::valid_id(id))
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        // Counted until the report is kept: closing the window waits for it.
        let _running = state.shutdown.load_running();
        let volume = crate::stick_approval::volume_identity(root).ok();
        let report = ask(
            &state,
            commands::STICKS_LOAD,
            json!({ "stick": stick, "playlists": playlists, "plan": plan, "approval": approval, "operation": operation }),
        );
        let name = report
            .as_ref()
            .ok()
            .and_then(|load| load["stick"]["name"].as_str().map(str::to_owned))
            .or_else(|| root.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| stick.clone());
        let kept = json!({
            "id": operation,
            "stick": { "path": stick, "name": name, "volume": volume },
            "finished": rbl_core::time::now(),
            "report": report.as_ref().ok(),
            "error": report.as_ref().err().map(|e| json!({ "code": e.detail, "message": e.message })),
        });
        if let Err(error) = crate::stick_load_reports::keep_report(state.backup_dir(), &kept) {
            tracing::error!(stick = %stick, %error, report = %kept, "a Stick load report could not be kept");
        }
        report.map(|mut load| {
            load["report_id"] = json!(operation);
            load
        })
    })
    .await
}

/// The Stick load reports kept and not yet dismissed (the Stick view shows
/// them, also for a Stick no longer mounted). Reads the app's state folder.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a command is handed its state by value"
)]
pub async fn stick_load_reports(state: tauri::State<'_, Arc<AppState>>) -> AppResult<Vec<Value>> {
    let state = Arc::clone(&state);
    blocking("stick_load_reports", move || {
        Ok(crate::stick_load_reports::kept_reports(state.backup_dir()))
    })
    .await
}

/// The DJ dismissed a kept report: removed from the app's state folder.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a command is handed its state by value"
)]
pub async fn stick_load_report_dismiss(
    state: tauri::State<'_, Arc<AppState>>,
    id: String,
) -> AppResult<()> {
    let state = Arc::clone(&state);
    blocking("stick_load_report_dismiss", move || {
        crate::stick_load_reports::dismiss_report(state.backup_dir(), &id).map_err(|e| {
            AppError::new(
                ErrorKind::Internal,
                "The load report could not be dismissed.",
            )
            .with_detail(e.to_string())
        })
    })
    .await
}
