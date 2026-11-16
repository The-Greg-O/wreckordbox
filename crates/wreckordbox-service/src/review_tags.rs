//! Proposed tags in Review (wbx-45.26).
//!
//! A Fact of a tag-panel dimension (a Tag schema dimension, or a `MyTag`
//! category when there is no schema) waits for the DJ the way a proposed
//! title does ([`crate::review_text`]): live, not the DJ's, newer than the
//! DJ's latest Fact of that kind, a known explicit selection that is not
//! what the track holds now, and not a suggestion the DJ rejected. It is a
//! row of `review.waiting` with field `tag`. The dimension and the terms
//! live in `before` and `after`, because one field variant cannot name
//! every dimension.
//!
//! The Intake queue shows exactly these rows, each with the token Review
//! gives it. Accepting is `review.accept`: the tag panel's own edit
//! (`assert`, so a deck dimension is the DJ's `MyTags` and a vocabulary
//! dimension is the DJ's Fact), outside the native asserting edit so the
//! two writers are not nested. A rejection is keyed on the dimension and
//! the terms, not on the word `tag`.
use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};
use wreckordbox_sidecar::review::Suggestion;
use wreckordbox_sidecar::{Fact, FactsSnapshot, SourceKind};

use crate::library::OpenLibrary;
use crate::projection::{Field, Held};
use crate::protocol::{LibraryChanged, ServiceError};
use crate::stems::Stems;
use crate::tag_panel::{self, PanelDimension};

/// Why every such row waits: only the DJ decides.
pub(crate) const REASON: &str = "waits_for_dj";

/// Whether `field` is a proposed tag.
pub(crate) fn is_tag(field: Field) -> bool {
    field == Field::Tag
}

/// The suggestion a rejection keys: the dimension and its explicit terms.
pub(crate) fn canonical(dimension: &str, terms: &[String]) -> i64 {
    let mut terms = terms.to_vec();
    terms.sort();
    crate::review_text::canonical(&format!("{dimension}\u{1}{}", terms.join("\u{1}")))
}

/// The dimension a tag row proposes, when it carries one.
pub(crate) fn dimension_of(held: &Held) -> Option<String> {
    held.after
        .get("dimension")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// The explicit terms a tag row proposes.
pub(crate) fn terms_of(value: &Value) -> Vec<String> {
    value
        .get("terms")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The field and canonical a rejection of this row records.
pub(crate) fn rejection_of(held: &Held) -> (String, i64) {
    let dimension = dimension_of(held).unwrap_or_default();
    let canonical = canonical(&dimension, &terms_of(&held.after));
    (dimension, canonical)
}

/// Every waiting tag, of `tracks` (every track when empty), in the
/// Library's track order, then the panel's dimension order, newest first.
/// A read: no Sidecar made, migrated or tidied.
pub(crate) fn waiting(
    library: &OpenLibrary,
    stems: &Stems,
    tracks: &[String],
) -> Result<Vec<Held>, ServiceError> {
    // Resolved once: not again for each track.
    let panel = tag_panel::Panel::read(library, stems)?;
    let dimensions = panel.dimensions();
    if dimensions.is_empty() {
        return Ok(Vec::new());
    }
    let (facts, rejected) = read(stems, dimensions)?;
    // Only a live Fact that is not the DJ's can wait: a track the DJ tagged
    // by hand, with no proposal, is not read at all.
    let proposals = || {
        facts
            .values()
            .flatten()
            .filter(|fact| fact.is_live() && fact.source.kind != SourceKind::Dj)
    };
    if proposals().next().is_none() {
        return Ok(Vec::new());
    }
    let index = library.index()?;
    let mut by_track: std::collections::BTreeMap<u32, String> = std::collections::BTreeMap::new();
    for fact in proposals() {
        if !tracks.is_empty() && !tracks.contains(&fact.track_id) {
            continue;
        }
        let Some(row) = index.row_of(&fact.track_id) else {
            continue;
        };
        by_track.entry(row).or_insert_with(|| fact.track_id.clone());
    }
    let mut rows = Vec::new();
    for (row, track) in by_track {
        let selection = panel.explicit_selection(library, stems, &track)?;
        let title = crate::review_text::native(&index, row, Field::Title);
        for dimension in dimensions {
            let of = match facts.get(&dimension.id) {
                Some(found) => found.as_slice(),
                None => &[],
            };
            let current = match selection.get(&dimension.id) {
                Some(found) => found.as_slice(),
                None => &[],
            };
            rows.extend(of_dimension(
                of, &track, &title, dimension, current, &rejected,
            ));
        }
    }
    Ok(rows)
}

/// The waiting rows among one track's Facts of one dimension.
pub(crate) fn of_dimension(
    facts: &[Fact],
    track: &str,
    title: &str,
    dimension: &PanelDimension,
    current: &[String],
    rejected: &HashSet<Suggestion>,
) -> Vec<Held> {
    let decided = facts
        .iter()
        .filter(|fact| {
            fact.track_id == track && fact.is_live() && fact.source.kind == SourceKind::Dj
        })
        .map(|fact| fact.id)
        .max();
    let mut rows: Vec<Held> = facts
        .iter()
        .filter(|fact| fact.track_id == track && fact.is_live())
        .filter(|fact| fact.source.kind != SourceKind::Dj)
        .filter(|fact| decided.is_none_or(|decision| fact.id > decision))
        .filter_map(|fact| {
            let proposed = tag_panel::explicit_terms(dimension, &tag_panel::term_ids(&fact.value));
            if proposed.is_empty() || proposed == current {
                return None;
            }
            let suggestion = Suggestion::new(
                track,
                &dimension.id,
                canonical(&dimension.id, &proposed),
                &fact.source,
            );
            if rejected.contains(&suggestion) {
                return None;
            }
            Some(Held {
                track: track.to_owned(),
                title: title.to_owned(),
                field: Field::Tag,
                before: payload(dimension, current),
                after: payload(dimension, &proposed),
                fact: fact.id,
                source: fact.source.clone(),
                confidence: fact.confidence,
                reason: REASON.to_owned(),
                threshold: None,
            })
        })
        .collect();
    rows.sort_by_key(|held| std::cmp::Reverse(held.fact));
    rows
}

/// Applies each tag row through the tag panel's edit. Each row is its own
/// Undo step. A later failure leaves the earlier rows written: the error
/// carries the Facts applied and the last Library change.
pub(crate) fn apply(
    library: &OpenLibrary,
    stems: &Stems,
    tags: &[Held],
) -> Result<Option<LibraryChanged>, (Vec<i64>, Option<LibraryChanged>, ServiceError)> {
    let mut applied = Vec::new();
    let mut changed = None;
    for held in tags {
        let Some(dimension) = dimension_of(held) else {
            return Err((
                applied,
                changed,
                ServiceError::new(
                    crate::protocol::codes::SERVICE_FAILED,
                    format!("Fact {} proposes no dimension", held.fact),
                ),
            ));
        };
        match tag_panel::accept_proposed(
            library,
            stems,
            &held.track,
            &dimension,
            &terms_of(&held.after),
        ) {
            Ok(next) => {
                applied.push(held.fact);
                if next.is_some() {
                    changed = next;
                }
            }
            Err(error) => return Err((applied, changed, error)),
        }
    }
    Ok(changed)
}

fn payload(dimension: &PanelDimension, terms: &[String]) -> Value {
    let label = terms
        .iter()
        .filter_map(|id| {
            dimension
                .tags
                .iter()
                .find(|tag| tag.id == *id)
                .map(|tag| tag.name.as_str())
        })
        .collect::<Vec<_>>()
        .join(", ");
    json!({
        "dimension": dimension.id,
        "terms": terms,
        "label": label,
        "dimension_label": dimension.label,
    })
}

type TagFacts = (HashMap<String, Vec<Fact>>, HashSet<Suggestion>);

fn read(stems: &Stems, dimensions: &[PanelDimension]) -> Result<TagFacts, ServiceError> {
    let failed = |why: String| ServiceError::new(crate::stems::codes::SIDECAR_FAILED, why);
    let kinds: Vec<&str> = dimensions
        .iter()
        .map(|dimension| dimension.id.as_str())
        .collect();
    stems
        .with_sidecar_wait(|open| -> Result<TagFacts, String> {
            let text = |error: wreckordbox_sidecar::SidecarError| error.to_string();
            let load = |facts: &dyn Fn(&str) -> Result<Vec<Fact>, String>,
                        rejected: HashSet<Suggestion>|
             -> Result<TagFacts, String> {
                let mut out = HashMap::new();
                for kind in &kinds {
                    out.insert((*kind).to_owned(), facts(kind)?);
                }
                Ok((out, rejected))
            };
            match open {
                Some(sidecar) => load(
                    &|kind| sidecar.facts_of_kind(kind).map_err(text),
                    sidecar.rejected_suggestions().map_err(text)?,
                ),
                None => match FactsSnapshot::open(stems.sidecar_dir()).map_err(text)? {
                    FactsSnapshot::Absent => Ok((HashMap::new(), HashSet::new())),
                    FactsSnapshot::Unknown(why) => Err(why),
                    FactsSnapshot::Open(reader) => load(
                        &|kind| reader.facts_of_kind(kind).map_err(text),
                        reader.rejected_suggestions().map_err(text)?,
                    ),
                },
            }
        })?
        .map_err(failed)
}

/// The slot two tag rows compete on: one dimension of one track.
pub(crate) fn compete_slot(held: &Held) -> String {
    dimension_of(held).unwrap_or_else(|| held.field.name().to_owned())
}
