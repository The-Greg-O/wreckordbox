//! Sticks in temporary folders for tests, here and in the app (feature
//! `testing`). Every Stick is written by the export pipeline into a folder
//! the test owns; nothing here touches a mounted Stick.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::missing_panics_doc)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rbl_export::{export, SourcePlaylist, SourceTrack};

/// The publication journal's folder at a Stick's root.
pub const JOURNAL: &str = ".rbxport-publication";
/// Our writers' lock file at a Stick's root.
pub const LOCK: &str = ".rbxport-write.lock";
/// Where both databases sit.
pub const PDB: &str = "PIONEER/rekordbox/export.pdb";
pub const ONE: &str = "PIONEER/rekordbox/exportLibrary.db";

fn track(dir: &Path, id: u64) -> SourceTrack {
    let path = dir.join(format!("source-{id}.mp3"));
    std::fs::write(&path, vec![u8::try_from(id).unwrap_or(0); 2048]).unwrap();
    SourceTrack {
        id,
        source_path: path,
        title: format!("Track {id}"),
        artist: "TRIODE".into(),
        analysis: vec![("DAT".into(), rbl_anlz::AnlzBuilder::new().path("/x.mp3").finish())],
        ..SourceTrack::default()
    }
}

/// Exports to `dest` a Stick with a folder "Sets" holding "Friday" (tracks
/// 1, 2) and a top-level "Warm Up" (track 3), both libraries.
pub fn stick(dest: &Path) {
    let src = tempfile::tempdir().unwrap();
    let tracks: Vec<SourceTrack> = (1..=4).map(|i| track(src.path(), i)).collect();
    let playlists = vec![
        SourcePlaylist { id: 10, name: "Sets".into(), folder: true, ..Default::default() },
        SourcePlaylist { id: 11, name: "Friday".into(), parent_id: 10, track_indices: vec![0, 1], ..Default::default() },
        SourcePlaylist { id: 12, name: "Warm Up".into(), track_indices: vec![2], ..Default::default() },
    ];
    export(dest, &tracks, &playlists).unwrap();
}

/// The playlists [`interrupt`]'s publication brings, in tree order.
pub const NEXT_PLAYLISTS: [&str; 2] = ["Saturday", "Closing"];

/// What an interrupted publication will publish once settled.
pub struct Interrupted {
    pub pdb: Vec<u8>,
    pub one: Vec<u8>,
}

/// Leaves at `root` the journal of a publication that stopped right after it
/// was committed and before any file was published: the two new databases
/// (a whole other export, playlists [`NEXT_PLAYLISTS`]) are images in the
/// journal, the Stick's own files untouched. Settling it publishes them.
/// The Stick's lock file is left as it was.
pub fn interrupt(root: &Path) -> Interrupted {
    let other = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let tracks: Vec<SourceTrack> = (5..=6).map(|i| track(src.path(), i)).collect();
    let playlists = vec![
        SourcePlaylist { id: 20, name: NEXT_PLAYLISTS[0].into(), track_indices: vec![0], ..Default::default() },
        SourcePlaylist { id: 21, name: NEXT_PLAYLISTS[1].into(), track_indices: vec![1], ..Default::default() },
    ];
    export(other.path(), &tracks, &playlists).unwrap();
    let next = Interrupted {
        pdb: std::fs::read(other.path().join(PDB)).unwrap(),
        one: std::fs::read(other.path().join(ONE)).unwrap(),
    };
    let journal = root.join(JOURNAL);
    for (path, bytes) in [(PDB, &next.pdb), (ONE, &next.one)] {
        let image = journal.join(path);
        std::fs::create_dir_all(image.parent().unwrap()).unwrap();
        std::fs::write(&image, bytes).unwrap();
    }
    std::fs::write(
        journal.join("publication.json"),
        format!(
            r#"[{{"path":"{PDB}","present":true,"had_target":true,"exact":true}},{{"path":"{ONE}","present":true,"had_target":true,"exact":true}}]"#
        ),
    )
    .unwrap();
    next
}

/// Removes our lock file, as on a Stick our writers never wrote to.
pub fn without_lock(root: &Path) {
    match std::fs::remove_file(root.join(LOCK)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => panic!("{e}"),
    }
}

/// Every file under `root` and its bytes, the directories too.
#[must_use]
pub fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
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
