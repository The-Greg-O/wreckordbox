//! Sticks in temporary folders for tests, here and in the app (feature
//! `testing`). Every Stick is written by the export pipeline into a folder
//! the test owns; nothing here touches a mounted Stick.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc
)]

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
        analysis: vec![(
            "DAT".into(),
            rbl_anlz::AnlzBuilder::new().path("/x.mp3").finish(),
        )],
        ..SourceTrack::default()
    }
}

/// Exports to `dest` a Stick with a folder "Sets" holding "Friday" (tracks
/// 1, 2) and a top-level "Warm Up" (track 3), both libraries.
pub fn stick(dest: &Path) {
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
    let next = next_export();
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

/// What a whole publication by a writer of ours leaves when it completes:
/// its lock file, both databases replaced by another export's (playlists
/// [`NEXT_PLAYLISTS`]), no journal.
pub fn publish_completely(root: &Path) -> Interrupted {
    let next = next_export();
    std::fs::write(root.join(LOCK), b"").unwrap();
    std::fs::write(root.join(PDB), &next.pdb).unwrap();
    std::fs::write(root.join(ONE), &next.one).unwrap();
    next
}

/// Another real export's two databases.
fn next_export() -> Interrupted {
    let other = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let tracks: Vec<SourceTrack> = (5..=6).map(|i| track(src.path(), i)).collect();
    let playlists = vec![
        SourcePlaylist {
            id: 20,
            name: NEXT_PLAYLISTS[0].into(),
            track_indices: vec![0],
            ..Default::default()
        },
        SourcePlaylist {
            id: 21,
            name: NEXT_PLAYLISTS[1].into(),
            track_indices: vec![1],
            ..Default::default()
        },
    ];
    export(other.path(), &tracks, &playlists).unwrap();
    Interrupted {
        pdb: std::fs::read(other.path().join(PDB)).unwrap(),
        one: std::fs::read(other.path().join(ONE)).unwrap(),
    }
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

/// The name "Warm Up" takes in a WAL-mode fixture's committed write.
pub const WAL_RENAMED: &str = "Warm Up (WAL)";

/// Opens a copy of the Stick's `exportLibrary.db` for writing, keyed, in WAL
/// mode with automatic checkpoints off, and renames "Warm Up" to
/// [`WAL_RENAMED`] in a committed transaction. Returns the open connection
/// and the folder it is in.
fn wal_writer(root: &Path) -> (rusqlite::Connection, tempfile::TempDir, PathBuf) {
    let work = tempfile::tempdir().unwrap();
    let db = work.path().join("exportLibrary.db");
    std::fs::copy(root.join(ONE), &db).unwrap();
    let conn = rusqlite::Connection::open(&db).unwrap();
    crate::one_library::keyed(&conn).unwrap();
    let mode: String = conn
        .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
    let changed = conn
        .execute(
            "UPDATE playlist SET name = ?1 WHERE name = 'Warm Up'",
            [WAL_RENAMED],
        )
        .unwrap();
    assert_eq!(changed, 1);
    (conn, work, db)
}

/// Leaves the Stick's `exportLibrary.db` in WAL mode with a committed write
/// held only in its `-wal`, and no `-shm`: a Stick pulled out while rekordbox
/// had it open. A correct read sees [`WAL_RENAMED`].
pub fn wal_pending(root: &Path) {
    let (conn, _work, db) = wal_writer(root);
    let wal = PathBuf::from(format!("{}-wal", db.display()));
    assert!(
        std::fs::metadata(&wal).unwrap().len() > 0,
        "the write is in the -wal"
    );
    std::fs::copy(&db, root.join(ONE)).unwrap();
    std::fs::copy(&wal, root.join(format!("{ONE}-wal"))).unwrap();
    drop(conn);
}

/// Leaves the Stick's `exportLibrary.db` in WAL mode, checkpointed and
/// closed: the WAL flag in its header, no companion beside it. A correct
/// read sees [`WAL_RENAMED`].
pub fn wal_checkpointed(root: &Path) {
    let (conn, _work, db) = wal_writer(root);
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .unwrap();
    drop(conn);
    for suffix in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
    }
    std::fs::copy(&db, root.join(ONE)).unwrap();
}

/// What differs between two [`snapshot`]s: each path added, removed or
/// changed, for an assertion message that names them.
#[must_use]
pub fn changes(
    before: &BTreeMap<PathBuf, Option<Vec<u8>>>,
    after: &BTreeMap<PathBuf, Option<Vec<u8>>>,
) -> Vec<String> {
    let mut out = Vec::new();
    for (path, bytes) in after {
        match before.get(path) {
            None => out.push(format!("added {}", path.display())),
            Some(old) if old != bytes => out.push(format!("changed {}", path.display())),
            Some(_) => {}
        }
    }
    for path in before.keys().filter(|p| !after.contains_key(*p)) {
        out.push(format!("removed {}", path.display()));
    }
    out
}

/// Asserts that the Stick at `root` is byte for byte as `before`, naming
/// what changed otherwise.
pub fn assert_unchanged(root: &Path, before: &BTreeMap<PathBuf, Option<Vec<u8>>>, what: &str) {
    let found = changes(before, &snapshot(root));
    assert!(found.is_empty(), "{what} changed the Stick: {found:?}");
}

/// The `MyTags` of [`tagged_stick`]: two categories, "Vibe" (Dark, Uplifting)
/// and "Situation" (Peak, Warm-up, and a second "Dark" of its own), as
/// `(id, seq, name, attribute, parent)` (wbx-45.12).
pub const TAGS: [(u64, u32, &str, u8, u64); 7] = [
    (2, 2, "Situation", 1, 0),
    (1, 1, "Vibe", 1, 0),
    (11, 1, "Dark", 0, 1),
    (12, 2, "Uplifting", 0, 1),
    (21, 1, "Peak", 0, 2),
    (22, 2, "Warm-up", 0, 2),
    (23, 3, "Dark", 0, 2),
];

/// The tracks of [`tagged_stick`]: `(title, tag ids)`. The first five are
/// the playlist "Warm Up"; "Elsewhere" is on the Stick but not in it.
pub const TAGGED_TRACKS: [(&str, &[u64]); 6] = [
    ("Low Sun", &[11, 21]),
    ("Paper", &[11, 12]),
    ("Kiln", &[12, 22]),
    ("Odd", &[23]),
    ("Blank", &[]),
    ("Elsewhere", &[11, 21]),
];

/// Exports to `dest` a Stick whose tracks carry `MyTags` ([`TAGS`],
/// [`TAGGED_TRACKS`]), both libraries and `exportExt.pdb`, with the
/// playlist "Warm Up" (so the WAL fixtures apply to it too).
pub fn tagged_stick(dest: &Path) {
    use rbl_export::{export_full, SourceMyTag};
    let src = tempfile::tempdir().unwrap();
    let tracks: Vec<SourceTrack> = TAGGED_TRACKS
        .iter()
        .enumerate()
        .map(|(at, (title, tags))| {
            let id = u64::try_from(at).unwrap() + 1;
            let mut t = track(src.path(), id);
            t.title = (*title).into();
            t.my_tags = tags.to_vec();
            t
        })
        .collect();
    let my_tags: Vec<SourceMyTag> = TAGS
        .iter()
        .map(|&(id, seq, name, attribute, parent)| SourceMyTag {
            id,
            seq,
            name: name.into(),
            attribute,
            parent,
        })
        .collect();
    let playlists = vec![SourcePlaylist {
        id: 12,
        name: "Warm Up".into(),
        track_indices: vec![0, 1, 2, 3, 4],
        ..Default::default()
    }];
    export_full(dest, &tracks, &playlists, &my_tags, None, None, &mut |_| {}).unwrap();
}
