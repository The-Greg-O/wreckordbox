//! The Review view's commands (wbx-74).
//!
//! Every one asks the Library service this window hosts, with wbx-27's
//! commands exactly as they are; nothing here decides anything:
//!
//! - `review_waiting` reads `review.waiting` (no filter: the whole queue,
//!   its `token` and, since wbx-74, its `runs`) and adds the deck's row of
//!   each listed track, built from the window's own index, so the view can
//!   load a track without opening a list. A read: nothing written.
//! - `review_accept {rows}` is `review.accept` with the rows the DJ chose,
//!   each with the row token of the listing the view SHOWS (wbx-74 review:
//!   an unrelated arrival does not refuse it), and the DJ declared.
//!   Never re-listed or retried here: a refusal (`review_changed`,
//!   `review_not_waiting`, `library_protected`) or a part-way accept
//!   (`review_partial`, with the Facts applied and not applied) is an
//!   answer with `ok: false` for the view to show.
//! - `review_reject {facts}` and `review_reopen {facts | rejection}` are
//!   `review.reject` / `review.reopen` with the DJ declared; `review_rejected`
//!   reads `review.rejected`.
//! - [`watch`] passes the service's notices that can change the queue
//!   (`sidecar_changed`, `facts_reattached`, `library_changed`,
//!   `review_decided`) to the window as one [`REVIEW_CHANGED_EVENT`],
//!   coalesced: a burst while a Run works is told a few times, not once
//!   per Fact. The view re-reads on it.
//!
//! Only a window hosting no service is an error.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Runtime};
use wreckordbox_service::review::{commands, RowToken};
use wreckordbox_service::ServiceError;

use crate::commands::blocking;
use crate::dto::RowDto;
use crate::error::{AppError, AppResult, ErrorKind};
use crate::state::{rows_to_dto, AppState};

/// The window event telling the view the queue may have changed: `{}`.
pub const REVIEW_CHANGED_EVENT: &str = "review:changed";

/// How long the queue must be quiet before a change is told.
const QUIET: Duration = Duration::from_millis(150);
/// The longest a change waits while notices keep coming.
const AT_MOST: Duration = Duration::from_millis(1000);

/// The queue as the view shows it.
#[derive(Debug, Clone, Serialize)]
pub struct ReviewListing {
    /// The service's `review.waiting` answer, untouched.
    pub listing: Value,
    /// The deck's row of each listed track, once each.
    pub rows: Vec<RowDto>,
}

fn no_service() -> AppError {
    AppError::new(
        ErrorKind::NotFound,
        "The Library service is not running in this window, so the Facts waiting for you cannot be read.",
    )
}

fn ask(state: &AppState, command: &str, args: Value) -> AppResult<Result<Value, ServiceError>> {
    state
        .with_service(|service| service.call(command, args))
        .ok_or_else(no_service)
}

fn dj() -> Value {
    json!({ "kind": "dj" })
}

/// The Fact identifiers after `label: [` in a part-way accept's message.
fn ids_after(message: &str, label: &str) -> Option<Vec<i64>> {
    let start = message.find(label)? + label.len();
    let rest = message[start..].strip_prefix('[')?;
    let inside = &rest[..rest.find(']')?];
    if inside.trim().is_empty() {
        return Some(Vec::new());
    }
    inside.split(',').map(|n| n.trim().parse().ok()).collect()
}

/// A decision's answer: the service's result with `ok: true`, or its
/// refusal with `ok: false`. A part-way accept also names the Facts it
/// applied and those it did not (`applied`, `notApplied`).
fn decision(answer: Result<Value, ServiceError>) -> Value {
    match answer {
        Ok(mut result) => {
            if let Some(object) = result.as_object_mut() {
                object.insert("ok".to_owned(), Value::Bool(true));
            }
            result
        }
        Err(error) => {
            let mut said = json!({ "code": error.code, "message": error.message });
            if error.code == wreckordbox_service::review::codes::PARTIAL {
                // "...; applied: [1, 2], ...; not applied: [3], still waiting"
                let not = ids_after(&error.message, "not applied: ");
                let applied = error
                    .message
                    .find("; applied: ")
                    .and_then(|at| ids_after(&error.message[at..], "; applied: "));
                said["applied"] = json!(applied);
                said["notApplied"] = json!(not);
            }
            json!({ "ok": false, "error": said })
        }
    }
}

/// The whole queue, with each listed track's deck row.
pub fn waiting(state: &Arc<AppState>) -> AppResult<ReviewListing> {
    let listing = ask(state, commands::REVIEW_WAITING, json!({}))?.map_err(|error| {
        AppError::new(
            ErrorKind::Internal,
            format!(
                "The Facts waiting for you could not be read: {}",
                error.message
            ),
        )
        .with_detail(error.code.clone())
    })?;
    let library = state.library()?;
    let mut seen = std::collections::HashSet::new();
    let mut rows = Vec::new();
    for held in listing["waiting"].as_array().into_iter().flatten() {
        let Some(id) = held["track"].as_str() else {
            continue;
        };
        if seen.insert(id.to_owned()) {
            if let Some(row) = library.row_of(id) {
                rows.push(row);
            }
        }
    }
    Ok(ReviewListing {
        listing,
        rows: rows_to_dto(&library, &rows, 0),
    })
}

/// The DJ's acceptance of the shown `rows`, each with its row token.
pub fn accept(state: &Arc<AppState>, rows: &[RowToken]) -> AppResult<Value> {
    Ok(decision(ask(
        state,
        commands::REVIEW_ACCEPT,
        json!({ "rows": rows, "source": dj() }),
    )?))
}

/// The DJ's rejection of `facts`.
pub fn reject(state: &Arc<AppState>, facts: &[i64]) -> AppResult<Value> {
    Ok(decision(ask(
        state,
        commands::REVIEW_REJECT,
        json!({ "facts": facts, "source": dj() }),
    )?))
}

/// The suggestions standing rejected.
pub fn rejected(state: &Arc<AppState>) -> AppResult<Value> {
    ask(state, commands::REVIEW_REJECTED, json!({}))?.map_err(|error| {
        AppError::new(
            ErrorKind::Internal,
            format!(
                "The rejected suggestions could not be read: {}",
                error.message
            ),
        )
        .with_detail(error.code.clone())
    })
}

/// The DJ's reopening of the rejected `facts`, or of one rejection as a
/// whole.
pub fn reopen(state: &Arc<AppState>, facts: &[i64], rejection: Option<i64>) -> AppResult<Value> {
    let args = match rejection {
        Some(entry) => json!({ "rejection": entry, "source": dj() }),
        None => json!({ "facts": facts, "source": dj() }),
    };
    Ok(decision(ask(state, commands::REVIEW_REOPEN, args)?))
}

/// Whether a watcher for the hosted service is running.
static WATCHING: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

fn tells_the_queue(event: &str) -> bool {
    matches!(
        event,
        "sidecar_changed" | "facts_reattached" | "library_changed"
    ) || event == wreckordbox_service::review_runs::REVIEW_DECIDED
}

/// Starts passing the hosted service's queue notices to the window, unless
/// a watcher is already running. It ends with the service; the next read
/// starts another.
pub fn watch<R: Runtime>(app: &AppHandle<R>, state: &AppState) {
    let mut slot = WATCHING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if slot
        .as_ref()
        .is_some_and(|alive| alive.load(Ordering::Acquire))
    {
        return;
    }
    let Some(heard) = state.with_service(wreckordbox_service::Service::events) else {
        return;
    };
    let alive = Arc::new(AtomicBool::new(true));
    *slot = Some(Arc::clone(&alive));
    let app = app.clone();
    std::thread::spawn(move || {
        let mut pending: Option<(Instant, Instant)> = None; // (first, last)
        loop {
            match heard.recv_timeout(Duration::from_millis(50)) {
                Ok(event) => {
                    if tells_the_queue(&event.event) {
                        let now = Instant::now();
                        pending = Some(pending.map_or((now, now), |(first, _)| (first, now)));
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if let Some((first, last)) = pending {
                if last.elapsed() >= QUIET || first.elapsed() >= AT_MOST {
                    let _ = app.emit(REVIEW_CHANGED_EVENT, json!({}));
                    pending = None;
                }
            }
        }
        alive.store(false, Ordering::Release);
    });
}

/// The whole queue (`ReviewView`); starts the watcher.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a command is handed its state by value"
)]
pub async fn review_waiting(
    app: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> AppResult<ReviewListing> {
    let state = Arc::clone(&state);
    blocking("review_waiting", move || {
        watch(&app, &state);
        waiting(&state)
    })
    .await
}

/// Accepts the chosen Facts of the listing the view shows.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a command is handed its state by value"
)]
pub async fn review_accept(
    state: tauri::State<'_, Arc<AppState>>,
    rows: Vec<RowToken>,
) -> AppResult<Value> {
    let state = Arc::clone(&state);
    blocking("review_accept", move || accept(&state, &rows)).await
}

/// Rejects the chosen Facts.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a command is handed its state by value"
)]
pub async fn review_reject(
    state: tauri::State<'_, Arc<AppState>>,
    facts: Vec<i64>,
) -> AppResult<Value> {
    let state = Arc::clone(&state);
    blocking("review_reject", move || reject(&state, &facts)).await
}

/// The suggestions standing rejected.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a command is handed its state by value"
)]
pub async fn review_rejected(state: tauri::State<'_, Arc<AppState>>) -> AppResult<Value> {
    let state = Arc::clone(&state);
    blocking("review_rejected", move || rejected(&state)).await
}

/// Reopens rejected Facts, or one rejection as a whole.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a command is handed its state by value"
)]
pub async fn review_reopen(
    state: tauri::State<'_, Arc<AppState>>,
    facts: Vec<i64>,
    rejection: Option<i64>,
) -> AppResult<Value> {
    let state = Arc::clone(&state);
    blocking("review_reopen", move || reopen(&state, &facts, rejection)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_way_accept_names_what_it_applied_and_what_it_did_not() {
        let said = decision(Err(ServiceError::new(
            wreckordbox_service::review::codes::PARTIAL,
            "the accept stopped part-way (write refused); applied: [3, 5], recorded as the DJ's and undoable in one step; not applied: [7], still waiting",
        )));
        assert_eq!(said["ok"], false);
        assert_eq!(said["error"]["applied"], json!([3, 5]));
        assert_eq!(said["error"]["notApplied"], json!([7]));
    }

    #[test]
    fn an_unreadable_part_way_message_names_nothing_rather_than_guess() {
        let said = decision(Err(ServiceError::new(
            wreckordbox_service::review::codes::PARTIAL,
            "stopped",
        )));
        assert_eq!(said["error"]["applied"], Value::Null);
        assert_eq!(said["error"]["notApplied"], Value::Null);
    }
}
