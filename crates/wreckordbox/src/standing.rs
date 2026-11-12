//! Text and JSON for the read-only standing-Fact query.
use serde_json::json;
use std::fmt::Write;
use wreckordbox_service::standing::{Standing, FACTS_STANDING};
use wreckordbox_service::{Client, ServiceError};

pub(crate) fn show(
    client: &mut Client,
    as_json: bool,
    track: &str,
    kind: &str,
) -> Result<String, ServiceError> {
    let result: Standing = client.call(FACTS_STANDING, json!({"id": track, "kind": kind}))?;
    if as_json {
        return Ok(json!(result).to_string());
    }
    let mut text = format!(
        "Track {} {:?}, {}\n",
        result.track.id, result.track.title, result.kind
    );
    if let Some(fact) = &result.standing {
        let _ = writeln!(
            text,
            "Standing Fact {}: {} ({}, confidence {}).",
            fact.id,
            fact.value,
            crate::facts::source_text(&fact.source),
            fact.confidence
        );
    }
    let _ = writeln!(text, "{}", result.reason.message);
    for fact in &result.superseded {
        let _ = writeln!(
            text,
            "Superseded Fact {}: {} ({}, confidence {}).",
            fact.id,
            fact.value,
            crate::facts::source_text(&fact.source),
            fact.confidence
        );
    }
    for fact in &result.stale {
        let _ = writeln!(
            text,
            "Stale Fact {}: {} (audio differs).",
            fact.id, fact.value
        );
    }
    let _ = write!(text, "Source rules: {}", result.rules_file.display());
    Ok(text)
}
