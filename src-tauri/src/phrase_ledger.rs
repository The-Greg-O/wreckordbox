//! Keep the small phrase section already in memory, not an analysis-file copy.
use serde_json::{json, Value};

pub(super) fn values(before: &rbl_anlz::Anlz, after: &[u8], relative: &str) -> Value {
    let relative = std::path::Path::new(relative).with_extension("EXT");
    let after = rbl_anlz::parse(after).ok().as_ref().and_then(section);
    match (section(before), after) {
        (Some(before), Some(after)) => json!({
            "analysis": relative, "before": before, "after": after
        }),
        _ => json!({"analysis": relative, "not_reversible_reason":
            "phrase metadata is unavailable or exceeds the 64 KiB recording bound"}),
    }
}

fn section(file: &rbl_anlz::Anlz) -> Option<Value> {
    let section = file
        .sections
        .iter()
        .find(|section| &section.tag.0 == b"PSSI")?;
    if section.header.len().saturating_add(section.payload.len()) > 64 * 1024 {
        return None;
    }
    Some(json!({"header": section.header, "payload": section.payload}))
}
