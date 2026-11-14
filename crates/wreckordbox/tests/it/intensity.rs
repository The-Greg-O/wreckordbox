//! `wreckordbox intensity`: measuring how hard a track hits, from audio
//! generated here so every answer is known by construction. The real
//! binary against a service on a fixture Library, with a second client
//! watching.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::common;
#[path = "../../../wreckordbox-intensity/tests/common/mod.rs"]
pub(crate) mod signals;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use common::Sandbox;
use serde_json::{json, Value};
use signals::{joined, loud_busy, loud_busy_at, quiet_sparse, RATE};

/// Writes `samples` as a 16-bit mono WAV at the generators' rate.
fn write_audio(path: &Path, samples: &[f32]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data_len = u32::try_from(samples.len() * 2).unwrap();
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * 32_767.0) as i16;
        out.extend_from_slice(&value.to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}

fn audio_path(sandbox: &Sandbox, name: &str) -> PathBuf {
    sandbox.path().join("audio").join(name)
}

/// Adds a track with this audio to the Library, as rekordbox imports one,
/// and answers its identifier. Before any service starts.
fn import(sandbox: &Sandbox, name: &str, samples: &[f32]) -> String {
    let path = audio_path(sandbox, name);
    write_audio(&path, samples);
    let mut id = String::new();
    sandbox.edit(|writer| id = writer.import_file(&path).unwrap());
    id
}

/// Gives a track a beat grid at `bpm` whose first bar begins at
/// `first_downbeat` seconds, in an analysis file where rekordbox keeps
/// one. Before any service starts.
fn give_beat_grid(sandbox: &Sandbox, track: &str, bpm: f64, first_downbeat: f64, seconds: f64) {
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
    let file = rbl_anlz::resolve(&sandbox.location().share_root, &relative);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let bytes = rbl_anlz::AnlzBuilder::new()
        .path("/audio/track.wav")
        .beat_grid(&beats)
        .finish();
    std::fs::write(&file, bytes).unwrap();
    sandbox.edit(|writer| {
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

fn ok(sandbox: &Sandbox, args: &[&str]) -> Value {
    let run = sandbox.cli(args);
    assert!(run.status.success(), "{args:?}: {}", run.stderr);
    run.json()
}

fn measure(sandbox: &Sandbox, tracks: &[&str]) -> Value {
    let mut args = vec!["--json", "intensity", "measure"];
    args.extend_from_slice(tracks);
    ok(sandbox, &args)
}

fn show(sandbox: &Sandbox, track: &str) -> Value {
    ok(sandbox, &["--json", "intensity", "show", track])
}

fn numbers(value: &Value) -> Vec<f64> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("not a list: {value}"))
        .iter()
        .map(|number| number.as_f64().unwrap())
        .collect()
}

/// Every table of the Library with its definition and its row count, and
/// the bytes of `master.db`.
fn library_state(sandbox: &Sandbox) -> (Vec<(String, String, i64)>, Vec<u8>) {
    let mut shape = Vec::new();
    sandbox.edit(|writer| {
        let db = writer.library().connection();
        let mut tables = db
            .prepare("SELECT name, coalesce(sql, '') FROM sqlite_master ORDER BY name")
            .unwrap();
        let named: Vec<(String, String)> = tables
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for (name, sql) in named {
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
    (shape, std::fs::read(sandbox.master_db()).unwrap())
}

/// Stops the service, so the Library can be looked at from here.
fn stop_service(sandbox: &Sandbox) {
    if let Ok(mut client) =
        wreckordbox_service::Client::connect(&sandbox.config().socket_path(sandbox.location()))
    {
        let pid = client.status().unwrap().pid;
        let _ = client.stop();
        let gone = Instant::now() + Duration::from_secs(10);
        while common::alive(u64::from(pid)) && Instant::now() < gone {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// The events a client has been sent, until none comes for a moment.
fn events(client: &mut wreckordbox_service::Client) -> Vec<wreckordbox_service::Event> {
    let mut seen = Vec::new();
    while let Some(event) = client.next_event(Duration::from_millis(300)).unwrap() {
        seen.push(event);
    }
    seen
}

/// Every event a client is sent up to the progress event that settles the
/// last of `total` tracks, each waited for as long as it takes (wbx-377):
/// an exact count that does not end at the first quiet moment.
fn until_settled(
    client: &mut wreckordbox_service::Client,
    total: u64,
) -> Vec<wreckordbox_service::Event> {
    let mut seen = Vec::new();
    loop {
        let event = client
            .next_event(Duration::from_secs(30))
            .unwrap()
            .unwrap_or_else(|| panic!("the last of {total} tracks was never settled: {seen:?}"));
        let last = event.event == "intensity_progress"
            && event.data["total"] == total
            && event.data["done"] == total
            && event.data["status"] != "measuring";
        seen.push(event);
        if last {
            return seen;
        }
    }
}

/// [`until_settled`], then whatever else comes in the window [`events`]
/// has always allowed.
fn settled_then_events(
    client: &mut wreckordbox_service::Client,
    total: u64,
) -> Vec<wreckordbox_service::Event> {
    let mut seen = until_settled(client, total);
    seen.extend(events(client));
    seen
}

#[test]
fn measuring_a_track_stores_a_summary_and_a_curve_that_rises_where_the_audio_does() {
    let sandbox = Sandbox::new();
    // Eight quiet, sparse seconds, then eight loud, busy ones.
    let track = import(
        &sandbox,
        "Build.wav",
        &joined(&[quiet_sparse(8.0), loud_busy(8.0)]),
    );
    let before = library_state(&sandbox);
    sandbox.status();
    let mut watcher = sandbox.client();
    watcher.subscribe().unwrap();

    let measured = measure(&sandbox, &[&track]);
    assert_eq!(measured["total"], 1);
    assert_eq!(measured["measured"], 1);
    assert_eq!(measured["decoded"], 1);
    assert_eq!(measured["cancelled"], false);
    let outcome = &measured["tracks"][0];
    assert_eq!(outcome["track"]["id"], track.as_str());
    assert_eq!(outcome["status"], "measured");
    let run = measured["run"].as_i64().expect("the measuring was a Run");

    // What was stored: a Measurement from a tool, tied to this audio.
    let shown = show(&sandbox, &track);
    let stored = &shown["intensity"];
    assert_eq!(stored["fact_id"], outcome["fact_id"]);
    assert_eq!(stored["run"], run);
    assert_eq!(
        stored["source"],
        json!({"kind": "tool", "name": "wreckordbox-intensity"})
    );
    assert_eq!(stored["stale"], false);
    assert!(stored["fingerprint"].as_str().unwrap().starts_with("a1:"));
    assert_eq!(shown["previous"], json!([]));

    let value = &stored["value"];
    assert_eq!(value["method"]["name"], "wreckordbox-intensity");
    assert_eq!(value["method"]["version"], 2);
    assert_eq!(
        value["method"]["features"],
        json!([
            "loudness",
            "low_end",
            "high_end",
            "onset_strength",
            "onset_density"
        ])
    );
    assert!(value["method"]["weights"]["loudness"].is_number());
    assert_eq!(value["summary"], outcome["summary"]);

    // The curve: one point a second, low for eight, high for eight, and
    // the step exactly where the audio changes.
    assert_eq!(value["curve"]["step_seconds"], 1.0);
    assert_eq!(value["curve"]["offset_seconds"], 0.0);
    let curve = numbers(&value["curve"]["intensity"]);
    assert_eq!(curve.len(), 16);
    let highest_quiet = curve[..8].iter().copied().fold(0.0, f64::max);
    let lowest_loud = curve[8..].iter().copied().fold(1.0, f64::min);
    assert!(lowest_loud - highest_quiet > 0.3, "{curve:?}");
    let biggest_step = (1..16)
        .max_by(|a, b| (curve[*a] - curve[*a - 1]).total_cmp(&(curve[*b] - curve[*b - 1])))
        .unwrap();
    assert_eq!(biggest_step, 8, "{curve:?}");
    let summary = value["summary"]["intensity"].as_f64().unwrap();
    assert!(
        highest_quiet < summary && summary < lowest_loud,
        "{summary} {curve:?}"
    );
    // The raw features are kept beside the combined curve.
    for feature in [
        "loudness",
        "low_end",
        "high_end",
        "onset_strength",
        "onset_density",
    ] {
        let raw = numbers(&value["curve"]["features"][feature]);
        assert_eq!(raw.len(), 16, "{feature}");
        assert!(raw[12] > raw[4], "{feature} rises too: {raw:?}");
    }
    assert!(
        value["summary"]["loudness_integrated_lufs"]
            .as_f64()
            .unwrap()
            < 0.0
    );
    // No beat grid, so no bars; the time curve is there all the same.
    assert_eq!(value["bars"], Value::Null);

    // It is an ordinary Fact of the built-in kind: a Measurement, so it
    // stands among the track's Facts once its audio is checked, and is
    // `unverified` until then.
    let unchecked = ok(
        &sandbox,
        &["--json", "facts", "show", &track, "--kind", "intensity"],
    );
    assert_eq!(unchecked["unverified"][0]["id"], outcome["fact_id"]);
    let facts = ok(
        &sandbox,
        &[
            "--json",
            "facts",
            "show",
            &track,
            "--kind",
            "intensity",
            "--check-audio",
        ],
    );
    assert_eq!(facts["facts"][0]["id"], outcome["fact_id"]);
    assert_eq!(facts["facts"][0]["nature"], "measurement");

    // The watcher was told how it went, and that the Sidecar changed; the
    // Library never did.
    let seen = settled_then_events(&mut watcher, 1);
    let progress: Vec<&Value> = seen
        .iter()
        .filter(|event| event.event == "intensity_progress")
        .map(|event| &event.data)
        .collect();
    assert_eq!(progress.len(), 2, "{seen:?}");
    assert_eq!(progress[0]["status"], "measuring");
    assert_eq!(progress[0]["done"], 0);
    assert_eq!(progress[1]["status"], "measured");
    assert_eq!(progress[1]["done"], 1);
    assert_eq!(progress[1]["total"], 1);
    assert_eq!(progress[1]["track_id"], track.as_str());
    assert!(seen.iter().any(|event| event.event == "sidecar_changed"
        && event.data["kind"] == "intensity"
        && event.data["fact_id"] == outcome["fact_id"]));
    assert!(
        seen.iter().all(|event| event.event != "library_changed"),
        "{seen:?}"
    );
    drop(watcher);
    stop_service(&sandbox);
    assert!(
        before == library_state(&sandbox),
        "measuring changed the Library"
    );
}

#[test]
fn a_louder_busier_file_measures_higher_and_several_tracks_are_one_run() {
    let sandbox = Sandbox::new();
    let hard = import(&sandbox, "Peak.wav", &loud_busy(10.0));
    let soft = import(&sandbox, "Drift.wav", &quiet_sparse(10.0));
    let between = import(&sandbox, "Roll.wav", &loud_busy_at(10.0, 100.0, 0.3));

    let measured = measure(&sandbox, &[&soft, &hard, &between]);
    assert_eq!(measured["measured"], 3);
    let summary = |track: &str| {
        let outcome = measured["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|outcome| outcome["track"]["id"] == track)
            .unwrap();
        outcome["summary"]["intensity"].as_f64().unwrap()
    };
    assert!(summary(&hard) > summary(&between), "{measured}");
    assert!(summary(&between) > summary(&soft), "{measured}");
    assert!(summary(&hard) - summary(&soft) > 0.3, "{measured}");
    // Answered in the order asked.
    assert_eq!(measured["tracks"][0]["track"]["id"], soft.as_str());
    assert_eq!(measured["tracks"][2]["track"]["id"], between.as_str());

    // One Run holds all three.
    let run = measured["run"].as_i64().unwrap();
    for track in [&hard, &soft, &between] {
        assert_eq!(show(&sandbox, track)["intensity"]["run"], run);
    }
}

#[test]
fn unchanged_audio_is_not_measured_again_and_the_same_audio_measures_the_same() {
    let sandbox = Sandbox::new();
    let audio = joined(&[loud_busy(5.0), quiet_sparse(4.0)]);
    let track = import(&sandbox, "Once.wav", &audio);
    // The same audio under another name is another track.
    let twin = import(&sandbox, "Twice.wav", &audio);

    let first = measure(&sandbox, &[&track]);
    assert_eq!(first["tracks"][0]["status"], "measured");
    let stored = show(&sandbox, &track);

    let again = measure(&sandbox, &[&track]);
    assert_eq!(again["tracks"][0]["status"], "reused");
    assert_eq!(again["reused"], 1);
    assert_eq!(again["measured"], 0);
    assert_eq!(again["decoded"], 0, "the audio was not decoded again");
    assert_eq!(
        again["run"],
        Value::Null,
        "nothing was done, so there is no Run"
    );
    assert_eq!(again["tracks"][0]["fact_id"], first["tracks"][0]["fact_id"]);
    assert_eq!(again["tracks"][0]["summary"], first["tracks"][0]["summary"]);
    assert_eq!(show(&sandbox, &track), stored, "nothing new was stored");

    // The text says so too.
    let text = sandbox.cli(&["intensity", "measure", &track]);
    assert!(text.status.success(), "{}", text.stderr);
    assert!(text.stdout.contains("already measured"), "{}", text.stdout);

    // The twin has the same audio, so it is not decoded either: what was
    // measured of that audio is recorded for it, value for value.
    let other = measure(&sandbox, &[&twin]);
    assert_eq!(other["decoded"], 0, "{other}");
    assert_eq!(other["tracks"][0]["status"], "reused");
    assert_eq!(
        other["tracks"][0]["reused_from"],
        first["tracks"][0]["fact_id"]
    );
    let twin_shown = show(&sandbox, &twin);
    assert_eq!(twin_shown["intensity"]["track_id"], twin.as_str());
    assert_eq!(
        twin_shown["intensity"]["fingerprint"],
        stored["intensity"]["fingerprint"]
    );
    assert_eq!(
        twin_shown["intensity"]["value"],
        stored["intensity"]["value"]
    );
    // And from then on it is the twin's own.
    let twin_again = measure(&sandbox, &[&twin]);
    assert_eq!(twin_again["tracks"][0]["reused_from"], Value::Null);
    assert_eq!(
        twin_again["tracks"][0]["fact_id"],
        other["tracks"][0]["fact_id"]
    );

    // Asked to, it measures again, to the same values.
    let forced = ok(
        &sandbox,
        &["--json", "intensity", "measure", &track, "--force"],
    );
    assert_eq!(forced["tracks"][0]["status"], "measured");
    assert_eq!(forced["decoded"], 1);
    let now = show(&sandbox, &track);
    assert_eq!(now["intensity"]["value"], stored["intensity"]["value"]);
    assert_eq!(now["previous"][0]["fact_id"], first["tracks"][0]["fact_id"]);
    assert_eq!(now["previous"][0]["superseded"], true);
}

#[test]
fn changed_audio_makes_the_old_measurement_stale_and_is_measured_afresh() {
    let sandbox = Sandbox::new();
    let track = import(&sandbox, "Swap.wav", &quiet_sparse(6.0));
    let first = measure(&sandbox, &[&track]);
    let old_fact = first["tracks"][0]["fact_id"].clone();
    let old_summary = first["tracks"][0]["summary"]["intensity"].as_f64().unwrap();

    // The DJ replaces the file with different audio.
    write_audio(&audio_path(&sandbox, "Swap.wav"), &loud_busy(6.0));

    // Reading finds out: the old Measurement is no longer the track's.
    let shown = show(&sandbox, &track);
    assert_eq!(shown["intensity"], Value::Null);
    assert_eq!(shown["previous"][0]["fact_id"], old_fact);
    assert_eq!(shown["previous"][0]["stale"], true);

    let second = measure(&sandbox, &[&track]);
    let outcome = &second["tracks"][0];
    assert_eq!(outcome["status"], "measured");
    assert_eq!(second["decoded"], 1);
    assert_ne!(outcome["fact_id"], old_fact);
    assert_ne!(outcome["fingerprint"], first["tracks"][0]["fingerprint"]);
    assert!(outcome["summary"]["intensity"].as_f64().unwrap() > old_summary + 0.3);

    let shown = show(&sandbox, &track);
    assert_eq!(shown["intensity"]["fact_id"], outcome["fact_id"]);
    assert_eq!(shown["intensity"]["stale"], false);
    assert_eq!(shown["previous"][0]["fact_id"], old_fact);
    assert_eq!(shown["previous"][0]["stale"], true);
}

#[test]
fn measuring_after_the_audio_changed_says_which_measurement_went_stale() {
    let sandbox = Sandbox::new();
    let track = import(&sandbox, "Swap.wav", &quiet_sparse(5.0));
    let first = measure(&sandbox, &[&track]);
    write_audio(&audio_path(&sandbox, "Swap.wav"), &loud_busy(5.0));

    let mut watcher = sandbox.client();
    watcher.subscribe().unwrap();
    let second = measure(&sandbox, &[&track]);
    assert_eq!(
        second["tracks"][0]["marked_stale"],
        json!([first["tracks"][0]["fact_id"]])
    );
    let seen = events(&mut watcher);
    assert!(
        seen.iter().any(|event| event.event == "sidecar_changed"
            && event.data["kind"] == "intensity"
            && event.data["stale"] == json!([first["tracks"][0]["fact_id"]])),
        "{seen:?}"
    );
}

#[test]
fn a_track_with_a_beat_grid_gets_a_point_per_bar_and_one_without_gets_the_time_curve() {
    let sandbox = Sandbox::new();
    // 120 BPM: a bar is two seconds. Three quiet bars, then three loud,
    // the first bar beginning half a second in.
    let audio = joined(&[quiet_sparse(6.5), loud_busy_at(6.5, 120.0, 1.0)]);
    let gridded = import(&sandbox, "Gridded.wav", &audio);
    let plain = import(&sandbox, "Plain.wav", &audio);
    give_beat_grid(&sandbox, &gridded, 120.0, 0.5, 13.0);

    let measured = measure(&sandbox, &[&gridded, &plain]);
    assert_eq!(measured["measured"], 2, "{measured}");
    assert_eq!(measured["tracks"][0]["bars"], 7);
    assert_eq!(measured["tracks"][1]["bars"], Value::Null);

    let with = show(&sandbox, &gridded)["intensity"]["value"].clone();
    let bars = &with["bars"];
    assert_eq!(
        numbers(&bars["starts_seconds"]),
        [0.5, 2.5, 4.5, 6.5, 8.5, 10.5, 12.5]
    );
    assert_eq!(bars["end_seconds"], 13.0);
    let per_bar = numbers(&bars["intensity"]);
    assert_eq!(per_bar.len(), 7);
    // The audio changes on the fourth bar line.
    let highest_quiet = per_bar[..3].iter().copied().fold(0.0, f64::max);
    let lowest_loud = per_bar[3..].iter().copied().fold(1.0, f64::min);
    assert!(lowest_loud - highest_quiet > 0.3, "{per_bar:?}");
    assert_eq!(numbers(&bars["features"]["onset_density"]).len(), 7);

    // Without a grid: no bars, and the same time curve as with one.
    let without = show(&sandbox, &plain)["intensity"]["value"].clone();
    assert_eq!(without["bars"], Value::Null);
    assert_eq!(numbers(&without["curve"]["intensity"]).len(), 13);
    assert_eq!(without["curve"], with["curve"]);
    assert_eq!(without["summary"], with["summary"]);
}

#[test]
fn a_playlist_is_measured_in_one_run_with_progress_and_a_bad_track_does_not_stop_the_rest() {
    let sandbox = Sandbox::new();
    let first = import(&sandbox, "One.wav", &loud_busy(4.0));
    let second = import(&sandbox, "Two.wav", &quiet_sparse(4.0));
    let third = import(&sandbox, "Three.wav", &loud_busy_at(4.0, 140.0, 0.5));
    // A fixture track has no audio file.
    let missing = rbl_db::fixture::track_id(3);
    let mut playlist = String::new();
    sandbox.edit(|writer| {
        playlist = writer.create_playlist("Warm up", "root").unwrap();
        writer
            .add_tracks(
                &playlist,
                &[
                    first.clone(),
                    missing.clone(),
                    second.clone(),
                    third.clone(),
                ],
            )
            .unwrap();
    });
    // One was measured before, with the watcher already listening: what
    // that measuring sends is consumed, not left to reach it late.
    sandbox.status();
    let mut watcher = sandbox.client();
    watcher.subscribe().unwrap();
    measure(&sandbox, &[&second]);
    until_settled(&mut watcher, 1);

    let run = sandbox.cli(&["--json", "intensity", "measure", "--playlist", "Warm up"]);
    assert!(run.status.success(), "{}", run.stderr);
    let measured = run.json();
    assert_eq!(measured["total"], 4);
    assert_eq!(measured["measured"], 2);
    assert_eq!(measured["reused"], 1);
    assert_eq!(measured["failed"], 1);
    assert_eq!(measured["decoded"], 2);
    let statuses: Vec<(&str, &str)> = measured["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|outcome| {
            (
                outcome["track"]["id"].as_str().unwrap(),
                outcome["status"].as_str().unwrap(),
            )
        })
        .collect();
    // In the playlist's order.
    assert_eq!(
        statuses,
        [
            (first.as_str(), "measured"),
            (missing.as_str(), "failed"),
            (second.as_str(), "reused"),
            (third.as_str(), "measured"),
        ]
    );
    assert_eq!(measured["tracks"][1]["error"]["code"], "track_file_missing");
    let run_id = measured["run"].as_i64().unwrap();
    assert_eq!(show(&sandbox, &first)["intensity"]["run"], run_id);
    assert_eq!(show(&sandbox, &third)["intensity"]["run"], run_id);

    // Progress: every track is reported once it is settled, counting up.
    let seen = settled_then_events(&mut watcher, 4);
    let settled: Vec<&Value> = seen
        .iter()
        .filter(|event| event.event == "intensity_progress" && event.data["status"] != "measuring")
        .map(|event| &event.data)
        .collect();
    assert_eq!(settled.len(), 4, "{seen:?}");
    let mut done: Vec<u64> = settled
        .iter()
        .map(|data| data["done"].as_u64().unwrap())
        .collect();
    done.sort_unstable();
    assert_eq!(done, [1, 2, 3, 4]);
    assert!(settled.iter().all(|data| data["total"] == 4));

    // In text, progress goes to standard error and the result to output.
    let text = sandbox.cli(&["intensity", "measure", "--playlist", "Warm up", "--force"]);
    assert!(text.status.success(), "{}", text.stderr);
    assert!(text.stderr.contains("[4/4]"), "{}", text.stderr);
    assert!(text.stdout.contains("Measured 3 tracks"), "{}", text.stdout);
    assert!(text.stdout.contains("1 failed"), "{}", text.stdout);

    // Nothing selected is an answer, not an error.
    let none = ok(
        &sandbox,
        &[
            "--json",
            "intensity",
            "measure",
            "--search",
            "no such track anywhere",
        ],
    );
    assert_eq!(none["total"], 0);
    assert_eq!(none["run"], Value::Null);
    // Naming nothing at all is refused, so the whole Library is never
    // measured by accident.
    let nothing = sandbox.cli(&["--json", "intensity", "measure"]);
    assert!(!nothing.status.success());
    assert_eq!(nothing.error_json()["error"]["code"], "bad_request");
}

/// Eight tracks long enough that measuring them one at a time can be
/// caught part-way.
fn eight_tracks(sandbox: &Sandbox) -> Vec<String> {
    (0..8)
        .map(|n| {
            import(
                sandbox,
                &format!("Long {n}.wav"),
                &loud_busy_at(12.0, 120.0 + f64::from(n), 0.8),
            )
        })
        .collect()
}

/// Starts the service told to measure one track at a time (it measures
/// several at once otherwise), and answers its process.
fn start_service_measuring_one_at_a_time(sandbox: &Sandbox) -> u64 {
    let run = sandbox.run(
        sandbox
            .command()
            .env("WRECKORDBOX_INTENSITY_JOBS", "1")
            .args(["--json", "status"]),
    );
    assert!(run.status.success(), "{}", run.stderr);
    run.json()["pid"].as_u64().unwrap()
}

/// Starts measuring every track and answers once the first
/// has been recorded, with the command still running. The caller waits
/// for it.
#[allow(clippy::zombie_processes)]
fn start_measuring_all(
    sandbox: &Sandbox,
    watcher: &mut wreckordbox_service::Client,
) -> std::process::Child {
    let child = sandbox
        .command()
        .args(["--json", "intensity", "measure", "--search", "Long"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        assert!(Instant::now() < deadline, "nothing was measured");
        if let Some(event) = watcher.next_event(Duration::from_millis(200)).unwrap() {
            if event.event == "intensity_progress" && event.data["status"] == "measured" {
                return child;
            }
        }
    }
}

#[test]
fn a_long_measuring_can_be_cancelled_and_keeps_what_it_finished() {
    let sandbox = Sandbox::new();
    let tracks = eight_tracks(&sandbox);
    start_service_measuring_one_at_a_time(&sandbox);
    let mut watcher = sandbox.client();
    watcher.subscribe().unwrap();
    let child = start_measuring_all(&sandbox, &mut watcher);

    // Another measuring is refused meanwhile; then this one is cancelled.
    let busy = sandbox.cli(&["--json", "intensity", "measure", &tracks[7]]);
    assert!(!busy.status.success());
    assert_eq!(busy.error_json()["error"]["code"], "intensity_busy");
    let cancelled = ok(&sandbox, &["--json", "intensity", "cancel"]);
    assert_eq!(cancelled["cancelled"], true);

    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let measured: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(measured["cancelled"], true);
    assert_eq!(measured["total"], 8);
    let done = measured["measured"].as_u64().unwrap();
    assert!((1..8).contains(&done), "{measured}");
    let skipped = measured["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|outcome| outcome["status"] == "cancelled")
        .count() as u64;
    assert_eq!(done + skipped, 8);

    // What was finished is kept, and the rest is measured next time.
    let rest = ok(
        &sandbox,
        &["--json", "intensity", "measure", "--search", "Long"],
    );
    assert_eq!(rest["cancelled"], false);
    assert_eq!(rest["reused"], done);
    assert_eq!(rest["measured"], 8 - done);
    // With nothing running there is nothing to cancel.
    assert_eq!(
        ok(&sandbox, &["--json", "intensity", "cancel"])["cancelled"],
        false
    );
}

#[test]
fn a_service_killed_part_way_leaves_a_sidecar_that_still_works() {
    let sandbox = Sandbox::new();
    eight_tracks(&sandbox);
    let pid = start_service_measuring_one_at_a_time(&sandbox);
    let mut watcher = sandbox.client();
    watcher.subscribe().unwrap();
    let mut child = start_measuring_all(&sandbox, &mut watcher);

    // No warning, no clean-up.
    let killed = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    let _ = child.wait();
    drop(watcher);
    let gone = Instant::now() + Duration::from_secs(10);
    while common::alive(pid) && Instant::now() < gone {
        std::thread::sleep(Duration::from_millis(20));
    }

    // A new service opens the same Sidecar: what was recorded is whole
    // and is reused, and the rest is measured.
    let rest = ok(
        &sandbox,
        &["--json", "intensity", "measure", "--search", "Long"],
    );
    assert_eq!(rest["total"], 8);
    assert_eq!(rest["failed"], 0, "{rest}");
    let reused = rest["reused"].as_u64().unwrap();
    assert!((1..8).contains(&reused), "{rest}");
    assert_eq!(rest["measured"], 8 - reused);
    for outcome in rest["tracks"].as_array().unwrap() {
        let shown = show(&sandbox, outcome["track"]["id"].as_str().unwrap());
        assert_eq!(shown["intensity"]["fact_id"], outcome["fact_id"]);
        assert_eq!(
            numbers(&shown["intensity"]["value"]["curve"]["intensity"]).len(),
            12
        );
    }
}

/// Records `value` as an Intensity of `track` from this tool, as if an
/// earlier build had measured it.
fn seed(sandbox: &Sandbox, track: &str, value: &Value) -> Value {
    ok(
        sandbox,
        &[
            "--json",
            "facts",
            "record",
            track,
            "intensity",
            &value.to_string(),
            "--source",
            "tool:wreckordbox-intensity",
            "--confidence",
            "1",
        ],
    )
}

#[test]
fn a_stored_value_of_an_older_method_or_not_a_valid_one_is_measured_again() {
    let sandbox = Sandbox::new();
    let measured = import(&sandbox, "Measured.wav", &loud_busy(4.0));
    let older = import(&sandbox, "Older.wav", &quiet_sparse(4.0));
    let broken = import(&sandbox, "Broken.wav", &loud_busy_at(4.0, 100.0, 0.5));
    let unfinished = import(&sandbox, "Unfinished.wav", &quiet_sparse(5.0));
    measure(&sandbox, &[&measured]);
    let good = show(&sandbox, &measured)["intensity"]["value"].clone();

    // A whole value, but by version 1 of the method.
    let mut old = good.clone();
    old["method"]["version"] = json!(1);
    seed(&sandbox, &older, &old);
    // An object, but not an Intensity.
    seed(&sandbox, &broken, &json!({"method": {"version": 2}}));
    // An Intensity whose number did not survive: NaN is stored as null.
    let mut nan = good.clone();
    nan["summary"]["intensity"] = Value::Null;
    seed(&sandbox, &unfinished, &nan);

    let again = measure(&sandbox, &[&older, &broken, &unfinished]);
    assert_eq!(again["decoded"], 3, "{again}");
    assert_eq!(again["measured"], 3, "{again}");
    for track in [&older, &broken, &unfinished] {
        let shown = show(&sandbox, track);
        assert_eq!(shown["intensity"]["value"]["method"]["version"], 2);
        assert!(shown["intensity"]["value"]["summary"]["intensity"].is_number());
    }
}

/// Every analysis file under the fixture's share root, with its bytes.
fn analysis_files(sandbox: &Sandbox) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.map(Result::unwrap) {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.to_string_lossy().contains("ANLZ") {
                out.push((path.clone(), std::fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(&sandbox.location().share_root, &mut out);
    out.sort();
    out
}

#[test]
fn measuring_an_analysed_track_leaves_its_analysis_tempo_and_key_as_they_were() {
    let sandbox = Sandbox::new();
    let track = import(&sandbox, "Analysed.wav", &loud_busy_at(9.0, 120.0, 1.0));
    give_beat_grid(&sandbox, &track, 120.0, 0.5, 9.0);
    sandbox.edit(|writer| {
        let changed = writer
            .library()
            .connection()
            .execute(
                "UPDATE djmdContent SET BPM = 12000, KeyID = '1' WHERE ID = ?1",
                [track.as_str()],
            )
            .unwrap();
        assert_eq!(changed, 1);
    });
    let tempo_and_key = |sandbox: &Sandbox| {
        let mut found = (0_i64, String::new());
        sandbox.edit(|writer| {
            found = writer
                .library()
                .connection()
                .query_row(
                    "SELECT BPM, coalesce(KeyID, '') FROM djmdContent WHERE ID = ?1",
                    [track.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
        });
        found
    };
    let analysis = analysis_files(&sandbox);
    assert!(!analysis.is_empty(), "the track has an analysis file");
    let before = (library_state(&sandbox), tempo_and_key(&sandbox));
    assert_eq!(before.1, (12000, "1".to_owned()));

    let measured = measure(&sandbox, &[&track]);
    assert_eq!(measured["measured"], 1);
    assert_eq!(measured["tracks"][0]["bars"], 5);
    show(&sandbox, &track);
    stop_service(&sandbox);

    assert!(
        analysis == analysis_files(&sandbox),
        "measuring changed an analysis file"
    );
    assert!(
        before == (library_state(&sandbox), tempo_and_key(&sandbox)),
        "measuring changed the Library"
    );
}

#[test]
fn the_bars_follow_a_beat_grid_added_or_corrected_after_measuring_without_decoding() {
    let sandbox = Sandbox::new();
    // 120 BPM: a bar is two seconds.
    let audio = joined(&[quiet_sparse(6.5), loud_busy_at(6.5, 120.0, 1.0)]);
    let track = import(&sandbox, "Later.wav", &audio);
    let first = measure(&sandbox, &[&track]);
    assert_eq!(first["tracks"][0]["bars"], Value::Null);
    let curve = show(&sandbox, &track)["intensity"]["value"]["curve"].clone();

    // rekordbox analyses it: a grid with its first bar half a second in.
    stop_service(&sandbox);
    give_beat_grid(&sandbox, &track, 120.0, 0.5, 13.0);
    let added = measure(&sandbox, &[&track]);
    let outcome = &added["tracks"][0];
    assert_eq!(outcome["status"], "reused", "{added}");
    assert_eq!(added["decoded"], 0);
    assert_eq!(outcome["bars"], 7);
    assert_eq!(outcome["reused_from"], first["tracks"][0]["fact_id"]);
    let value = show(&sandbox, &track)["intensity"]["value"].clone();
    assert_eq!(value["curve"], curve);
    assert_eq!(
        numbers(&value["bars"]["starts_seconds"]),
        [0.5, 2.5, 4.5, 6.5, 8.5, 10.5, 12.5]
    );
    let per_bar = numbers(&value["bars"]["intensity"]);
    assert!(per_bar[3] - per_bar[2] > 0.3, "{per_bar:?}");

    // The DJ moves the downbeat: the next reading has the new bars, made
    // from the stored value.
    stop_service(&sandbox);
    give_beat_grid(&sandbox, &track, 120.0, 1.5, 13.0);
    let shown = show(&sandbox, &track);
    let bars = &shown["intensity"]["value"]["bars"];
    assert_eq!(
        numbers(&bars["starts_seconds"]),
        [1.5, 3.5, 5.5, 7.5, 9.5, 11.5]
    );
    assert_ne!(shown["intensity"]["fact_id"], outcome["fact_id"]);
    assert_eq!(shown["intensity"]["value"]["curve"], curve);
    // Measuring now has nothing to do.
    let settled = measure(&sandbox, &[&track]);
    assert_eq!(settled["decoded"], 0);
    assert_eq!(settled["run"], Value::Null);
    assert_eq!(
        settled["tracks"][0]["fact_id"],
        shown["intensity"]["fact_id"]
    );

    // And once more through measuring: tempo corrected to 128 BPM.
    stop_service(&sandbox);
    give_beat_grid(&sandbox, &track, 128.0, 0.5, 13.0);
    let corrected = measure(&sandbox, &[&track]);
    assert_eq!(corrected["decoded"], 0);
    assert_eq!(corrected["tracks"][0]["status"], "reused");
    let bars = &show(&sandbox, &track)["intensity"]["value"]["bars"];
    assert_eq!(numbers(&bars["starts_seconds"])[1], 2.375);
}

#[test]
fn audio_replaced_while_it_is_measured_is_measured_as_it_is_now() {
    let sandbox = Sandbox::new();
    let track = import(&sandbox, "Moving.wav", &quiet_sparse(5.0));
    let pause = sandbox.path().join("pause");
    std::fs::create_dir_all(&pause).unwrap();
    let marker = pause.join(format!("{track}.decode"));
    std::fs::write(&marker, b"").unwrap();
    let started = sandbox.run(
        sandbox
            .command()
            .env("WRECKORDBOX_INTENSITY_TEST_PAUSE", &pause)
            .args(["--json", "status"]),
    );
    assert!(started.status.success(), "{}", started.stderr);
    let child = sandbox
        .command()
        .args(["--json", "intensity", "measure", &track])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    // Fingerprinted as the quiet file; replaced by a loud one before it is
    // decoded.
    let reached = pause.join(format!("{track}.decode.reached"));
    let deadline = Instant::now() + Duration::from_secs(60);
    while !reached.exists() {
        assert!(Instant::now() < deadline, "the measuring never paused");
        std::thread::sleep(Duration::from_millis(10));
    }
    write_audio(&audio_path(&sandbox, "Moving.wav"), &loud_busy(5.0));
    std::fs::remove_file(&marker).unwrap();

    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let measured: Value = serde_json::from_slice(&output.stdout).unwrap();
    let outcome = &measured["tracks"][0];
    assert_eq!(outcome["status"], "measured", "{measured}");
    // What was recorded is the loud audio, under the loud audio's name.
    let shown = show(&sandbox, &track);
    assert_eq!(shown["intensity"]["fingerprint"], outcome["fingerprint"]);
    assert_eq!(shown["intensity"]["stale"], false);
    let loud_only = import_elsewhere_fingerprint(&sandbox, &loud_busy(5.0));
    assert_eq!(outcome["fingerprint"], loud_only);
    assert!(
        outcome["summary"]["intensity"].as_f64().unwrap() > 0.5,
        "{outcome}"
    );
}

/// The fingerprint a track with this audio gets, read through the service
/// from a track that holds nothing else.
fn import_elsewhere_fingerprint(sandbox: &Sandbox, samples: &[f32]) -> Value {
    stop_service(sandbox);
    let other = import(sandbox, "Reference.wav", samples);
    let measured = ok(
        sandbox,
        &["--json", "intensity", "measure", &other, "--force"],
    );
    measured["tracks"][0]["fingerprint"].clone()
}

#[test]
fn a_track_removed_and_added_again_keeps_its_intensity_without_decoding() {
    let sandbox = Sandbox::new();
    let audio = joined(&[loud_busy(4.0), quiet_sparse(3.0)]);
    let track = import(&sandbox, "Again.wav", &audio);
    let first = measure(&sandbox, &[&track]);
    assert_eq!(first["tracks"][0]["status"], "measured");
    let fact = first["tracks"][0]["fact_id"].clone();

    // Removed from the Library and imported again: a new identifier.
    stop_service(&sandbox);
    let mut again = String::new();
    sandbox.edit(|writer| {
        writer.delete_track(&track).unwrap();
        again = writer
            .import_file(&audio_path(&sandbox, "Again.wav"))
            .unwrap();
    });
    assert_ne!(track, again);

    sandbox.status();
    let mut watcher = sandbox.client();
    watcher.subscribe().unwrap();
    // The Measurement is re-attached by its fingerprint, as every Fact is,
    // and is the answer: the same Fact, nothing decoded, nothing new.
    let measured = measure(&sandbox, &[&again]);
    assert_eq!(measured["tracks"][0]["status"], "reused", "{measured}");
    assert_eq!(measured["decoded"], 0);
    assert_eq!(measured["tracks"][0]["fact_id"], fact);
    assert_eq!(measured["tracks"][0]["reused_from"], Value::Null);
    let seen = events(&mut watcher);
    let reattached = seen
        .iter()
        .find(|event| event.event == "facts_reattached")
        .unwrap_or_else(|| panic!("{seen:?}"));
    assert_eq!(reattached.data["from_track"], track.as_str());
    assert_eq!(reattached.data["to_track"], again.as_str());
    let shown = show(&sandbox, &again);
    assert_eq!(shown["intensity"]["fact_id"], fact);
    assert_eq!(shown["intensity"]["track_id"], again.as_str());
}

#[test]
fn audio_changed_and_changed_back_makes_the_measurement_current_again() {
    let sandbox = Sandbox::new();
    let audio = joined(&[loud_busy(4.0), quiet_sparse(3.0)]);
    let track = import(&sandbox, "Back.wav", &audio);
    let first = measure(&sandbox, &[&track]);
    let fact = first["tracks"][0]["fact_id"].clone();

    let path = audio_path(&sandbox, "Back.wav");
    let original = std::fs::read(&path).unwrap();
    write_audio(&path, &quiet_sparse(7.0));
    let shown = show(&sandbox, &track);
    assert_eq!(shown["intensity"], Value::Null, "{shown}");
    assert_eq!(shown["previous"][0]["fact_id"], fact);
    assert_eq!(shown["previous"][0]["stale"], true);

    // The original audio is back: its Measurement is current again, not
    // copied, and measuring reuses it without decoding.
    std::fs::write(&path, original).unwrap();
    let shown = show(&sandbox, &track);
    assert_eq!(shown["intensity"]["fact_id"], fact, "{shown}");
    assert_eq!(shown["intensity"]["stale"], false);
    let measured = measure(&sandbox, &[&track]);
    assert_eq!(measured["tracks"][0]["status"], "reused");
    assert_eq!(measured["tracks"][0]["fact_id"], fact);
    assert_eq!(measured["tracks"][0]["reused_from"], Value::Null);
    assert_eq!(measured["decoded"], 0);
}
