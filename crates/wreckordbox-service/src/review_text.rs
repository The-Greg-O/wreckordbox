//! Proposed titles and artists in Review (wbx-66).
//!
//! Projection never writes a title or an artist, so a `title` or `artist`
//! Fact from an agent or a tool waits for the DJ instead: it is a row of
//! `review.waiting` beside the Energy and Prep colour Projection held back,
//! with the same shape ([`Held`], field `title` or `artist`, `before` the
//! Library's text, `after` the proposal, reason `waits_for_dj`), the same
//! tokens, and the same decisions: `review.accept` (the DJ's hand set
//! through the edit path, recorded as the DJ's) and `review.reject`.
//!
//! A text Fact waits when it is live (its source has not replaced it), its
//! source is not the DJ, the DJ has recorded no Fact of that kind for the
//! track since (any DJ edit of the field records one, so an accept or a
//! typed title ends every older proposal), its value is not empty and not
//! the text the Library holds, and the DJ has not rejected that suggestion.
//! The Intake queue shows exactly these rows ([`crate::intake`]), each with
//! the token Review gives it, so a decision in either place is the same
//! decision and ends the row in both.
//!
//! A rejection is keyed on the proposal's [`canonical`] number: a stable
//! hash of the trimmed text, since a rejection names a number.
use std::collections::HashSet;

use serde_json::json;
use wreckordbox_sidecar::review::Suggestion;
use wreckordbox_sidecar::{Fact, FactsSnapshot, SourceKind};

use crate::library::OpenLibrary;
use crate::projection::{Field, Held};
use crate::protocol::ServiceError;
use crate::stems::Stems;

/// The fields whose proposals wait here, in the order rows are listed.
pub(crate) const FIELDS: [Field; 2] = [Field::Title, Field::Artist];

/// Why every such row waits: only the DJ decides.
pub(crate) const REASON: &str = "waits_for_dj";

/// The suggestions standing rejected.
pub(crate) type Rejected = HashSet<Suggestion>;

/// Each text field's Facts, and the standing rejections.
type TextFacts = (Vec<(Field, Vec<Fact>)>, Rejected);

/// Whether `field` is one of these text fields.
pub(crate) fn is_text(field: Field) -> bool {
    FIELDS.contains(&field)
}

/// A proposal's text as a rejection keys it: the FNV-1a hash of the
/// trimmed text, kept positive.
pub(crate) fn canonical(text: &str) -> i64 {
    let hash = text
        .trim()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        });
    i64::try_from(hash >> 1).unwrap_or(0)
}

/// What the Library holds now for the text `field` of the track at `row`.
pub(crate) fn native(index: &rbl_index::Library, row: u32, field: Field) -> String {
    match field {
        Field::Artist => index.artist_name(row).to_owned(),
        _ => index.title.get(row as usize).to_owned(),
    }
}

/// The waiting rows among one track's Facts of the text `field` (any
/// order), newest first; `held` is the Library's text and `title` the
/// track's title as shown.
pub(crate) fn of_track(
    facts: &[Fact],
    track: &str,
    title: &str,
    field: Field,
    held: &str,
    rejected: &Rejected,
) -> Vec<Held> {
    let kind = field.name();
    let decided = facts
        .iter()
        .filter(|f| f.kind == kind && f.is_live() && f.source.kind == SourceKind::Dj)
        .map(|f| f.id)
        .max();
    let mut rows: Vec<Held> = facts
        .iter()
        .filter(|f| f.kind == kind && f.track_id == track && f.is_live())
        .filter(|f| f.source.kind != SourceKind::Dj)
        .filter(|f| decided.is_none_or(|decision| f.id > decision))
        .filter_map(|f| {
            let value = f.value.as_str()?.trim();
            if value.is_empty() || value == held.trim() {
                return None;
            }
            let suggestion = Suggestion::new(track, kind, canonical(value), &f.source);
            if rejected.contains(&suggestion) {
                return None;
            }
            Some(Held {
                track: track.to_owned(),
                title: title.to_owned(),
                field,
                before: json!(held),
                after: json!(value),
                fact: f.id,
                source: f.source.clone(),
                confidence: f.confidence,
                reason: REASON.to_owned(),
                threshold: None,
            })
        })
        .collect();
    rows.sort_by_key(|h| std::cmp::Reverse(h.fact));
    rows
}

/// Every waiting title and artist, of `tracks` (every track when empty),
/// in the Library's track order, then title before artist, newest first.
/// A read: no Sidecar made, migrated or tidied.
pub(crate) fn waiting(
    library: &OpenLibrary,
    stems: &Stems,
    tracks: &[String],
) -> Result<Vec<Held>, ServiceError> {
    let (facts, rejected) = read(stems)?;
    if facts.iter().all(|(_, of)| of.is_empty()) {
        return Ok(Vec::new());
    }
    let index = library.index()?;
    let mut by_row: std::collections::BTreeMap<(u32, usize), Vec<Fact>> =
        std::collections::BTreeMap::new();
    for (at, (_, of)) in facts.into_iter().enumerate() {
        for fact in of {
            if !tracks.is_empty() && !tracks.contains(&fact.track_id) {
                continue;
            }
            // Facts about a track no longer in the Library wait nowhere.
            let Some(row) = index.row_of(&fact.track_id) else {
                continue;
            };
            by_row.entry((row, at)).or_default().push(fact);
        }
    }
    let mut rows = Vec::new();
    for ((row, at), facts) in by_row {
        let field = FIELDS[at];
        let Some(track) = facts.first().map(|f| f.track_id.clone()) else {
            continue;
        };
        let title = native(&index, row, Field::Title);
        let held = native(&index, row, field);
        rows.extend(of_track(&facts, &track, &title, field, &held, &rejected));
    }
    Ok(rows)
}

/// Each text field's Facts and the standing rejections.
fn read(stems: &Stems) -> Result<TextFacts, ServiceError> {
    let failed = |why: String| ServiceError::new(crate::stems::codes::SIDECAR_FAILED, why);
    stems
        .with_sidecar_wait(|open| -> Result<TextFacts, String> {
            let text = |e: wreckordbox_sidecar::SidecarError| e.to_string();
            match open {
                Some(sidecar) => {
                    let mut facts = Vec::new();
                    for field in FIELDS {
                        facts.push((field, sidecar.facts_of_kind(field.name()).map_err(text)?));
                    }
                    Ok((facts, sidecar.rejected_suggestions().map_err(text)?))
                }
                None => match FactsSnapshot::open(stems.sidecar_dir()).map_err(text)? {
                    FactsSnapshot::Absent => Ok((Vec::new(), HashSet::new())),
                    FactsSnapshot::Unknown(why) => Err(why),
                    FactsSnapshot::Open(reader) => {
                        let mut facts = Vec::new();
                        for field in FIELDS {
                            facts.push((field, reader.facts_of_kind(field.name()).map_err(text)?));
                        }
                        Ok((facts, reader.rejected_suggestions().map_err(text)?))
                    }
                },
            }
        })?
        .map_err(failed)
}

#[cfg(test)]
mod tests {
    use super::canonical;

    #[test]
    fn the_same_text_trimmed_is_the_same_suggestion_and_another_is_not() {
        assert_eq!(canonical("A Title"), canonical("  A Title "));
        assert_ne!(canonical("A Title"), canonical("A Title 2"));
        assert!(canonical("anything") >= 0);
    }
}
