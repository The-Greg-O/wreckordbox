//! The GRID hand action uses the same explicit-set path as Energy/MyTags.
//! Tool analysis continues to use its bounded Analysis Operation, not this
//! window setter. Local editor-file cleanup belongs after this function returns.
use crate::{commands::write_error, state::AppState, AppResult};
use rbl_library::{Change, Touched};

pub(super) fn set(state: &AppState, track: &str, on: bool) -> AppResult<Option<Change>> {
    let _gate = state.edit_gate().lock();
    // Skip the entire recording path for a native no-op. Check the switch
    // first so a protected request cannot proceed to local lock cleanup.
    if state.core().protection().protect_library {
        return Err(write_error(rbl_library::protection::refusal()));
    }
    if super::database_locked(&state.location()?, track)? == on {
        return Ok(None);
    }
    let ((), change) = state
        .core()
        .asserting_edit(
            state,
            Touched::Metadata(vec![track.to_owned()]),
            "Analysis Lock",
            |writer| {
                let edits = writer.set_analysis_lock_with_undo(track, on)?;
                Ok(((), edits.into_iter().collect()))
            },
        )
        .map_err(write_error)?;
    Ok(Some(change))
}
