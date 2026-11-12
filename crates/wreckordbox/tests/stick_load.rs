//! Loading a Stick through the service (wbx-70): `wreckordbox stick list`,
//! `stick dry-run` and `stick load`, the real binary against a service on a
//! throwaway Library.
//!
//! Every Stick here is a temporary folder carrying the stand-in marker
//! (`RBXPORT_TEST=1`), named to the service as the only mounted volumes;
//! a sandbox's service never looks at the machine's real mounts. What a load
//! wrote is read back with the non-recovering Stick reader. The test plays
//! the DJ when it unlocks a Stick.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::Sandbox;
use rbl_export::device_library::Format;
use serde_json::Value;

/// A Library of real (silent) audio files in playlists, and Sticks.
struct Rig {
    sandbox: Sandbox,
    _sticks: tempfile::TempDir,
    paths: Vec<PathBuf>,
    a: String,
    b: String,
    c: String,
    /// "Set" holds A then B; "Other" holds C.
    set: String,
    other: String,
}

impl Rig {
    /// Sticks named `names` (a name ending in `*` is a Spare stick).
    fn new(names: &[&str]) -> Self {
        Self::with_free(names, None)
    }

    /// As [`Rig::new`], each Stick reporting `free` bytes free.
    fn with_free(names: &[&str], free: Option<u64>) -> Self {
        let mut sandbox = Sandbox::empty();
        let sticks = tempfile::tempdir().unwrap();
        let mut paths = Vec::new();
        for name in names {
            let spare = name.ends_with('*');
            let path = sticks.path().join(name.trim_end_matches('*'));
            std::fs::create_dir(&path).unwrap();
            std::fs::write(
                path.join(wreckordbox_stick_protection::STAND_IN_MARKER),
                b"",
            )
            .unwrap();
            if spare {
                std::fs::write(path.join(wreckordbox_stick_protection::SPARE_MARKER), b"").unwrap();
            }
            paths.push(path);
        }
        sandbox.sticks.clone_from(&paths);
        sandbox.stick_free_bytes = free;
        let a = sandbox.import_track("Alpha.wav", 2);
        let b = sandbox.import_track("Bravo.wav", 2);
        let c = sandbox.import_track("Charlie.wav", 2);
        let mut set = String::new();
        let mut other = String::new();
        sandbox.edit(|writer| {
            set = writer.create_playlist("Set", "root").unwrap();
            writer.add_tracks(&set, &[a.clone(), b.clone()]).unwrap();
            other = writer.create_playlist("Other", "root").unwrap();
            writer.add_tracks(&other, std::slice::from_ref(&c)).unwrap();
        });
        Self {
            sandbox,
            _sticks: sticks,
            paths,
            a,
            b,
            c,
            set,
            other,
        }
    }

    fn path(&self, index: usize) -> &str {
        self.paths[index].to_str().unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        let run = self.sandbox.cli(&all);
        assert!(run.status.success(), "{args:?}: {}", run.stderr);
        run.json()
    }

    /// The DJ unlocks the Stick, by name.
    fn unlock(&self, index: usize) {
        let name = self.paths[index].file_name().unwrap().to_str().unwrap();
        let run = self
            .sandbox
            .cli(&["stick", "unlock", self.path(index), "--name", name]);
        assert!(run.status.success(), "{}", run.stderr);
    }

    fn dry_run(&self, index: usize, playlists: &[&str]) -> Value {
        let mut args = vec!["stick", "dry-run", self.path(index)];
        args.extend_from_slice(playlists);
        self.json(&args)
    }

    fn load(&self, index: usize, playlists: &[&str], plan: &str) -> common::Run {
        let mut args = vec!["--json", "stick", "load", self.path(index)];
        args.extend_from_slice(playlists);
        args.extend_from_slice(&["--plan", plan]);
        self.sandbox.cli(&args)
    }

    /// Dry run, then load with its plan: the whole confirmed load.
    fn load_confirmed(&self, index: usize, playlists: &[&str]) -> Value {
        let dry = self.dry_run(index, playlists);
        let plan = dry["plan"].as_str().expect(&dry.to_string()).to_owned();
        let run = self.load(index, playlists, &plan);
        assert!(run.status.success(), "{}", run.stderr);
        run.json()
    }

    /// The playlists on the Stick, each with its tracks' titles, read back
    /// with the non-recovering reader.
    fn on_stick(&self, index: usize) -> Vec<(String, Vec<String>)> {
        let library =
            wreckordbox_stick_outline::library(&self.paths[index], Format::DeviceLibrary).unwrap();
        library
            .nodes
            .iter()
            .filter(|n| !n.folder)
            .map(|n| {
                (
                    n.name.clone(),
                    n.tracks
                        .iter()
                        .map(|id| library.track(*id).unwrap().title.clone())
                        .collect(),
                )
            })
            .collect()
    }
}

/// Every file under `root` with its bytes: the whole state of a Stick.
fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                stack.push(entry.path());
            } else {
                out.push((
                    entry.path().display().to_string(),
                    std::fs::read(entry.path()).unwrap_or_default(),
                ));
            }
        }
    }
    out.sort();
    out
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

#[test]
fn mounted_sticks_are_listed_with_kind_protection_last_load_and_free_space() {
    let rig = Rig::with_free(&["FRIDAY", "SPARE*"], Some(900_000_000));

    let listed = rig.json(&["stick", "list"]);

    let sticks = listed["sticks"].as_array().unwrap();
    assert_eq!(sticks.len(), 2, "{listed}");
    let gig = sticks.iter().find(|s| s["path"] == rig.path(0)).unwrap();
    let spare = sticks.iter().find(|s| s["path"] == rig.path(1)).unwrap();
    assert_eq!(gig["name"], "FRIDAY");
    assert_eq!(gig["kind"], "gig");
    assert_eq!(spare["kind"], "spare");
    assert_eq!(gig["protected"], true, "protected by default");
    assert_eq!(gig["free_bytes"], 900_000_000);
    assert_eq!(gig["last_loaded"], Value::Null, "never loaded");

    rig.unlock(0);
    rig.load_confirmed(0, &[&rig.set]);
    let listed = rig.json(&["stick", "list"]);
    let gig = listed["sticks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["path"] == rig.path(0))
        .unwrap()
        .clone();
    assert_eq!(gig["protected"], false);
    assert!(gig["last_loaded"].is_string(), "{gig}");

    let text = rig.sandbox.cli(&["stick", "list"]);
    assert!(text.status.success(), "{}", text.stderr);
    assert!(text.stdout.contains("FRIDAY"), "{}", text.stdout);
    assert!(text.stdout.contains("Gig stick"), "{}", text.stdout);
    assert!(text.stdout.contains("Spare stick"), "{}", text.stdout);
}

#[test]
fn a_dry_run_reports_every_change_and_writes_nothing() {
    let rig = Rig::new(&["FRIDAY"]);
    let before = snapshot(&rig.paths[0]);

    let dry = rig.dry_run(0, &[&rig.set, &rig.other]);

    assert_eq!(snapshot(&rig.paths[0]), before, "nothing written");
    assert!(dry["plan"].is_string(), "{dry}");
    assert_eq!(dry["stick"]["kind"], "gig");
    assert_eq!(dry["stick"]["protected"], true);
    let changes = &dry["changes"];
    assert_eq!(
        titles(&changes["tracks"]["added"]),
        ["Alpha", "Bravo", "Charlie"]
    );
    assert_eq!(changes["counts"]["tracks_added"], 3);
    assert_eq!(changes["counts"]["tracks_removed"], 0);
    assert_eq!(
        changes["playlists"]["created"],
        serde_json::json!(["Set", "Other"])
    );
    assert_eq!(changes["counts"]["playlists_created"], 2);
    assert!(dry["space"]["needed_bytes"].as_u64().unwrap() > 0, "{dry}");
    assert!(dry["space"]["free_before"].as_u64().is_some(), "{dry}");

    let text = rig
        .sandbox
        .cli(&["stick", "dry-run", rig.path(0), &rig.set, &rig.other]);
    assert!(text.status.success(), "{}", text.stderr);
    assert!(
        text.stdout.contains("Nothing was written"),
        "{}",
        text.stdout
    );
    assert!(text.stdout.contains("Alpha"), "{}", text.stdout);
    assert!(text.stdout.contains("--plan"), "{}", text.stdout);
    assert_eq!(snapshot(&rig.paths[0]), before, "nothing written");
}

#[test]
fn a_load_on_a_protected_stick_is_refused_and_writes_nothing() {
    let rig = Rig::new(&["FRIDAY"]);
    let dry = rig.dry_run(0, &[&rig.set]);
    let plan = dry["plan"].as_str().unwrap();
    let before = snapshot(&rig.paths[0]);

    let refused = rig.load(0, &[&rig.set], plan);

    assert!(!refused.status.success());
    assert_eq!(
        refused.error_json()["error"]["code"],
        "stick_protected",
        "{}",
        refused.stderr
    );
    assert_eq!(snapshot(&rig.paths[0]), before, "nothing written");
}

#[test]
fn a_load_needs_the_plan_of_its_own_dry_run() {
    let rig = Rig::new(&["FRIDAY", "SATURDAY"]);
    rig.unlock(0);
    rig.unlock(1);
    let before = snapshot(&rig.paths[0]);

    // No plan at all: the flag is required.
    let missing = rig.sandbox.cli(&["stick", "load", rig.path(0), &rig.set]);
    assert!(!missing.status.success());
    // A plan id that was never issued.
    let unknown = rig.load(0, &[&rig.set], "0123456789abcdef");
    assert_eq!(
        unknown.error_json()["error"]["code"],
        "stick_load_unconfirmed",
        "{}",
        unknown.stderr
    );
    // A plan for other playlists.
    let other = rig.dry_run(0, &[&rig.other]);
    let wrong = rig.load(0, &[&rig.set], other["plan"].as_str().unwrap());
    assert_eq!(
        wrong.error_json()["error"]["code"],
        "stick_load_unconfirmed",
        "{}",
        wrong.stderr
    );
    // A plan for another Stick.
    let elsewhere = rig.dry_run(1, &[&rig.set]);
    let moved = rig.load(0, &[&rig.set], elsewhere["plan"].as_str().unwrap());
    assert_eq!(
        moved.error_json()["error"]["code"],
        "stick_load_unconfirmed",
        "{}",
        moved.stderr
    );
    assert_eq!(snapshot(&rig.paths[0]), before, "nothing written");

    // A plan is used once.
    let dry = rig.dry_run(0, &[&rig.set]);
    let plan = dry["plan"].as_str().unwrap();
    let first = rig.load(0, &[&rig.set], plan);
    assert!(first.status.success(), "{}", first.stderr);
    let again = rig.load(0, &[&rig.set], plan);
    assert_eq!(
        again.error_json()["error"]["code"],
        "stick_load_unconfirmed",
        "{}",
        again.stderr
    );
}

#[test]
fn a_load_does_what_its_dry_run_said_and_names_each_track_as_it_is_written() {
    let rig = Rig::new(&["FRIDAY"]);
    rig.unlock(0);
    let dry = rig.dry_run(0, &[&rig.set, &rig.other]);
    let plan = dry["plan"].as_str().unwrap().to_owned();
    // A second client hears the load's progress.
    let mut listener = rig.sandbox.client();
    listener.subscribe().unwrap();

    let run = rig.load(0, &[&rig.set, &rig.other], &plan);

    assert!(run.status.success(), "{}", run.stderr);
    let load = run.json();
    assert_eq!(load["outcome"], "loaded", "{load}");
    assert_eq!(
        load["planned"], dry["changes"],
        "the load's plan is its dry run's"
    );
    assert_eq!(
        titles(&load["written"]["tracks"]),
        ["Alpha", "Bravo", "Charlie"]
    );
    assert_eq!(load["not_written"]["tracks"], serde_json::json!([]));
    assert_eq!(
        rig.on_stick(0),
        [
            (
                "Set".to_owned(),
                vec!["Alpha".to_owned(), "Bravo".to_owned()]
            ),
            ("Other".to_owned(), vec!["Charlie".to_owned()]),
        ]
    );
    let mut named = Vec::new();
    while let Some(event) = listener.next_event(Duration::from_millis(500)).unwrap() {
        if event.event == "stick_load_progress" {
            assert_eq!(event.data["stick"], rig.path(0));
            if let Some(title) = event.data["track"]["title"].as_str() {
                named.push(title.to_owned());
            }
        }
    }
    for title in ["Alpha", "Bravo", "Charlie"] {
        assert!(
            named.iter().any(|t| t == title),
            "{title} not named: {named:?}"
        );
    }
    let _ = (&rig.a, &rig.b, &rig.c);
}

#[test]
fn a_second_dry_run_names_changed_fields_and_what_comes_off() {
    let rig = Rig::new(&["FRIDAY"]);
    rig.unlock(0);
    rig.load_confirmed(0, &[&rig.set, &rig.other]);
    let rated = rig.sandbox.cli(&["tracks", "rate", &rig.a, "4"]);
    assert!(rated.status.success(), "{}", rated.stderr);

    let dry = rig.dry_run(0, &[&rig.set]);

    let changes = &dry["changes"];
    assert_eq!(changes["tracks"]["added"], serde_json::json!([]));
    let changed = changes["tracks"]["changed"].as_array().unwrap();
    assert_eq!(changed.len(), 1, "{changes}");
    assert_eq!(changed[0]["title"], "Alpha");
    assert_eq!(changed[0]["fields"], serde_json::json!(["Energy"]));
    assert_eq!(titles(&changes["tracks"]["removed"]), ["Charlie"]);
    assert_eq!(
        changes["playlists"]["removed"],
        serde_json::json!(["Other"])
    );
    assert_eq!(changes["counts"]["tracks_changed"], 1);
    assert_eq!(changes["counts"]["tracks_removed"], 1);

    let load = rig.load(0, &[&rig.set], dry["plan"].as_str().unwrap());
    assert!(load.status.success(), "{}", load.stderr);
    assert_eq!(
        rig.on_stick(0),
        [(
            "Set".to_owned(),
            vec!["Alpha".to_owned(), "Bravo".to_owned()]
        )]
    );
}

#[test]
fn a_load_that_would_not_fit_is_refused_before_writing_saying_by_how_much() {
    let rig = Rig::with_free(&["FRIDAY"], Some(1_000));
    rig.unlock(0);
    let dry = rig.dry_run(0, &[&rig.set]);
    let short = dry["space"]["shortfall_bytes"]
        .as_u64()
        .expect(&dry.to_string());
    assert!(short > 0);
    let before = snapshot(&rig.paths[0]);

    // The load checks the room itself, before anything is written.
    let refused = rig.load(0, &[&rig.set], dry["plan"].as_str().unwrap());
    assert!(!refused.status.success());
    let error = &refused.error_json()["error"];
    assert_eq!(error["code"], "stick_too_small", "{}", refused.stderr);
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains(&short.to_string()),
        "says by how much: {error}"
    );
    let text = rig
        .sandbox
        .cli(&["stick", "dry-run", rig.path(0), &rig.set]);
    assert!(text.stdout.contains("too little room"), "{}", text.stdout);
    assert_eq!(snapshot(&rig.paths[0]), before, "nothing written");
}

#[test]
fn a_failed_load_says_what_was_and_was_not_written() {
    let rig = Rig::new(&["FRIDAY"]);
    rig.unlock(0);
    rig.load_confirmed(0, &[&rig.other]);
    let dry = rig.dry_run(0, &[&rig.set, &rig.other]);
    // Stops the next load once it has started writing (test mode only).
    std::fs::write(
        rig.sandbox
            .state_dir()
            .join(wreckordbox_service::stick_load::TEST_FAIL_AFTER_FILE),
        "1",
    )
    .unwrap();

    let run = rig.load(0, &[&rig.set, &rig.other], dry["plan"].as_str().unwrap());

    assert!(!run.status.success(), "a failed load fails");
    let load = run.json();
    assert_eq!(load["outcome"], "failed", "{load}");
    assert!(load["error"]["message"].is_string(), "{load}");
    assert_eq!(titles(&load["written"]["tracks"]), ["Charlie"], "{load}");
    assert_eq!(
        titles(&load["not_written"]["tracks"]),
        ["Alpha", "Bravo"],
        "{load}"
    );
    assert_eq!(load["written"]["playlists"], serde_json::json!(["Other"]));
    // The Stick holds what it held before: the earlier load, whole.
    assert_eq!(
        rig.on_stick(0),
        [("Other".to_owned(), vec!["Charlie".to_owned()])]
    );
}

#[test]
fn only_a_stick_is_loaded() {
    let rig = Rig::new(&[]);
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().to_str().unwrap();

    let refused = rig
        .sandbox
        .cli(&["--json", "stick", "dry-run", path, &rig.set]);

    assert!(!refused.status.success());
    assert_eq!(
        refused.error_json()["error"]["code"],
        "not_a_stick",
        "{}",
        refused.stderr
    );
    assert_eq!(snapshot(folder.path()), Vec::new());
}

/// A 16-bit stereo 44.1 kHz WAV tone of `frames` sample frames.
fn write_tone(path: &Path, frames: u32, hertz: f64) {
    let data_len = frames * 4;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&44_100_u32.to_le_bytes());
    out.extend_from_slice(&(44_100_u32 * 4).to_le_bytes());
    out.extend_from_slice(&4_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for frame in 0..frames {
        let t = f64::from(frame) / 44_100.0;
        let sample = ((t * hertz * std::f64::consts::TAU).sin() * 12_000.0) as i16;
        out.extend_from_slice(&sample.to_le_bytes());
        out.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}

/// Records four synthetic Stems for the track whose audio is `audio`, made
/// from that audio as it is now. Before any service starts.
fn record_stems(sandbox: &Sandbox, track: &str, audio: &Path) {
    use wreckordbox_sidecar::stems::{NewStems, Role};
    use wreckordbox_sidecar::{Sidecar, Source, SourceKind};
    // The imported tracks are two seconds long.
    let frames = 2 * 44_100;
    let sidecar = Sidecar::open(&sandbox.sidecar_dir()).unwrap();
    let source = Source::new(SourceKind::Tool, "separation-engine");
    let run = sidecar
        .begin_run(&source, "Synthetic Stems for a test")
        .unwrap();
    let workspace = sidecar.workspace(run).unwrap();
    let files: Vec<(Role, PathBuf)> = Role::ALL
        .into_iter()
        .zip([880.0, 660.0, 110.0, 220.0])
        .map(|(role, hertz)| {
            let path = workspace.join(format!("{role}.wav"));
            write_tone(&path, frames, hertz);
            (role, path)
        })
        .collect();
    let fingerprint = wreckordbox_sidecar::fingerprint_file(audio).unwrap();
    sidecar
        .record_stems(&NewStems {
            track_id: track,
            fingerprint: &fingerprint,
            parent_samples: u64::from(frames),
            sample_rate: 44_100,
            files: &files,
            source: &source,
            produced_by: serde_json::json!({"engine": "test"}),
            run,
        })
        .unwrap();
    sidecar.finish_run(run, "recorded").unwrap();
}

#[test]
fn a_spare_stick_gets_the_native_stems_of_tracks_that_have_them() {
    let rig = Rig::new(&["SPARE*", "FRIDAY"]);
    record_stems(
        &rig.sandbox,
        &rig.a,
        &rig.sandbox.path().join("audio").join("Alpha.wav"),
    );
    rig.unlock(0);

    let dry = rig.dry_run(0, &[&rig.set]);
    assert_eq!(titles(&dry["changes"]["native_stems"]), ["Alpha"], "{dry}");
    assert_eq!(dry["changes"]["counts"]["native_stems"], 1);
    // A Gig stick never gets Native stems.
    let gig = rig.dry_run(1, &[&rig.set]);
    assert_eq!(gig["changes"]["native_stems"], serde_json::json!([]));

    let run = rig.load(0, &[&rig.set], dry["plan"].as_str().unwrap());

    assert!(run.status.success(), "{}", run.stderr);
    let load = run.json();
    assert_eq!(load["outcome"], "loaded", "{load}");
    assert_eq!(load["native_stems"][0]["track"]["title"], "Alpha");
    assert_eq!(load["native_stems"][0]["outcome"], "written", "{load}");
    let folder = rig.paths[0].join("STEMS").join("Alpha");
    for file in ["vocals.ogg", "other.ogg", "bass.ogg", "drums.ogg"] {
        assert!(
            folder.join(file).is_file(),
            "{} missing",
            folder.join(file).display()
        );
    }
}
