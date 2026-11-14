//! The GRID hand action uses the same explicit-set path as Energy/MyTags.
//! No production tool calls this adapter. A synthetic Analysis Operation test
//! verifies source preservation. Editor-file cleanup follows this function.
use crate::{commands::write_error, state::AppState, AppResult};
use rbl_library::{Change, Touched};

pub(super) fn set(state: &AppState, track: &str, on: bool) -> AppResult<Option<Change>> {
    set_many(state, &[track.to_owned()], on)
}

pub(super) fn set_many(state: &AppState, tracks: &[String], on: bool) -> AppResult<Option<Change>> {
    let _gate = state.edit_gate().lock();
    // Skip the entire recording path for a native no-op. Check the switch
    // first so a protected request cannot proceed to local lock cleanup.
    if state.core().protection().protect_library {
        return Err(write_error(rbl_library::protection::refusal()));
    }
    let db = rbl_db::Library::open(state.location()?, rbl_db::OpenMode::ReadOnly)
        .map_err(write_error)?;
    let mut changed = false;
    for track in tracks {
        changed |= super::locked_in(&db, track).map_err(write_error)? != on;
    }
    drop(db);
    if !changed {
        return Ok(None);
    }
    let ((), change) = state
        .core()
        .asserting_edit(
            state,
            Touched::Metadata(tracks.to_vec()),
            "Analysis Lock",
            |writer| {
                let edits = writer.set_analysis_locks_with_undo(tracks, on)?;
                Ok(((), edits))
            },
        )
        .map_err(write_error)?;
    Ok(Some(change))
}
