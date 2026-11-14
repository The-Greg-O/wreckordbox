//! What a Stick preview reads (wbx-cne): a Spare-stick dry run fingerprints
//! only the tracks that have Stem Facts, and may reuse a fingerprint while
//! the audio file's identity is unchanged; a load always fingerprints
//! afresh under the Stick's lock, so audio changed behind an unchanged size
//! and modification time is still refused.
//!
//! The real binary against a service on a throwaway Library; every Stick is
//! a temporary folder. What the service read is its own `/proc/<pid>/io`
//! `rchar` (bytes read through read-like system calls, any file).
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::common;
// The independent Native stems reader, as `stems_write` also includes it.
#[allow(clippy::duplicate_mod)]
#[path = "../../../wreckordbox-native-stems/tests/deck/mod.rs"]
mod deck;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::Sandbox;
use serde_json::Value;

/// A Library of `tracks` silent audio files of `seconds` each in one
/// playlist "Set", and one Spare stick.
struct Rig {
    sandbox: Sandbox,
    _sticks: tempfile::TempDir,
    stick: PathBuf,
    /// Each track's id and audio file, in playlist order.
    tracks: Vec<(String, PathBuf)>,
    set: String,
}

impl Rig {
    fn new(tracks: usize, seconds: u32) -> Self {
        let mut sandbox = Sandbox::empty();
        // Long enough to keep one service up across the dry runs measured.
        sandbox.idle_exit_secs = 120;
        let sticks = tempfile::tempdir().unwrap();
        let stick = sticks.path().join("SPARE");
        std::fs::create_dir(&stick).unwrap();
        std::fs::write(
            stick.join(wreckordbox_stick_protection::STAND_IN_MARKER),
            b"",
        )
        .unwrap();
        std::fs::write(stick.join(wreckordbox_stick_protection::SPARE_MARKER), b"").unwrap();
        sandbox.sticks = vec![stick.clone()];
        let audio = sandbox.path().join("audio");
        std::fs::create_dir_all(&audio).unwrap();
        let paths: Vec<PathBuf> = (0..tracks)
            .map(|i| {
                let path = audio.join(format!("Track {i:04}.wav"));
                write_wav(&path, seconds, 0);
                path
            })
            .collect();
        let mut ids = Vec::new();
        let mut set = String::new();
        sandbox.edit(|writer| {
            for path in &paths {
                ids.push(writer.import_file(path).unwrap());
            }
            set = writer.create_playlist("Set", "root").unwrap();
            writer.add_tracks(&set, &ids).unwrap();
        });
        Self {
            sandbox,
            _sticks: sticks,
            stick,
            tracks: ids.into_iter().zip(paths).collect(),
            set,
        }
    }

    fn stick(&self) -> &str {
        self.stick.to_str().unwrap()
    }

    /// The DJ unlocks the Spare stick.
    fn unlock(&self) {
        let run = self.sandbox.cli(&["stick", "unlock", self.stick()]);
        assert!(run.status.success(), "{}", run.stderr);
    }

    /// The service's pid, starting it if need be.
    fn service(&self) -> u64 {
        self.sandbox.status()["pid"].as_u64().unwrap()
    }

    fn dry_run(&self) -> Value {
        let run = self
            .sandbox
            .cli(&["--json", "stick", "dry-run", self.stick(), &self.set]);
        assert!(run.status.success(), "{}", run.stderr);
        run.json()
    }

    /// A dry run, with the bytes the service read during it and how long
    /// it took.
    fn measured_dry_run(&self) -> (Value, u64, std::time::Duration) {
        let pid = self.service();
        let before = read_bytes(pid);
        let started = Instant::now();
        let dry = self.dry_run();
        let took = started.elapsed();
        assert_eq!(self.service(), pid, "one service throughout");
        (dry, read_bytes(pid) - before, took)
    }

    fn load(&self, plan: &str) -> common::Run {
        self.sandbox.cli(&[
            "--json",
            "stick",
            "load",
            self.stick(),
            &self.set,
            "--plan",
            plan,
        ])
    }

    fn audio_bytes(&self) -> u64 {
        self.tracks
            .iter()
            .map(|(_, path)| std::fs::metadata(path).unwrap().len())
            .sum()
    }
}

/// Bytes the process `pid` has read so far, from any file or socket.
fn read_bytes(pid: u64) -> u64 {
    let io = std::fs::read_to_string(format!("/proc/{pid}/io")).unwrap();
    io.lines()
        .find_map(|line| line.strip_prefix("rchar: "))
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

/// A mono 16-bit 8 kHz WAV of `seconds`, every sample `level`.
fn write_wav(path: &Path, seconds: u32, level: i16) {
    let rate: u32 = 8_000;
    let data_len = rate * 2 * seconds;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for _ in 0..rate * seconds {
        out.extend_from_slice(&level.to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}

/// A stereo 16-bit 44.1 kHz tone of `frames`, peaking at `amplitude`.
fn write_tone(path: &Path, frames: u32, hertz: f64, amplitude: f64) {
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
        let sample = ((t * hertz * std::f64::consts::TAU).sin() * amplitude) as i16;
        out.extend_from_slice(&sample.to_le_bytes());
        out.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}

/// How loud each role's tone is in an ordinary synthetic set: vocals,
/// other, bass, drums.
const LEVELS: [f64; 4] = [12_000.0, 12_000.0, 12_000.0, 12_000.0];

/// Records four synthetic Stems of `seconds` for the track whose audio is
/// `audio`, made from that audio as it is now, each role's tone at its
/// level in `levels` (so two sets can be told apart once encoded). Answers
/// the set's Fact id. Before any service starts.
fn record_stems(
    sandbox: &Sandbox,
    track: &str,
    audio: &Path,
    seconds: u32,
    levels: [f64; 4],
) -> i64 {
    use wreckordbox_sidecar::stems::{NewStems, Role};
    use wreckordbox_sidecar::{Sidecar, Source, SourceKind};
    let frames = seconds * 44_100;
    let sidecar = Sidecar::open(&sandbox.sidecar_dir()).unwrap();
    let source = Source::new(SourceKind::Tool, "separation-engine");
    let run = sidecar
        .begin_run(&source, "Synthetic Stems for a test")
        .unwrap();
    let workspace = sidecar.workspace(run).unwrap();
    let files: Vec<(Role, PathBuf)> = Role::ALL
        .into_iter()
        .zip([880.0, 660.0, 110.0, 220.0])
        .zip(levels)
        .map(|((role, hertz), level)| {
            let path = workspace.join(format!("{role}.wav"));
            write_tone(&path, frames, hertz, level);
            (role, path)
        })
        .collect();
    let fingerprint = wreckordbox_sidecar::fingerprint_file(audio).unwrap();
    let set = sidecar
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
    set.fact_id
}

/// Plants `fingerprint` as the service's kept preview fingerprint of
/// `path`, under the file's identity when the next dry run or load reads it
/// (test mode): a wrong kept value no identity check would catch.
fn inject_preview_fingerprint(sandbox: &Sandbox, path: &Path, fingerprint: &str) {
    std::fs::write(
        sandbox
            .state_dir()
            .join(wreckordbox_service::preview_fingerprints::TEST_INJECT_FILE),
        format!("{fingerprint}\t{}\n", path.display()),
    )
    .unwrap();
}

/// Starts a load that pauses at `stage` and waits until it has.
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
            rig.stick(),
            &rig.set,
            "--plan",
            plan,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let paused = state.join(wreckordbox_service::stick_load::TEST_PAUSED_FILE);
    let until = Instant::now() + Duration::from_secs(60);
    while !paused.exists() {
        assert!(Instant::now() < until, "the load never reached {stage}");
        std::thread::sleep(Duration::from_millis(20));
    }
    child
}

fn release_pause(rig: &Rig) {
    std::fs::remove_file(
        rig.sandbox
            .state_dir()
            .join(wreckordbox_service::stick_load::TEST_PAUSE_AT_FILE),
    )
    .unwrap();
}

fn finished(child: std::process::Child) -> Value {
    let mut child = child;
    let until = Instant::now() + Duration::from_secs(60);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= until {
            let _ = child.kill();
            panic!("the load did not end");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output().unwrap();
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("no JSON: {}", String::from_utf8_lossy(&output.stderr)))
}

/// Every file under `dir`, relative, for "nothing was written there".
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path.strip_prefix(dir).unwrap().to_path_buf());
            }
        }
    }
    out.sort();
    out
}

/// Rewrites `path` with other audio of the same length, then puts its
/// modification time back: only its content (and its change time, which no
/// one can set) says it changed.
fn change_audio_keeping_size_and_mtime(path: &Path, seconds: u32) {
    let before = std::fs::metadata(path).unwrap();
    let mtime = before.modified().unwrap();
    write_wav(path, seconds, 1_000);
    let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified(mtime).unwrap();
    drop(file);
    let after = std::fs::metadata(path).unwrap();
    assert_eq!(after.len(), before.len());
    assert_eq!(after.modified().unwrap(), mtime);
    assert_eq!(
        std::os::unix::fs::MetadataExt::ino(&after),
        std::os::unix::fs::MetadataExt::ino(&before)
    );
}

fn titles(lines: &Value) -> Vec<String> {
    lines
        .as_array()
        .unwrap_or_else(|| panic!("not a list: {lines}"))
        .iter()
        .map(|line| line["title"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_spare_stick_dry_run_reads_no_audio_of_tracks_without_stems() {
    // Forty tracks of 30 s: 19 MB of audio, far more than the Library, the
    // Stick and the Sidecar together.
    let rig = Rig::new(40, 30);
    let audio = rig.audio_bytes();

    let (dry, read, _) = rig.measured_dry_run();

    assert_eq!(dry["changes"]["counts"]["tracks_added"], 40, "{dry}");
    assert_eq!(dry["changes"]["native_stems"], serde_json::json!([]));
    assert!(
        read < audio / 10,
        "the dry run read {read} bytes; the tracks' audio is {audio} bytes and none has Stems"
    );
}

#[test]
fn audio_changed_behind_its_size_and_mtime_is_not_previewed_as_current() {
    let rig = Rig::new(3, 2);
    let (alpha, alpha_audio) = &rig.tracks[0];
    record_stems(&rig.sandbox, alpha, alpha_audio, 2, LEVELS);
    let first = rig.dry_run();
    assert_eq!(titles(&first["changes"]["native_stems"]), ["Track 0000"]);

    change_audio_keeping_size_and_mtime(alpha_audio, 2);
    let second = rig.dry_run();

    assert_eq!(
        second["changes"]["native_stems"],
        serde_json::json!([]),
        "the Stems were made from other audio: {second}"
    );
}

#[test]
fn audio_changed_behind_its_size_and_mtime_after_the_dry_run_is_refused_at_load() {
    let rig = Rig::new(3, 2);
    let (alpha, alpha_audio) = &rig.tracks[0];
    record_stems(&rig.sandbox, alpha, alpha_audio, 2, LEVELS);
    rig.unlock();
    let dry = rig.dry_run();
    assert_eq!(titles(&dry["changes"]["native_stems"]), ["Track 0000"]);

    change_audio_keeping_size_and_mtime(alpha_audio, 2);
    let refused = rig.load(dry["plan"].as_str().unwrap());

    assert!(!refused.status.success(), "{}", refused.stdout);
    let error = &refused.error_json()["error"];
    assert_eq!(error["code"], "stick_load_changed", "{}", refused.stderr);
    let message = error["message"].as_str().unwrap();
    assert!(message.contains("native_stems"), "{message}");
    assert!(
        !rig.stick.join("STEMS").exists(),
        "no Stems written to the Stick"
    );
    assert!(
        !rig.stick.join("PIONEER").exists() && !rig.stick.join("Contents").exists(),
        "nothing loaded"
    );
}

#[test]
fn a_wrong_kept_fingerprint_is_refused_when_the_load_plans_again() {
    let rig = Rig::new(3, 2);
    let (alpha, alpha_audio) = &rig.tracks[0];
    let made_from = wreckordbox_sidecar::fingerprint_file(alpha_audio).unwrap();
    record_stems(&rig.sandbox, alpha, alpha_audio, 2, LEVELS);
    rig.unlock();
    // The audio changes, so the Stems are stale; the kept fingerprint, under
    // the file's complete identity as it is now, still says the old audio.
    write_wav(alpha_audio, 2, 1_000);
    inject_preview_fingerprint(&rig.sandbox, alpha_audio, made_from.as_str());
    let dry = rig.dry_run();
    assert_eq!(
        titles(&dry["changes"]["native_stems"]),
        ["Track 0000"],
        "the preview is fooled: {dry}"
    );

    let before = files_under(&rig.stick);

    let refused = rig.load(dry["plan"].as_str().unwrap());

    assert!(!refused.status.success(), "{}", refused.stdout);
    let error = &refused.error_json()["error"];
    assert_eq!(error["code"], "stick_load_changed", "{}", refused.stderr);
    assert!(
        error["message"].as_str().unwrap().contains("native_stems"),
        "{error}"
    );
    assert_eq!(
        files_under(&rig.stick),
        before,
        "nothing written to the Stick"
    );
}

#[test]
fn a_wrong_kept_fingerprint_is_refused_by_the_check_before_each_stem_set() {
    let rig = Rig::new(3, 2);
    let (alpha, alpha_audio) = &rig.tracks[0];
    let made_from = wreckordbox_sidecar::fingerprint_file(alpha_audio).unwrap();
    record_stems(&rig.sandbox, alpha, alpha_audio, 2, LEVELS);
    rig.unlock();
    let dry = rig.dry_run();
    assert_eq!(titles(&dry["changes"]["native_stems"]), ["Track 0000"]);
    // The load has planned and copied the audio; before its Native stems,
    // the track's audio changes while a wrong kept fingerprint says it has
    // not.
    let child = paused_load(&rig, "publishing", dry["plan"].as_str().unwrap());
    write_wav(alpha_audio, 2, 1_000);
    inject_preview_fingerprint(&rig.sandbox, alpha_audio, made_from.as_str());
    release_pause(&rig);

    let load = finished(child);

    assert_eq!(load["native_stems"][0]["outcome"], "not_written", "{load}");
    assert!(
        load["native_stems"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("changed since the dry run"),
        "{load}"
    );
    assert!(
        files_under(&rig.stick.join("STEMS")).is_empty(),
        "no Stems published: {:?}",
        files_under(&rig.stick)
    );
}

#[test]
fn a_load_after_repeated_previews_publishes_exactly_the_stems_it_approved() {
    let rig = Rig::new(3, 2);
    let (alpha, alpha_audio) = &rig.tracks[0];
    // An earlier set, superseded, loud where the approved one is quiet.
    let earlier = [16_000.0, 12_000.0, 8_000.0, 4_000.0];
    let approved_levels = [4_000.0, 8_000.0, 12_000.0, 16_000.0];
    let superseded = record_stems(&rig.sandbox, alpha, alpha_audio, 2, earlier);
    let approved = record_stems(&rig.sandbox, alpha, alpha_audio, 2, approved_levels);
    assert_ne!(superseded, approved);
    rig.unlock();
    rig.dry_run();
    let dry = rig.dry_run();
    assert_eq!(titles(&dry["changes"]["native_stems"]), ["Track 0000"]);

    let run = rig.load(dry["plan"].as_str().unwrap());

    assert!(run.status.success(), "{}", run.stderr);
    let load = run.json();
    assert_eq!(load["outcome"], "loaded", "{load}");
    assert_eq!(load["native_stems"][0]["outcome"], "written", "{load}");
    assert_eq!(load["native_stems"][0]["stems_fact"], approved, "{load}");
    // What the Stick holds, decoded by the independent reader: each role at
    // the approved set's level, not the superseded set's.
    let folder = rig.stick.join("STEMS").join("Track 0000");
    for (file, level) in ["vocals.ogg", "other.ogg", "bass.ogg", "drums.ogg"]
        .into_iter()
        .zip(approved_levels)
    {
        let decoded = deck::decode(&folder.join(file))
            .unwrap_or_else(|| panic!("{file} does not decode: {:?}", files_under(&rig.stick)));
        let peak = f64::from(decoded.peak(0, decoded.frames));
        assert!(
            (peak - level).abs() < level * 0.15,
            "{file} peaks at {peak}, the approved set's tone is {level}"
        );
    }
}

/// The measurement the ticket asks for, not a check: run it by name with
/// `--ignored --nocapture`. 2,000 tracks, five with Stems; a cold dry run,
/// then a warm one in the same service.
#[test]
#[ignore = "a measurement, run by hand"]
fn measure_a_two_thousand_track_spare_stick_dry_run() {
    let rig = Rig::new(2_000, 2);
    for (id, audio) in rig.tracks.iter().step_by(400) {
        record_stems(&rig.sandbox, id, audio, 2, LEVELS);
    }
    let audio = rig.audio_bytes();
    let (first, cold, cold_time) = rig.measured_dry_run();
    let (_, warm, warm_time) = rig.measured_dry_run();
    assert_eq!(first["changes"]["counts"]["native_stems"], 5, "{first}");
    eprintln!(
        "MEASURE tracks=2000 with_stems=5 audio_bytes={audio} \
         cold_read={cold} cold_ms={} warm_read={warm} warm_ms={}",
        cold_time.as_millis(),
        warm_time.as_millis()
    );
}
