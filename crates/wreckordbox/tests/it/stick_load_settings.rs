//! A load writes the deck's settings files (wbx-8z2).
//!
//! A deck reads its colours, columns and fields from five files in the
//! Stick's `PIONEER` folder: `DEVSETTING.DAT`, `MYSETTING.DAT`,
//! `MYSETTING2.DAT`, `DJMMYSETTING.DAT` and `djprofile.nxs` (confirmed on
//! the deck 2026-11-15: those five are all it needs). A load gives a Stick
//! the Library's My Settings, the files the window keeps beside its state
//! folder (`usb-settings`, filled by rebuild-from-stick; Round-trip import
//! brings three of the five), in the same publication as the export; a Stick's own
//! files are kept unless the DJ asks for them to be replaced. The dry run
//! says, file by file, what the load does, and writes nothing.
//!
//! Sticks are temporary folders; the test plays the DJ when it unlocks one.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::common;

use std::path::{Path, PathBuf};

use common::Sandbox;
use serde_json::{json, Value};

const FILES: [&str; 5] = [
    "DEVSETTING.DAT",
    "MYSETTING.DAT",
    "MYSETTING2.DAT",
    "DJMMYSETTING.DAT",
    "djprofile.nxs",
];

/// CRC-16/XMODEM, as the players check these files.
fn xmodem(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for byte in bytes {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 == 0 {
                crc << 1
            } else {
                (crc << 1) ^ 0x1021
            };
        }
    }
    crc
}

/// A settings file in the players' shape: the 104-byte header (the length
/// of its strings, brand, software, version, the body's length), a body of
/// `fill` bytes and its checksum (`DJMMYSETTING.DAT`'s covers the header
/// too), then two zero bytes.
fn setting(name: &str, body: usize, fill: u8) -> Vec<u8> {
    let mut out = vec![0u8; 104];
    out[0..4].copy_from_slice(&96u32.to_le_bytes());
    out[4..14].copy_from_slice(b"PIONEER DJ");
    out[36..45].copy_from_slice(b"rekordbox");
    out[68..74].copy_from_slice(b"7.2.11");
    out[100..104].copy_from_slice(&(body as u32).to_le_bytes());
    out.extend(std::iter::repeat_n(fill, body));
    let crc = if name == "DJMMYSETTING.DAT" {
        xmodem(&out)
    } else {
        xmodem(&out[104..])
    };
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

/// The DJ's own: the five files a Library's My Settings hold.
fn library_settings() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("DEVSETTING.DAT", setting("DEVSETTING.DAT", 32, 0x11)),
        ("MYSETTING.DAT", setting("MYSETTING.DAT", 40, 0x22)),
        ("MYSETTING2.DAT", setting("MYSETTING2.DAT", 40, 0x33)),
        ("DJMMYSETTING.DAT", setting("DJMMYSETTING.DAT", 52, 0x44)),
        ("djprofile.nxs", b"a DJ profile".to_vec()),
    ]
}

struct Rig {
    sandbox: Sandbox,
    _sticks: tempfile::TempDir,
    stick: PathBuf,
    set: String,
}

impl Rig {
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
        let mut set = String::new();
        sandbox.edit(|writer| {
            set = writer.create_playlist("Set", "root").unwrap();
            writer.add_tracks(&set, std::slice::from_ref(&a)).unwrap();
        });
        Self {
            sandbox,
            _sticks: sticks,
            stick,
            set,
        }
    }

    /// The Library's My Settings, where the window keeps them.
    fn my_settings(&self) -> PathBuf {
        self.sandbox
            .state_dir()
            .parent()
            .unwrap()
            .join("usb-settings")
    }

    fn give_library_settings(&self) {
        std::fs::create_dir_all(self.my_settings()).unwrap();
        for (name, bytes) in library_settings() {
            std::fs::write(self.my_settings().join(name), bytes).unwrap();
        }
    }

    fn pioneer(&self) -> PathBuf {
        self.stick.join("PIONEER")
    }

    fn path(&self) -> &str {
        self.stick.to_str().unwrap()
    }

    fn unlock(&self) {
        let name = self.stick.file_name().unwrap().to_str().unwrap().to_owned();
        let run = self
            .sandbox
            .cli(&["stick", "unlock", self.path(), "--name", &name]);
        assert!(run.status.success(), "{}", run.stderr);
    }

    fn dry_run(&self, extra: &[&str]) -> Value {
        let mut args = vec!["--json", "stick", "dry-run", self.path(), self.set.as_str()];
        args.extend_from_slice(extra);
        let run = self.sandbox.cli(&args);
        assert!(run.status.success(), "{}", run.stderr);
        run.json()
    }

    fn load(&self, plan: &str, extra: &[&str]) -> common::Run {
        let mut args = vec![
            "--json",
            "stick",
            "load",
            self.path(),
            self.set.as_str(),
            "--plan",
            plan,
        ];
        args.extend_from_slice(extra);
        self.sandbox.cli(&args)
    }

    fn load_confirmed(&self, extra: &[&str]) -> (Value, Value) {
        let dry = self.dry_run(extra);
        let plan = dry["plan"]
            .as_str()
            .unwrap_or_else(|| panic!("no plan: {dry}"))
            .to_owned();
        let run = self.load(&plan, extra);
        assert!(run.status.success(), "{}", run.stderr);
        (dry, run.json())
    }

    fn on_stick(&self, name: &str) -> Option<Vec<u8>> {
        std::fs::read(self.pioneer().join(name)).ok()
    }
}

/// The dry run's line for each settings file: `(file, action)`.
fn actions(dry: &Value) -> Vec<(String, String)> {
    dry["changes"]["settings"]
        .as_array()
        .unwrap_or_else(|| panic!("no settings: {dry}"))
        .iter()
        .map(|l| {
            (
                l["file"].as_str().unwrap().to_owned(),
                l["action"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn all(action: &str) -> Vec<(String, String)> {
    FILES
        .iter()
        .map(|f| ((*f).to_owned(), action.to_owned()))
        .collect()
}

fn folder(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            if entry.file_type().unwrap().is_dir() {
                stack.push(entry.path());
            } else {
                out.push((entry.path(), std::fs::read(entry.path()).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a_load_gives_a_fresh_stick_the_librarys_five_settings_files() {
    let rig = Rig::new();
    rig.give_library_settings();
    rig.unlock();

    let before = folder(&rig.stick);
    let dry = rig.dry_run(&[]);
    assert_eq!(folder(&rig.stick), before, "a dry run writes nothing");
    assert_eq!(actions(&dry), all("write"), "{dry}");

    let plan = dry["plan"].as_str().unwrap().to_owned();
    let run = rig.load(&plan, &[]);
    assert!(run.status.success(), "{}", run.stderr);
    let load = run.json();
    assert_eq!(load["outcome"], "loaded", "{load}");
    for (name, bytes) in library_settings() {
        assert_eq!(
            rig.on_stick(name).as_deref(),
            Some(bytes.as_slice()),
            "{name} on the Stick"
        );
    }
    let mut written: Vec<String> =
        serde_json::from_value(load["written"]["settings"].clone()).unwrap();
    written.sort();
    let mut expected: Vec<String> = FILES.iter().map(|f| (*f).to_owned()).collect();
    expected.sort();
    assert_eq!(written, expected, "{load}");
}

#[test]
fn a_sticks_own_settings_are_kept_unless_the_dj_asks_to_replace_them() {
    let rig = Rig::new();
    rig.give_library_settings();
    rig.unlock();
    // The Stick's own, set on a player: MYSETTING.DAT and DEVSETTING.DAT.
    std::fs::create_dir_all(rig.pioneer()).unwrap();
    let own_my = setting("MYSETTING.DAT", 40, 0x99);
    let own_dev = setting("DEVSETTING.DAT", 32, 0x98);
    std::fs::write(rig.pioneer().join("MYSETTING.DAT"), &own_my).unwrap();
    std::fs::write(rig.pioneer().join("DEVSETTING.DAT"), &own_dev).unwrap();

    let (dry, load) = rig.load_confirmed(&[]);
    assert_eq!(
        actions(&dry),
        vec![
            ("DEVSETTING.DAT".to_owned(), "keep".to_owned()),
            ("MYSETTING.DAT".to_owned(), "keep".to_owned()),
            ("MYSETTING2.DAT".to_owned(), "write".to_owned()),
            ("DJMMYSETTING.DAT".to_owned(), "write".to_owned()),
            ("djprofile.nxs".to_owned(), "write".to_owned()),
        ]
    );
    assert_eq!(load["outcome"], "loaded", "{load}");
    assert_eq!(rig.on_stick("MYSETTING.DAT"), Some(own_my.clone()), "kept");
    assert_eq!(
        rig.on_stick("DEVSETTING.DAT"),
        Some(own_dev.clone()),
        "kept"
    );
    assert_eq!(
        rig.on_stick("MYSETTING2.DAT"),
        Some(library_settings()[2].1.clone())
    );

    // A load whose dry run did not ask to replace them cannot replace them.
    let dry = rig.dry_run(&[]);
    let plan = dry["plan"].as_str().unwrap().to_owned();
    let refused = rig.load(&plan, &["--replace-settings"]);
    assert!(!refused.status.success(), "{}", refused.stdout);
    assert_eq!(
        rig.on_stick("MYSETTING.DAT"),
        Some(own_my),
        "still the Stick's own"
    );

    // Asked for, they are replaced.
    let (dry, load) = rig.load_confirmed(&["--replace-settings"]);
    assert_eq!(
        &actions(&dry)[..2],
        &[
            ("DEVSETTING.DAT".to_owned(), "replace".to_owned()),
            ("MYSETTING.DAT".to_owned(), "replace".to_owned()),
        ]
    );
    assert_eq!(load["outcome"], "loaded", "{load}");
    assert_eq!(
        rig.on_stick("MYSETTING.DAT"),
        Some(library_settings()[1].1.clone())
    );
    assert_eq!(
        rig.on_stick("DEVSETTING.DAT"),
        Some(library_settings()[0].1.clone())
    );
}

#[test]
fn without_my_settings_a_stick_gets_rekordbox_display_defaults_and_is_told_what_is_missing() {
    let rig = Rig::new();
    rig.unlock();

    let (dry, load) = rig.load_confirmed(&[]);

    let lines = dry["changes"]["settings"].as_array().unwrap().clone();
    assert_eq!(lines[0]["file"], "DEVSETTING.DAT");
    assert_eq!(lines[0]["action"], "write");
    assert_eq!(lines[0]["from"], "defaults", "{dry}");
    for line in &lines[1..] {
        assert_eq!(line["action"], "unavailable", "{line}");
        assert!(
            line["reason"]
                .as_str()
                .is_some_and(|r| r.contains("rebuild-from-stick")),
            "{line}"
        );
        // Round-trip import brings the three `*MYSETTING*.DAT` files only.
        assert_eq!(
            line["reason"].as_str().unwrap().contains("Round-trip"),
            line["file"] != "djprofile.nxs",
            "{line}"
        );
    }
    assert_eq!(load["outcome"], "loaded", "{load}");
    let dev = rig
        .on_stick("DEVSETTING.DAT")
        .expect("DEVSETTING.DAT written");
    assert_eq!(dev.len(), 140);
    for name in &FILES[1..] {
        assert_eq!(rig.on_stick(name), None, "{name} invented");
    }
    // A broken file in My Settings is not given to a deck.
    std::fs::create_dir_all(rig.my_settings()).unwrap();
    let mut broken = setting("MYSETTING2.DAT", 40, 0x33);
    broken[110] ^= 0xff;
    std::fs::write(rig.my_settings().join("MYSETTING2.DAT"), broken).unwrap();
    let dry = rig.dry_run(&[]);
    let line = dry["changes"]["settings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["file"] == "MYSETTING2.DAT")
        .unwrap()
        .clone();
    assert_eq!(line["action"], "unavailable", "{line}");
    assert_eq!(
        line["reason"],
        json!("MYSETTING2.DAT in My Settings is damaged (its checksum does not match)")
    );
}

/// A WAV file of `seconds` of silence (16-bit mono, 8 kHz).
fn write_wav(path: &Path, seconds: u32) {
    let rate: u32 = 8_000;
    let data = rate * 2 * seconds;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    out.resize(44 + data as usize, 0);
    std::fs::write(path, out).unwrap();
}

/// A Stick from another computer holding one track, "Low Sun", and the
/// DJ's five settings files; the same track in a music folder here.
fn gig_stick_from_elsewhere(dir: &Path) -> (PathBuf, PathBuf) {
    let elsewhere = dir.join("other-computer");
    let music = dir.join("music");
    let gig = dir.join("GIG");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::create_dir_all(&music).unwrap();
    std::fs::create_dir_all(&gig).unwrap();
    std::fs::write(gig.join(wreckordbox_stick_protection::STAND_IN_MARKER), b"").unwrap();
    let audio = elsewhere.join("Low Sun.wav");
    write_wav(&audio, 3);
    std::fs::copy(&audio, music.join("Low Sun.wav")).unwrap();
    let track = rbl_export::SourceTrack {
        id: 500,
        source_path: audio,
        title: "Low Sun".into(),
        artist: "Somebody".into(),
        ..Default::default()
    };
    let playlist = rbl_export::SourcePlaylist {
        id: 11,
        name: "Friday".into(),
        track_indices: vec![0],
        ..Default::default()
    };
    let sync = rbl_export::SyncSource {
        db_id: 0x0bad_cafe,
        ..Default::default()
    };
    rbl_export::export_full(
        &gig,
        &[track],
        &[playlist],
        &[],
        None,
        Some(&sync),
        &mut |_| {},
    )
    .unwrap();
    std::fs::remove_dir_all(rbl_export::export_root(&gig).join("rbxport")).unwrap();
    for (name, bytes) in library_settings() {
        std::fs::write(rbl_export::export_root(&gig).join(name), bytes).unwrap();
    }
    (gig, music)
}

#[test]
fn rebuild_from_stick_carries_the_sticks_settings_into_my_settings_for_the_next_load() {
    let rig = Rig::new();
    let dirs = tempfile::tempdir().unwrap();
    let (gig, music) = gig_stick_from_elsewhere(dirs.path());
    let rebuild = |extra: &[&str]| -> Value {
        let mut args = vec![
            "--json",
            "library",
            "rebuild-from-stick",
            gig.to_str().unwrap(),
            "--music",
            music.to_str().unwrap(),
        ];
        args.extend_from_slice(extra);
        let run = rig.sandbox.cli(&args);
        assert!(run.status.success(), "{}", run.stderr);
        run.json()
    };

    let dry = rebuild(&["--dry-run"]);
    let carried: Vec<(String, String)> = dry["settings"]
        .as_array()
        .unwrap_or_else(|| panic!("no settings: {dry}"))
        .iter()
        .map(|l| {
            (
                l["file"].as_str().unwrap().to_owned(),
                l["action"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(carried, all("carry"), "{dry}");
    assert!(!rig.my_settings().exists(), "a dry run writes nothing");

    let applied = rebuild(&[]);
    assert_eq!(applied["complete"], true, "{applied}");
    for (name, bytes) in library_settings() {
        assert_eq!(
            std::fs::read(rig.my_settings().join(name)).ok(),
            Some(bytes),
            "{name} in My Settings"
        );
    }

    // The next load gives a fresh Stick those files.
    rig.unlock();
    let (dry, load) = rig.load_confirmed(&[]);
    assert_eq!(actions(&dry), all("write"), "{dry}");
    assert_eq!(load["outcome"], "loaded", "{load}");
    for (name, bytes) in library_settings() {
        assert_eq!(
            rig.on_stick(name),
            Some(bytes),
            "{name} on the loaded Stick"
        );
    }
}

/// The command a dry run prints is the one to type: with
/// `--replace-settings` when the dry run had it, since the plan is bound to
/// it and a load without it is refused.
#[test]
fn the_load_command_a_dry_run_prints_repeats_replace_settings() {
    let rig = Rig::new();
    rig.give_library_settings();
    rig.unlock();
    std::fs::create_dir_all(rig.pioneer()).unwrap();
    std::fs::write(
        rig.pioneer().join("MYSETTING.DAT"),
        setting("MYSETTING.DAT", 40, 0x99),
    )
    .unwrap();
    let printed = |extra: &[&str]| -> String {
        let mut args = vec!["stick", "dry-run", rig.path(), rig.set.as_str()];
        args.extend_from_slice(extra);
        let run = rig.sandbox.cli(&args);
        assert!(run.status.success(), "{}", run.stderr);
        run.stdout
            .lines()
            .find_map(|l| l.strip_prefix("To load exactly this: wreckordbox "))
            .unwrap_or_else(|| panic!("no command printed: {}", run.stdout))
            .to_owned()
    };

    let plain = printed(&[]);
    assert!(!plain.contains("--replace-settings"), "{plain}");

    let replacing = printed(&["--replace-settings"]);
    assert!(replacing.ends_with(" --replace-settings"), "{replacing}");
    // Typed as printed, it loads and replaces the Stick's own file.
    let mut args = vec!["--json"];
    args.extend(replacing.split(' '));
    let run = rig.sandbox.cli(&args);
    assert!(run.status.success(), "{replacing}: {}", run.stderr);
    assert_eq!(run.json()["outcome"], "loaded");
    assert_eq!(
        rig.on_stick("MYSETTING.DAT"),
        Some(library_settings()[1].1.clone())
    );
}

/// The plan id is bound to the bytes the dry run saw, not only to what is
/// done with each file: My Settings changed since, the load is refused.
#[test]
fn a_load_is_refused_when_my_settings_changed_since_its_dry_run() {
    let rig = Rig::new();
    rig.give_library_settings();
    rig.unlock();
    let dry = rig.dry_run(&[]);
    assert_eq!(actions(&dry), all("write"), "{dry}");
    let plan = dry["plan"].as_str().unwrap().to_owned();

    // Another file as good as the first: the same action, other bytes.
    std::fs::write(
        rig.my_settings().join("MYSETTING.DAT"),
        setting("MYSETTING.DAT", 40, 0x77),
    )
    .unwrap();

    let refused = rig.load(&plan, &[]);
    assert!(!refused.status.success(), "{}", refused.stdout);
    assert_eq!(rig.on_stick("MYSETTING.DAT"), None, "nothing written");
}

/// Without `--json` too, a rebuild says what became of each settings file:
/// a damaged one, or one the Stick lacks, is not carried, and the DJ is told.
#[test]
fn rebuild_from_stick_says_in_words_what_became_of_each_settings_file() {
    let rig = Rig::new();
    let dirs = tempfile::tempdir().unwrap();
    let (gig, music) = gig_stick_from_elsewhere(dirs.path());
    let export = rbl_export::export_root(&gig);
    let mut broken = setting("MYSETTING2.DAT", 40, 0x33);
    broken[110] ^= 0xff;
    std::fs::write(export.join("MYSETTING2.DAT"), broken).unwrap();
    std::fs::remove_file(export.join("djprofile.nxs")).unwrap();
    let rebuild = |extra: &[&str]| -> String {
        let mut args = vec![
            "library",
            "rebuild-from-stick",
            gig.to_str().unwrap(),
            "--music",
            music.to_str().unwrap(),
        ];
        args.extend_from_slice(extra);
        let run = rig.sandbox.cli(&args);
        assert!(run.status.success(), "{}", run.stderr);
        run.stdout
    };

    let dry = rebuild(&["--dry-run"]);
    assert!(dry.contains("  MYSETTING.DAT: would be carried"), "{dry}");
    assert!(
        dry.contains(
            "  MYSETTING2.DAT: NOT carried, damaged: MYSETTING2.DAT on the Stick is damaged"
        ),
        "{dry}"
    );
    assert!(dry.contains("  djprofile.nxs: not on the Stick"), "{dry}");
    assert!(!rig.my_settings().exists(), "a dry run writes nothing");

    let applied = rebuild(&[]);
    assert!(applied.contains("  MYSETTING.DAT: carried"), "{applied}");
    assert!(
        applied.contains("  MYSETTING2.DAT: NOT carried, damaged"),
        "{applied}"
    );
    assert!(
        applied.contains("  djprofile.nxs: not on the Stick"),
        "{applied}"
    );
    assert!(rig.my_settings().join("MYSETTING.DAT").is_file());
    assert!(!rig.my_settings().join("MYSETTING2.DAT").exists());
    assert!(!rig.my_settings().join("djprofile.nxs").exists());
}
