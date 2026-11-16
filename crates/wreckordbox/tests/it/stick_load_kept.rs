//! A load onto a Stick that already holds another export (wbx-dc0).
//!
//! The export keeps what another writer put on a Stick (another Library's
//! export, rekordbox's from another computer), as rekordbox's own export
//! does: its tracks, their audio where it is, and its playlists. The dry
//! run must say so truthfully: those tracks are kept, not added; their
//! audio is not copied and is not counted in the room needed; their
//! playlists stay. Only this Library's chosen tracks are added.
//!
//! The Stick is a temporary folder; the "other export" is written to it
//! directly with `rbl_export` from another Library (another `DBID`), and
//! wreckordbox's own record removed, which leaves what rekordbox on another
//! computer leaves. (A Stick holding another Library's wreckordbox record is
//! refused by the export itself, and so by the dry run.)
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::common;

use std::path::{Path, PathBuf};

use common::Sandbox;
use rbl_export::device_library::Format;
use serde_json::Value;

/// Another Library's identifier: not the sandbox's.
const OTHER_DB_ID: u64 = 0x0bad_cafe;
/// Each of the other export's audio files, in bytes.
const OTHER_AUDIO_BYTES: usize = 3_000_000;
const OTHER_TRACKS: usize = 6;

struct Rig {
    sandbox: Sandbox,
    _sticks: tempfile::TempDir,
    _other_music: tempfile::TempDir,
    stick: PathBuf,
    set: String,
}

/// A WAV file of `frames` frames of silence (16-bit mono, 44.1 kHz),
/// padded to `bytes` when that is larger.
fn wav(path: &Path, frames: u32, bytes: usize) {
    let data = frames * 2;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&44_100u32.to_le_bytes());
    out.extend_from_slice(&88_200u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    out.resize(out.len() + data as usize, 0);
    if out.len() < bytes {
        out.resize(bytes, 0);
    }
    std::fs::write(path, out).unwrap();
}

impl Rig {
    /// A Library with "Set" (Alpha, Bravo), and a Stick holding another
    /// Library's export: `OTHER_TRACKS` tracks in "Their Set".
    fn new() -> Self {
        let mut sandbox = Sandbox::empty();
        let sticks = tempfile::tempdir().unwrap();
        let stick = sticks.path().join("FRIDAY");
        std::fs::create_dir(&stick).unwrap();
        std::fs::write(
            stick.join(wreckordbox_stick_protection::STAND_IN_MARKER),
            b"",
        )
        .unwrap();
        sandbox.sticks = vec![stick.clone()];
        let a = sandbox.import_track("Alpha.wav", 2);
        let b = sandbox.import_track("Bravo.wav", 2);
        let mut set = String::new();
        sandbox.edit(|writer| {
            set = writer.create_playlist("Set", "root").unwrap();
            writer.add_tracks(&set, &[a.clone(), b.clone()]).unwrap();
        });

        let other_music = tempfile::tempdir().unwrap();
        let tracks: Vec<rbl_export::SourceTrack> = (0..OTHER_TRACKS)
            .map(|i| {
                let path = other_music.path().join(format!("Theirs {i}.wav"));
                wav(&path, 44_100, OTHER_AUDIO_BYTES);
                rbl_export::SourceTrack {
                    id: 500 + i as u64,
                    source_path: path,
                    title: format!("Theirs {i}"),
                    artist: "Someone Else".into(),
                    album: "Elsewhere".into(),
                    duration_sec: 1,
                    sample_rate: 44_100,
                    ..Default::default()
                }
            })
            .collect();
        let playlists = vec![rbl_export::SourcePlaylist {
            id: 77,
            name: "Their Set".into(),
            track_indices: (0..OTHER_TRACKS).collect(),
            ..Default::default()
        }];
        let sync = rbl_export::sync_record::SyncSource {
            db_id: OTHER_DB_ID,
            ..Default::default()
        };
        rbl_export::export_full(
            &stick,
            &tracks,
            &playlists,
            &[],
            None,
            Some(&sync),
            &mut |_| {},
        )
        .unwrap();
        // rekordbox keeps no record of ours: a Stick it wrote has the
        // databases (and its own sync records) only.
        std::fs::remove_dir_all(rbl_export::export_root(&stick).join("rbxport")).unwrap();
        Self {
            sandbox,
            _sticks: sticks,
            _other_music: other_music,
            stick,
            set,
        }
    }

    fn path(&self) -> &str {
        self.stick.to_str().unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        let run = self.sandbox.cli(&all);
        assert!(run.status.success(), "{args:?}: {}", run.stderr);
        run.json()
    }

    fn dry_run(&self) -> Value {
        self.json(&["stick", "dry-run", self.path(), &self.set])
    }

    fn playlists_on_stick(&self) -> Vec<(String, Vec<String>)> {
        let library =
            wreckordbox_stick_outline::library(&self.stick, Format::DeviceLibrary).unwrap();
        let mut out: Vec<(String, Vec<String>)> = library
            .nodes
            .iter()
            .filter(|n| !n.folder)
            .map(|n| {
                let mut titles: Vec<String> = n
                    .tracks
                    .iter()
                    .map(|id| library.track(*id).unwrap().title.clone())
                    .collect();
                titles.sort();
                (n.name.clone(), titles)
            })
            .collect();
        out.sort();
        out
    }
}

fn titles(lines: &Value) -> Vec<String> {
    let mut titles: Vec<String> = lines
        .as_array()
        .unwrap_or_else(|| panic!("not a list: {lines}"))
        .iter()
        .map(|l| l["title"].as_str().unwrap().to_owned())
        .collect();
    titles.sort();
    titles
}

fn theirs() -> Vec<String> {
    (0..OTHER_TRACKS).map(|i| format!("Theirs {i}")).collect()
}

#[test]
fn a_dry_run_onto_another_export_adds_only_the_chosen_tracks_and_says_what_stays() {
    let rig = Rig::new();

    let dry = rig.dry_run();
    let changes = &dry["changes"];

    assert_eq!(
        titles(&changes["tracks"]["added"]),
        ["Alpha", "Bravo"],
        "{changes}"
    );
    assert_eq!(changes["counts"]["tracks_added"], 2, "{changes}");
    assert_eq!(changes["counts"]["tracks_removed"], 0, "{changes}");
    assert_eq!(titles(&changes["tracks"]["kept"]), theirs(), "{changes}");
    assert_eq!(changes["counts"]["tracks_kept"], OTHER_TRACKS, "{changes}");
    assert_eq!(changes["playlists"]["created"], serde_json::json!(["Set"]));
    assert_eq!(
        changes["playlists"]["kept"],
        serde_json::json!(["Their Set"])
    );
    assert_eq!(changes["counts"]["playlists_kept"], 1);
    assert_eq!(changes["playlists"]["removed"], serde_json::json!([]));
    // Their audio is not copied again: the room needed is far below it.
    let needed = dry["space"]["needed_bytes"].as_u64().unwrap();
    let theirs_bytes = (OTHER_TRACKS * OTHER_AUDIO_BYTES) as u64;
    assert!(
        needed < theirs_bytes / 2,
        "needed {needed} counts their {theirs_bytes} bytes again"
    );
}

#[test]
fn the_load_does_what_that_dry_run_said() {
    let rig = Rig::new();
    let audio = |stick: &Path| -> Vec<(PathBuf, u64)> {
        let mut out = Vec::new();
        let mut stack = vec![stick.join("Contents")];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                if entry.file_type().unwrap().is_dir() {
                    stack.push(entry.path());
                } else {
                    use std::os::unix::fs::MetadataExt;
                    out.push((entry.path(), entry.metadata().unwrap().ino()));
                }
            }
        }
        out.sort();
        out
    };
    let theirs_before = audio(&rig.stick);
    assert_eq!(theirs_before.len(), OTHER_TRACKS);

    let name = rig.stick.file_name().unwrap().to_str().unwrap().to_owned();
    let run = rig
        .sandbox
        .cli(&["stick", "unlock", rig.path(), "--name", &name]);
    assert!(run.status.success(), "{}", run.stderr);
    let dry = rig.dry_run();
    let plan = dry["plan"].as_str().unwrap().to_owned();
    let run = rig.sandbox.cli(&[
        "--json",
        "stick",
        "load",
        rig.path(),
        &rig.set,
        "--plan",
        &plan,
    ]);
    assert!(run.status.success(), "{}", run.stderr);
    let load = run.json();
    assert_eq!(load["outcome"], "loaded", "{load}");

    assert_eq!(
        rig.playlists_on_stick(),
        vec![
            (
                "Set".to_owned(),
                vec!["Alpha".to_owned(), "Bravo".to_owned()]
            ),
            ("Their Set".to_owned(), theirs()),
        ],
        "the Stick keeps the other export's playlist, as the dry run said"
    );
    let after = audio(&rig.stick);
    for (path, inode) in &theirs_before {
        assert!(
            after.iter().any(|(p, i)| p == path && i == inode),
            "{} was copied again or removed",
            path.display()
        );
    }
}
