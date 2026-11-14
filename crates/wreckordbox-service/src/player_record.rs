//! Persist player scalar edits in the same shape Projection and Unwind read.
//! Used by both immediate recording and spill replay; never derives DJ Facts.
use serde_json::{json, Value};

pub(super) struct Description {
    pub kind: String,
    pub label: String,
    pub payload: Value,
    pub reversible: bool,
}

pub(super) fn describe(kind: &str, label: &str, record: &Value) -> Description {
    let player = matches!(kind, "link" | "round_trip");
    if player {
        if let Some(payload) = record.get("track_edit") {
            if super::stored::restore("track", 1, payload).is_some() {
                let mut payload = payload.clone();
                if let Some(warning) = record.get("order_unavailable") {
                    payload["order_unavailable"] = warning.clone();
                }
                return Description {
                    kind: "track".into(),
                    label: label.into(),
                    payload,
                    reversible: true,
                };
            }
        }
    }
    let reason = if player {
        "Previous player cue, tag-list or history rows and analysis artifacts were not retained; Unwind cannot reverse this operation"
    } else {
        super::bulk::REASON
    };
    Description {
        kind: kind.into(),
        label: format!("{label}. Cannot reverse: {reason}"),
        payload: json!({"records":[record],"not_reversible_reason":reason}),
        reversible: false,
    }
}
