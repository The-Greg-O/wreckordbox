//! A Stick's playlists read for the tree: never a write to the Stick.
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

use rbl_export::device_library::{self, Edit, Format};
use rbl_export::{export, SourcePlaylist, SourceTrack};
use wreckordbox_stick_outline::outline;

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
fn a_stick_with_an_unfinished_write_is_left_exactly_as_it_is() {
    let dest = tempfile::tempdir().unwrap();
    stick(dest.path());
    interrupted_publication(dest.path());
    let before = snapshot(dest.path());

    let found = outline(dest.path());

    assert_eq!(
        snapshot(dest.path()),
        before,
        "not a byte on the Stick changed, the journal included"
    );
    assert!(found.pending, "the unfinished write is reported");
    assert!(
        found.libraries.is_empty(),
        "a half-published Stick's playlists are not shown"
    );
}

/// The fixture above is a real interrupted publication: the device library's
/// own reader, which an explicit open of a Stick uses, settles it and so
/// changes the Stick. This is what the outline must not do.
#[test]
fn the_explicit_reader_would_settle_that_same_unfinished_write() {
    let dest = tempfile::tempdir().unwrap();
    stick(dest.path());
    interrupted_publication(dest.path());
    let before = snapshot(dest.path());

    let _ = device_library::read(dest.path(), Format::DeviceLibrary);

    assert_ne!(snapshot(dest.path()), before, "recovery rewrote the Stick");
    assert!(!rbl_export::recovery_pending(dest.path()));
}

#[test]
fn reading_a_stick_creates_no_lock_or_other_file() {
    let dest = tempfile::tempdir().unwrap();
    stick(dest.path());
    let before = snapshot(dest.path());
    let found = outline(dest.path());
    assert_eq!(snapshot(dest.path()), before);
    assert!(!found.pending);
}

#[test]
fn both_libraries_are_listed_with_their_playlists_in_tree_order() {
    let dest = tempfile::tempdir().unwrap();
    stick(dest.path());
    let found = outline(dest.path());
    let formats: Vec<_> = found.libraries.iter().map(|l| l.format).collect();
    assert_eq!(formats, ["deviceLibrary", "oneLibrary"]);
    for library in &found.libraries {
        let shape: Vec<_> = library
            .nodes
            .iter()
            .map(|n| (n.name.as_str(), n.folder, n.depth, n.count))
            .collect();
        assert_eq!(
            shape,
            [
                ("Sets", true, 0, 1),
                ("Friday", false, 1, 2),
                ("Warm Up", false, 0, 1)
            ],
            "{}",
            library.format
        );
    }
    // The same ids the device library's own reader gives, so a row opens the
    // same playlist through the existing view.
    let read = device_library::read(dest.path(), Format::DeviceLibrary).unwrap();
    let ids: Vec<_> = read.nodes.iter().map(|n| n.id).collect();
    assert_eq!(
        found.libraries[0]
            .nodes
            .iter()
            .map(|n| n.id)
            .collect::<Vec<_>>(),
        ids
    );
}

#[test]
fn a_playlist_made_in_one_library_shows_in_that_library_alone() {
    let dest = tempfile::tempdir().unwrap();
    stick(dest.path());
    device_library::apply(
        dest.path(),
        Format::OneLibrary,
        &Edit::Create {
            parent: 0,
            name: "Only Here".into(),
            folder: false,
        },
    )
    .unwrap();
    let found = outline(dest.path());
    let names = |i: usize| {
        found.libraries[i]
            .nodes
            .iter()
            .map(|n| n.name.clone())
            .collect::<Vec<_>>()
    };
    assert!(!names(0).contains(&"Only Here".to_string()));
    assert!(names(1).contains(&"Only Here".to_string()));
}

#[test]
fn a_volume_with_no_export_has_nothing_to_list() {
    let dest = tempfile::tempdir().unwrap();
    assert_eq!(outline(dest.path()), Default::default());
}

#[test]
fn the_outline_is_sent_in_the_device_tree_shape() {
    let dest = tempfile::tempdir().unwrap();
    stick(dest.path());
    let found = outline(dest.path());
    let json = serde_json::to_value(&found).unwrap();
    let (folder, friday) = (&found.libraries[0].nodes[0], &found.libraries[0].nodes[1]);
    let sent = &json["libraries"][0]["nodes"][1];
    assert_eq!(sent["id"], serde_json::json!(friday.id.to_string()));
    assert_eq!(sent["parentId"], serde_json::json!(folder.id.to_string()));
    assert_eq!(sent["name"], "Friday");
    assert_eq!(json["libraries"][0]["format"], "deviceLibrary");
    assert_eq!(json["pending"], false);
}

/// A Stick the DJ declared safe to overwrite carries the Spare stick marker
/// at its root; the window asks for a shorter hold before writing to it
/// (wbx-45.4 review, finding 5). Any other Stick is a Gig stick.
#[test]
fn the_spare_stick_marker_is_reported_without_writing() {
    let dest = tempfile::tempdir().unwrap();
    stick(dest.path());
    assert!(!outline(dest.path()).spare, "no marker: a Gig stick");

    std::fs::write(dest.path().join(".rbxport-disposable"), b"").unwrap();
    let before = snapshot(dest.path());
    let found = outline(dest.path());
    assert!(found.spare, "the marker is read");
    assert_eq!(snapshot(dest.path()), before);
    assert_eq!(serde_json::to_value(&found).unwrap()["spare"], true);

    interrupted_publication(dest.path());
    let pending = outline(dest.path());
    assert!(pending.pending && pending.spare, "an unfinished write does not hide the marker");
}
