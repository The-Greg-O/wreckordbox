//! Save the Track Filter bar's filter as an intelligent playlist (wbx-58).
//!
//! The bar sends its filter in the Library service's terms (the
//! `tracks.query` fields `toServiceSelection` makes, see
//! `crates/wreckordbox-service/src/track_filter.rs`), plus what the service
//! form loses: the open list when it is not All Tracks (`within`) and
//! whether Key match or BPM ±6% is on (`follows`). [`rule_for`] turns it
//! into an intelligent playlist's rule that admits exactly the tracks the
//! filter shows on All Tracks, resolved against the Library as the filter
//! resolves it, or refuses it with a reason the DJ can act on:
//!
//! - **One logic for the whole rule.** An intelligent playlist joins all its
//!   conditions with AND ("all of the following") or all with OR ("any
//!   of"); it does not nest groups (`rbl_index::smart` reads only a flat
//!   rule, as rekordbox writes one). The filter is AND across fields and OR
//!   within one (several keys, several Prep colours, Energy 5 or unrated,
//!   a category's Any). So a field with more than one choice is saved only
//!   when it is the whole filter (an "any of" rule); together with anything
//!   else it is refused, naming the field.
//! - **Keys are identities.** A picked key takes every spelling the Library
//!   holds (`8A`, `Am`, Open Key `1m`), each an `equal` condition on the
//!   name, as the rule compares keys by name. A key spelled more than one
//!   way is therefore several choices.
//! - **Energy** is the rating, saved as one `equal` or `in range` when its
//!   levels run on (unrated is 0, so "unrated or 1 to 3" is `0..3`).
//! - **Prep colour** is `grouping` by colour id, 0 for none.
//! - **BPM** is the bar's span in hundredths, `in range`, both ends
//!   inclusive, exactly as the filter sends it.
//! - **MyTags** are `myTag contains` by the tag's id, one condition per tag
//!   (All) or an "any of" (Any).
//! - **Refused outright:** text (an intelligent playlist has no "every word
//!   somewhere" condition), Key match and BPM ±6% (they follow the loaded
//!   track), a filter on any list but All Tracks (an intelligent playlist
//!   chooses from the whole Library), a MyTag the Library no longer holds,
//!   an empty filter, an empty name.
//!
//! The save itself is the window's Create Intelligent Playlist edit, at the
//! top level of the tree: the one recorded edit path (gated, refused under
//! Library Protection, told to the Ledger as the DJ's), one Undo step
//! labelled "Save Filter as Intelligent Playlist".
#![allow(
    clippy::doc_markdown,
    reason = "MyTag is the glossary's word for the thing, not a code item"
)]

use std::sync::{Arc, Mutex};

use rbl_index::smart::{Condition, Group, Item, Logic, Operator, Property};
use rbl_index::{Library, SmartRule, COLOR_NAMES};
use tauri::State;
use wreckordbox_service::track_filter::MyTagPick;

use crate::commands::{recorded_edit_with, Touched};
use crate::state::{AppState, LibraryEdit};
use crate::{AppError, AppResult, ErrorKind};

/// The Undo step's and the Ledger entry's label.
pub const LABEL: &str = "Save Filter as Intelligent Playlist";

/// The filter as the bar sends it: `tracks.query`'s Track Filter fields,
/// the text, the open list and the fields that follow the loaded track.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SaveFilterDto {
    /// The filter's text, as the list's search box holds it.
    pub search: String,
    /// The open list's name when it is not All Tracks.
    pub within: Option<String>,
    /// `"key"` for Key match, `"bpm"` for BPM ±6%, when switched on.
    pub follows: Vec<String>,
    /// Key identities (`8A`, or a name off the wheel).
    pub keys: Vec<String>,
    /// Colour names, `none` for no Prep colour.
    pub prep_colours: Vec<String>,
    /// `N` or `low..high`, 0 for unrated.
    pub energies: Vec<String>,
    /// `low..high` in BPM to the hundredth, inclusive.
    pub bpm_ranges: Vec<String>,
    pub my_tag_picks: Vec<MyTagPick>,
}

/// The new playlist and the Library's generation after the save.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedFilterPlaylistDto {
    pub id: String,
    pub generation: u32,
}

/// Saves the filter as an intelligent playlist named `name`, at the top level of
/// the tree; refused, with nothing written, when an intelligent playlist
/// cannot hold it.
#[tauri::command]
pub async fn save_filter_playlist<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, Arc<AppState>>,
    name: String,
    filter: SaveFilterDto,
) -> AppResult<SavedFilterPlaylistDto> {
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err(refused("The intelligent playlist needs a name."));
    }
    let library = state.library()?;
    let rule = rule_for(&library, &filter).map_err(|reason| refused(&reason))?;
    drop(library);
    let made: Arc<Mutex<Option<String>>> = Arc::default();
    let kept = Arc::clone(&made);
    let history = recorded_edit_with(
        app,
        state,
        "save_filter_playlist",
        Touched::Playlists,
        move |w| {
            w.create_smart_playlist_with_undo(&name, rbl_db::write::ROOT, |id| {
                rule.to_xml(id.parse().unwrap_or(0))
            })
        },
        move |history, (id, creation)| {
            if let Ok(mut slot) = kept.lock() {
                *slot = Some(id);
            }
            history.record(LibraryEdit::CreatePlaylist(creation), LABEL);
        },
    )
    .await?;
    let id = made
        .lock()
        .ok()
        .and_then(|mut slot| slot.take())
        .unwrap_or_default();
    Ok(SavedFilterPlaylistDto {
        id,
        generation: history.generation,
    })
}

fn refused(reason: &str) -> AppError {
    AppError::new(ErrorKind::Malformed, reason.to_owned())
}

/// One field of the filter as rule conditions: the alternatives any one of
/// which admits a track. `what` names it when it has more than one.
struct Clause {
    what: String,
    choices: Vec<Condition>,
}

fn condition(property: Property, operator: Operator, left: String, right: &str) -> Condition {
    Condition {
        property,
        operator,
        left,
        right: right.to_owned(),
        unit: String::new(),
    }
}

/// The rule that admits exactly what `filter` shows on All Tracks, or why
/// there is none.
///
/// # Errors
/// A sentence for the DJ saying what an intelligent playlist cannot hold.
pub fn rule_for(library: &Library, filter: &SaveFilterDto) -> Result<SmartRule, String> {
    let text = filter.search.trim();
    if !text.is_empty() {
        return Err(format!(
            "An intelligent playlist cannot search for text, so it cannot keep \"{text}\". Clear the text to save this filter."
        ));
    }
    if let Some(list) = &filter.within {
        return Err(format!(
            "An intelligent playlist chooses from the whole Library, so it cannot keep this filter to \"{list}\". Open All Tracks to save it."
        ));
    }
    let following: Vec<&str> = ["key", "bpm"]
        .into_iter()
        .filter(|field| filter.follows.iter().any(|f| f == field))
        .map(|field| {
            if field == "key" {
                "Key match"
            } else {
                "BPM ±6%"
            }
        })
        .collect();
    if !following.is_empty() {
        let (names, verb) = if following.len() == 1 {
            (following[0].to_owned(), "follows")
        } else {
            (following.join(" and "), "follow")
        };
        return Err(format!(
            "{names} {verb} the loaded track, which an intelligent playlist cannot. Turn it off, or pick keys and a BPM range, to save this filter."
        ));
    }

    let mut clauses = Vec::new();
    if let Some(clause) = keys(library, &filter.keys) {
        clauses.push(clause);
    }
    if let Some(clause) = colours(&filter.prep_colours)? {
        clauses.push(clause);
    }
    if let Some(clause) = energy(&filter.energies)? {
        clauses.push(clause);
    }
    for text in &filter.bpm_ranges {
        let (low, high) = bpm_range(text)?;
        clauses.push(Clause {
            what: format!("BPM {text}"),
            choices: vec![condition(
                Property::Bpm,
                Operator::InRange,
                low.to_string(),
                &high.to_string(),
            )],
        });
    }
    for pick in &filter.my_tag_picks {
        clauses.extend(tags(library, pick)?);
    }

    if clauses.is_empty() {
        return Err(
            "Nothing is picked in the Track Filter, so there is nothing to save.".to_owned(),
        );
    }
    let several: Vec<&Clause> = clauses.iter().filter(|c| c.choices.len() > 1).collect();
    let logic = match (several.len(), clauses.len()) {
        (0, _) => Logic::All,
        (1, 1) => Logic::Any,
        _ => {
            let named: Vec<&str> = several.iter().map(|c| c.what.as_str()).collect();
            let named = join_and(&named);
            let rest = if several.len() == clauses.len() {
                "together"
            } else {
                "together with the rest of this filter"
            };
            return Err(format!(
                "An intelligent playlist joins all its conditions with AND or all with OR, so it cannot hold {named} {rest}. Keep one choice in each field, or save that field alone."
            ));
        }
    };
    let items = clauses
        .into_iter()
        .flat_map(|c| c.choices)
        .map(Item::Condition)
        .collect();
    Ok(SmartRule {
        root: Group { logic, items },
    })
}

fn join_and(parts: &[&str]) -> String {
    match parts {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// Every spelling the Library holds of the picked keys, each a choice.
fn keys(library: &Library, identities: &[String]) -> Option<Clause> {
    if identities.is_empty() {
        return None;
    }
    let mut spellings: Vec<String> = Vec::new();
    let mut per_key: Vec<(String, Vec<String>)> = Vec::new();
    for identity in identities {
        let mut names: Vec<String> = Vec::new();
        for id in library.key_ids_identified(std::slice::from_ref(identity)) {
            let name = library.keys.name(id).trim().to_owned();
            if !name.is_empty() && !names.contains(&name) {
                names.push(name);
            }
        }
        // No track can be in a key the Library does not hold: the rule
        // names it as picked, and admits none, as the filter does.
        if names.is_empty() {
            names.push(identity.clone());
        }
        for name in &names {
            if !spellings.contains(name) {
                spellings.push(name.clone());
            }
        }
        per_key.push((identity.clone(), names));
    }
    let what = if let [(identity, names)] = per_key.as_slice() {
        format!(
            "the key {identity}, spelled {} ways in the Library ({})",
            names.len(),
            names.join(", ")
        )
    } else {
        format!("{} keys ({})", identities.len(), identities.join(", "))
    };
    Some(Clause {
        what,
        choices: spellings
            .into_iter()
            .map(|name| condition(Property::Key, Operator::Equal, name, ""))
            .collect(),
    })
}

fn colours(names: &[String]) -> Result<Option<Clause>, String> {
    if names.is_empty() {
        return Ok(None);
    }
    let mut choices = Vec::new();
    let mut shown = Vec::new();
    for name in names {
        let id = if name.eq_ignore_ascii_case("none") {
            shown.push("None".to_owned());
            0
        } else {
            let at = COLOR_NAMES
                .iter()
                .position(|known| known.eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("{name:?} is not a Prep colour."))?;
            shown.push(COLOR_NAMES[at].to_owned());
            at + 1
        };
        choices.push(condition(
            Property::Color,
            Operator::Equal,
            id.to_string(),
            "",
        ));
    }
    Ok(Some(Clause {
        what: format!("{} Prep colours ({})", names.len(), shown.join(", ")),
        choices,
    }))
}

/// The picked Energy levels as the fewest rating conditions: one per run
/// of levels that follow on.
fn energy(texts: &[String]) -> Result<Option<Clause>, String> {
    if texts.is_empty() {
        return Ok(None);
    }
    let mut levels = [false; 6];
    for text in texts {
        let (low, high) = energy_range(text)?;
        for level in low..=high {
            levels[usize::from(level)] = true;
        }
    }
    let mut runs: Vec<(u8, u8)> = Vec::new();
    for level in 0..=5_u8 {
        if !levels[usize::from(level)] {
            continue;
        }
        match runs.last_mut() {
            Some((_, high)) if *high + 1 == level => *high = level,
            _ => runs.push((level, level)),
        }
    }
    let said: Vec<String> = runs
        .iter()
        .map(|&(low, high)| match (low, high) {
            (0, 0) => "unrated".to_owned(),
            (0, high) => format!("unrated to {high}"),
            (low, high) if low == high => low.to_string(),
            (low, high) => format!("{low} to {high}"),
        })
        .collect();
    // Unrated last, as the bar says it.
    let mut said = said;
    if said.first().is_some_and(|s| s == "unrated") && said.len() > 1 {
        said.rotate_left(1);
    }
    let choices = runs
        .into_iter()
        .map(|(low, high)| {
            if low == high {
                condition(Property::Rating, Operator::Equal, low.to_string(), "0")
            } else {
                condition(
                    Property::Rating,
                    Operator::InRange,
                    low.to_string(),
                    &high.to_string(),
                )
            }
        })
        .collect();
    Ok(Some(Clause {
        what: format!("Energy {}", said.join(" or ")),
        choices,
    }))
}

/// A category's picks: one clause per tag (All), or one of them all (Any).
/// A tag name the category holds twice is either of its ids.
fn tags(library: &Library, pick: &MyTagPick) -> Result<Vec<Clause>, String> {
    let categories = library.my_tags();
    let Some(at) = categories.iter().position(|c| c.name == pick.category) else {
        return Err(format!(
            "The MyTag category {} is no longer in the Library, so this filter cannot be saved.",
            pick.category
        ));
    };
    let ids = library.tag_picks_named(at, &pick.tags);
    let mut per_tag = Vec::new();
    for (tag, ids) in pick.tags.iter().zip(ids) {
        if ids.is_empty() {
            return Err(format!(
                "{}: {tag} is no longer in the Library, so this filter cannot be saved.",
                pick.category
            ));
        }
        let choices: Vec<Condition> = ids
            .iter()
            .map(|id| condition(Property::MyTag, Operator::Contains, id.to_string(), ""))
            .collect();
        per_tag.push((tag, choices));
    }
    if pick.any && per_tag.len() > 1 {
        let names: Vec<&str> = per_tag.iter().map(|(tag, _)| tag.as_str()).collect();
        return Ok(vec![Clause {
            what: format!("{}: any of {}", pick.category, names.join(", ")),
            choices: per_tag.into_iter().flat_map(|(_, c)| c).collect(),
        }]);
    }
    Ok(per_tag
        .into_iter()
        .map(|(tag, choices)| Clause {
            what: format!(
                "{}: {tag}, which the Library holds {} times",
                pick.category,
                choices.len()
            ),
            choices,
        })
        .collect())
}

/// `N` or `low..high`, Energy 0 to 5; an open low end is 1, an open high 5
/// (as `tracks.query` reads it).
fn energy_range(text: &str) -> Result<(u8, u8), String> {
    let refuse = || format!("{text:?} is not an Energy.");
    let end = |part: &str, open: u8| -> Result<u8, String> {
        let part = part.trim();
        if part.is_empty() {
            return Ok(open);
        }
        part.parse::<u8>()
            .ok()
            .filter(|n| *n <= 5)
            .ok_or_else(refuse)
    };
    if let Some((low, high)) = text.trim().split_once("..") {
        let (low, high) = (end(low, 1)?, end(high, 5)?);
        Ok((low.min(high), low.max(high)))
    } else {
        let one = end(text, 0)?;
        Ok((one, one))
    }
}

/// `low..high` in BPM to the hundredth, as hundredths. A span that admits
/// nothing (low above high) has no rule.
fn bpm_range(text: &str) -> Result<(u32, u32), String> {
    let refuse = || format!("{text:?} is not a tempo range.");
    let hundredths = |part: &str| -> Result<u32, String> {
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
    let (low, high) = (hundredths(low)?, hundredths(high)?);
    if low > high {
        return Err("The BPM range admits no tempo, so there is nothing to save.".to_owned());
    }
    Ok((low, high))
}
