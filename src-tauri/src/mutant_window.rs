//! The window's side of Mutants following playlist edits (wbx-80).
//!
//! - A folder drop's or an import's Replace existing writes the Library
//!   through [`write_replacing`]: the core's prepared write, declared as one
//!   that replaces entries, so the Ledger keeps a plan for every Mutant of
//!   the playlists it may replace before the write is made and can carry
//!   them to the new entries even if the window dies before the change is
//!   recorded. The change's history entry is recorded by the caller as
//!   before, and the Ledger moves the Mutants with it.
//! - Remove from Collection says which Mutants go with the tracks before
//!   the DJ confirms ([`collection_removal_mutants`]): the hosted service's
//!   `tracks.remove_from_collection` asked without `confirm`, which reads
//!   and writes nothing.

use std::sync::Arc;

use rbl_library::ledger::Preparation;
use rbl_library::History;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::State;
use wreckordbox_service::playlist_edits::commands::TRACKS_REMOVE_FROM_COLLECTION;

use crate::commands::blocking;
use crate::error::{AppError, AppResult, ErrorKind};
use crate::state::AppState;

/// A write that may replace playlists' entries (a Replace existing), when
/// `replaces`; otherwise the window's ordinary write outside the history.
/// Take the edit gate first and hold it to the history entry.
///
/// A replacing write is a write of the history's own (`History::Caller`):
/// the caller then records its entry, or, when nothing was replaced after
/// all, tells the history it made an unrecorded write
/// (`EditHistory::unrecorded`).
///
/// # Errors
/// The write's own.
pub(crate) fn write_replacing<T>(
    state: &AppState,
    replaces: bool,
    edit: impl FnOnce(&mut rbl_db::write::Writer) -> Result<T, rbl_db::DbError>,
) -> Result<T, rbl_db::DbError> {
    if !replaces {
        return state.write(edit);
    }
    let preparation = Preparation {
        replaces_entries: true,
        ..Preparation::default()
    };
    state
        .core()
        .write_then_prepared(state, History::Caller, &preparation, edit, |_, done| {
            Ok(done)
        })
}

/// One Mutant that would go with the tracks, as the confirmation names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovalMutantDto {
    pub id: i64,
    pub playlist_id: String,
    pub playlist_name: String,
    /// Its place in the playlist, from 1, when the entry is still there.
    pub position: Option<u64>,
    pub track_id: String,
    pub title: String,
    /// What differs from the parent, in words ("+1 st").
    pub description: String,
}

/// The Mutants that Remove from Collection would remove with `tracks`.
/// Reads only; an empty list when the window hosts no service (there is
/// then no Sidecar to have Mutants in).
#[tauri::command]
pub async fn collection_removal_mutants(
    state: State<'_, Arc<AppState>>,
    tracks: Vec<String>,
) -> AppResult<Vec<RemovalMutantDto>> {
    let state = Arc::clone(&state);
    blocking("collection_removal_mutants", move || {
        removal_mutants(&state, &tracks)
    })
    .await
}

/// [`collection_removal_mutants`], blocking.
///
/// # Errors
/// The service could not answer.
pub fn removal_mutants(state: &AppState, tracks: &[String]) -> AppResult<Vec<RemovalMutantDto>> {
    if tracks.is_empty() {
        return Ok(Vec::new());
    }
    let asked = state.with_service(|service| {
        service.call(
            TRACKS_REMOVE_FROM_COLLECTION,
            json!({ "tracks": tracks, "confirm": false }),
        )
    });
    let Some(answer) = asked else {
        return Ok(Vec::new());
    };
    let answer = answer.map_err(|error| {
        AppError::new(
            ErrorKind::NotFound,
            format!(
                "The Mutants of these tracks could not be read: {}",
                error.message
            ),
        )
    })?;
    Ok(answer["mutants"]
        .as_array()
        .map(|mutants| mutants.iter().map(dto).collect())
        .unwrap_or_default())
}

fn dto(mutant: &Value) -> RemovalMutantDto {
    let text = |value: &Value| value.as_str().unwrap_or_default().to_owned();
    RemovalMutantDto {
        id: mutant["id"].as_i64().unwrap_or_default(),
        playlist_id: text(&mutant["playlist"]["id"]),
        playlist_name: text(&mutant["playlist"]["name"]),
        position: mutant["position"].as_u64(),
        track_id: text(&mutant["parent"]["id"]),
        title: text(&mutant["parent"]["title"]),
        description: text(&mutant["derived"]["description"]),
    }
}
