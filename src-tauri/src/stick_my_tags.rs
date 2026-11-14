//! A Stick's own MyTags in the view shell's Track Filter (wbx-45.12).
//!
//! `wreckordbox_stick_outline` reads them through the read that cannot
//! recover or write a Stick; this turns them into what the bar and the
//! Stick list work with: the categories the bar offers (by name, in the
//! Stick's order, so a pick's `category` is a position among them), each
//! pick resolved to the Stick's tag ids with the Library's own rule
//! (`rbl_index::tag_picks_in`), and the ids each row carries.

use rbl_index::{TagCategory, TagFilter};
use wreckordbox_stick_outline::StickMyTags;

use crate::dto::{TagCategoryDto, TagPickDto};

/// A tag id as the Library's filter holds one. A Stick id past `i32` (none
/// is in practice) is a tag no pick can reach.
fn wide(id: u32) -> i32 {
    i32::try_from(id).unwrap_or(i32::MIN)
}

/// The Stick's categories as the Library's are held.
#[must_use]
pub fn categories(tags: &StickMyTags) -> Vec<TagCategory> {
    tags.categories
        .iter()
        .map(|c| TagCategory {
            name: c.name.clone(),
            tags: c.tags.iter().map(|t| t.name.clone()).collect(),
            ids: c.tags.iter().map(|t| wide(t.id)).collect(),
        })
        .collect()
}

/// What the bar offers on a Stick list.
#[must_use]
pub fn choices(tags: &StickMyTags) -> Vec<TagCategoryDto> {
    tags.categories
        .iter()
        .map(|c| TagCategoryDto {
            name: c.name.clone(),
            tags: c.tags.iter().map(|t| t.name.clone()).collect(),
        })
        .collect()
}

/// The bar's picks against the Stick's categories: a column with nothing
/// picked is left out; a category past the Stick's, or a name its category
/// does not hold, admits nothing, as on the Library.
#[must_use]
pub fn filters(picks: &[TagPickDto], tags: &StickMyTags) -> Vec<TagFilter> {
    let held = categories(tags);
    picks
        .iter()
        .filter(|p| !p.tags.is_empty())
        .map(|p| TagFilter {
            picks: rbl_index::tag_picks_in(&held, p.category, &p.tags),
            any: p.any,
        })
        .collect()
}

/// The tag ids the track `id` of this library carries.
#[must_use]
pub fn carried(tags: Option<&StickMyTags>, id: u32) -> Vec<i32> {
    tags.and_then(|t| t.tracks.get(&id))
        .map(|held| held.iter().copied().map(wide).collect())
        .unwrap_or_default()
}
