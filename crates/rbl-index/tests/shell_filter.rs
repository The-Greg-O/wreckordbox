//! What the view shell's Track Filter bar (wbx-57) adds to the track filter:
//! BPM as ranges in hundredths, keys by their musical identity whatever the
//! spelling, and the same filter over a Stick's own rows.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::path::PathBuf;

use rbl_index::device::{device_filter_values, DeviceFilter, DeviceRow, DeviceView};
use rbl_index::testing::{library_from, TestTrack};
use rbl_index::{BpmFilter, SearchField, SortColumn, TrackFilter, TrackSource, ViewSpec};

fn track(id: u64, bpm: u32, key: &'static str) -> TestTrack {
    TestTrack {
        id,
        title: "t",
        artist: "a",
        bpm_x100: bpm,
        key,
        ..TestTrack::default()
    }
}

fn ids(lib: &rbl_index::Library, filter: TrackFilter) -> Vec<u64> {
    lib.open_view(&ViewSpec {
        source: TrackSource::Collection,
        sort: SortColumn::TrackNo,
        descending: false,
        query: String::new(),
        filter,
    })
    .rows
    .iter()
    .map(|&r| lib.ids[r as usize])
    .collect()
}

fn ranges(ranges: &[(u32, u32)]) -> TrackFilter {
    TrackFilter {
        bpm: Some(BpmFilter {
            ranges_x100: ranges.to_vec(),
            ..BpmFilter::default()
        }),
        ..TrackFilter::default()
    }
}

#[test]
fn bpm_ranges_in_hundredths_are_inclusive_and_any_one_admits() {
    let lib = library_from(&[
        track(1, 12349, "Am"),
        track(2, 12350, "Am"),
        track(3, 12849, "Am"),
        track(4, 12850, "Am"),
        track(5, 6400, "Am"),
        track(6, 0, "Am"),
    ]);
    assert_eq!(ids(&lib, ranges(&[(12350, 12849)])), [2, 3]);
    assert_eq!(
        ids(&lib, ranges(&[(12350, 12849), (6000, 6500)])),
        [2, 3, 5]
    );
    // An empty range admits nothing: an intersection that came out empty.
    assert_eq!(ids(&lib, ranges(&[(1, 0)])), Vec::<u64>::new());
}

#[test]
fn a_key_identity_takes_every_spelling_of_that_key_and_only_it() {
    let lib = library_from(&[
        track(1, 0, "Am"),
        track(2, 0, "8A"),
        track(3, 0, "1m"),
        track(4, 0, "C"),
        track(5, 0, "Unknown"),
        track(6, 0, "unknown"),
    ]);
    let keys = |names: &[&str]| TrackFilter {
        keys: Some(
            lib.key_ids_identified(&names.iter().map(|n| n.to_string()).collect::<Vec<_>>()),
        ),
        ..TrackFilter::default()
    };
    assert_eq!(ids(&lib, keys(&["8A"])), [1, 2, 3]);
    assert_eq!(ids(&lib, keys(&["Am"])), [1, 2, 3]);
    assert_eq!(ids(&lib, keys(&["8B"])), [4]);
    // A name off the wheel is matched exactly as stored.
    assert_eq!(ids(&lib, keys(&["Unknown"])), [5]);
    assert_eq!(ids(&lib, keys(&["9A"])), Vec::<u64>::new());
}

fn row(id: u32, key: &str, bpm_x100: u32, rating: u8, color: u8) -> DeviceRow {
    DeviceRow {
        id,
        position: id,
        path: PathBuf::new(),
        title: format!("Track {id}"),
        artist: String::new(),
        album: String::new(),
        genre: String::new(),
        label: String::new(),
        key: key.to_owned(),
        comment: String::new(),
        date_added: String::new(),
        bpm_x100,
        duration_sec: 300,
        rating,
        color,
        analysed: true,
        has_artwork: false,
        cues: Vec::new(),
    }
}

fn stick() -> Vec<DeviceRow> {
    vec![
        row(1, "Am", 12350, 5, 5),
        row(2, "8A", 12849, 3, 0),
        row(3, "Em", 12850, 0, 2),
        row(4, "", 0, 4, 5),
    ]
}

fn device_ids(filter: &DeviceFilter, query: &str) -> Vec<u32> {
    let spec = ViewSpec {
        source: TrackSource::Collection,
        sort: SortColumn::TrackNo,
        descending: false,
        query: query.to_owned(),
        filter: TrackFilter::default(),
    };
    DeviceView::open_filtered(stick(), &spec, SearchField::All, filter)
        .rows
        .iter()
        .map(|r| r.id)
        .collect()
}

#[test]
fn a_stick_list_is_narrowed_by_its_own_fields() {
    assert_eq!(device_ids(&DeviceFilter::default(), ""), [1, 2, 3, 4]);
    let keys = DeviceFilter {
        keys: Some(vec!["8A".to_owned()]),
        ..DeviceFilter::default()
    };
    assert_eq!(device_ids(&keys, ""), [1, 2]);
    let energy = DeviceFilter {
        ratings: Some(vec![0, 3]),
        ..DeviceFilter::default()
    };
    assert_eq!(device_ids(&energy, ""), [2, 3]);
    let no_colour = DeviceFilter {
        colors: Some(vec![0]),
        ..DeviceFilter::default()
    };
    assert_eq!(device_ids(&no_colour, ""), [2]);
    let bpm = DeviceFilter {
        bpm: Some(BpmFilter {
            ranges_x100: vec![(12350, 12849)],
            ..BpmFilter::default()
        }),
        ..DeviceFilter::default()
    };
    assert_eq!(device_ids(&bpm, ""), [1, 2]);
    // Text and fields together.
    assert_eq!(device_ids(&keys, "track 2"), [2]);
}

#[test]
fn a_stick_lists_choices_from_its_own_rows() {
    let values = device_filter_values(&stick());
    let keys: Vec<(String, u32)> = values
        .keys
        .iter()
        .map(|c| (c.value.clone(), c.count))
        .collect();
    // Wheel order, each spelling as stored (8A's two, by name), then 9A; a
    // blank key is not offered.
    assert_eq!(
        keys,
        [
            ("8A".to_owned(), 1),
            ("Am".to_owned(), 1),
            ("Em".to_owned(), 1)
        ]
    );
    assert!(values.tags.is_empty());
}
