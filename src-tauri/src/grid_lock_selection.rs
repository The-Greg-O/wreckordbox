//! Selection-wide window action; native acceptance precedes editor cleanup.
use super::GridEditor;
use crate::{state::AppState, AppError, AppResult};

pub(super) fn set<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &AppState,
    editor: &GridEditor,
    tracks: &[String],
    on: bool,
) -> AppResult<()> {
    let change = super::lock_recording::set_many(state, tracks, on)?;
    // The native gate is released. Clear legacy editor locks without holding
    // it, and keep trying cleanup after a failure: all native locks committed.
    let mut errors = Vec::new();
    let mut local_changed = false;
    for track in tracks {
        local_changed |= editor.is_locked(track);
        if let Err(error) = editor.set_locked(track, false) {
            errors.push(format!("{track}: {}", error.message));
        }
    }
    // One refresh also reaches the grid listeners; no per-track event burst.
    if change.is_some() || local_changed {
        let generation = change.map_or_else(|| state.core().generation(), |c| c.generation);
        let _ = tauri::Emitter::emit(app, "library:changed", generation);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::internal(format!(
            "Native analysis locks were set for [{}]; editor cleanup failed for {}",
            tracks.join(", "),
            errors.join("; "),
        )))
    }
}
