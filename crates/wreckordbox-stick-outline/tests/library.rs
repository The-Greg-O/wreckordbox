//! A Stick library read in full for showing it: never a recovery.
//!
//! Every Stick here is written by the export pipeline into a temporary
//! directory; nothing touches a mounted Stick.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rbl_export::device_library::{self, Format};
use rbl_export::{export, SourcePlaylist, SourceTrack};
use wreckordbox_stick_outline::{library, ReadError};

fn track(dir: &Path, id: u64) -> SourceTrack {
    let path = dir.join(format!("source-{id}.mp3"));
    std::fs::write(&path, vec![id as u8; 2048]).unwrap();
    SourceTrack {
        id,
        source_path: path,
        title: format!("Track {id}"),
        artist: "TRIODE".into(),
        analysis: vec![(
            "DAT".into(),
            rbl_anlz::AnlzBuilder::new().path("/x.mp3").finish(),
        )],
        ..SourceTrack::default()
    }
}

/// A Stick with a folder holding one list, and a list at the top level.
fn stick(dest: &Path) {
    let src = tempfile::tempdir().unwrap();
    let tracks: Vec<SourceTrack> = (1..=4).map(|i| track(src.path(), i)).collect();
    let playlists = vec![
        SourcePlaylist {
            id: 10,
            name: "Sets".into(),
            folder: true,
            ..Default::default()
        },
        SourcePlaylist {
            id: 11,
            name: "Friday".into(),
            parent_id: 10,
            track_indices: vec![0, 1],
            ..Default::default()
        },
        SourcePlaylist {
            id: 12,
            name: "Warm Up".into(),
            track_indices: vec![2],
            ..Default::default()
        },
    ];
    export(dest, &tracks, &playlists).unwrap();
}

/// Every file under `root` and its bytes, the directories too.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            if path.is_dir() {
                out.insert(relative, None);
                stack.push(path);
            } else {
                out.insert(relative, Some(std::fs::read(&path).unwrap()));
            }
        }
    }
    out
}

/// What an export that stopped part way leaves: export.pdb already
/// replaced, the Stick's own copy set aside in the journal, and the new
/// database still an image there. Recovery would roll this back.
fn interrupted_publication(root: &Path) {
    let pdb = "PIONEER/rekordbox/export.pdb";
    let db = "PIONEER/rekordbox/exportLibrary.db";
    let before = std::fs::read(root.join(pdb)).unwrap();
    std::fs::write(root.join(pdb), b"the new export.pdb").unwrap();
    let journal = root.join(".rbxport-publication");
    let image = journal.join(db);
    std::fs::create_dir_all(image.parent().unwrap()).unwrap();
    std::fs::write(&image, b"the new database").unwrap();
    let aside = journal.join(".previous").join(pdb);
    std::fs::create_dir_all(aside.parent().unwrap()).unwrap();
    std::fs::write(&aside, before).unwrap();
    std::fs::write(
        journal.join("publication.json"),
        format!(
            r#"[{{"path":"{pdb}","present":true,"had_target":true,"exact":true}},{{"path":"{db}","present":true,"had_target":true,"exact":true}}]"#
        ),
    )
    .unwrap();
}

#[test]
fn reading_a_library_of_a_stick_with_an_unfinished_write_changes_nothing() {
    let dest = tempfile::tempdir().unwrap();
    stick(dest.path());
    interrupted_publication(dest.path());
    let before = snapshot(dest.path());

    for format in [Format::DeviceLibrary, Format::OneLibrary] {
        let read = library(dest.path(), format);
        assert!(
            matches!(read, Err(ReadError::UnfinishedWrite)),
            "{format:?}: the unfinished write is reported"
        );
    }

    assert_eq!(
        snapshot(dest.path()),
        before,
        "not a byte on the Stick changed, the journal included"
    );
}

#[test]
fn a_settled_stick_reads_as_the_device_library_reads_it() {
    let dest = tempfile::tempdir().unwrap();
    stick(dest.path());
    let before = snapshot(dest.path());
    for format in [Format::DeviceLibrary, Format::OneLibrary] {
        let ours = library(dest.path(), format).unwrap();
        assert_eq!(ours, device_library::read(dest.path(), format).unwrap());
    }
    assert_eq!(snapshot(dest.path()), before, "no lock or other file made");
}

#[test]
fn settling_is_the_explicit_action_that_does_recover() {
    let dest = tempfile::tempdir().unwrap();
    stick(dest.path());
    interrupted_publication(dest.path());
    let before = snapshot(dest.path());

    wreckordbox_stick_outline::settle(dest.path()).unwrap();

    assert_ne!(snapshot(dest.path()), before, "the Stick was settled");
    assert!(!rbl_export::recovery_pending(dest.path()));
    assert!(
        !matches!(
            library(dest.path(), Format::DeviceLibrary),
            Err(ReadError::UnfinishedWrite)
        ),
        "the Stick no longer reports an unfinished write"
    );
}
