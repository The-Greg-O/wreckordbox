//! A library on a USB stick as a track list: one of its playlists, or all
//! its tracks, as rekordbox's Devices tree opens them.
//!
//! The rows come from the stick's own database, not from this library: a
//! stick's tracks are copies with their own ids, and rekordbox shows what
//! the stick says about them. The caller reads the stick; this orders and
//! searches what it read the way a [`crate::View`] orders and searches the
//! collection, so the two lists behave alike under the same header click.

use std::path::PathBuf;

use crate::strings::fold;
use crate::{Cue, SearchField, SortColumn, ViewSpec};

/// One row: a track of the stick's library.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceRow {
    /// The track's id in the stick's library.
    pub id: u32,
    /// Its place in the playlist, from 1, or in the library for All Tracks.
    pub position: u32,
    /// Where the audio is: the stick's mount point joined with the
    /// library's volume-relative path.
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub label: String,
    pub key: String,
    pub comment: String,
    pub date_added: String,
    pub bpm_x100: u32,
    pub duration_sec: u32,
    /// Stars, 0 to 5.
    pub rating: u8,
    pub color: u8,
    /// Whether the stick's library names an analysis file for the track.
    pub analysed: bool,
    /// Whether the stick's library names artwork for the track.
    pub has_artwork: bool,
    /// Cues read from this device library when the view was opened.
    pub cues: Vec<Cue>,
}

/// The rows a view shows, ordered and searched.
#[derive(Debug, Default)]
pub struct DeviceView {
    pub rows: Vec<DeviceRow>,
}

impl DeviceView {
    /// Orders and searches `rows`, given in the list's own order.
    ///
    /// Columns the stick's library does not carry keep that order, as a
    /// playlist does with no sort.
    #[must_use]
    pub fn open(mut rows: Vec<DeviceRow>, spec: &ViewSpec, field: SearchField) -> Self {
        let query = fold(spec.query.trim());
        if !query.is_empty() {
            rows.retain(|row| matches(row, &query, field));
        }
        match spec.sort {
            SortColumn::Bpm => rows.sort_by_key(|r| r.bpm_x100),
            SortColumn::Duration => rows.sort_by_key(|r| r.duration_sec),
            SortColumn::Rating => rows.sort_by_key(|r| r.rating),
            SortColumn::Color => rows.sort_by_key(|r| r.color),
            SortColumn::KeyCamelot => rows.sort_by_key(|r| crate::key::camelot_rank(&r.key)),
            SortColumn::Key => rows.sort_by(|a, b| crate::key::cmp_names(&a.key, &b.key)),
            SortColumn::DateAdded => rows.sort_by(|a, b| a.date_added.cmp(&b.date_added)),
            column => {
                if let Some(text) = text_of(column) {
                    let mut keyed: Vec<(String, DeviceRow)> = rows
                        .into_iter()
                        .map(|r| (folded(column, text(&r)), r))
                        .collect();
                    keyed.sort_by(|a, b| a.0.cmp(&b.0));
                    rows = keyed.into_iter().map(|(_, r)| r).collect();
                }
            }
        }
        if spec.descending {
            rows.reverse();
        }
        Self { rows }
    }

    /// [`open`](Self::open), narrowed first by the view shell's Track Filter
    /// over the Stick's own fields (wbx-57).
    #[must_use]
    pub fn open_filtered(
        mut rows: Vec<DeviceRow>,
        spec: &ViewSpec,
        field: SearchField,
        filter: &DeviceFilter,
    ) -> Self {
        if !filter.is_empty() {
            rows.retain(|row| filter.admits(row));
        }
        Self::open(rows, spec, field)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// A window of rows, clamped to what exists.
    #[must_use]
    pub fn window(&self, offset: usize, len: usize) -> &[DeviceRow] {
        let start = offset.min(self.rows.len());
        let end = start.saturating_add(len).min(self.rows.len());
        self.rows.get(start..end).unwrap_or(&[])
    }
}

type Text = fn(&DeviceRow) -> &str;

fn text_of(column: SortColumn) -> Option<Text> {
    Some(match column {
        SortColumn::Title => |r| &r.title,
        SortColumn::Artist => |r| &r.artist,
        SortColumn::Album => |r| &r.album,
        SortColumn::Genre => |r| &r.genre,
        SortColumn::Label => |r| &r.label,
        SortColumn::Comment => |r| &r.comment,
        SortColumn::FileName => |r| r.path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
        SortColumn::Location => |r| r.path.to_str().unwrap_or(""),
        _ => return None,
    })
}

/// The same folds the collection's ranks use.
fn folded(column: SortColumn, text: &str) -> String {
    match column {
        SortColumn::FileName | SortColumn::Location => crate::strings::fold_smart(text),
        _ => fold(text),
    }
}

/// Every token of the query in the field searched, or in any of them.
fn matches(row: &DeviceRow, query: &str, field: SearchField) -> bool {
    let fields: &[&str] = match field {
        SearchField::Title => &[&row.title],
        SearchField::Artist => &[&row.artist],
        SearchField::Album => &[&row.album],
        SearchField::Genre => &[&row.genre],
        SearchField::Label => &[&row.label],
        SearchField::Comment => &[&row.comment],
        SearchField::All => &[
            &row.title,
            &row.artist,
            &row.album,
            &row.genre,
            &row.label,
            &row.comment,
            &row.key,
        ],
        _ => &[],
    };
    let folded: Vec<String> = fields.iter().map(|f| fold(f)).collect();
    query
        .split_whitespace()
        .all(|token| folded.iter().any(|f| f.contains(token)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn row(id: u32, title: &str, artist: &str, bpm: u32, key: &str) -> DeviceRow {
        DeviceRow {
            id,
            position: id,
            title: title.into(),
            artist: artist.into(),
            bpm_x100: bpm,
            key: key.into(),
            ..DeviceRow::default()
        }
    }

    fn rows() -> Vec<DeviceRow> {
        vec![
            row(1, "Zebra", "ARTBAT", 12_800, "Am"),
            row(2, "Ápple", "Meduza", 13_000, "C"),
            row(3, "Mango", "Tujamo", 12_400, "Am"),
        ]
    }

    fn spec(sort: SortColumn, descending: bool, query: &str) -> ViewSpec {
        ViewSpec {
            source: crate::TrackSource::Collection,
            sort,
            descending,
            query: query.into(),
            filter: crate::TrackFilter::default(),
        }
    }

    fn ids(view: &DeviceView) -> Vec<u32> {
        view.rows.iter().map(|r| r.id).collect()
    }

    #[test]
    fn no_sort_keeps_the_lists_own_order() {
        assert_eq!(
            ids(&DeviceView::open(
                rows(),
                &spec(SortColumn::TrackNo, false, ""),
                SearchField::All
            )),
            vec![1, 2, 3]
        );
        assert_eq!(
            ids(&DeviceView::open(
                rows(),
                &spec(SortColumn::TrackNo, true, ""),
                SearchField::All
            )),
            vec![3, 2, 1]
        );
    }

    #[test]
    fn text_sorts_fold_accents_and_case_the_way_the_collection_does() {
        assert_eq!(
            ids(&DeviceView::open(
                rows(),
                &spec(SortColumn::Title, false, ""),
                SearchField::All
            )),
            vec![2, 3, 1]
        );
        assert_eq!(
            ids(&DeviceView::open(
                rows(),
                &spec(SortColumn::Bpm, true, ""),
                SearchField::All
            )),
            vec![2, 1, 3]
        );
    }

    #[test]
    fn a_search_keeps_rows_with_every_token() {
        assert_eq!(
            ids(&DeviceView::open(
                rows(),
                &spec(SortColumn::TrackNo, false, "apple meduza"),
                SearchField::All
            )),
            vec![2]
        );
        assert_eq!(
            ids(&DeviceView::open(
                rows(),
                &spec(SortColumn::TrackNo, false, "am"),
                SearchField::All
            )),
            vec![1, 3]
        );
        assert!(DeviceView::open(
            rows(),
            &spec(SortColumn::TrackNo, false, "zebra"),
            SearchField::Artist
        )
        .is_empty());
    }
}

/// The view shell's Track Filter over a Stick's rows (wbx-57): the fields a
/// Stick's library carries. Every column given must agree; `None` is no
/// constraint. MyTags are not here: the Stick's library as read carries none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceFilter {
    /// Key identities, as [`crate::Library::key_ids_identified`] reads them.
    pub keys: Option<Vec<String>>,
    /// Stars, 0 to 5; 0 is a track with no Energy.
    pub ratings: Option<Vec<u8>>,
    /// Colour ids, 1 to 8; 0 is a track with no Prep colour.
    pub colors: Option<Vec<u8>>,
    pub bpm: Option<crate::BpmFilter>,
}

impl DeviceFilter {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_none() && self.ratings.is_none() && self.colors.is_none() && self.bpm.is_none()
    }

    /// Whether a row passes every column given.
    #[must_use]
    pub fn admits(&self, row: &DeviceRow) -> bool {
        if let Some(keys) = &self.keys {
            if !keys
                .iter()
                .any(|wanted| crate::key::same_key(&row.key, wanted))
            {
                return false;
            }
        }
        if let Some(ratings) = &self.ratings {
            if !ratings.contains(&row.rating.min(5)) {
                return false;
            }
        }
        if let Some(colors) = &self.colors {
            if !colors.contains(&row.color) {
                return false;
            }
        }
        if let Some(bpm) = &self.bpm {
            if !bpm.admits(row.bpm_x100) {
                return false;
            }
        }
        true
    }
}

/// What a Stick list offers the bar: its keys as stored, in wheel order, and
/// its whole BPMs, from its own rows. No MyTag categories: the Stick's
/// library as read carries none.
#[must_use]
pub fn device_filter_values(rows: &[DeviceRow]) -> crate::FilterValues {
    use std::collections::HashMap;
    let mut keys: HashMap<&str, u32> = HashMap::new();
    let mut bpms: HashMap<u32, u32> = HashMap::new();
    for row in rows {
        if !row.key.trim().is_empty() {
            *keys.entry(row.key.as_str()).or_insert(0) += 1;
        }
        let whole = crate::filter::whole_bpm(row.bpm_x100);
        if whole > 0 {
            *bpms.entry(whole).or_insert(0) += 1;
        }
    }
    let mut keys: Vec<crate::Counted<String>> = keys
        .into_iter()
        .map(|(value, count)| crate::Counted {
            value: value.to_owned(),
            count,
        })
        .collect();
    keys.sort_by_cached_key(|c| (crate::key::camelot_rank(&c.value), fold(&c.value)));
    let mut bpms: Vec<crate::Counted<u32>> = bpms
        .into_iter()
        .map(|(value, count)| crate::Counted { value, count })
        .collect();
    bpms.sort_unstable_by_key(|c| c.value);
    crate::FilterValues {
        bpms,
        keys,
        tags: Vec::new(),
    }
}
