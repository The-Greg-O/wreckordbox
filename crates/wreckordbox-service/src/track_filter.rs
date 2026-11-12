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
//! - `my_tag_picks`: one entry per category, `{"category", "tags", "any"}`,
//!   the tags resolved within that category only (`tag_picks_named`). Within
//!   a pick, `any` is OR and otherwise AND; every pick must agree.
#![allow(
    clippy::doc_markdown,
    reason = "MyTag is the glossary's word for the thing, not a code item"
)]

use rbl_index::{TagFilter, TrackFilter, COLOR_NAMES};
use serde::{Deserialize, Serialize};

use crate::protocol::{codes, ServiceError};
use crate::tracks::{TrackSelection, MYTAG_NOT_FOUND};

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
        let names: Vec<String> = selection
            .key
            .iter()
            .chain(&selection.keys)
            .cloned()
            .collect();
        filter.keys = Some(index.key_ids_named(&names));
    }
    if !selection.prep_colours.is_empty() {
        let mut colours = filter.colors.take().unwrap_or_default();
        for name in &selection.prep_colours {
            colours.push(colour_id(name)?);
        }
        filter.colors = Some(colours);
    }
    for pick in &selection.my_tag_picks {
        filter.tags.push(tag_filter(index, pick)?);
    }
    Ok(())
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

fn tag_filter(index: &rbl_index::Library, pick: &MyTagPick) -> Result<TagFilter, ServiceError> {
    if pick.tags.is_empty() {
        return Err(ServiceError::new(
            codes::BAD_REQUEST,
            format!(
                "the MyTag category {:?} is picked with no tags in it",
                pick.category
            ),
        ));
    }
    let category = index
        .my_tags()
        .iter()
        .position(|category| category.name == pick.category)
        .ok_or_else(|| {
            ServiceError::new(
                MYTAG_NOT_FOUND,
                format!(
                    "there is no MyTag category {:?} in the Library",
                    pick.category
                ),
            )
        })?;
    let picks = index.tag_picks_named(category, &pick.tags);
    if let Some((missing, _)) = pick.tags.iter().zip(&picks).find(|(_, ids)| ids.is_empty()) {
        return Err(ServiceError::new(
            MYTAG_NOT_FOUND,
            format!(
                "there is no MyTag {missing:?} in the category {:?}",
                pick.category
            ),
        ));
    }
    Ok(TagFilter {
        picks,
        any: pick.any,
    })
}
