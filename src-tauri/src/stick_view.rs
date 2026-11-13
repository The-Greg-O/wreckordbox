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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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

/// Stick loads the window has running, so closing it waits for them.
static LOADS_RUNNING: AtomicUsize = AtomicUsize::new(0);
/// Set once the window began closing behind a running load.
static CLOSING: AtomicBool = AtomicBool::new(false);

/// Counts one running load until dropped.
struct Running;

impl Running {
    fn start() -> Self {
        LOADS_RUNNING.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        LOADS_RUNNING.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The app is asked to exit (the window closed). With a Stick load running,
/// the exit waits: on a thread of its own, never the interface's, the
/// service is stopped, which asks the load to stop where it safely can and
/// waits for it to end and send its report (logged by the load command);
/// then the app exits. Nothing abandons the load part-way.
pub fn exit_requested<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    api: &tauri::ExitRequestApi,
) {
    if LOADS_RUNNING.load(Ordering::Acquire) == 0 || CLOSING.swap(true, Ordering::AcqRel) {
        return;
    }
    api.prevent_exit();
    let handle = handle.clone();
    std::thread::spawn(move || {
        tauri::Manager::state::<Arc<AppState>>(&handle).stop_service();
        handle.exit(0);
    });
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
/// (`stick_write_approve(plan)`). Stick Protection is checked here first,
/// under the Stick's lock; the token is used up by the service under its
/// own hold of that lock, and only for exactly `plan` and the result it
/// was approved for (a fresh dry run's plan with an earlier hold, or a plan
/// whose Native stems or audio changed since, is refused). The window's
/// hosting lock is not held while the load runs.
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
        let _running = Running::start();
        let report = ask(
            &state,
            commands::STICKS_LOAD,
            json!({ "stick": stick, "playlists": playlists, "plan": plan, "approval": approval }),
        );
        // Said in the log too, so a report is kept when nothing is left to show it.
        match &report {
            Ok(load) => tracing::info!(stick = %stick, outcome = %load["outcome"], "Stick load finished"),
            Err(error) => tracing::warn!(stick = %stick, error = %error.message, "Stick load refused or failed"),
        }
        report
    })
    .await
}
