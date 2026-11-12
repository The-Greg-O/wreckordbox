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
        if let Some(free) = free {
            sandbox.set_stick_free_bytes(free);
        }
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
        let plan = dry["plan"]
            .as_str()
            .unwrap_or_else(|| panic!("no plan: {dry}"))
            .to_owned();
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

/// Every file and folder under `root`, each file with its bytes: the whole
/// state of a folder, to show nothing was written, made or removed.
fn snapshot(root: &Path) -> Vec<(String, Option<Vec<u8>>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let entry = entry.unwrap();
            let name = entry.path().display().to_string();
            if entry.file_type().unwrap().is_dir() {
                out.push((format!("{name}/"), None));
                stack.push(entry.path());
            } else {
                out.push((name, Some(std::fs::read(entry.path()).unwrap_or_default())));
            }
        }
    }
    out.sort();
    out
}

fn assert_unchanged(root: &Path, before: &[(String, Option<Vec<u8>>)], what: &str) {
    let after = snapshot(root);
    let names = |s: &[(String, Option<Vec<u8>>)]| -> Vec<String> {
        s.iter().map(|(n, _)| n.clone()).collect()
    };
    assert_eq!(
        names(&after),
        names(before),
        "{what}: files or folders made or removed"
    );
    assert!(after == before, "{what}: a file's bytes changed");
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
        .unwrap_or_else(|| panic!("no shortfall: {dry}"));
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

impl Rig {
    /// Reverses the entries of the playlist `playlist`.
    fn reverse(&self, playlist: &str) {
        let tracks = self.json(&["playlists", "tracks", playlist]);
        let mut entries: Vec<String> = tracks["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["entry_id"].as_str().unwrap().to_owned())
            .collect();
        entries.reverse();
        let mut args = vec!["playlists", "reorder", playlist];
        args.extend(entries.iter().map(String::as_str));
        let run = self.sandbox.cli(&args);
        assert!(run.status.success(), "{}", run.stderr);
    }

    /// Arms the test-mode hook that stops the next load once it has
    /// started writing.
    fn fail_next_load(&self) {
        std::fs::write(
            self.sandbox
                .state_dir()
                .join(wreckordbox_service::stick_load::TEST_FAIL_AFTER_FILE),
            "1",
        )
        .unwrap();
    }
}

#[test]
fn a_failed_load_tells_changes_not_made_from_content_that_needed_none() {
    let rig = Rig::new(&["FRIDAY"]);
    rig.unlock(0);
    rig.load_confirmed(0, &[&rig.set, &rig.other]);
    // An existing track's field and an existing playlist's order change.
    let rated = rig.sandbox.cli(&["tracks", "rate", &rig.a, "4"]);
    assert!(rated.status.success(), "{}", rated.stderr);
    rig.reverse(&rig.set);
    let dry = rig.dry_run(0, &[&rig.set, &rig.other]);
    assert_eq!(
        dry["changes"]["playlists"]["reordered"],
        serde_json::json!(["Set"]),
        "{dry}"
    );
    rig.fail_next_load();

    let run = rig.load(0, &[&rig.set, &rig.other], dry["plan"].as_str().unwrap());

    assert!(!run.status.success(), "a failed load fails");
    let load = run.json();
    assert_eq!(load["outcome"], "failed", "{load}");
    assert!(load["error"]["message"].is_string(), "{load}");
    // The old Energy and the old order are still on the Stick: not written,
    // and nothing is claimed written.
    assert_eq!(titles(&load["not_written"]["tracks"]), ["Alpha"], "{load}");
    assert_eq!(
        load["not_written"]["playlists"],
        serde_json::json!(["Set"]),
        "{load}"
    );
    assert_eq!(load["written"]["tracks"], serde_json::json!([]), "{load}");
    assert_eq!(
        load["written"]["playlists"],
        serde_json::json!([]),
        "{load}"
    );
    // What needed no change is neither.
    assert_eq!(
        titles(&load["unchanged"]["tracks"]),
        ["Bravo", "Charlie"],
        "{load}"
    );
    assert_eq!(
        load["unchanged"]["playlists"],
        serde_json::json!(["Other"]),
        "{load}"
    );
    assert_eq!(
        rig.on_stick(0),
        [
            (
                "Set".to_owned(),
                vec!["Alpha".to_owned(), "Bravo".to_owned()]
            ),
            ("Other".to_owned(), vec!["Charlie".to_owned()]),
        ],
        "the Stick holds what it held before"
    );

    // Done for real, the same changes read back as written.
    let dry = rig.dry_run(0, &[&rig.set, &rig.other]);
    let load = rig.load(0, &[&rig.set, &rig.other], dry["plan"].as_str().unwrap());
    assert!(load.status.success(), "{}", load.stderr);
    let load = load.json();
    assert_eq!(titles(&load["written"]["tracks"]), ["Alpha"], "{load}");
    assert_eq!(
        load["written"]["playlists"],
        serde_json::json!(["Set"]),
        "{load}"
    );
    assert_eq!(
        load["not_written"]["tracks"],
        serde_json::json!([]),
        "{load}"
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

#[test]
fn a_dry_run_writes_nothing_to_a_populated_wal_unfinished_or_spare_stick_or_the_sidecar() {
    let mut rig = Rig::new(&["FRIDAY", "SPARE*", "WAL", "WAL2", "HALF"]);
    // The WAL fixtures rename a playlist called "Warm Up".
    let mut warm_up = String::new();
    let c = rig.c.clone();
    rig.sandbox.edit(|writer| {
        warm_up = writer.create_playlist("Warm Up", "root").unwrap();
        writer
            .add_tracks(&warm_up, std::slice::from_ref(&c))
            .unwrap();
    });
    rig.other = warm_up;
    for stick in 0..5 {
        rig.unlock(stick);
        rig.load_confirmed(stick, &[&rig.set, &rig.other]);
    }
    let rated = rig.sandbox.cli(&["tracks", "rate", &rig.a, "4"]);
    assert!(rated.status.success(), "{}", rated.stderr);

    // A populated Gig stick with changes to make.
    let before = snapshot(&rig.paths[0]);
    let dry = rig.dry_run(0, &[&rig.set]);
    assert_eq!(dry["changes"]["counts"]["tracks_changed"], 1, "{dry}");
    assert_unchanged(&rig.paths[0], &before, "a populated Stick");

    // A populated Spare stick (its Sidecar case: the test below).
    let sidecar = rig.sandbox.sidecar_dir();
    let before = snapshot(&rig.paths[1]);
    let sidecar_before = snapshot(&sidecar);
    rig.dry_run(1, &[&rig.set]);
    assert_unchanged(&rig.paths[1], &before, "a Spare stick");
    assert_unchanged(&sidecar, &sidecar_before, "the Sidecar");

    // WAL mode, checkpointed and with a pending -wal: no plan id, said why.
    wreckordbox_stick_outline::testing::wal_checkpointed(&rig.paths[2]);
    let before = snapshot(&rig.paths[2]);
    let dry = rig.dry_run(2, &[&rig.set, &rig.other]);
    assert_eq!(
        dry["plan"],
        Value::Null,
        "an incomplete plan is never executable: {dry}"
    );
    assert!(
        dry["not_loadable"].as_str().unwrap().contains("WAL"),
        "{dry}"
    );
    assert_unchanged(&rig.paths[2], &before, "a WAL-mode Stick");
    let text = rig
        .sandbox
        .cli(&["stick", "dry-run", rig.path(2), &rig.set]);
    assert!(
        text.stdout.contains("It cannot be loaded"),
        "{}",
        text.stdout
    );
    wreckordbox_stick_outline::testing::wal_pending(&rig.paths[3]);
    let before = snapshot(&rig.paths[3]);
    let dry = rig.dry_run(3, &[&rig.set]);
    assert_eq!(dry["plan"], Value::Null, "{dry}");
    assert_unchanged(&rig.paths[3], &before, "a Stick with a pending -wal");

    // An unfinished publication: refused, nothing settled.
    wreckordbox_stick_outline::testing::interrupt(&rig.paths[4]);
    let before = snapshot(&rig.paths[4]);
    let refused = rig
        .sandbox
        .cli(&["--json", "stick", "dry-run", rig.path(4), &rig.set]);
    assert_eq!(
        refused.error_json()["error"]["code"],
        "stick_unfinished_write",
        "{}",
        refused.stderr
    );
    assert_unchanged(&rig.paths[4], &before, "a Stick with an unfinished write");
}

#[test]
fn a_dry_run_on_a_spare_stick_never_opens_a_writing_sidecar() {
    let rig = Rig::new(&["SPARE*"]);
    // The service starts with no Sidecar; one appears after it started.
    let listed = rig.json(&["stick", "list"]);
    assert_eq!(listed["sticks"].as_array().unwrap().len(), 1);
    record_stems(
        &rig.sandbox,
        &rig.a,
        &rig.sandbox.path().join("audio").join("Alpha.wav"),
    );
    let sidecar = rig.sandbox.sidecar_dir();
    let before = snapshot(&sidecar);
    let stick = snapshot(&rig.paths[0]);

    let dry = rig.dry_run(0, &[&rig.set]);

    assert_eq!(titles(&dry["changes"]["native_stems"]), ["Alpha"], "{dry}");
    assert_unchanged(&sidecar, &before, "the Sidecar");
    assert_unchanged(&rig.paths[0], &stick, "the Spare stick");
}

#[test]
fn stems_finished_between_the_dry_run_and_the_load_are_refused() {
    let rig = Rig::new(&["SPARE*"]);
    rig.unlock(0);
    let dry = rig.dry_run(0, &[&rig.set]);
    assert_eq!(
        dry["changes"]["native_stems"],
        serde_json::json!([]),
        "{dry}"
    );
    // A separation finishes: the Library generation does not move.
    let separated = rig.sandbox.cli(&["--json", "stems", "separate", &rig.a]);
    assert!(separated.status.success(), "{}", separated.stderr);
    let before = snapshot(&rig.paths[0]);

    let refused = rig.load(0, &[&rig.set], dry["plan"].as_str().unwrap());

    assert!(!refused.status.success());
    let error = &refused.error_json()["error"];
    assert_eq!(error["code"], "stick_load_changed", "{}", refused.stderr);
    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains("native_stems: now #"),
        "says what changed: {message}"
    );
    assert!(message.contains("Alpha"), "{message}");
    assert_unchanged(&rig.paths[0], &before, "nothing written");
    // A fresh dry run reports the Native stems and loads them.
    let dry = rig.dry_run(0, &[&rig.set]);
    assert_eq!(titles(&dry["changes"]["native_stems"]), ["Alpha"], "{dry}");
}

#[test]
fn a_source_audio_file_changed_between_the_dry_run_and_the_load_is_refused() {
    let rig = Rig::new(&["FRIDAY"]);
    rig.unlock(0);
    let dry = rig.dry_run(0, &[&rig.set]);
    let audio = rig.sandbox.path().join("audio").join("Bravo.wav");
    let mut bytes = std::fs::read(&audio).unwrap();
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    std::fs::write(&audio, bytes).unwrap();
    let before = snapshot(&rig.paths[0]);

    let refused = rig.load(0, &[&rig.set], dry["plan"].as_str().unwrap());

    let error = &refused.error_json()["error"];
    assert_eq!(error["code"], "stick_load_changed", "{}", refused.stderr);
    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains("tracks: now") && message.contains("Bravo"),
        "{message}"
    );
    assert!(
        message.contains("dry run") || message.contains("again"),
        "{message}"
    );
    assert_unchanged(&rig.paths[0], &before, "nothing written");
}

#[test]
fn a_repeat_load_whose_native_stems_do_not_fit_large_clusters_is_refused() {
    let rig = Rig::new(&["SPARE*"]);
    record_stems(
        &rig.sandbox,
        &rig.a,
        &rig.sandbox.path().join("audio").join("Alpha.wav"),
    );
    rig.unlock(0);
    let first = rig.load_confirmed(0, &[&rig.set]);
    assert_eq!(first["native_stems"][0]["outcome"], "written", "{first}");
    // exFAT on a large Stick: 128 KiB clusters, and little room left.
    let cluster = 128 * 1024;
    rig.sandbox.set_stick_cluster_bytes(cluster);
    rig.sandbox.set_stick_free_bytes(600_000);

    let dry = rig.dry_run(0, &[&rig.set]);

    // Four Stem files, the identity record and the folder, each at least a
    // cluster, written beside the set they replace.
    let stems = dry["space"]["native_stems_bytes"].as_u64().unwrap();
    assert!(stems >= 6 * cluster, "{dry}");
    assert!(dry["space"]["shortfall_bytes"].as_u64().is_some(), "{dry}");
    let before = snapshot(&rig.paths[0]);
    let refused = rig.load(0, &[&rig.set], dry["plan"].as_str().unwrap());
    assert_eq!(
        refused.error_json()["error"]["code"],
        "stick_too_small",
        "{}",
        refused.stderr
    );
    assert_unchanged(&rig.paths[0], &before, "nothing written");
}

#[test]
fn a_title_two_tracks_share_is_reported_with_an_unknown_id() {
    let mut sandbox_tracks = Rig::new(&["FRIDAY"]);
    let twin = sandbox_tracks.sandbox.import_track("twin/Alpha.wav", 2);
    let mut both = String::new();
    let a = sandbox_tracks.a.clone();
    sandbox_tracks.sandbox.edit(|writer| {
        both = writer.create_playlist("Twins", "root").unwrap();
        writer
            .add_tracks(&both, &[a.clone(), twin.clone()])
            .unwrap();
    });
    sandbox_tracks.set = both;
    let rig = sandbox_tracks;
    rig.unlock(0);
    let dry = rig.dry_run(0, &[&rig.set]);
    let mut listener = rig.sandbox.client();
    listener.subscribe().unwrap();

    let run = rig.load(0, &[&rig.set], dry["plan"].as_str().unwrap());

    assert!(run.status.success(), "{}", run.stderr);
    let mut alpha = Vec::new();
    while let Some(event) = listener.next_event(Duration::from_millis(500)).unwrap() {
        if event.event == "stick_load_progress" && event.data["track"]["title"] == "Alpha" {
            alpha.push(event.data["track"]["id"].clone());
        }
    }
    assert!(!alpha.is_empty(), "Alpha was named");
    assert!(
        alpha.iter().all(Value::is_null),
        "never a guessed id: {alpha:?}"
    );
}

/// Starts `stick load` in the background, paused at `stage` once it is
/// writing, and waits until it is.
fn paused_load(rig: &Rig, stage: &str, plan: &str) -> std::process::Child {
    let state = rig.sandbox.state_dir();
    std::fs::write(
        state.join(wreckordbox_service::stick_load::TEST_PAUSE_AT_FILE),
        stage,
    )
    .unwrap();
    let child = rig
        .sandbox
        .command()
        .args([
            "--json",
            "stick",
            "load",
            rig.path(0),
            &rig.set,
            "--plan",
            plan,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let paused = state.join(wreckordbox_service::stick_load::TEST_PAUSED_FILE);
    let until = std::time::Instant::now() + Duration::from_secs(60);
    while !paused.exists() {
        assert!(
            std::time::Instant::now() < until,
            "the load never reached {stage}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    child
}

/// The window taking the Library over: a `stop` sent to the service while
/// the load is paused at `stage`, then the load let go.
fn taken_over_at(stage: &str) -> (Rig, Value) {
    let rig = Rig::new(&["FRIDAY"]);
    rig.unlock(0);
    let dry = rig.dry_run(0, &[&rig.set]);
    let child = paused_load(&rig, stage, dry["plan"].as_str().unwrap());
    let mut window = rig.sandbox.client();
    window.stop().unwrap();
    std::fs::remove_file(
        rig.sandbox
            .state_dir()
            .join(wreckordbox_service::stick_load::TEST_PAUSE_AT_FILE),
    )
    .unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let load: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|_| {
        panic!(
            "the caller gets the load's report, not a disconnect: {stdout:?} {:?}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert!(
        !rig.paths[0].join(".rbxport-publication").exists(),
        "no unfinished write is left"
    );
    (rig, load)
}

#[test]
fn a_window_taking_over_during_copying_stops_the_load_with_a_full_report() {
    let (rig, load) = taken_over_at("copying");

    assert_eq!(load["outcome"], "failed", "{load}");
    assert_eq!(load["error"]["code"], "service_failed", "{load}");
    assert!(
        load["error"]["message"]
            .as_str()
            .unwrap()
            .contains("stopped"),
        "{load}"
    );
    assert_eq!(
        titles(&load["not_written"]["tracks"]),
        ["Alpha", "Bravo"],
        "{load}"
    );
    assert_eq!(load["written"]["tracks"], serde_json::json!([]), "{load}");
    assert!(
        wreckordbox_stick_outline::library(&rig.paths[0], Format::DeviceLibrary).is_err(),
        "nothing was published"
    );
}

#[test]
fn a_window_taking_over_during_publication_waits_for_it_and_reports_it() {
    let (rig, load) = taken_over_at("publishing");

    // Publication is not cut off: either it was done and is reported so,
    // or it was stopped before it began and nothing is on the Stick.
    match load["outcome"].as_str().unwrap() {
        "loaded" => assert_eq!(
            rig.on_stick(0),
            [(
                "Set".to_owned(),
                vec!["Alpha".to_owned(), "Bravo".to_owned()]
            )]
        ),
        "failed" => {
            assert_eq!(
                titles(&load["not_written"]["tracks"]),
                ["Alpha", "Bravo"],
                "{load}"
            );
            assert!(
                wreckordbox_stick_outline::library(&rig.paths[0], Format::DeviceLibrary).is_err()
            );
        }
        other => panic!("{other}: {load}"),
    }
}
