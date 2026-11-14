//! Splitting a track's Stems from the Intake view's large player (wbx-67).
//!
//! Both commands go to the Library service this window hosts. Neither is a
//! Library edit: a separation writes the Sidecar, through the service.
//!
//! - `intake_stems_status` asks `stems.status`, which only reads: whether
//!   the track has current, stale or no Stems, and which track any client
//!   is separating now.
//! - `intake_stems_separate` asks `stems.separate` and is answered when the
//!   separation is done (minutes for a real track). Meanwhile the
//!   service's `stems_progress` events for that track are passed to the
//!   window as [`STEMS_PROGRESS_EVENT`], coalesced to a few a second. A
//!   refusal or failure (another track being separated, the audio file
//!   missing, the engine failing, a Sidecar on a protected Stick) is an
//!   answer with `ok: false` and the service's code and reason, so the
//!   window can show why; only a window hosting no service is an error.
//!
//! The service itself never separates a track twice at once: a second
//! request for the same track joins the first, and a request for another
//! track is answered `separation_busy`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Runtime};
use wreckordbox_service::stems::{commands, events, StemsProgress};
use wreckordbox_service::stems_status::STEMS_STATUS;

use crate::commands::blocking;
use crate::error::{AppError, AppResult, ErrorKind};
use crate::state::AppState;

/// The window event carrying a separation's progress:
/// `{ trackId, stage, fraction }`.
pub const STEMS_PROGRESS_EVENT: &str = "stems:progress";

/// The least time between two progress events for one separation; a new
/// stage is always told at once.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

fn no_service() -> AppError {
    AppError::new(
        ErrorKind::NotFound,
        "The Library service is not running in this window, so Stems cannot be read or split.",
    )
}

/// The track's Stems now, read without writing anything.
pub fn status(state: &AppState, id: &str) -> AppResult<Value> {
    state
        .with_service(|service| service.call(STEMS_STATUS, json!({ "id": id })))
        .ok_or_else(no_service)?
        .map_err(|error| {
            AppError::new(
                ErrorKind::Internal,
                format!("{}: {}", error.code, error.message),
            )
        })
}

/// Separates the track's Stems through the hosted service and waits for
/// the answer, telling the window the separation's progress on the way.
/// Current Stems are answered as they are unless `again` (the DJ
/// confirmed splitting them again).
pub fn separate<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    id: &str,
    again: bool,
) -> AppResult<Value> {
    // Listen before asking, so no early progress is missed.
    let (caller, heard) = state
        .with_service(|service| (service.caller(), service.events()))
        .ok_or_else(no_service)?;
    let done = Arc::new(AtomicBool::new(false));
    let forwarder = {
        let (app, done, track) = (app.clone(), Arc::clone(&done), id.to_owned());
        std::thread::spawn(move || {
            let mut last: Option<(Instant, String)> = None;
            let mut held: Option<StemsProgress> = None;
            let tell = |progress: &StemsProgress| {
                let _ = app.emit(
                    STEMS_PROGRESS_EVENT,
                    json!({ "trackId": progress.track_id, "stage": progress.stage, "fraction": progress.fraction }),
                );
            };
            while !done.load(Ordering::Acquire) {
                let Ok(event) = heard.recv_timeout(Duration::from_millis(50)) else {
                    if let (Some(progress), Some((at, _))) = (&held, &last) {
                        if at.elapsed() >= PROGRESS_EVERY {
                            tell(progress);
                            last = Some((Instant::now(), progress.stage.clone()));
                            held = None;
                        }
                    }
                    continue;
                };
                if event.event != events::STEMS_PROGRESS {
                    continue;
                }
                let Ok(progress) = serde_json::from_value::<StemsProgress>(event.data) else {
                    continue;
                };
                if progress.track_id != track {
                    continue;
                }
                let due = last.as_ref().is_none_or(|(at, stage)| {
                    *stage != progress.stage || at.elapsed() >= PROGRESS_EVERY
                });
                if due {
                    tell(&progress);
                    last = Some((Instant::now(), progress.stage.clone()));
                    held = None;
                } else {
                    held = Some(progress);
                }
            }
        })
    };
    let answer = caller.call(
        commands::STEMS_SEPARATE,
        json!({ "id": id, "again": again }),
    );
    done.store(true, Ordering::Release);
    let _ = forwarder.join();
    Ok(match answer {
        Ok(separated) => {
            let status = status(state, id).unwrap_or(Value::Null);
            json!({ "ok": true, "reused": separated["reused"], "status": status })
        }
        Err(error) => json!({
            "ok": false,
            "error": { "code": error.code, "message": error.message },
        }),
    })
}

/// Whether the track has Stems now (the large player's Stems control).
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a command is handed its state by value"
)]
pub async fn intake_stems_status(
    state: tauri::State<'_, Arc<AppState>>,
    id: String,
) -> AppResult<Value> {
    let state = Arc::clone(&state);
    blocking("intake_stems_status", move || status(&state, &id)).await
}

/// Splits the track's Stems; answered when the separation is done.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a command is handed its state by value"
)]
pub async fn intake_stems_separate<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, Arc<AppState>>,
    id: String,
    again: Option<bool>,
) -> AppResult<Value> {
    let state = Arc::clone(&state);
    blocking("intake_stems_separate", move || {
        separate(&app, &state, &id, again.unwrap_or(false))
    })
    .await
}
