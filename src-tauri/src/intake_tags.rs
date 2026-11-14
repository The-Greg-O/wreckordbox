//! The Intake view's tag panel (wbx-65).
//!
//! Both the panel's description and every click are the hosted Library
//! service's (`intake.tag_panel`, `intake.set_tags`): the service checks a
//! click against the Tag schema as it is now and makes it ONE entry of the
//! edit history this window shares with it, whether it changed the track's
//! `MyTags` or only the DJ's Fact of a dimension that does not reach the
//! deck. So the window's Undo takes back the DJ's last action whichever
//! store it touched, and Library Protection refuses every click.

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
                "The Library service is not running in this window, so the tags cannot be read or set.",
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

/// One click of the panel: `dimension` of `track` set to exactly `terms`
/// (the explicit selection), from the panel at `version`. The answer is
/// the service's `TagsSet`. A refusal keeps the service's code as the
/// error's detail: `tag_panel_stale` (read again, click again),
/// `library_protected`, `tag_not_settable`, `tag_not_in_dimension`.
pub fn set_tags(
    state: &Arc<AppState>,
    track: &str,
    dimension: &str,
    terms: &[String],
    version: &str,
    drop_unmapped: &[String],
    comments: bool,
) -> AppResult<Value> {
    ask(
        state,
        tag_panel::commands::INTAKE_SET_TAGS,
        json!({
            "id": track,
            "dimension": dimension,
            "terms": terms,
            "version": version,
            "drop_unmapped": drop_unmapped,
            "comments": comments,
            // A click of the panel: the DJ's selection (wbx-29).
            "assert": true,
        }),
    )?
    .map_err(|error| {
        let kind = match error.code.as_str() {
            "library_protected" | "write_refused" => ErrorKind::ReadOnly,
            "tag_panel_stale" => ErrorKind::Conflict,
            _ => ErrorKind::Malformed,
        };
        AppError::new(kind, error.message.clone()).with_detail(error.code.clone())
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

/// One click of the tag panel.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    clippy::too_many_arguments,
    reason = "a command is handed its state and arguments by value"
)]
pub async fn intake_set_tags(
    state: tauri::State<'_, Arc<AppState>>,
    track: String,
    dimension: String,
    terms: Vec<String>,
    version: String,
    drop_unmapped: Option<Vec<String>>,
    comments: Option<bool>,
) -> AppResult<Value> {
    let state = Arc::clone(&state);
    blocking("intake_set_tags", move || {
        set_tags(
            &state,
            &track,
            &dimension,
            &terms,
            &version,
            &drop_unmapped.unwrap_or_default(),
            comments.unwrap_or(false),
        )
    })
    .await
}
