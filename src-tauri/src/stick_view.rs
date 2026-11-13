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
use crate::stick_approval::{bound, StickOperation};

/// Runs one Stick command on the service this window hosts.
fn ask(state: &AppState, command: &str, args: Value) -> AppResult<Value> {
    state
        .with_service(|service| service.call(command, args))
        .ok_or_else(|| {
            AppError::new(
                ErrorKind::NotFound,
                "The Library service is not running in this window, so the Stick cannot be read or loaded.",
            )
        })?
        .map_err(|error| failed(&error))
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

/// What the hold confirmed for a load of `playlists`: the Sync the window
/// plans for them, with nothing else (no defaults, no compatibility format,
/// nothing deleted), as the service loads them.
fn held_operation(playlists: &[String]) -> StickOperation {
    StickOperation::Sync {
        playlists: playlists.to_vec(),
        delete_unlisted_music: false,
        automatic: false,
        defaults: bound::<()>(None),
        compatibility: bound::<()>(None),
    }
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

/// The load the DJ held for: `approval` is the window's token for a sync of
/// exactly `playlists` onto `stick` (the hold), `plan` the dry run's plan id.
/// Stick Protection and the token are checked, under the Stick's lock, before
/// the service is asked; the service checks protection again under its own.
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
        let operation = held_operation(&playlists);
        {
            // The window's lock order: the Library edit gate, then the Stick.
            // Both are let go before the service runs the load: it takes the
            // Stick's lock itself.
            let gate = state.edit_gate().lock();
            let (_, generation) = state.library_at_generation()?;
            let access = crate::stick_guard::enter_stick(state.backup_dir(), root)?;
            crate::stick_approval::consume(approval.as_deref(), &operation, root, generation)?;
            drop(access);
            drop(gate);
        }
        ask(
            &state,
            commands::STICKS_LOAD,
            json!({ "stick": stick, "playlists": playlists, "plan": plan }),
        )
    })
    .await
}
