//! Library tracks added to one of a Stick's playlists, as one approved
//! write (wbx-45.4, third review): the tracks the Stick lacks are exported
//! to it, then every track, in the order dropped, is resolved to the
//! Stick's own row natively and added. One lock, one token, bound to the
//! exact ordered Library tracks, the duplicates answer, the defaults and
//! the format, and the Library generation; nothing the window counted.

use std::path::Path;
use std::sync::Arc;

use tauri::State;

use crate::commands::{blocking, reserve_export, track_export_selection, write_export_registered};
use crate::device_library::DeviceEditResultDto;
use crate::device_settings::StickDefaultsDto;
use crate::error::AppResult;
use crate::state::AppState;
use crate::stick_approval::{bound, StickOperation};

#[tauri::command]
#[allow(
    clippy::too_many_arguments,
    reason = "the window's arguments, as the other Stick writes take them"
)]
pub async fn device_playlist_add_library_tracks<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, Arc<AppState>>,
    path: String,
    format: String,
    playlist: String,
    tracks: Vec<String>,
    duplicates: bool,
    defaults: Option<StickDefaultsDto>,
    compatibility_format: Option<rbl_export::CompatibilityFormat>,
    // The DJ's approval of this exact write (`stick_write_plan`).
    approval: Option<String>,
) -> AppResult<DeviceEditResultDto> {
    let state = Arc::clone(&state);
    blocking("device_playlist_add_library_tracks", move || {
        let stick = Path::new(&path);
        // The lock order (`stick_guard`): the Library edit gate first, held
        // while the Library is read; then the Stick, protected or not.
        let gate = state.edit_gate().lock();
        let (library, generation) = state.library_at_generation()?;
        let access = crate::stick_guard::enter_stick(state.backup_dir(), stick)?;
        crate::stick_approval::consume(
            approval.as_deref(),
            &StickOperation::AddLibraryTracks {
                format: format.clone(),
                playlist: playlist.clone(),
                tracks: tracks.clone(),
                duplicates,
                defaults: bound(defaults.as_ref()),
                compatibility: bound(compatibility_format.as_ref()),
            },
            stick,
            generation,
        )?;
        let resolved = crate::device_library::resolve_library_tracks(
            &state, stick, &format, &playlist, &tracks, duplicates,
        )?;
        if !resolved.missing.is_empty() {
            let reservation = reserve_export(&app, stick)?;
            let selection = track_export_selection(
                &state,
                &library,
                &state.share_root(),
                stick,
                &resolved.missing,
            )?;
            drop(gate);
            let report = write_export_registered(
                &app,
                stick,
                &selection,
                defaults.as_ref(),
                compatibility_format,
                &reservation,
                &access,
            )?;
            let _ = tauri::Emitter::emit(&app, "export:done", &report);
        }
        crate::device_library::add_library_rows(&access, &state, stick, &format, &playlist, &tracks, duplicates)
    })
    .await
}
