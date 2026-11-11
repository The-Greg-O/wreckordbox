//! How the Library's vocabulary and lists are read for showing: the My Tag
//! catalogue in rekordbox's order, and a playlist or history tree flattened
//! in the order the window's rail shows it.
//!
//! Nothing here knows about a window or a socket. The desktop shell turns
//! these into what its rail and its scripting bridge hand out, and the
//! Library service into what its clients are sent, so the two cannot
//! disagree about the order of a list or what counts as a folder.

use rbl_db::export_info::MyTagRow;
use rbl_index::{Playlists, NO_ID};

/// The My Tag categories and the tags under them, in rekordbox's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MyTagCatalogue<'a> {
    /// The categories, in their `Seq` order.
    pub categories: Vec<&'a MyTagRow>,
    /// The tags grouped by their category's place, each category's in `Seq`
    /// order; tags of no live category last.
    pub tags: Vec<&'a MyTagRow>,
}

impl<'a> MyTagCatalogue<'a> {
    /// The tags of one category, in their order.
    pub fn tags_of<'s>(&'s self, category: &'s MyTagRow) -> impl Iterator<Item = &'a MyTagRow> + 's {
        self.tags.iter().copied().filter(move |tag| tag.parent == category.id)
    }
}

/// Orders `rows`, as [`rbl_db::export_info::my_tags`] reads them (in `Seq`
/// order), into a catalogue.
///
/// Category identity is kept by id, so two categories or two tags may share a
/// name.
#[must_use]
pub fn my_tag_catalogue(rows: &[MyTagRow]) -> MyTagCatalogue<'_> {
    let categories: Vec<&MyTagRow> = rows.iter().filter(|row| row.attribute == 1).collect();
    // The rows come in `Seq` order and the sort is stable, so each
    // category's tags stay as rekordbox lists them.
    let mut tags: Vec<&MyTagRow> = rows.iter().filter(|row| row.attribute == 0).collect();
    tags.sort_by_key(|tag| categories.iter().position(|category| category.id == tag.parent).unwrap_or(usize::MAX));
    MyTagCatalogue { categories, tags }
}

/// One playlist, folder or history of a tree, where the rail shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListEntry {
    /// Its index into the [`Playlists`] it came from.
    pub index: usize,
    /// 1 at the top of the tree.
    pub depth: u32,
    /// A folder by its attribute, or by what is under it: a history year is
    /// a folder only in the second sense, an empty playlist folder only in
    /// the first.
    pub folder: bool,
    /// An intelligent playlist: a rule, not a membership. Never a folder.
    pub smart: bool,
    /// Its name as shown: a history's month folder by the month's name.
    pub name: String,
    /// How many lists sit directly under it.
    pub children: usize,
    /// How many tracks it lists itself. None for an intelligent playlist,
    /// whose tracks are whatever its rule admits when it is opened.
    pub members: usize,
}

/// A month folder's name as rekordbox shows it: `djmdHistory` stores the
/// month as its number.
#[must_use]
pub fn month_name(number: &str) -> Option<&'static str> {
    const MONTHS: [&str; 12] = [
        "January", "February", "March", "April", "May", "June",
        "July", "August", "September", "October", "November", "December",
    ];
    let month = number.parse::<usize>().ok()?;
    MONTHS.get(month.checked_sub(1)?).copied()
}

/// Flattens one list tree depth-first, in `Seq` order, or for a `calendar`
/// with the year and month folders in date order.
///
/// A calendar is filed by date rather than by hand: folders are a year and a
/// month, which rekordbox shows in calendar order under their month's name,
/// not in the order they were made. Sessions keep their `Seq`, which is the
/// order they were played in.
#[must_use]
pub fn list_tree(lists: &Playlists, calendar: bool) -> Vec<ListEntry> {
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); lists.len()];
    let mut roots: Vec<usize> = Vec::new();
    for index in 0..lists.len() {
        match lists.parent.get(index).copied() {
            Some(parent) if parent != NO_ID && (parent as usize) < lists.len() => {
                if let Some(bucket) = children.get_mut(parent as usize) {
                    bucket.push(index);
                }
            }
            _ => roots.push(index),
        }
    }
    if calendar {
        // A year is "2026" and a month "9": the number is the date. A folder
        // named anything else sorts after the dated ones, in `Seq` order.
        let by_date = |index: &usize| -> (u64, u32) {
            if lists.is_folder(*index) {
                (lists.name(*index).parse::<u64>().unwrap_or(u64::MAX), 0)
            } else {
                (u64::MAX, lists.seq.get(*index).copied().unwrap_or(u32::MAX))
            }
        };
        roots.sort_by_key(by_date);
        for bucket in &mut children {
            bucket.sort_by_key(by_date);
        }
    }

    // Iterative, with a visited set: a corrupt parent cycle must not recurse
    // forever or blow the stack.
    let mut entries = Vec::with_capacity(lists.len());
    let mut stack: Vec<(usize, u32)> = roots.iter().rev().map(|&i| (i, 1_u32)).collect();
    let mut visited = vec![false; lists.len()];
    while let Some((index, depth)) = stack.pop() {
        if visited.get(index).copied().unwrap_or(true) {
            continue;
        }
        if let Some(slot) = visited.get_mut(index) {
            *slot = true;
        }
        let under = children.get(index).map_or(0, Vec::len);
        let folder = lists.is_folder(index) || under > 0;
        let name = lists.name(index);
        // Depth 2 under the section's heading is the month, filed in a year.
        let name = match (calendar && folder && depth == 2, month_name(name)) {
            (true, Some(month)) => month.to_owned(),
            _ => name.to_owned(),
        };
        entries.push(ListEntry {
            index,
            depth,
            folder,
            smart: !folder && lists.is_smart(index),
            name,
            children: under,
            members: lists.members.get(index).map_or(0, Vec::len),
        });
        if let Some(below) = children.get(index) {
            for &child in below.iter().rev() {
                stack.push((child, depth + 1));
            }
        }
    }
    entries
}
