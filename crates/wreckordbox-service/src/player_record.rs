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
        if record["status"] == "partial_or_failed" {
            // A native prefix or the whole write may have committed before
            // its error. Projection must not fill or replace those fields.
            // Keep the exact bounded targets, without another Library read.
            let target = &record["target"];
            let mut tracks = target["tracks"].as_array().cloned().unwrap_or_default();
            if let Some(track) = target["track"].as_str() {
                tracks.push(Value::from(track));
            }
            return Description {
                kind: "unconfirmed".into(),
                label: format!("{label}: some player edits may have committed before failure"),
                payload: json!({"tracks":tracks,"records":[record]}),
                reversible: false,
            };
        }
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
