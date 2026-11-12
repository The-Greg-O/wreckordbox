//! Retain the grid values the editor already has. No reads or fingerprints.
use super::GridAction;
use rbl_library::{file_edit::FileEdit, file_record::FileRecord};

pub(super) fn begin<'a>(
    guard: &'a mut FileEdit<'_>,
    track: &str,
    action: GridAction,
    relative: &str,
    before: &[rbl_anlz::Beat],
    after: &[rbl_anlz::Beat],
) -> crate::AppResult<FileRecord<'a>> {
    let values = |beats: &[rbl_anlz::Beat]| {
        beats
            .iter()
            .map(|beat| (beat.time_ms, beat.beat_number, beat.tempo_x100))
            .collect::<Vec<_>>()
    };
    guard.recording_values("window_grid", track, serde_json::json!({
        "analysis": relative,
        "before": values(before), "after": values(after),
        "step": match action { GridAction::Edit { .. } => "write", GridAction::Undo => "undo", GridAction::Redo => "redo" }
    })).map_err(crate::commands::write_error)
}
