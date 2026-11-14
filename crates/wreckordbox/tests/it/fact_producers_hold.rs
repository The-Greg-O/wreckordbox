//! Background Fact producers hold the shared Fact coordination through
//! their commit (wbx-j8k): Review's accept never runs while a separation,
//! an Intensity measuring or a Tag schema conversion is recording, and the
//! hold deadlocks with nothing (an accept, Projection's apply, a Stick
//! load). Interleavings are made with the service's test-mode holds
//! (`WRECKORDBOX_REVIEW_GATE`, `hold-producer-<name>`): a file handshake,
//! never a timed window. No real separation is run: the engine's fake
//! worker stands in.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
use crate::common;
use crate::review::{fact, token};
#[path = "../../../wreckordbox-intensity/tests/common/mod.rs"]
mod signals;

use std::path::Path;
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use common::Sandbox;
use serde_json::{json, Value};

/// The service's test-mode holds: `hold-P` stops at point `P`, says so in
/// `at-P`, and waits for `go-P`. A Fact command kept waiting by the
/// coordination writes `facts-waiting`.
struct Gate {
    dir: tempfile::TempDir,
}

impl Gate {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn hold(&self, point: &str) {
        std::fs::write(self.path().join(format!("hold-{point}")), b"").unwrap();
    }

    fn go(&self, point: &str) {
        std::fs::write(self.path().join(format!("go-{point}")), b"").unwrap();
    }

    fn has(&self, name: &str) -> bool {
        self.path().join(name).exists()
    }

    fn reached(&self, name: &str) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !self.has(name) {
            assert!(Instant::now() < deadline, "{name} never appeared");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// A service of this test's, killed and waited for when dropped.
struct Serving(Child);

impl Drop for Serving {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn serve_with(s: &Sandbox, env: &[(&str, &Path)]) -> Serving {
    let mut command = s.command();
    for (name, value) in env {
        command.env(name, value);
    }
    let child = command
        .arg("serve")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let socket = s.config().socket_path(s.location());
    let deadline = Instant::now() + Duration::from_secs(10);
    while wreckordbox_service::Client::connect(&socket).is_err() {
        assert!(Instant::now() < deadline, "the service did not come up");
        std::thread::sleep(Duration::from_millis(25));
    }
    Serving(child)
}

fn gated(s: &Sandbox, gate: &Gate) -> Serving {
    serve_with(s, &[("WRECKORDBOX_REVIEW_GATE", gate.path())])
}

/// A CLI command running alongside, its JSON output read when it ends.
fn spawn(s: &Sandbox, args: &[&str]) -> Child {
    let mut words = vec!["--json"];
    words.extend_from_slice(args);
    s.command()
        .args(&words)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

/// Waits for `child` to end (at most a minute: a deadlock fails the test
/// rather than hanging it) and reads its JSON answer.
fn finished(mut child: Child) -> (bool, Value) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("the command never finished: a deadlock");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let output = child.wait_with_output().unwrap();
    let text = if output.status.success() {
        String::from_utf8(output.stdout).unwrap()
    } else {
        String::from_utf8(output.stderr).unwrap()
    };
    let value = serde_json::from_str(text.trim()).unwrap_or(json!({ "text": text }));
    (output.status.success(), value)
}

/// Waits until the accept is kept waiting by the coordination, failing if
/// it finishes first (it ran while the producer was recording).
fn kept_waiting(gate: &Gate, accept: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !gate.has("facts-waiting") {
        if let Some(status) = accept.try_wait().unwrap() {
            panic!("the accept ran while a producer was recording ({status})");
        }
        assert!(Instant::now() < deadline, "the accept never waited");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(accept.try_wait().unwrap().is_none());
}

/// What the Library file holds, read past the service.
fn db_rating(s: &Sandbox, id: &str) -> i64 {
    let db = rbl_db::Library::open(s.location().clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    db.connection()
        .query_row(
            "SELECT COALESCE(Rating, 0) FROM djmdContent WHERE ID = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
}

/// How many Facts of `kind` the Sidecar holds for `track`, read past the
/// service (a read through it would wait for the hold too).
fn facts_in_sidecar(s: &Sandbox, track: &str, kind: &str) -> i64 {
    let db = rusqlite::Connection::open_with_flags(
        s.sidecar_dir().join("sidecar.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    db.query_row(
        "SELECT COUNT(*) FROM fact WHERE track_id = ?1 AND kind = ?2",
        [track, kind],
        |r| r.get(0),
    )
    .unwrap()
}

/// `review accept` of one Fact, by the DJ, with the listing's token.
fn accept(s: &Sandbox, fact: i64, token: &str) -> Child {
    spawn(
        s,
        &[
            "review",
            "accept",
            &fact.to_string(),
            "--token",
            token,
            "--source",
            "dj",
        ],
    )
}

#[test]
fn an_accept_waits_while_a_separation_records_its_stems_and_then_is_written() {
    let s = Sandbox::new();
    let track = s.import_track("Night Bus.wav", 1);
    let gate = Gate::new();
    let _service = gated(&s, &gate);
    // No policy: the agent's Energy waits for the DJ.
    let energy = fact(&s, "10001", "energy", "4", "agent:tagger", "0.7");
    let t = token(&s);

    gate.hold("producer-stems");
    let separating = spawn(&s, &["stems", "separate", &track]);
    gate.reached("at-producer-stems");
    let mut accepting = accept(&s, energy, &t);

    // The separation is between its last observation and its commit: the
    // accept is kept out, nothing of it written.
    kept_waiting(&gate, &mut accepting);
    assert_eq!(db_rating(&s, "10001"), 0);
    assert_eq!(facts_in_sidecar(&s, &track, "stems"), 0);

    gate.go("producer-stems");
    let (separated, answer) = finished(separating);
    assert!(separated, "{answer}");
    let (accepted, answer) = finished(accepting);
    assert!(accepted, "{answer}");
    assert_eq!(facts_in_sidecar(&s, &track, "stems"), 1);
    assert_eq!(db_rating(&s, "10001"), 4);
}

/// Adds a track with this audio (16-bit mono WAV at the generators' rate)
/// to the Library, as rekordbox imports one. Before any service starts.
fn import_audio(s: &Sandbox, name: &str, samples: &[f32]) -> String {
    let path = s.path().join("audio").join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data_len = u32::try_from(samples.len() * 2).unwrap();
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&signals::RATE.to_le_bytes());
    out.extend_from_slice(&(signals::RATE * 2).to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * 32_767.0) as i16;
        out.extend_from_slice(&value.to_le_bytes());
    }
    std::fs::write(&path, out).unwrap();
    let mut id = String::new();
    s.edit(|writer| id = writer.import_file(&path).unwrap());
    id
}

#[test]
fn an_accept_waits_while_intensity_records_a_measurement_and_then_is_written() {
    let s = Sandbox::new();
    let track = import_audio(&s, "Loud.wav", &signals::loud_busy(3.0));
    let gate = Gate::new();
    let _service = gated(&s, &gate);
    let energy = fact(&s, "10001", "energy", "4", "agent:tagger", "0.7");
    let t = token(&s);

    gate.hold("producer-intensity");
    let measuring = spawn(&s, &["intensity", "measure", &track]);
    gate.reached("at-producer-intensity");
    let mut accepting = accept(&s, energy, &t);

    kept_waiting(&gate, &mut accepting);
    assert_eq!(db_rating(&s, "10001"), 0);
    assert_eq!(facts_in_sidecar(&s, &track, "intensity"), 0);

    gate.go("producer-intensity");
    let (measured, answer) = finished(measuring);
    assert!(measured, "{answer}");
    assert_eq!(answer["measured"], 1, "{answer}");
    let (accepted, answer) = finished(accepting);
    assert!(accepted, "{answer}");
    assert_eq!(facts_in_sidecar(&s, &track, "intensity"), 1);
    assert_eq!(db_rating(&s, "10001"), 4);
}

/// Gives a track a beat grid at `bpm` from `first_downbeat` seconds, in an
/// analysis file where rekordbox keeps one. While no service runs.
fn give_beat_grid(s: &Sandbox, track: &str, bpm: f64, first_downbeat: f64, seconds: f64) {
    let mut beats = Vec::new();
    let mut at = first_downbeat;
    let mut number = 0_u16;
    while at < seconds {
        beats.push(rbl_anlz::Beat {
            beat_number: number % 4 + 1,
            tempo_x100: (bpm * 100.0) as u16,
            time_ms: (at * 1000.0).round() as u32,
        });
        number += 1;
        at = first_downbeat + f64::from(number) * 60.0 / bpm;
    }
    let relative = format!("/PIONEER/USBANLZ/{track}/ANLZ0000.DAT");
    let file = rbl_anlz::resolve(&s.location().share_root, &relative);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let bytes = rbl_anlz::AnlzBuilder::new()
        .path("/audio/track.wav")
        .beat_grid(&beats)
        .finish();
    std::fs::write(&file, bytes).unwrap();
    s.edit(|writer| {
        let changed = writer
            .library()
            .connection()
            .execute(
                "UPDATE djmdContent SET AnalysisDataPath = ?1 WHERE ID = ?2",
                (relative.as_str(), track),
            )
            .unwrap();
        assert_eq!(changed, 1);
    });
}

/// Stops whatever service runs on the sandbox and waits until it is gone.
fn stop_service(s: &Sandbox) {
    let socket = s.config().socket_path(s.location());
    if let Ok(mut client) = wreckordbox_service::Client::connect(&socket) {
        let _ = client.stop();
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while wreckordbox_service::Client::connect(&socket).is_ok() {
        assert!(Instant::now() < deadline, "the service did not stop");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn an_accept_waits_while_reading_intensity_records_bars_for_a_new_beat_grid() {
    let s = Sandbox::new();
    let track = import_audio(&s, "Later.wav", &signals::loud_busy(5.0));
    let measured = s.cli(&["--json", "intensity", "measure", &track]);
    assert!(measured.status.success(), "{}", measured.stderr);
    stop_service(&s);
    // rekordbox analyses it after the measuring: reading it makes the bars.
    give_beat_grid(&s, &track, 120.0, 0.5, 5.0);
    let gate = Gate::new();
    let _service = gated(&s, &gate);
    let energy = fact(&s, "10001", "energy", "4", "agent:tagger", "0.7");
    let t = token(&s);

    gate.hold("producer-realign");
    let showing = spawn(&s, &["intensity", "show", &track]);
    gate.reached("at-producer-realign");
    let mut accepting = accept(&s, energy, &t);

    kept_waiting(&gate, &mut accepting);
    assert_eq!(db_rating(&s, "10001"), 0);
    assert_eq!(facts_in_sidecar(&s, &track, "intensity"), 1);

    gate.go("producer-realign");
    let (shown, answer) = finished(showing);
    assert!(shown, "{answer}");
    assert!(answer["intensity"]["value"]["bars"].is_object(), "{answer}");
    let (accepted, answer) = finished(accepting);
    assert!(accepted, "{answer}");
    assert_eq!(facts_in_sidecar(&s, &track, "intensity"), 2);
    assert_eq!(db_rating(&s, "10001"), 4);
}

#[test]
fn an_accept_waits_while_a_tag_schema_conversion_records_the_djs_tags() {
    let s = Sandbox::new();
    let [_, mood, _, _] = s.four_categories();
    let dark = s.create_my_tag_in(&mood, "Dark");
    s.detail_track(
        1,
        &common::Details {
            my_tags: &[dark.as_str()],
            ..common::Details::default()
        },
    );
    let gate = Gate::new();
    let _service = gated(&s, &gate);
    let energy = fact(&s, "10002", "energy", "4", "agent:tagger", "0.7");
    let t = token(&s);

    gate.hold("producer-schema");
    let converting = spawn(&s, &["schema", "convert"]);
    gate.reached("at-producer-schema");
    let mut accepting = accept(&s, energy, &t);

    kept_waiting(&gate, &mut accepting);
    assert_eq!(db_rating(&s, "10002"), 0);
    assert_eq!(facts_in_sidecar(&s, "10001", "mood"), 0);

    gate.go("producer-schema");
    let (converted, answer) = finished(converting);
    assert!(converted, "{answer}");
    let (accepted, answer) = finished(accepting);
    assert!(accepted, "{answer}");
    assert_eq!(facts_in_sidecar(&s, "10001", "mood"), 1);
    assert_eq!(db_rating(&s, "10002"), 4);
}

/// A fixture Library with a track to separate, a playlist "Set" of another
/// on a stand-in Stick the DJ unlocked, and an Energy policy: an agent's
/// answer on 10001 applies (0.9), its answer on 10002 waits (0.6).
struct Rig {
    s: Sandbox,
    _sticks: tempfile::TempDir,
    stick: std::path::PathBuf,
    separated: String,
    set: String,
}

impl Rig {
    fn new() -> Self {
        let mut s = Sandbox::new();
        let sticks = tempfile::tempdir().unwrap();
        let stick = sticks.path().join("FRIDAY");
        std::fs::create_dir(&stick).unwrap();
        std::fs::write(
            stick.join(wreckordbox_stick_protection::STAND_IN_MARKER),
            b"",
        )
        .unwrap();
        s.sticks = vec![stick.clone()];
        let separated = s.import_track("Night Bus.wav", 1);
        let loaded = s.import_track("Alpha.wav", 1);
        let mut set = String::new();
        s.edit(|writer| {
            set = writer.create_playlist("Set", "root").unwrap();
            writer.add_tracks(&set, &[loaded.clone()]).unwrap();
        });
        crate::review::policy(&s, "[energy]\napply = 0.8\n");
        Self {
            s,
            _sticks: sticks,
            stick,
            separated,
            set,
        }
    }

    fn stick(&self) -> &str {
        self.stick.to_str().unwrap()
    }

    /// The DJ unlocks the Stick by name and dry-runs the load: its plan.
    fn unlock_and_plan(&self) -> String {
        let run = self
            .s
            .cli(&["stick", "unlock", self.stick(), "--name", "FRIDAY"]);
        assert!(run.status.success(), "{}", run.stderr);
        let dry = crate::review::ok(&self.s, &["stick", "dry-run", self.stick(), &self.set]);
        dry["plan"].as_str().unwrap().to_owned()
    }

    fn load(&self, plan: &str) -> Child {
        spawn(
            &self.s,
            &["stick", "load", self.stick(), &self.set, "--plan", plan],
        )
    }

    /// Facts: one that Projection applies, one that waits; the token.
    fn facts(&self) -> (i64, String) {
        fact(&self.s, "10001", "energy", "3", "agent:tagger", "0.9");
        let waiting = fact(&self.s, "10002", "energy", "4", "agent:tagger", "0.6");
        (waiting, token(&self.s))
    }

    /// A separation held between its observation and its commit.
    fn held_separation(&self, gate: &Gate) -> Child {
        gate.hold("producer-stems");
        let child = spawn(&self.s, &["stems", "separate", &self.separated]);
        gate.reached("at-producer-stems");
        child
    }

    fn stems_recorded(&self) -> i64 {
        facts_in_sidecar(&self.s, &self.separated, "stems")
    }
}

#[test]
fn while_a_producer_holds_its_commit_projection_applies_and_a_stick_loads_and_only_an_accept_waits(
) {
    let rig = Rig::new();
    let gate = Gate::new();
    let _service = gated(&rig.s, &gate);
    let (waiting, t) = rig.facts();
    let separating = rig.held_separation(&gate);

    // Neither needs the Fact coordination: both finish with it held.
    let (projected, answer) = finished(spawn(&rig.s, &["project"]));
    assert!(projected, "{answer}");
    assert_eq!(db_rating(&rig.s, "10001"), 3);
    // (A load is confirmed against the Library as it is after the apply.)
    let plan = rig.unlock_and_plan();
    let (loaded, answer) = finished(rig.load(&plan));
    assert!(loaded, "{answer}");
    assert_eq!(rig.stems_recorded(), 0);
    let mut accepting = accept(&rig.s, waiting, &t);
    kept_waiting(&gate, &mut accepting);

    gate.go("producer-stems");
    let (separated, answer) = finished(separating);
    assert!(separated, "{answer}");
    let (accepted, answer) = finished(accepting);
    assert!(accepted, "{answer}");
    assert_eq!(rig.stems_recorded(), 1);
    assert_eq!(db_rating(&rig.s, "10002"), 4);
}

#[test]
fn a_producer_commits_while_projection_holds_the_library_and_an_accept_queued_behind_it_finishes(
) {
    let rig = Rig::new();
    let gate = Gate::new();
    let projection = tempfile::tempdir().unwrap();
    let _service = serve_with(
        &rig.s,
        &[
            ("WRECKORDBOX_REVIEW_GATE", gate.path()),
            ("WRECKORDBOX_PROJECTION_GATE", projection.path()),
        ],
    );
    let (waiting, t) = rig.facts();
    let separating = rig.held_separation(&gate);
    // Projection's apply stops between its plan and its write, holding
    // the Library and the edit gate.
    let projecting = spawn(&rig.s, &["project"]);
    let paused = projection.path().join("paused");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !paused.exists() {
        assert!(Instant::now() < deadline, "the apply never paused");
        std::thread::sleep(Duration::from_millis(5));
    }

    // The producer's commit needs neither: it is recorded meanwhile.
    gate.go("producer-stems");
    let (separated, answer) = finished(separating);
    assert!(separated, "{answer}");
    assert_eq!(rig.stems_recorded(), 1);
    // An accept takes the coordination and waits for the Library.
    let accepting = accept(&rig.s, waiting, &t);
    std::fs::write(projection.path().join("go"), b"").unwrap();
    let (projected, answer) = finished(projecting);
    assert!(projected, "{answer}");
    let (accepted, answer) = finished(accepting);
    assert!(accepted, "{answer}");
    assert_eq!(db_rating(&rig.s, "10001"), 3);
    assert_eq!(db_rating(&rig.s, "10002"), 4);
}

#[test]
fn a_producer_commits_while_a_stick_load_is_part_way_and_both_finish() {
    let rig = Rig::new();
    let gate = Gate::new();
    let _service = gated(&rig.s, &gate);
    let plan = rig.unlock_and_plan();
    let separating = rig.held_separation(&gate);
    // The load stops part-way, after its native step, before reading back.
    let state = rig.s.state_dir();
    let pause = state.join(wreckordbox_service::stick_load::TEST_PAUSE_AT_FILE);
    std::fs::write(&pause, "read_back").unwrap();
    let loading = rig.load(&plan);
    let paused = state.join(wreckordbox_service::stick_load::TEST_PAUSED_FILE);
    let deadline = Instant::now() + Duration::from_secs(60);
    while !paused.exists() {
        assert!(Instant::now() < deadline, "the load never paused");
        std::thread::sleep(Duration::from_millis(5));
    }

    gate.go("producer-stems");
    let (separated, answer) = finished(separating);
    assert!(separated, "{answer}");
    assert_eq!(rig.stems_recorded(), 1);
    std::fs::remove_file(&pause).unwrap();
    let (loaded, answer) = finished(loading);
    assert!(loaded, "{answer}");
}
