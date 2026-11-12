//! The Track Filter bar's fields on `tracks.query` (wbx-57).
//!
//! The window's Track Filter bar picks several keys, several Prep colours,
//! and MyTags within a category with that category's AND / OR switch. These
//! are the same picks the window hands its list (`TrackFilter` in
//! `rbl_index`), resolved from names exactly as the window resolves them, so
//! a filter sent here and the same filter in the window narrow to the same
//! tracks:
//!
//! - `keys`: every key id carrying one of the names (`key_ids_named`); a
//!   track in any of them passes. A single `key` joins the list.
//! - `prep_colours`: colour names (any case), a track with any of them
//!   passes. A single `prep_colour` joins the list.
//! - `keys` are key identities: a key on the Camelot wheel in any spelling
//!   takes every spelling of it (`8A`, `Am`, Open Key `1m`); a name off the
//!   wheel is matched exactly (`key_ids_identified`).
//! - `energies`: values or ranges, `0` for none; a track with any passes.
//! - `bpm_ranges`: BPM ranges to the hundredth, inclusive; a track in any
//!   passes (`BpmFilter::ranges_x100`). A range whose low end is above its
//!   high end admits nothing.
//! - `my_tag_picks`: one entry per category, `{"category", "tags", "any"}`,
//!   the tags resolved within that category only (`tag_picks_named`). Within
//!   a pick, `any` is OR and otherwise AND; every pick must agree. A tag or a
//!   category the Library no longer holds is left out of the filter and
//!   named in the page's `dropped`, as the window drops it from the bar.
#![allow(
    clippy::doc_markdown,
    reason = "MyTag is the glossary's word for the thing, not a code item"
)]

use rbl_index::{TagFilter, TrackFilter, COLOR_NAMES};
use serde::{Deserialize, Serialize};

use crate::protocol::{codes, ServiceError};
use crate::tracks::TrackSelection;

/// The MyTags picked within one category, as one of the Track Filter bar's
/// category menus holds them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MyTagPick {
    /// The category by its name.
    pub category: String,
    /// Tag names within that category. At least one.
    pub tags: Vec<String>,
    /// `true`: a track carrying any of the tags passes. `false` (the
    /// default): it must carry every one.
    pub any: bool,
}

/// Adds the bar's fields of `selection` to the picks already made from its
/// single-value fields.
pub(crate) fn extend(
    index: &rbl_index::Library,
    selection: &TrackSelection,
    filter: &mut TrackFilter,
) -> Result<(), ServiceError> {
    if !selection.keys.is_empty() {
        let mut ids = index.key_ids_identified(&selection.keys);
        if let Some(single) = &selection.key {
            ids.extend(index.key_ids_named(std::slice::from_ref(single)));
        }
        filter.keys = Some(ids);
    }
    if !selection.energies.is_empty() {
        let mut ratings = filter.ratings.take().unwrap_or_default();
        for text in &selection.energies {
            let (low, high) = energy_range(text)?;
            ratings.extend(low..=high);
        }
        filter.ratings = Some(ratings);
    }
    if !selection.bpm_ranges.is_empty() {
        let ranges = selection
            .bpm_ranges
            .iter()
            .map(|text| bpm_range(text))
            .collect::<Result<Vec<_>, _>>()?;
        filter.bpm = Some(rbl_index::BpmFilter {
            ranges_x100: ranges,
            ..rbl_index::BpmFilter::default()
        });
    }
    if !selection.prep_colours.is_empty() {
        let mut colours = filter.colors.take().unwrap_or_default();
        for name in &selection.prep_colours {
            colours.push(colour_id(name)?);
        }
        filter.colors = Some(colours);
    }
    for pick in &selection.my_tag_picks {
        if let Some(tags) = tag_filter(index, pick)? {
            filter.tags.push(tags);
        }
    }
    Ok(())
}

/// The picks of `my_tag_picks` the Library no longer holds.
pub(crate) fn dropped(index: &rbl_index::Library, selection: &TrackSelection) -> Vec<String> {
    let mut out = Vec::new();
    let categories = index.my_tags();
    for pick in &selection.my_tag_picks {
        match categories.iter().find(|c| c.name == pick.category) {
            None => out.push(pick.category.clone()),
            Some(category) => out.extend(
                pick.tags
                    .iter()
                    .filter(|tag| !category.tags.contains(tag))
                    .map(|tag| format!("{}: {tag}", pick.category)),
            ),
        }
    }
    out
}

/// `N` or `low..high`, Energy 0 to 5; an open low end is 1, an open high 5.
fn energy_range(text: &str) -> Result<(u8, u8), ServiceError> {
    let refuse = || {
        ServiceError::new(
            codes::BAD_REQUEST,
            format!("{text:?} is not an Energy: give 0 to 5, or a range like 3..5"),
        )
    };
    let end = |part: &str, open: u8| -> Result<u8, ServiceError> {
        let part = part.trim();
        if part.is_empty() {
            return Ok(open);
        }
        part.parse::<u8>()
            .ok()
            .filter(|n| *n <= 5)
            .ok_or_else(refuse)
    };
    match text.trim().split_once("..") {
        Some((low, high)) => Ok((end(low, 1)?, end(high, 5)?)),
        None => {
            let one = end(text, 0)?;
            Ok((one, one))
        }
    }
}

/// `low..high` in BPM to the hundredth, as hundredths.
fn bpm_range(text: &str) -> Result<(u32, u32), ServiceError> {
    let refuse = || {
        ServiceError::new(
            codes::BAD_REQUEST,
            format!("{text:?} is not a tempo range: give one like 123.50..128.49"),
        )
    };
    let hundredths = |part: &str| -> Result<u32, ServiceError> {
        let bpm: f64 = part.trim().parse().map_err(|_| refuse())?;
        if !bpm.is_finite() || !(0.0..100_000.0).contains(&bpm) {
            return Err(refuse());
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "bounded just above"
        )]
        let value = (bpm * 100.0).round() as u32;
        Ok(value)
    };
    let (low, high) = text.split_once("..").ok_or_else(refuse)?;
    Ok((hundredths(low)?, hundredths(high)?))
}

/// A colour's `ColorID`, 1 to 8, or 0 for `none`.
fn colour_id(name: &str) -> Result<u8, ServiceError> {
    if name.eq_ignore_ascii_case("none") {
        return Ok(0);
    }
    COLOR_NAMES
        .iter()
        .position(|known| known.eq_ignore_ascii_case(name))
        .and_then(|at| u8::try_from(at + 1).ok())
        .ok_or_else(|| {
            ServiceError::new(
                codes::BAD_REQUEST,
                format!(
                    "{name:?} is not a Prep colour: it is one of {}, or none",
                    COLOR_NAMES.join(", ")
                ),
            )
        })
}

/// A pick's tag column, or `None` when nothing of it is left in the
/// Library (see [`dropped`]).
fn tag_filter(
    index: &rbl_index::Library,
    pick: &MyTagPick,
) -> Result<Option<TagFilter>, ServiceError> {
    if pick.tags.is_empty() {
        return Err(ServiceError::new(
            codes::BAD_REQUEST,
            format!(
                "the MyTag category {:?} is picked with no tags in it",
                pick.category
            ),
        ));
    }
    let Some(category) = index
        .my_tags()
        .iter()
        .position(|category| category.name == pick.category)
    else {
        return Ok(None);
    };
    let picks: Vec<Vec<i32>> = index
        .tag_picks_named(category, &pick.tags)
        .into_iter()
        .filter(|ids| !ids.is_empty())
        .collect();
    Ok((!picks.is_empty()).then_some(TagFilter {
        picks,
        any: pick.any,
    }))
}
