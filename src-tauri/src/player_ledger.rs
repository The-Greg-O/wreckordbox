//! Values already obtained by a player write; no extra reads or DJ Facts.
use rbl_db::write::TrackEdit;
use rbl_library::history::LibraryEdit;
use serde_json::{json, Value};

pub(crate) fn rating(edit: TrackEdit) -> Value {
    let track = edit.parts().track;
    let payload = wreckordbox_service::ledger::stored::store(&LibraryEdit::Track(vec![edit]));
    json!({"operation":"rating","track":track,"track_edit":payload})
}

/// Compact values from the received message, never a Library snapshot.
pub(crate) fn link_edit(edit: &rbl_link::Edit) -> serde_json::Value {
    use rbl_link::Edit;
    use serde_json::json;
    match edit {
        Edit::Tag { track, add } => {
            json!({"operation":if *add {"tag_add"} else {"tag_remove"},"track":track.to_string()})
        }
        Edit::ClearTags => json!({"operation":"tag_clear"}),
        Edit::Rating { track, stars } => {
            json!({"operation":"rating","track":track.to_string(),"after":stars})
        }
        Edit::SaveCue { track, cue } | Edit::DeleteCue { track, cue } => {
            json!({"operation":if matches!(edit, Edit::SaveCue { .. }) {"cue_save"} else {"cue_delete"},"track":track.to_string(),"cue":{"kind":cue.kind,"in_ms":cue.in_ms,"out_ms":cue.out_ms}})
        }
        Edit::HotCueBankCue { bank, cue } => {
            json!({"operation":"hot_cue_bank","bank":bank,"slot":cue.slot,"track":cue.content.to_string()})
        }
        _ => json!({"operation":"unsupported_player_edit"}),
    }
}
