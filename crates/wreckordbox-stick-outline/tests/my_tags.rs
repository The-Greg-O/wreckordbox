//! A Stick's MyTags (wbx-45.12), read beside the protected reader through
//! the same read that cannot recover or write a Stick: the categories and
//! tags in rekordbox's order, and which tracks carry which, from
//! `exportExt.pdb` for the Device Library and `exportLibrary.db` for
//! OneLibrary. Temporary-folder Sticks, compared byte for byte.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::path::Path;

use rbl_export::device_library::Format;
use wreckordbox_stick_outline::testing::{
    assert_unchanged, snapshot, stick, tagged_stick, wal_pending, PDB,
};
use wreckordbox_stick_outline::{read, StickMyTags};

/// Categories as `(name, [tag names])`, in order.
fn names(tags: &StickMyTags) -> Vec<(String, Vec<String>)> {
    tags.categories
        .iter()
        .map(|c| {
            (
                c.name.clone(),
                c.tags.iter().map(|t| t.name.clone()).collect(),
            )
        })
        .collect()
}

/// Each track's tags as `title -> ["Category: Tag"]`, by title.
fn carried(root: &Path, format: Format, tags: &StickMyTags) -> Vec<(String, Vec<String>)> {
    let library = read(root, |stick| stick.library(format)).unwrap();
    let label = |id: u32| {
        tags.categories
            .iter()
            .find_map(|c| {
                c.tags
                    .iter()
                    .find(|t| t.id == id)
                    .map(|t| format!("{}: {}", c.name, t.name))
            })
            .unwrap_or_else(|| format!("unknown {id}"))
    };
    library
        .tracks
        .iter()
        .map(|t| {
            let mut held: Vec<String> = tags
                .tracks
                .get(&t.id)
                .into_iter()
                .flatten()
                .map(|&id| label(id))
                .collect();
            held.sort();
            (t.title.clone(), held)
        })
        .collect()
}

fn expected_carried() -> Vec<(String, Vec<String>)> {
    [
        ("Low Sun", vec!["Situation: Peak", "Vibe: Dark"]),
        ("Paper", vec!["Vibe: Dark", "Vibe: Uplifting"]),
        ("Kiln", vec!["Situation: Warm-up", "Vibe: Uplifting"]),
        ("Odd", vec!["Situation: Dark"]),
        ("Blank", vec![]),
        ("Elsewhere", vec!["Situation: Peak", "Vibe: Dark"]),
    ]
    .into_iter()
    .map(|(t, tags)| (t.to_owned(), tags.into_iter().map(str::to_owned).collect()))
    .collect()
}

fn expected_names() -> Vec<(String, Vec<String>)> {
    vec![
        ("Vibe".into(), vec!["Dark".into(), "Uplifting".into()]),
        (
            "Situation".into(),
            vec!["Peak".into(), "Warm-up".into(), "Dark".into()],
        ),
    ]
}

#[test]
fn both_libraries_carry_the_sticks_own_categories_tags_and_memberships() {
    let dest = tempfile::tempdir().unwrap();
    tagged_stick(dest.path());
    let before = snapshot(dest.path());
    for format in [Format::DeviceLibrary, Format::OneLibrary] {
        let tags = read(dest.path(), |stick| stick.my_tags(format)).unwrap();
        assert_eq!(
            names(&tags),
            expected_names(),
            "{format:?}: categories by Seq, tags by Seq"
        );
        assert_eq!(
            carried(dest.path(), format, &tags),
            expected_carried(),
            "{format:?}"
        );
    }
    assert_unchanged(dest.path(), &before, "reading a Stick's MyTags");
}

#[test]
fn a_wal_mode_one_library_is_read_without_touching_the_stick() {
    let dest = tempfile::tempdir().unwrap();
    tagged_stick(dest.path());
    wal_pending(dest.path());
    let before = snapshot(dest.path());
    let tags = read(dest.path(), |stick| stick.my_tags(Format::OneLibrary)).unwrap();
    assert_eq!(names(&tags), expected_names());
    assert_unchanged(dest.path(), &before, "reading a WAL-mode Stick's MyTags");
}

#[test]
fn a_stick_without_tags_has_none_and_a_broken_tag_file_is_an_error() {
    let dest = tempfile::tempdir().unwrap();
    stick(dest.path());
    let ext = dest.path().join(PDB).with_file_name("exportExt.pdb");
    std::fs::remove_file(&ext).ok();
    let before = snapshot(dest.path());
    for format in [Format::DeviceLibrary, Format::OneLibrary] {
        let tags = read(dest.path(), |stick| stick.my_tags(format)).unwrap();
        assert!(
            tags.categories.is_empty() && tags.tracks.is_empty(),
            "{format:?}"
        );
    }
    assert_unchanged(dest.path(), &before, "reading a Stick with no MyTags");

    std::fs::write(&ext, b"not a database").unwrap();
    let error = read(dest.path(), |stick| stick.my_tags(Format::DeviceLibrary)).unwrap_err();
    assert!(error.to_string().contains("exportExt.pdb"), "{error}");
}
