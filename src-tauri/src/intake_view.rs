//! The Intake view's reads (wbx-62).
//!
//! Intake and Problems are queries of the Library service (wbx-61), which
//! this window hosts; nothing here reads the Library by itself. Each queued
//! track comes back beside the row a deck loads, built from the window's
//! own index the way a list's rows are, so the view can load the track it
//! shows without opening a second list.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wreckordbox_service::{intake, ServiceError};

use crate::commands::blocking;
use crate::dto::RowDto;
use crate::error::{AppError, AppResult, ErrorKind};
use crate::state::{rows_to_dto, AppState};

/// Which queue the Intake view shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IntakeQueueName {
    /// The tracks with no Prep colour.
    Intake,
    /// The tracks whose Prep colour is the one the DJ uses for Problem.
    Problems,
}

/// Why a queue cannot be listed now, said for the DJ.
#[derive(Debug, Clone, Serialize)]
pub struct Unavailable {
    pub code: String,
    pub message: String,
}

/// A page of a queue.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntakePage {
    /// The service's answer as it gave it (`Queue`, see `service-protocol`);
    /// absent when the queue cannot be listed.
    pub queue: Option<Value>,
    /// The deck's row for each of the queue's tracks, in the same order.
    pub rows: Vec<RowDto>,
    /// Why the queue could not be listed: Problems when no colour label is
    /// named Problem.
    pub unavailable: Option<Unavailable>,
}

/// Runs one Intake command on the service this window hosts.
fn ask(state: &AppState, command: &str, args: Value) -> AppResult<Result<Value, ServiceError>> {
    state.with_service(|service| service.call(command, args)).ok_or_else(|| {
        AppError::new(
            ErrorKind::NotFound,
            "The Library service is not running in this window, so the Intake queue cannot be read.",
        )
    })
}

/// One page of a queue, with each track's row. A queue the service cannot
/// list for a reason the DJ can fix (no colour label named Problem) is an
/// answer with `unavailable`, not an error; anything else is an error.
pub fn queue_page(
    state: &Arc<AppState>,
    queue: IntakeQueueName,
    offset: usize,
    limit: usize,
) -> AppResult<IntakePage> {
    let command = match queue {
        IntakeQueueName::Intake => intake::commands::INTAKE_LIST,
        IntakeQueueName::Problems => intake::commands::INTAKE_PROBLEMS,
    };
    let answer = match ask(state, command, json!({ "offset": offset, "limit": limit }))? {
        Ok(answer) => answer,
        Err(error)
            if error.code == intake::codes::PROBLEM_COLOUR_NOT_FOUND
                || error.code == intake::codes::PROBLEM_COLOUR_AMBIGUOUS =>
        {
            return Ok(IntakePage {
                queue: None,
                rows: Vec::new(),
                unavailable: Some(Unavailable {
                    code: error.code,
                    message: error.message,
                }),
            });
        }
        Err(error) => return Err(failed(&error)),
    };
    let library = state.library()?;
    // The service and the window share one Library core, so every id it
    // lists is a row of the window's index; one that is not (a change between
    // the two reads) is left out of both lists, keeping them aligned.
    let mut answer = answer;
    let mut kept_rows = Vec::new();
    if let Some(tracks) = answer.get_mut("tracks").and_then(Value::as_array_mut) {
        tracks.retain(|track| {
            let row = track
                .get("id")
                .and_then(Value::as_str)
                .and_then(|id| library.row_of(id));
            if let Some(row) = row {
                kept_rows.push(row);
            }
            row.is_some()
        });
    }
    let mut rows = rows_to_dto(&library, &kept_rows, offset);
    for (position, row) in rows.iter_mut().enumerate() {
        row.track_no = u32::try_from(offset + position + 1).unwrap_or(u32::MAX);
    }
    Ok(IntakePage {
        queue: Some(answer),
        rows,
        unavailable: None,
    })
}

/// How many tracks wait in each queue, for the view's tab and the tree:
/// the service's `intake.counts` answer as it gave it.
pub fn counts(state: &Arc<AppState>) -> AppResult<Value> {
    ask(state, intake::commands::INTAKE_COUNTS, json!({}))?.map_err(|error| failed(&error))
}

/// The DJ's eight Prep colours and which mean Clean and Problem (wbx-63):
/// the service's `intake.colours` answer as it gave it.
pub fn colours(state: &Arc<AppState>) -> AppResult<Value> {
    ask(state, intake::commands::INTAKE_COLOURS, json!({}))?.map_err(|error| failed(&error))
}

/// Sets one track's Prep colour, by colour label number (1 to 8), or clears
/// it (`None`), through the hosted service's `tracks.set_prep_colour`: the
/// Library's single edit path, one Track Edit the window's Undo takes back,
/// refused under Library Protection. The service tells the window, so the
/// queue and its counts re-read.
pub fn set_prep_colour(state: &Arc<AppState>, id: &str, colour: Option<u8>) -> AppResult<Value> {
    let named = match colour {
        None => "none",
        Some(number) => usize::from(number)
            .checked_sub(1)
            .and_then(|at| rbl_index::COLOR_NAMES.get(at))
            .copied()
            .ok_or_else(|| {
                AppError::new(
                    ErrorKind::Malformed,
                    format!("There is no colour label {number}."),
                )
            })?,
    };
    ask(
        state,
        intake::commands::TRACKS_SET_PREP_COLOUR,
        json!({ "id": id, "prep_colour": named }),
    )?
    .map_err(|error| {
        // A refusal (Library Protection, rekordbox running) is the Library
        // being read-only for now; anything else is a failure to say.
        let kind = if error.code == "library_protected" || error.code == "write_refused" {
            ErrorKind::ReadOnly
        } else {
            ErrorKind::Internal
        };
        AppError::new(kind, error.message.clone()).with_detail(error.code.clone())
    })
}

fn failed(error: &ServiceError) -> AppError {
    AppError::new(
        ErrorKind::Internal,
        format!("The Intake queue could not be read: {}", error.message),
    )
    .with_detail(error.code.clone())
}

/// A page of the Intake or Problems queue (`IntakeView`).
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a command is handed its state by value"
)]
pub async fn intake_queue(
    state: tauri::State<'_, Arc<AppState>>,
    queue: IntakeQueueName,
    offset: usize,
    limit: usize,
) -> AppResult<IntakePage> {
    let state = Arc::clone(&state);
    blocking("intake_queue", move || {
        queue_page(&state, queue, offset, limit)
    })
    .await
}

/// The Intake and Problems counts (the Intake tab, the tree).
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a command is handed its state by value"
)]
pub async fn intake_counts(state: tauri::State<'_, Arc<AppState>>) -> AppResult<Value> {
    let state = Arc::clone(&state);
    blocking("intake_counts", move || counts(&state)).await
}

/// The DJ's Prep colours (the Intake view's Prep colour buttons).
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a command is handed its state by value"
)]
pub async fn intake_colours(state: tauri::State<'_, Arc<AppState>>) -> AppResult<Value> {
    let state = Arc::clone(&state);
    blocking("intake_colours", move || colours(&state)).await
}

/// Sets or clears a track's Prep colour from the Intake view.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a command is handed its state by value"
)]
pub async fn intake_set_prep_colour(
    state: tauri::State<'_, Arc<AppState>>,
    id: String,
    colour: Option<u8>,
) -> AppResult<Value> {
    let state = Arc::clone(&state);
    blocking("intake_set_prep_colour", move || {
        set_prep_colour(&state, &id, colour)
    })
    .await
}
