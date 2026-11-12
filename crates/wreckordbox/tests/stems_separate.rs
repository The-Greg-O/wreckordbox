//! `wreckordbox stems separate` against a Library service on a throwaway
//! Library, with the separation engine's real worker protocol over its fake
//! backend: the real binary, its output, and what is left in the Sidecar.
//! No model is ever run.
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

/// A 16-bit stereo 44.1 kHz WAV file of `frames` sample frames, every
/// sample `value`, optionally followed by a chunk of metadata (which is
/// not audio).
fn write_audio(path: &Path, frames: u32, value: i16, comment: Option<&str>) {
    let data_len = frames * 4;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&0_u32.to_le_bytes());
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
    for _ in 0..frames * 2 {
        out.extend_from_slice(&value.to_le_bytes());
    }
    if let Some(comment) = comment {
        let mut text = comment.as_bytes().to_vec();
        text.push(0);
        if text.len() % 2 == 1 {
            text.push(0);
        }
        let mut list = b"INFOICMT".to_vec();
        list.extend_from_slice(&u32::try_from(text.len()).unwrap().to_le_bytes());
        list.extend_from_slice(&text);
        out.extend_from_slice(b"LIST");
        out.extend_from_slice(&u32::try_from(list.len()).unwrap().to_le_bytes());
        out.extend_from_slice(&list);
    }
    let riff_len = u32::try_from(out.len() - 8).unwrap();
    out[4..8].copy_from_slice(&riff_len.to_le_bytes());
    std::fs::write(path, out).unwrap();
}

/// Where the sandbox keeps the audio file called `name`.
fn audio_path(sandbox: &Sandbox, name: &str) -> PathBuf {
    let dir = sandbox.path().join("audio");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// Adds a track of `frames` sample frames to the Library, as rekordbox
/// imports one, and answers its identifier. Before any service starts.
fn import(sandbox: &Sandbox, name: &str, frames: u32) -> String {
    let path = audio_path(sandbox, name);
    write_audio(&path, frames, 0, None);
    let mut id = String::new();
    sandbox.edit(|writer| id = writer.import_file(&path).unwrap());
    id
}

/// How many sample frames a 16-bit stereo WAV file written with a plain
/// 44-byte header holds, worked out from its size alone.
fn frames_in(path: &Path) -> u64 {
    (std::fs::metadata(path).unwrap().len() - 44) / 4
}

fn separate(sandbox: &Sandbox, track: &str) -> serde_json::Value {
    let run = sandbox.cli(&["--json", "stems", "separate", track]);
    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(run.stderr, "");
    run.json()
}

fn roles(stems: &serde_json::Value) -> Vec<&str> {
    stems["stems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|stem| stem["role"].as_str().unwrap())
        .collect()
}

fn paths(stems: &serde_json::Value) -> Vec<PathBuf> {
    stems["stems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|stem| PathBuf::from(stem["path"].as_str().unwrap()))
        .collect()
}

#[test]
fn separating_a_track_keeps_four_stems_in_the_sidecar_each_as_long_as_their_parent() {
    let sandbox = Sandbox::new();
    let track = import(&sandbox, "Night Bus.wav", 4_410);

    let separated = separate(&sandbox, &track);

    assert_eq!(separated["track"]["id"], track.as_str());
    assert_eq!(separated["track"]["title"], "Night Bus");
    assert_eq!(separated["reused"], false);
    let stems = &separated["stems"];
    assert_eq!(roles(stems), ["vocals", "other", "bass", "drums"]);
    assert_eq!(stems["samples"], 4_410);
    assert_eq!(stems["sample_rate"], 44_100);
    let sidecar = sandbox.sidecar_dir().canonicalize().unwrap();
    for path in paths(stems) {
        assert!(path.starts_with(&sidecar), "{}", path.display());
        assert_eq!(frames_in(&path), 4_410, "{}", path.display());
    }
    assert_eq!(sandbox.engine_runs(), 1);
}

fn show(sandbox: &Sandbox, track: &str) -> serde_json::Value {
    let run = sandbox.cli(&["--json", "stems", "show", track]);
    assert!(run.status.success(), "{}", run.stderr);
    run.json()
}

/// Every file under `dir`, at any depth.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            found.extend(files_under(&path));
        } else {
            found.push(path);
        }
    }
    found
}

/// The Stem audio the Sidecar holds, for any track.
fn stem_files(sandbox: &Sandbox) -> Vec<PathBuf> {
    files_under(&sandbox.sidecar_dir().join("stems"))
}

#[test]
fn the_sidecar_records_the_stems_their_parent_its_fingerprint_and_what_made_them() {
    let sandbox = Sandbox::new();
    let track = import(&sandbox, "Night Bus.wav", 4_410);
    let untouched = import(&sandbox, "Other Tune.wav", 2_000);

    let separated = separate(&sandbox, &track);
    let shown = show(&sandbox, &track);

    assert_eq!(shown["track"]["id"], track.as_str());
    let stems = &shown["stems"];
    // What `separate` answered is what the Sidecar holds.
    assert_eq!(*stems, separated["stems"]);
    assert_eq!(stems["track_id"], track.as_str());
    assert_eq!(roles(stems), ["vocals", "other", "bass", "drums"]);
    assert_eq!(stems["samples"], 4_410);
    let fingerprint = stems["fingerprint"].as_str().unwrap();
    assert!(fingerprint.starts_with("a1:"), "{fingerprint}");
    assert_eq!(fingerprint.len(), 3 + 64);
    assert_eq!(
        stems["source"],
        serde_json::json!({ "kind": "tool", "name": "separation-engine" })
    );
    assert_eq!(stems["confidence"], 1.0);
    assert_eq!(stems["produced_by"]["model"], "fake-separator");
    assert_eq!(stems["produced_by"]["engine_version"], "0.1.0");
    assert!(stems["run"].as_i64().unwrap() > 0);
    assert!(stems["recorded_at"].as_i64().unwrap() > 1_700_000_000_000);
    assert_eq!(stems["stale"], false);
    assert_eq!(shown["previous"], serde_json::json!([]));
    for stem in stems["stems"].as_array().unwrap() {
        assert_eq!(stem["format"], "wav");
        assert_eq!(stem["channels"], 2);
        assert_eq!(stem["bits_per_sample"], 16);
    }

    // A track that was never separated has none, and says so.
    let none = show(&sandbox, &untouched);
    assert_eq!(none["stems"], serde_json::Value::Null);
    let text = sandbox.cli(&["stems", "show", &untouched]);
    assert_eq!(
        text.stdout,
        format!("Track {untouched} \"Other Tune\" has no Stems.\n")
    );
}

#[test]
fn a_stem_set_that_is_not_as_long_as_its_parent_is_refused_and_nothing_is_kept() {
    let mut sandbox = Sandbox::new();
    sandbox.engine_options = vec!["--short".into(), "drums".into()];
    let track = import(&sandbox, "Night Bus.wav", 4_410);

    let run = sandbox.cli(&["--json", "stems", "separate", &track]);

    assert!(!run.status.success());
    assert_eq!(run.stdout, "");
    let error = run.error_json();
    assert_eq!(error["error"]["code"], "stem_length_mismatch");
    let message = error["error"]["message"].as_str().unwrap();
    assert!(message.contains("drums"), "{message}");
    assert!(message.contains("4409"), "{message}");
    assert!(message.contains("4410"), "{message}");
    // The engine did run; what it made was not accepted.
    assert_eq!(sandbox.engine_runs(), 1);
    assert_eq!(show(&sandbox, &track)["stems"], serde_json::Value::Null);
    assert_eq!(stem_files(&sandbox), Vec::<PathBuf>::new());
}

#[test]
fn separating_an_unchanged_track_again_reuses_its_stems_without_running_the_engine() {
    let sandbox = Sandbox::new();
    let track = import(&sandbox, "Night Bus.wav", 4_410);
    let first = separate(&sandbox, &track);

    let again = separate(&sandbox, &track);

    assert_eq!(again["reused"], true);
    assert_eq!(again["stems"], first["stems"]);
    assert_eq!(again["marked_stale"], serde_json::json!([]));
    assert_eq!(sandbox.engine_runs(), 1);
    assert_eq!(stem_files(&sandbox).len(), 4);

    // Rewriting the file's tags is not a change to its audio.
    write_audio(
        &audio_path(&sandbox, "Night Bus.wav"),
        4_410,
        0,
        Some("retagged"),
    );
    let retagged = separate(&sandbox, &track);
    assert_eq!(retagged["reused"], true);
    assert_eq!(retagged["stems"], first["stems"]);
    assert_eq!(sandbox.engine_runs(), 1);

    let text = sandbox.cli(&["stems", "separate", &track]);
    assert!(text.status.success(), "{}", text.stderr);
    assert!(
        text.stdout.starts_with(&format!(
            "Track {track} \"Night Bus\" already has Stems from this audio; they were not made again.\n"
        )),
        "{}",
        text.stdout
    );
    assert_eq!(text.stderr, "");
}

#[test]
fn a_track_whose_audio_changed_is_separated_afresh_and_its_old_stems_are_marked_stale() {
    let sandbox = Sandbox::new();
    let track = import(&sandbox, "Night Bus.wav", 4_410);
    let first = separate(&sandbox, &track);
    let old_fact = first["stems"]["fact_id"].as_i64().unwrap();

    // The same file, the same length, different audio.
    write_audio(&audio_path(&sandbox, "Night Bus.wav"), 4_410, 1_000, None);
    let second = separate(&sandbox, &track);

    assert_eq!(second["reused"], false);
    assert_eq!(sandbox.engine_runs(), 2);
    assert_eq!(second["marked_stale"], serde_json::json!([old_fact]));
    let new_fact = second["stems"]["fact_id"].as_i64().unwrap();
    assert_ne!(new_fact, old_fact);
    assert_ne!(
        second["stems"]["fingerprint"],
        first["stems"]["fingerprint"]
    );
    assert_eq!(second["stems"]["stale"], false);

    // The old Stems are still there, marked, not overwritten.
    let shown = show(&sandbox, &track);
    assert_eq!(shown["stems"]["fact_id"], new_fact);
    let previous = shown["previous"].as_array().unwrap();
    assert_eq!(previous.len(), 1);
    assert_eq!(previous[0]["fact_id"], old_fact);
    assert_eq!(previous[0]["stale"], true);
    assert_eq!(previous[0]["fingerprint"], first["stems"]["fingerprint"]);
    for path in paths(&first["stems"])
        .iter()
        .chain(&paths(&second["stems"]))
    {
        assert!(path.is_file(), "{}", path.display());
    }
    assert_eq!(stem_files(&sandbox).len(), 8);

    // And the new ones are reused in their turn.
    assert_eq!(separate(&sandbox, &track)["reused"], true);
    assert_eq!(sandbox.engine_runs(), 2);
}

#[test]
fn a_track_that_is_not_there_or_has_no_audio_file_fails_without_running_the_engine() {
    let sandbox = Sandbox::new();

    let unknown = sandbox.cli(&["--json", "stems", "separate", "999"]);
    assert!(!unknown.status.success());
    assert_eq!(unknown.error_json()["error"]["code"], "track_not_found");

    // A fixture track's audio file does not exist.
    let missing = sandbox.cli(&["stems", "separate", "10003"]);
    assert!(!missing.status.success());
    assert_eq!(missing.stdout, "");
    assert!(
        missing
            .stderr
            .starts_with("wreckordbox: the audio file of track 10003 \"Track 003\" is missing"),
        "{}",
        missing.stderr
    );
    let coded = sandbox.cli(&["--json", "stems", "separate", "10003"]);
    assert_eq!(coded.error_json()["error"]["code"], "track_file_missing");

    assert_eq!(sandbox.engine_runs(), 0);
    assert_eq!(stem_files(&sandbox), Vec::<PathBuf>::new());
}

#[test]
fn an_engine_that_fails_or_cannot_be_run_is_reported_and_nothing_is_kept() {
    let mut sandbox = Sandbox::new();
    sandbox.engine_options = vec!["--fail".into(), "model".into()];
    let track = import(&sandbox, "Night Bus.wav", 4_410);

    let failed = sandbox.cli(&["--json", "stems", "separate", &track]);
    assert!(!failed.status.success());
    let error = failed.error_json();
    assert_eq!(error["error"]["code"], "separation_failed");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("model"),
        "{error}"
    );
    assert_eq!(show(&sandbox, &track)["stems"], serde_json::Value::Null);
    assert_eq!(stem_files(&sandbox), Vec::<PathBuf>::new());

    // No engine at all where the service was told to look.
    let nowhere = Sandbox::new();
    let track = import(&nowhere, "Night Bus.wav", 4_410);
    let run = nowhere.run(
        nowhere
            .command()
            .env(
                "WRECKORDBOX_SEPARATION_COMMAND",
                "/nowhere/python -m stemsplitter_engine",
            )
            .args(["--json", "stems", "separate", &track]),
    );
    assert!(!run.status.success());
    assert_eq!(
        run.error_json()["error"]["code"],
        "separation_engine_missing"
    );
}

#[test]
fn progress_is_printed_by_hand_and_sent_to_other_clients_as_events_that_are_not_library_changes() {
    let sandbox = Sandbox::new();
    let track = import(&sandbox, "Night Bus.wav", 4_410);
    sandbox.status();
    let mut watcher = sandbox.client();
    watcher.subscribe().unwrap();

    let run = sandbox.cli(&["stems", "separate", &track]);

    assert!(run.status.success(), "{}", run.stderr);
    assert!(
        run.stdout.starts_with(&format!(
            "Separated track {track} \"Night Bus\" into 4 Stems.\n4410 samples at 44100 Hz, made by fake-separator\n"
        )),
        "{}",
        run.stdout
    );
    for role in ["vocals", "other", "bass", "drums"] {
        assert!(
            run.stdout.contains(&format!("\n{role:<11}  ")),
            "{}",
            run.stdout
        );
    }
    assert_eq!(
        run.stderr,
        "loading model: 0%\nseparating: 10%\nwriting: 90%\n"
    );

    let mut events = Vec::new();
    while let Some(event) = watcher.next_event(Duration::from_millis(500)).unwrap() {
        events.push(event);
    }
    let names: Vec<&str> = events.iter().map(|event| event.event.as_str()).collect();
    assert_eq!(
        names,
        [
            "stems_progress",
            "stems_progress",
            "stems_progress",
            "sidecar_changed"
        ]
    );
    assert_eq!(
        events[1].data,
        serde_json::json!({ "track_id": track, "stage": "separating", "fraction": 0.15 })
    );
    let fact = show(&sandbox, &track)["stems"]["fact_id"].clone();
    assert_eq!(
        events[3].data,
        serde_json::json!({ "track_id": track, "kind": "stems", "fact_id": fact, "stale": [] })
    );
}

/// The schema of the Library's database and how many rows each table has.
fn library_shape(sandbox: &Sandbox) -> Vec<(String, String, i64)> {
    let mut shape = Vec::new();
    sandbox.edit(|writer| {
        let db = writer.library().connection();
        let mut tables = db
            .prepare("SELECT name, coalesce(sql, '') FROM sqlite_master ORDER BY name")
            .unwrap();
        let named: Vec<(String, String, String)> = tables
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    String::new(),
                ))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for (name, sql, _) in named {
            let rows = if sql.starts_with("CREATE TABLE") {
                db.query_row(&format!("SELECT count(*) FROM \"{name}\""), [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap()
            } else {
                0
            };
            shape.push((name, sql, rows));
        }
    });
    shape
}

#[test]
fn stems_and_facts_add_no_table_column_or_row_to_the_library_and_live_in_a_folder_beside_it() {
    let sandbox = Sandbox::new();
    let track = import(&sandbox, "Night Bus.wav", 4_410);
    let before = library_shape(&sandbox);
    let library_dir = sandbox.master_db().parent().unwrap().to_path_buf();

    // With nothing saying where the Sidecar goes, it goes beside the
    // Library: a folder of its own next to `master.db`.
    let run = sandbox.run(
        sandbox
            .command()
            .env_remove("WRECKORDBOX_SIDECAR_DIR")
            .args(["--json", "stems", "separate", &track]),
    );
    assert!(run.status.success(), "{}", run.stderr);

    let sidecar = library_dir
        .join("wreckordbox-sidecar")
        .canonicalize()
        .unwrap();
    assert!(sidecar.join("sidecar.db").is_file());
    for path in paths(&run.json()["stems"]) {
        assert!(path.starts_with(&sidecar), "{}", path.display());
        assert!(path.is_file());
    }
    // A Fact recorded by hand goes the same way: into the Sidecar beside
    // the Library, and nowhere in it.
    let fact = sandbox.cli(&[
        "--json",
        "facts",
        "record",
        &track,
        "genre",
        "Dub Techno",
        "--source",
        "agent:tagger",
        "--confidence",
        "0.8",
    ]);
    assert!(fact.status.success(), "{}", fact.stderr);
    let listed = sandbox.cli(&["--json", "facts", "show", &track]);
    assert!(listed.status.success(), "{}", listed.stderr);
    let kinds: Vec<String> = listed.json()["facts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|fact| fact["kind"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(kinds, ["genre", "stems"]);

    assert!(!sandbox.sidecar_dir().exists());
    assert_eq!(library_shape(&sandbox), before);
}

/// Starts `stems separate` for `track` as a client that will be dropped
/// part-way, and waits until the engine is at work on it.
fn start_separating(sandbox: &Sandbox, track: &str) -> std::process::Child {
    let child = sandbox
        .command()
        .args(["--json", "stems", "separate", track])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let started = (0..400).any(|_| {
        std::thread::sleep(Duration::from_millis(25));
        sandbox.engine_runs() == 1
    });
    assert!(started, "the engine was never asked to separate");
    child
}

/// Whether any process on this machine was started with this sandbox's
/// folder on its command line and is called `needle`.
fn running_here(sandbox: &Sandbox, needle: &str) -> bool {
    let folder = sandbox.path().to_str().unwrap().to_owned();
    std::fs::read_dir("/proc")
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| std::fs::read(entry.path().join("cmdline")).ok())
        .map(|cmdline| String::from_utf8_lossy(&cmdline).into_owned())
        .any(|cmdline| cmdline.contains(&folder) && cmdline.contains(needle))
}

#[test]
fn a_client_that_leaves_does_not_stop_the_separation_and_asking_again_waits_for_it() {
    let mut sandbox = Sandbox::new();
    sandbox.engine_options = vec!["--delay".into(), "3".into()];
    let track = import(&sandbox, "Night Bus.wav", 4_410);
    let other = import(&sandbox, "Other Tune.wav", 2_000);
    sandbox.status();

    let mut leaver = start_separating(&sandbox, &track);
    leaver.kill().unwrap();
    leaver.wait().unwrap();

    // One separation at a time: another track is turned away, by name.
    let busy = sandbox.cli(&["--json", "stems", "separate", &other]);
    assert!(!busy.status.success());
    let error = busy.error_json();
    assert_eq!(error["error"]["code"], "separation_busy");
    assert!(
        error["error"]["message"].as_str().unwrap().contains(&track),
        "{error}"
    );
    // Every other command is answered meanwhile.
    assert_eq!(show(&sandbox, &track)["stems"], serde_json::Value::Null);

    // Asking for the same track again joins the one in progress.
    let joined = separate(&sandbox, &track);
    assert_eq!(joined["reused"], false);
    assert_eq!(
        roles(&joined["stems"]),
        ["vocals", "other", "bass", "drums"]
    );
    assert_eq!(sandbox.engine_runs(), 1);
    assert_eq!(show(&sandbox, &track)["stems"], joined["stems"]);
    assert_eq!(stem_files(&sandbox).len(), 4);
}

#[test]
fn a_service_that_stops_part_way_stops_the_engine_and_leaves_the_sidecar_as_it_was() {
    let mut sandbox = Sandbox::new();
    sandbox.engine_options = vec!["--delay".into(), "60".into()];
    let track = import(&sandbox, "Night Bus.wav", 4_410);
    sandbox.status();

    let mut waiting = start_separating(&sandbox, &track);
    assert!(running_here(&sandbox, "fake_worker.py"));
    sandbox.client().stop().unwrap();

    // The client that was waiting is told, not left hanging.
    assert!(!waiting.wait().unwrap().success());
    let gone = (0..200).any(|_| {
        std::thread::sleep(Duration::from_millis(25));
        !running_here(&sandbox, "fake_worker.py")
    });
    assert!(gone, "the engine is still running");

    // Nothing of the half-made Stems is in the Sidecar, on disk or on
    // record; the next service separates the track from the start.
    sandbox.engine_options = Vec::new();
    assert_eq!(show(&sandbox, &track)["stems"], serde_json::Value::Null);
    assert_eq!(
        files_under(&sandbox.sidecar_dir().join("work")),
        Vec::<PathBuf>::new()
    );
    assert_eq!(stem_files(&sandbox), Vec::<PathBuf>::new());
    let separated = separate(&sandbox, &track);
    assert_eq!(separated["reused"], false);
    assert_eq!(sandbox.engine_runs(), 2);
}
