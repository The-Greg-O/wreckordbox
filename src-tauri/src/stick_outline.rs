//! The shell's tree: each mounted Stick's playlists, read without writing
//! to the Stick (ticket #52).
//!
//! Unlike `device_libraries`, which an explicit open of a Stick calls and
//! which first finishes an interrupted export, this never recovers anything:
//! an unfinished write is reported as `pending` and nothing is listed.

use crate::commands::blocking;
use crate::error::{AppError, AppResult, ErrorKind};

#[tauri::command]
pub async fn stick_outline(path: String) -> AppResult<wreckordbox_stick_outline::Outline> {
    blocking("stick_outline", move || {
        let mount = std::path::PathBuf::from(path);
        if !mount.is_dir() {
            return Err(AppError::new(
                ErrorKind::NotFound,
                "That device is no longer connected.",
            ));
        }
        Ok(wreckordbox_stick_outline::outline(&mount))
    })
    .await
}
