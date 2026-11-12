//! The Intake view's tag panel (wbx-65).
//!
//! The panel's description (its dimensions, which reach the deck, their
//! tags, and what a track carries) is the hosted Library service's
//! `intake.tag_panel`. A deck-facing tag is toggled with the window's own
//! My Tag edit (`details::put_my_tag`): the single edit path, one undoable
//! step, refused under Library Protection. A dimension that does not reach
//! the deck is the DJ's Fact of its kind, recorded here through the
//! service's `facts.record`: a Sidecar write, not a Library edit.

use std::sync::Arc;

use serde_json::{json, Value};
use wreckordbox_service::{tag_panel, ServiceError};

use crate::commands::blocking;
use crate::error::{AppError, AppResult, ErrorKind};
use crate::state::AppState;

fn ask(state: &AppState, command: &str, args: Value) -> AppResult<Result<Value, ServiceError>> {
    state
        .with_service(|service| service.call(command, args))
        .ok_or_else(|| {
            AppError::new(
                ErrorKind::NotFound,
                "The Library service is not running in this window, so the tags cannot be read.",
            )
        })
}

/// The panel's description, with `track`'s tags when one is given.
pub fn tag_panel(state: &Arc<AppState>, track: Option<&str>) -> AppResult<Value> {
    let args = track.map_or_else(|| json!({}), |id| json!({ "id": id }));
    ask(state, tag_panel::INTAKE_TAG_PANEL, args)?.map_err(|error| {
        AppError::new(
            ErrorKind::Internal,
            format!("The tags could not be read: {}", error.message),
        )
        .with_detail(error.code.clone())
    })
}

/// Sets the DJ's tags of one dimension that does not reach the deck to
/// exactly `terms` (an empty list clears them), as the DJ's Fact.
pub fn set_tag_fact(
    state: &Arc<AppState>,
    track: &str,
    dimension: &str,
    terms: &[String],
) -> AppResult<Value> {
    ask(
        state,
        "facts.record",
        json!({
            "description": "Tags set in Intake",
            "source": { "kind": "dj" },
            "facts": [{ "track": track, "kind": dimension, "value": terms }],
        }),
    )?
    .map_err(|error| {
        AppError::new(
            ErrorKind::Malformed,
            format!("Not saved: {}", error.message),
        )
        .with_detail(error.code.clone())
    })
}

/// The tag panel's description (`IntakeTagPanel`).
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a command is handed its state by value"
)]
pub async fn intake_tag_panel(
    state: tauri::State<'_, Arc<AppState>>,
    track: Option<String>,
) -> AppResult<Value> {
    let state = Arc::clone(&state);
    blocking("intake_tag_panel", move || {
        tag_panel(&state, track.as_deref())
    })
    .await
}

/// Sets a vocabulary dimension's tags on a track from the tag panel.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a command is handed its state by value"
)]
pub async fn intake_set_tag_fact(
    state: tauri::State<'_, Arc<AppState>>,
    track: String,
    dimension: String,
    terms: Vec<String>,
) -> AppResult<Value> {
    let state = Arc::clone(&state);
    blocking("intake_set_tag_fact", move || {
        set_tag_fact(&state, &track, &dimension, &terms)
    })
    .await
}
