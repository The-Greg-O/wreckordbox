//! A backup before a large Run (wbx-25) through the real CLI and a service
//! on a fixture Library: a projection, a MyTag projection and a Review
//! accept that will change more tracks than the threshold back up first,
//! and their Runs name the backup; smaller ones, and dry runs, do not; a
//! backup that fails or is refused stops the Run with nothing changed.
//! Never the real Library, real backups or a real Stick.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
use crate::common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::Sandbox;
use serde_json::{json, Value};

fn ok(s: &Sandbox, args: &[&str]) -> Value {
    let mut words = vec!["--json"];
    words.extend_from_slice(args);
    let result = s.cli(&words);
    assert!(
        result.status.success(),
        "{args:?}: {}{}",
        result.stdout,
        result.stderr
    );
    result.json()
}

fn refused(s: &Sandbox, args: &[&str]) -> Value {
    let mut words = vec!["--json"];
    words.extend_from_slice(args);
    let result = s.cli(&words);
    assert!(!result.status.success(), "{args:?}: {}", result.stdout);
    result.error_json()["error"].clone()
}

fn policy(s: &Sandbox, text: &str) {
    std::fs::create_dir_all(s.sidecar_dir()).unwrap();
    std::fs::write(s.sidecar_dir().join("projection-policy.toml"), text).unwrap();
}

fn energy(s: &Sandbox, track: &str, value: &str) {
    ok(
        s,
        &[
            "facts",
            "record",
            track,
            "energy",
            value,
            "--source",
            "agent:tagger",
            "--confidence",
            "0.9",
        ],
    );
}

fn ratings(s: &Sandbox) -> Vec<Value> {
    ["10001", "10002", "10003"]
        .iter()
        .map(|t| rating_of(s, t))
        .collect()
}

fn rating_of(s: &Sandbox, track: &str) -> Value {
    ok(s, &["tracks", "show", track])["rating"].clone()
}

/// Every file in `dir` named as a backup, or left by one (`.partial-`).
fn backups_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            name.starts_with("rbexport-") || name.starts_with(".partial-")
        })
        .collect();
    found.sort();
    found
}

/// The backups `wreckordbox backup list` shows.
fn listed(s: &Sandbox) -> Vec<String> {
    ok(s, &["backup", "list"])["backups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|backup| backup["path"].as_str().unwrap().to_owned())
        .collect()
}

/// The `large_run` every entry of the Run `answer` names was recorded with.
fn recorded(s: &Sandbox, answer: &Value) -> Value {
    let run = answer["run"]["id"].as_i64().expect("the apply made a Run");
    let shown = ok(s, &["runs", "show", &run.to_string()]);
    let entries = shown["entries"].as_array().unwrap();
    assert!(!entries.is_empty(), "{shown}");
    let first = entries[0]["command"]["args"]["large_run"].clone();
    for entry in entries {
        assert_eq!(entry["command"]["args"]["large_run"], first, "{shown}");
    }
    first
}

fn stop_service(s: &Sandbox) {
    let socket = s.config().socket_path(s.location());
    if let Ok(mut client) = wreckordbox_service::Client::connect(&socket) {
        let _ = client.stop();
    }
    let gone = (0..400).any(|_| {
        std::thread::sleep(Duration::from_millis(25));
        wreckordbox_service::Client::connect(&socket).is_err()
    });
    assert!(gone, "the service did not stop");
    common::released::wait_until_released(s, "the stopped service");
}

/// Starts a service here with `env` set, and waits until it is up.
fn serve_with(s: &Sandbox, env: &[(&str, &str)]) -> std::process::Child {
    let mut command = s.command();
    for (name, value) in env {
        command.env(name, value);
    }
    let child = command
        .arg("serve")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let socket = s.config().socket_path(s.location());
    assert!(
        (0..400).any(|_| {
            std::thread::sleep(Duration::from_millis(25));
            wreckordbox_service::Client::connect(&socket).is_ok()
        }),
        "the service did not come up"
    );
    child
}

/// Three tracks with an agent's Energy waiting to be projected.
fn three_to_project(s: &Sandbox, policy_text: &str) {
    policy(s, policy_text);
    ok(s, &["status"]);
    energy(s, "10001", "4");
    energy(s, "10002", "2");
    energy(s, "10003", "5");
}

#[test]
fn a_projection_above_the_threshold_is_backed_up_first_and_its_run_names_the_backup() {
    let s = Sandbox::new();
    three_to_project(&s, "[energy]\napply = 0.5\n\n[backup]\nabove_tracks = 2\n");
    let before: Vec<Value> = ["10001", "10002", "10003"]
        .iter()
        .map(|t| rating_of(&s, t))
        .collect();

    // A dry run takes no backup and writes nothing.
    let dry = ok(&s, &["project", "--dry-run"]);
    assert_eq!(dry["changes"].as_array().unwrap().len(), 3);
    assert!(dry.get("large_run").is_none(), "{dry}");
    assert_eq!(backups_in(&s.state_dir()), Vec::<PathBuf>::new());

    let applied = ok(&s, &["project"]);
    let taken = &applied["large_run"];
    assert_eq!(taken["tracks"], 3, "{applied}");
    assert_eq!(taken["above"], 2);
    let path = taken["backup"]["path"]
        .as_str()
        .expect("a backup was taken");
    assert_eq!(backups_in(&s.state_dir()), vec![PathBuf::from(path)]);
    assert_eq!(listed(&s), vec![path.to_owned()]);
    // The Run's Ledger record names it, with the threshold in force.
    assert_eq!(recorded(&s, &applied), *taken);
    assert_ne!(rating_of(&s, "10001"), before[0]);

    // The backup is of the Library before the Run: restored, the Energy
    // the projection wrote is gone again.
    stop_service(&s);
    let restored = s.cli(&["--json", "backup", "restore", path, "--yes"]);
    assert!(
        restored.status.success(),
        "{}{}",
        restored.stdout,
        restored.stderr
    );
    let after: Vec<Value> = ["10001", "10002", "10003"]
        .iter()
        .map(|t| rating_of(&s, t))
        .collect();
    assert_eq!(after, before);
}

#[test]
fn a_run_at_the_threshold_takes_no_backup_and_records_the_threshold_in_force() {
    let s = Sandbox::new();
    three_to_project(&s, "[energy]\napply = 0.5\n\n[backup]\nabove_tracks = 3\n");
    let applied = ok(&s, &["project"]);
    assert_eq!(
        applied["large_run"],
        json!({"tracks": 3, "run_tracks": 3, "above": 3, "backup": null}),
        "{applied}"
    );
    assert_eq!(recorded(&s, &applied), applied["large_run"]);
    assert_eq!(backups_in(&s.state_dir()), Vec::<PathBuf>::new());
}

#[test]
fn with_no_threshold_in_the_policy_the_default_is_in_force() {
    let s = Sandbox::new();
    three_to_project(&s, "[energy]\napply = 0.5\n");
    let applied = ok(&s, &["project"]);
    assert_eq!(
        applied["large_run"],
        json!({"tracks": 3, "run_tracks": 3, "above": wreckordbox_service::large_run::DEFAULT_ABOVE, "backup": null})
    );
}

/// The count is the apply's own plan, not a dry run's: a dry run made when
/// one track was waiting does not let a three-track apply skip its backup.
#[test]
fn the_count_is_of_the_plan_applied_not_an_earlier_dry_run() {
    let s = Sandbox::new();
    policy(&s, "[energy]\napply = 0.5\n\n[backup]\nabove_tracks = 2\n");
    ok(&s, &["status"]);
    energy(&s, "10001", "4");
    let dry = ok(&s, &["project", "--dry-run"]);
    assert_eq!(dry["changes"].as_array().unwrap().len(), 1);
    energy(&s, "10002", "2");
    energy(&s, "10003", "5");
    // The checked plan is refused as stale, with nothing written...
    let stale = refused(&s, &["project", "--expect", dry["plan"].as_str().unwrap()]);
    assert_eq!(stale["code"], "projection_changed");
    assert_eq!(backups_in(&s.state_dir()), Vec::<PathBuf>::new());
    // ...and the apply that writes counts what it writes.
    let applied = ok(&s, &["project"]);
    assert_eq!(applied["large_run"]["tracks"], 3);
    assert!(
        applied["large_run"]["backup"]["path"].is_string(),
        "{applied}"
    );
}

#[test]
fn a_backup_that_fails_stops_the_run_with_nothing_changed_or_recorded() {
    let s = Sandbox::new();
    three_to_project(&s, "[energy]\napply = 0.5\n\n[backup]\nabove_tracks = 2\n");
    let unchanged = ratings(&s);
    let runs_before = ok(&s, &["runs", "list"])["total"].clone();
    stop_service(&s);
    let mut service = serve_with(&s, &[("WRECKORDBOX_RUN_BACKUP_FAULT", "disk_full")]);
    let error = refused(&s, &["project"]);
    assert_eq!(error["code"], "run_backup_failed", "{error}");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("nothing was changed"),
        "{error}"
    );
    assert_eq!(ratings(&s), unchanged);
    assert_eq!(ok(&s, &["runs", "list"])["total"], runs_before);
    // Nothing is left in the backup folder, not even the partial archive.
    assert_eq!(backups_in(&s.state_dir()), Vec::<PathBuf>::new());
    service.kill().unwrap();
    service.wait().unwrap();
}

#[test]
fn a_backup_folder_on_a_protected_stick_stops_the_run_and_writes_nothing_there() {
    let s = Sandbox::new();
    let stick = s.path().join("stick");
    std::fs::create_dir_all(&stick).unwrap();
    std::fs::write(
        stick.join(wreckordbox_stick_protection::STAND_IN_MARKER),
        b"",
    )
    .unwrap();
    let folder = stick.join("backups");
    std::fs::create_dir_all(s.state_dir()).unwrap();
    std::fs::write(
        s.state_dir().join(rbl_backup::DESTINATION_FILE),
        serde_json::to_vec(&folder).unwrap(),
    )
    .unwrap();
    three_to_project(&s, "[energy]\napply = 0.5\n\n[backup]\nabove_tracks = 2\n");
    let unchanged = ratings(&s);
    let runs_before = ok(&s, &["runs", "list"])["total"].clone();
    let error = refused(&s, &["project"]);
    assert_eq!(error["code"], "stick_protected", "{error}");
    assert_eq!(ratings(&s), unchanged);
    assert_eq!(ok(&s, &["runs", "list"])["total"], runs_before);
    let on_stick: Vec<_> = std::fs::read_dir(&stick)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(
        on_stick,
        vec![std::ffi::OsString::from(
            wreckordbox_stick_protection::STAND_IN_MARKER
        )]
    );
}

#[test]
fn library_protection_stops_a_large_run_before_its_backup() {
    let s = Sandbox::new();
    three_to_project(&s, "[energy]\napply = 0.5\n\n[backup]\nabove_tracks = 2\n");
    let unchanged = ratings(&s);
    s.protect(true);
    let error = refused(&s, &["project"]);
    assert_eq!(error["code"], "library_protected", "{error}");
    assert_eq!(backups_in(&s.state_dir()), Vec::<PathBuf>::new());
    s.protect(false);
    assert_eq!(ratings(&s), unchanged);
}

#[test]
fn a_threshold_that_is_not_a_number_of_tracks_changes_nothing() {
    let s = Sandbox::new();
    three_to_project(
        &s,
        "[energy]\napply = 0.5\n\n[backup]\nabove_tracks = \"many\"\n",
    );
    let unchanged = ratings(&s);
    let error = refused(&s, &["project"]);
    assert_eq!(error["code"], "projection_policy_invalid", "{error}");
    assert_eq!(ratings(&s), unchanged);
    assert_eq!(backups_in(&s.state_dir()), Vec::<PathBuf>::new());
}

/// The service dies after the backup and before the first change: the
/// Library is unchanged, no Run or barrier is left, and the backup is an
/// ordinary complete one, listed and kept.
#[test]
fn a_crash_between_the_backup_and_the_first_change_leaves_a_kept_backup_and_no_change() {
    let s = Sandbox::new();
    three_to_project(&s, "[energy]\napply = 0.5\n\n[backup]\nabove_tracks = 2\n");
    let unchanged = ratings(&s);
    let runs_before = ok(&s, &["runs", "list"])["total"].clone();
    stop_service(&s);
    let gate = tempfile::tempdir().unwrap();
    let mut service = serve_with(
        &s,
        &[("WRECKORDBOX_PROJECTION_GATE", gate.path().to_str().unwrap())],
    );
    let mut apply = s
        .command()
        .args(["--json", "project"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while !gate.path().join("paused").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the apply never paused"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let taken = backups_in(&s.state_dir());
    assert_eq!(
        taken.len(),
        1,
        "the backup is taken before the pause: {taken:?}"
    );
    service.kill().unwrap();
    service.wait().unwrap();
    assert!(!apply.wait().unwrap().success(), "the apply was cut off");

    assert_eq!(ratings(&s), unchanged);
    assert_eq!(ok(&s, &["runs", "list"])["total"], runs_before);
    assert_eq!(ok(&s, &["projection", "barriers"])["barriers"], json!([]));
    assert_eq!(listed(&s), vec![taken[0].to_string_lossy().into_owned()]);
    // And the Run can be made again: it backs up again first.
    let applied = ok(&s, &["project"]);
    assert!(applied["large_run"]["backup"]["path"].is_string());
    assert_eq!(backups_in(&s.state_dir()).len(), 2);
}

const SCHEMA: &str = r#"version = 1

[genre]
terms = [
  { id = "house", name = "House" },
]

[dimensions.mood]
terms = [
  { id = "dark", name = "Dark" },
]

[dimensions.era]
terms = [
  { id = "nineties", name = "90s" },
]

[dimensions.situation]
terms = [
  { id = "peak", name = "Peak" },
]

[deck]
categories = [
  { dimension = "mood", label = "Mood" },
  { dimension = "genre", label = "Sub-genre" },
  { dimension = "era", label = "Era" },
]

[deck.genre]
child_prefix = "> "
tag_parents = true
"#;

#[test]
fn a_large_mytag_projection_is_backed_up_first_and_a_dry_run_is_not() {
    let s = Sandbox::new();
    let [_, mood, _, sub_genre] = s.four_categories();
    s.create_my_tag_in(&mood, "Dark");
    s.create_my_tag_in(&sub_genre, "House");
    std::fs::create_dir_all(s.sidecar_dir()).unwrap();
    std::fs::write(s.sidecar_dir().join("tag-schema.toml"), SCHEMA).unwrap();
    policy(&s, "[mytags]\napply = 0.5\n\n[backup]\nabove_tracks = 2\n");
    ok(&s, &["status"]);
    for track in ["10001", "10002", "10003"] {
        ok(
            &s,
            &[
                "facts",
                "record",
                track,
                "mood",
                "dark",
                "--source",
                "agent:tagger",
                "--confidence",
                "0.9",
            ],
        );
    }
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(dry["changes"].as_array().unwrap().len(), 3, "{dry}");
    assert!(dry.get("large_run").is_none());
    assert_eq!(backups_in(&s.state_dir()), Vec::<PathBuf>::new());
    let applied = ok(&s, &["project", "--mytags"]);
    assert_eq!(applied["large_run"]["tracks"], 3, "{applied}");
    let path = applied["large_run"]["backup"]["path"].as_str().unwrap();
    assert_eq!(backups_in(&s.state_dir()), vec![PathBuf::from(path)]);
    assert_eq!(recorded(&s, &applied), applied["large_run"]);
}

#[test]
fn a_large_review_accept_is_backed_up_first() {
    let s = Sandbox::new();
    three_to_project(
        &s,
        "[energy]\napply = \"wait\"\n\n[backup]\nabove_tracks = 2\n",
    );
    let waiting = ok(&s, &["review", "waiting"]);
    assert_eq!(waiting["count"], 3);
    assert_eq!(backups_in(&s.state_dir()), Vec::<PathBuf>::new());
    let token = waiting["token"].as_str().unwrap();
    let accepted = ok(
        &s,
        &[
            "review", "accept", "--all", "--token", token, "--source", "dj",
        ],
    );
    assert_eq!(accepted["count"], 3, "{accepted}");
    assert_eq!(accepted["large_run"]["tracks"], 3);
    let path = accepted["large_run"]["backup"]["path"].as_str().unwrap();
    assert_eq!(backups_in(&s.state_dir()), vec![PathBuf::from(path)]);
    assert_eq!(recorded(&s, &accepted), accepted["large_run"]);
    assert_eq!(rating_of(&s, "10001"), json!(4));
}

/// The cost asked for: the backup a projection of a 2,000-track Library
/// takes first, against the same apply with no backup.
#[test]
fn backing_up_before_projecting_two_thousand_tracks_is_measured() {
    let measure = |above: usize| {
        let s = Sandbox::with_shape(rbl_db::fixture::Shape {
            tracks: 2000,
            ..rbl_db::fixture::Shape::default()
        });
        policy(
            &s,
            &format!("[energy]\napply = 0.5\n\n[backup]\nabove_tracks = {above}\n"),
        );
        ok(&s, &["status"]);
        let lines: String = (0..2000)
            .map(|i| {
                json!({"track": rbl_db::fixture::track_id(i), "kind": "energy", "value": 1 + i % 5, "confidence": 0.9})
                    .to_string()
                    + "\n"
            })
            .collect();
        let file = s.path().join("facts.jsonl");
        std::fs::write(&file, lines).unwrap();
        ok(
            &s,
            &[
                "facts",
                "record-many",
                "--file",
                file.to_str().unwrap(),
                "--source",
                "agent:bulk",
            ],
        );
        let started = std::time::Instant::now();
        let applied = ok(&s, &["project"]);
        (applied, started.elapsed())
    };
    let (without, plain) = measure(100_000);
    assert!(without["large_run"]["backup"].is_null());
    let (with, backed_up) = measure(200);
    let backup = &with["large_run"]["backup"];
    assert!(backup["path"].is_string(), "{with}");
    eprintln!(
        "2,000 tracks: apply {plain:?} without a backup, {backed_up:?} with one; the backup itself {} ms, {} bytes",
        backup["took_ms"], backup["bytes"]
    );
}

/// Everything in the Sidecar a restore must bring back as it was: every
/// Fact, every Run and Ledger entry, and both kinds of projection marks;
/// leaving out the restore's own record (its `restore` entry and Run).
fn sidecar_state(s: &Sandbox) -> Vec<String> {
    let db = rusqlite::Connection::open_with_flags(
        s.sidecar_dir().join("sidecar.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let rows = |sql: &str| -> Vec<String> {
        let mut statement = db.prepare(sql).unwrap();
        let count = statement.column_count();
        statement
            .query_map([], |row| {
                Ok((0..count)
                    .map(|i| match row.get_ref(i).unwrap() {
                        rusqlite::types::ValueRef::Null => "NULL".to_owned(),
                        rusqlite::types::ValueRef::Integer(n) => n.to_string(),
                        rusqlite::types::ValueRef::Real(r) => r.to_string(),
                        rusqlite::types::ValueRef::Text(t) => {
                            String::from_utf8_lossy(t).into_owned()
                        }
                        rusqlite::types::ValueRef::Blob(b) => format!("{b:?}"),
                    })
                    .collect::<Vec<_>>()
                    .join("|"))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let restore_runs = "SELECT run_id FROM ledger WHERE kind = 'restore'";
    let mut state = Vec::new();
    for (name, sql) in [
        ("fact", "SELECT * FROM fact ORDER BY id".to_owned()),
        (
            "ledger",
            format!("SELECT * FROM ledger WHERE run_id NOT IN ({restore_runs}) ORDER BY id"),
        ),
        (
            "run",
            format!("SELECT * FROM run WHERE id NOT IN ({restore_runs}) ORDER BY id"),
        ),
        (
            "energy and prep colour marks",
            "SELECT key, value FROM meta WHERE key LIKE 'projection_row:%' ORDER BY key".to_owned(),
        ),
        (
            "mytag marks",
            "SELECT key, value FROM meta WHERE key LIKE 'projection_tag:%' ORDER BY key".to_owned(),
        ),
    ] {
        let found = rows(&sql);
        state.push(format!("{name}: {}", found.len()));
        state.extend(found.into_iter().map(|row| format!("{name}: {row}")));
    }
    state
}

/// Review finding 4: restored, the backup gives back the whole Sidecar as
/// it was before the Run (Facts, Ledger, both kinds of marks), with the
/// Library, not only the Energy the Run wrote.
#[test]
fn restoring_a_large_runs_backup_gives_back_the_library_and_the_whole_sidecar_before_it() {
    let s = Sandbox::new();
    let [_, mood, _, sub_genre] = s.four_categories();
    s.create_my_tag_in(&mood, "Dark");
    s.create_my_tag_in(&sub_genre, "House");
    std::fs::create_dir_all(s.sidecar_dir()).unwrap();
    std::fs::write(s.sidecar_dir().join("tag-schema.toml"), SCHEMA).unwrap();
    policy(
        &s,
        "[energy]\napply = 0.5\n\n[mytags]\napply = 0.5\n\n[backup]\nabove_tracks = 2\n",
    );
    ok(&s, &["status"]);
    // Small Runs first, which leave marks of both kinds.
    energy(&s, "10001", "3");
    assert!(ok(&s, &["project"])["large_run"]["backup"].is_null());
    ok(
        &s,
        &[
            "facts",
            "record",
            "10001",
            "mood",
            "dark",
            "--source",
            "agent:tagger",
            "--confidence",
            "0.9",
        ],
    );
    assert!(ok(&s, &["project", "--mytags"])["large_run"]["backup"].is_null());
    for (track, value) in [("10002", "4"), ("10003", "2"), ("10004", "5")] {
        energy(&s, track, value);
    }
    // Read with no service holding the Sidecar open.
    stop_service(&s);
    let before = sidecar_state(&s);
    assert!(
        before
            .iter()
            .any(|row| row.starts_with("energy and prep colour marks: projection_row:"))
            && before
                .iter()
                .any(|row| row.starts_with("mytag marks: projection_tag:")),
        "{before:#?}"
    );
    let tracks = ["10001", "10002", "10003", "10004"];
    let ratings_before: Vec<Value> = tracks.iter().map(|t| rating_of(&s, t)).collect();

    let applied = ok(&s, &["project"]);
    let path = applied["large_run"]["backup"]["path"]
        .as_str()
        .unwrap()
        .to_owned();
    stop_service(&s);
    assert_ne!(sidecar_state(&s), before, "the Run changed the Sidecar");

    let restored = s.cli(&["--json", "backup", "restore", &path, "--yes"]);
    assert!(
        restored.status.success(),
        "{}{}",
        restored.stdout,
        restored.stderr
    );
    stop_service(&s);
    assert_eq!(sidecar_state(&s), before);
    let ratings_after: Vec<Value> = tracks.iter().map(|t| rating_of(&s, t)).collect();
    assert_eq!(ratings_after, ratings_before);
}

fn run_started(s: &Sandbox) -> String {
    let started = ok(
        s,
        &[
            "runs",
            "start",
            "--source",
            "agent:tagger",
            "--description",
            "Energy by halves",
        ],
    );
    started["run"]["id"]
        .as_i64()
        .or_else(|| started["id"].as_i64())
        .unwrap_or_else(|| panic!("{started}"))
        .to_string()
}

/// Review finding 3: a Run is counted as a whole. Commands joining one Run
/// are counted with what the Run already changed; the backup is taken
/// before the command that takes the Run over the threshold, once.
#[test]
fn a_joined_run_is_counted_whole_and_backed_up_before_the_command_that_crosses_the_threshold() {
    let s = Sandbox::new();
    policy(&s, "[energy]\napply = 0.5\n\n[backup]\nabove_tracks = 2\n");
    ok(&s, &["status"]);
    for (track, value) in [
        ("10001", "4"),
        ("10002", "2"),
        ("10003", "5"),
        ("10004", "1"),
    ] {
        energy(&s, track, value);
    }
    let run = run_started(&s);
    let project = |track: &str| ok(&s, &["project", "--track", track, "--run", &run]);
    let first = project("10001");
    assert_eq!(first["large_run"]["run_tracks"], 1, "{first}");
    assert!(first["large_run"]["backup"].is_null());
    let second = project("10002");
    assert_eq!(second["large_run"]["run_tracks"], 2);
    assert!(second["large_run"]["backup"].is_null());
    let third = project("10003");
    assert_eq!(third["large_run"]["tracks"], 1);
    assert_eq!(third["large_run"]["run_tracks"], 3);
    let path = third["large_run"]["backup"]["path"]
        .as_str()
        .expect("the crossing command backs up")
        .to_owned();
    let fourth = project("10004");
    assert_eq!(fourth["large_run"]["run_tracks"], 4);
    assert!(
        fourth["large_run"]["backup"].is_null(),
        "once per Run: {fourth}"
    );
    assert_eq!(fourth["large_run"]["run_backup"], json!(path));
    assert_eq!(backups_in(&s.state_dir()).len(), 1);
    // The Ledger says which of the Run's writes the backup precedes.
    let shown = ok(&s, &["runs", "show", &run]);
    let preceded: Vec<Value> = shown["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["command"]["args"]["large_run"]["backup"]["path"].clone())
        .collect();
    assert_eq!(
        preceded,
        vec![json!(null), json!(null), json!(path), json!(null)],
        "{shown}"
    );
}

/// Review finding 5: a backup folder inside what the backup reads (the
/// analysis or artwork tree, the Sidecar) is refused before anything is
/// written, as the window's backup refuses it.
#[test]
fn a_backup_folder_inside_what_the_backup_reads_is_refused_with_nothing_changed() {
    for inside in ["analysis", "artwork", "sidecar"] {
        let s = Sandbox::new();
        let folder = match inside {
            "analysis" => rbl_backup::analysis_dir(s.location()).join("backups"),
            "artwork" => rbl_backup::artwork_dir(s.location()).join("backups"),
            _ => s.sidecar_dir().join("backups"),
        };
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::create_dir_all(s.state_dir()).unwrap();
        std::fs::write(
            s.state_dir().join(rbl_backup::DESTINATION_FILE),
            serde_json::to_vec(&folder).unwrap(),
        )
        .unwrap();
        three_to_project(&s, "[energy]\napply = 0.5\n\n[backup]\nabove_tracks = 2\n");
        let unchanged = ratings(&s);
        let error = refused(&s, &["project"]);
        assert_eq!(error["code"], "run_backup_failed", "{inside}: {error}");
        assert_eq!(ratings(&s), unchanged, "{inside}");
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0, "{inside}");
    }
}

/// Review (e): Review's accept of chosen rows, each by its own token (the
/// window's way), goes through the same backup.
#[test]
fn a_large_review_accept_of_chosen_rows_is_backed_up_first() {
    let s = Sandbox::new();
    three_to_project(
        &s,
        "[energy]\napply = \"wait\"\n\n[backup]\nabove_tracks = 2\n",
    );
    let mut client = s.client();
    let waiting: Value = client.call("review.waiting", json!({})).unwrap();
    let rows: Vec<Value> = waiting["row_tokens"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(fact, token)| json!({"fact": fact.parse::<i64>().unwrap(), "token": token}))
        .collect();
    assert_eq!(rows.len(), 3);
    client.in_run(Some(
        serde_json::from_value(json!({"source": {"kind": "dj"}})).unwrap(),
    ));
    let accepted: Value = client
        .call(
            "review.accept",
            json!({"rows": rows, "source": {"kind": "dj"}}),
        )
        .unwrap();
    assert_eq!(accepted["count"], 3, "{accepted}");
    assert_eq!(accepted["large_run"]["tracks"], 3);
    let path = accepted["large_run"]["backup"]["path"].as_str().unwrap();
    assert_eq!(backups_in(&s.state_dir()), vec![PathBuf::from(path)]);
}

/// Review finding 7: how long each lock is held for a 20,000-track fixture
/// Library with three analysis files a track (60,000 files, 1 KiB each:
/// staging hard-links them, so its time is by count, not size). Run by
/// hand: `--ignored large_run_backup::twenty_thousand`.
#[test]
#[ignore = "a measurement, about a minute; run by hand"]
fn twenty_thousand_tracks_lock_times() {
    let s = Sandbox::with_shape(rbl_db::fixture::Shape {
        tracks: 20_000,
        ..rbl_db::fixture::Shape::default()
    });
    let tree = rbl_backup::analysis_dir(s.location());
    for i in 0..20_000 {
        let folder = tree
            .join(format!("P{:03X}", i / 256))
            .join(format!("{i:08X}"));
        std::fs::create_dir_all(&folder).unwrap();
        for ext in ["DAT", "EXT", "2EX"] {
            std::fs::write(folder.join(format!("ANLZ0000.{ext}")), vec![7_u8; 1024]).unwrap();
        }
    }
    policy(
        &s,
        "[energy]\napply = 0.5\n\n[backup]\nabove_tracks = 200\n",
    );
    ok(&s, &["status"]);
    let lines: String = (0..20_000)
        .map(|i| {
            json!({"track": rbl_db::fixture::track_id(i), "kind": "energy", "value": 1 + i % 5, "confidence": 0.9})
                .to_string()
                + "\n"
        })
        .collect();
    let file = s.path().join("facts.jsonl");
    std::fs::write(&file, lines).unwrap();
    ok(
        &s,
        &[
            "facts",
            "record-many",
            "--file",
            file.to_str().unwrap(),
            "--source",
            "agent:bulk",
        ],
    );
    let started = std::time::Instant::now();
    let applied = ok(&s, &["project"]);
    let took = started.elapsed();
    let backup = &applied["large_run"]["backup"];
    assert!(backup["path"].is_string(), "{applied}");
    eprintln!(
        "20,000 tracks, 60,000 analysis files: apply {took:?}; backup {} ms (edit gate and analysis lock held {} ms; archive written with only the Stick, if any, held {} ms), {} bytes",
        backup["took_ms"], backup["gate_ms"], backup["publish_ms"], backup["bytes"]
    );
}

// Review round 2.

/// The backup's test holds in the service's state folder
/// (`large_run::HOLDS`): armed, reached, released once or for good.
fn holds(s: &Sandbox) -> PathBuf {
    s.state_dir().join(wreckordbox_service::large_run::HOLDS)
}

fn arm(s: &Sandbox, point: &str) {
    std::fs::create_dir_all(holds(s)).unwrap();
    std::fs::write(holds(s).join(point), b"").unwrap();
}

fn reached(s: &Sandbox, point: &str) {
    let marker = holds(s).join(format!("{point}.reached"));
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while !marker.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the backup never reached {point}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Lets the backup held at `point` go on, keeping the hold for the next.
fn release_once(s: &Sandbox, point: &str) {
    std::fs::remove_file(holds(s).join(format!("{point}.reached"))).unwrap();
}

fn release(s: &Sandbox, point: &str) {
    let _ = std::fs::remove_file(holds(s).join(point));
}

fn spawned(s: &Sandbox, args: &[&str]) -> std::process::Child {
    let mut words = vec!["--json"];
    words.extend_from_slice(args);
    s.command()
        .args(&words)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap()
}

/// Its JSON answer (or error) once it ends; a minute at most.
fn answered(mut child: std::process::Child) -> (bool, Value) {
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            panic!("the command never finished");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let output = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(if output.status.success() {
        &output.stdout
    } else {
        &output.stderr
    })
    .into_owned();
    let value = serde_json::from_str(text.trim()).unwrap_or(json!({ "text": text }));
    (output.status.success(), value)
}

/// A stand-in for a separation's worker (no real separation is run): it
/// begins its Run in the Sidecar, past the service, as the worker does
/// (`begin_run`, which takes no Library lock). Its identifier.
fn a_separation_begins_its_run(s: &Sandbox) -> i64 {
    let db = rusqlite::Connection::open(s.sidecar_dir().join("sidecar.db")).unwrap();
    db.busy_timeout(Duration::from_secs(10)).unwrap();
    db.execute(
        "INSERT INTO run (source_kind, source_name, description, started_at)
         VALUES ('tool', 'separation-engine', 'Separate (a stand-in)', 0)",
        [],
    )
    .unwrap();
    db.last_insert_rowid()
}

/// The same worker publishing its four files into the Run's folder, as
/// `record_stems` moves them, before it records their Fact.
fn and_publishes_its_files(s: &Sandbox, track: &str, run: i64) -> PathBuf {
    let folder = s
        .sidecar_dir()
        .join("stems")
        .join(track)
        .join(format!("run-{run}"));
    std::fs::create_dir_all(&folder).unwrap();
    for role in ["vocals", "other", "bass", "drums"] {
        std::fs::write(folder.join(format!("{role}.wav")), b"RIFF stand-in").unwrap();
    }
    folder
}

fn entries_of(archive: &str) -> Vec<String> {
    rbl_backup::archive::open(Path::new(archive))
        .unwrap()
        .file_names()
        .map(str::to_owned)
        .collect()
}

/// Each Run in the Sidecar, as its identifier and description (identifiers
/// alone can repeat after a restore).
fn runs_in_sidecar(s: &Sandbox) -> Vec<(i64, String)> {
    let db = rusqlite::Connection::open_with_flags(
        s.sidecar_dir().join("sidecar.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let mut statement = db
        .prepare("SELECT id, description FROM run ORDER BY id")
        .unwrap();
    let rows = statement
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    rows.map(Result::unwrap).collect()
}

/// Every Stem file the Sidecar's Facts name, and every file under its
/// `stems/` folder.
fn stems_named_and_on_disk(s: &Sandbox) -> (Vec<String>, Vec<String>) {
    let db = rusqlite::Connection::open_with_flags(
        s.sidecar_dir().join("sidecar.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let mut statement = db
        .prepare(
            "SELECT json_extract(stem.value, '$.file') FROM fact, json_each(fact.value, '$.stems') AS stem
             WHERE fact.kind = 'stems' ORDER BY 1",
        )
        .unwrap();
    let named: Vec<String> = statement
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut on_disk = Vec::new();
    let mut pending = vec![s.sidecar_dir().join("stems")];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            if entry.file_type().unwrap().is_dir() {
                pending.push(entry.path());
            } else {
                on_disk.push(
                    entry
                        .path()
                        .strip_prefix(s.sidecar_dir())
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    on_disk.sort();
    (named, on_disk)
}

/// A track with Stems already (the engine's fake worker), Stem audio asked
/// for in backups, and three tracks to project above a threshold of two.
fn separated_and_three_to_project(s: &Sandbox) -> String {
    let track = s.import_track("Night Bus.wav", 1);
    ok(s, &["stems", "separate", &track]);
    std::fs::create_dir_all(s.state_dir()).unwrap();
    std::fs::write(
        s.state_dir().join(rbl_backup::sidecar_part::OPTIONS_FILE),
        br#"{"stems": true}"#,
    )
    .unwrap();
    three_to_project(s, "[energy]\napply = 0.5\n\n[backup]\nabove_tracks = 2\n");
    track
}

/// Review round 2, blocker 1: a separation begins its Run and publishes
/// its files after the backup has snapshotted the Sidecar, and has not yet
/// recorded its Fact. The backup holds only the Stem files its own
/// snapshot names, so none of that Run's, and its restore is a state that
/// existed: every Stem file in it is one a Fact names.
#[test]
fn stem_files_published_after_the_sidecar_snapshot_are_not_in_the_backup() {
    let s = Sandbox::new();
    let track = separated_and_three_to_project(&s);
    let (named, on_disk) = stems_named_and_on_disk(&s);
    assert_eq!(named.len(), 4, "{named:?}");
    assert_eq!(on_disk, named);

    arm(&s, "sidecar-snapshotted");
    let apply = spawned(&s, &["project"]);
    reached(&s, "sidecar-snapshotted");
    let run = a_separation_begins_its_run(&s);
    let published = and_publishes_its_files(&s, &track, run);
    release(&s, "sidecar-snapshotted");
    let (applied, answer) = answered(apply);
    assert!(applied, "{answer}");
    let archive = answer["large_run"]["backup"]["path"]
        .as_str()
        .expect("a backup")
        .to_owned();

    let entries = entries_of(&archive);
    let late = format!("/run-{run}/");
    assert!(
        !entries.iter().any(|name| name.contains(&late)),
        "files of a Run the snapshot does not name: {entries:#?}"
    );
    for file in &named {
        assert!(
            entries.contains(&format!("sidecar/{file}")),
            "{file} is named by the snapshot: {entries:#?}"
        );
    }

    stop_service(&s);
    let restored = s.cli(&["--json", "backup", "restore", &archive, "--yes"]);
    assert!(
        restored.status.success(),
        "{}{}",
        restored.stdout,
        restored.stderr
    );
    stop_service(&s);
    let (named_after, on_disk_after) = stems_named_and_on_disk(&s);
    assert_eq!(named_after, named);
    assert_eq!(
        on_disk_after, named,
        "every Stem file a Fact names, no other"
    );
    assert!(!published.exists());
}

/// Review round 2, blocker 1 (b): a separation begins its Run after the
/// backup and before the Run's first write. Run creation moves the mark
/// the write checks, so the backup is taken again and the one the Run
/// names holds that Run.
#[test]
fn a_run_begun_after_the_backup_and_before_the_write_is_in_the_backup_the_run_names() {
    let s = Sandbox::new();
    three_to_project(&s, "[energy]\napply = 0.5\n\n[backup]\nabove_tracks = 2\n");
    arm(&s, "backed-up");
    let apply = spawned(&s, &["project"]);
    reached(&s, "backed-up");
    let run = a_separation_begins_its_run(&s);
    release(&s, "backed-up");
    let (applied, answer) = answered(apply);
    assert!(applied, "{answer}");
    let archive = answer["large_run"]["backup"]["path"]
        .as_str()
        .expect("a backup")
        .to_owned();
    assert_eq!(
        backups_in(&s.state_dir()),
        vec![PathBuf::from(&archive)],
        "the first backup, of an earlier moment, is removed"
    );
    stop_service(&s);
    let restored = s.cli(&["--json", "backup", "restore", &archive, "--yes"]);
    assert!(restored.status.success(), "{}", restored.stderr);
    stop_service(&s);
    assert!(
        runs_in_sidecar(&s).contains(&(run, "Separate (a stand-in)".to_owned())),
        "the Run begun before the write is in the backup"
    );
}

/// Review round 2, item 4: a backup invalidated every time it is taken is
/// taken exactly three times, each one checked, and then the Run is
/// refused with nothing changed and none of the three kept.
#[test]
fn a_backup_invalidated_each_time_is_taken_three_times_then_the_run_is_refused() {
    let s = Sandbox::new();
    three_to_project(&s, "[energy]\napply = 0.5\n\n[backup]\nabove_tracks = 2\n");
    let unchanged = ratings(&s);
    arm(&s, "backed-up");
    let mut apply = spawned(&s, &["project"]);
    for _ in 0..3 {
        reached(&s, "backed-up");
        a_separation_begins_its_run(&s);
        release_once(&s, "backed-up");
    }
    // No fourth: the apply ends, refused, and nothing reaches the hold.
    let marker = holds(&s).join("backed-up.reached");
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while apply.try_wait().unwrap().is_none() {
        assert!(
            !marker.exists(),
            "a fourth backup was taken after three were checked"
        );
        assert!(
            std::time::Instant::now() < deadline,
            "the apply never ended"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    release(&s, "backed-up");
    let (applied, error) = answered(apply);
    assert!(!applied, "{error}");
    assert_eq!(error["error"]["code"], "run_backup_failed", "{error}");
    assert_eq!(ratings(&s), unchanged);
    assert_eq!(backups_in(&s.state_dir()), Vec::<PathBuf>::new());
}

/// Review round 2, blocker 2: a joined Run crossed by a command other than
/// a projection (here `tracks rate`) is backed up before that command's
/// write, in either order; a Run that stays at the threshold is not.
#[test]
fn a_joined_run_crossed_by_a_rating_is_backed_up_first_in_either_order() {
    for rating_last in [true, false] {
        let s = Sandbox::new();
        policy(&s, "[energy]\napply = 0.5\n\n[backup]\nabove_tracks = 2\n");
        ok(&s, &["status"]);
        energy(&s, "10001", "4");
        energy(&s, "10002", "2");
        let run = run_started(&s);
        let project = |track: &str| ok(&s, &["project", "--track", track, "--run", &run]);
        let rate = || ok(&s, &["tracks", "rate", "10003", "3", "--run", &run]);
        let crossing = if rating_last {
            assert!(project("10001")["large_run"]["backup"].is_null());
            assert!(project("10002")["large_run"]["backup"].is_null());
            assert_eq!(backups_in(&s.state_dir()), Vec::<PathBuf>::new());
            let rated = rate();
            assert_eq!(rated["rating"], 3, "{rated}");
            rated
        } else {
            rate();
            assert!(project("10001")["large_run"]["backup"].is_null());
            assert_eq!(backups_in(&s.state_dir()), Vec::<PathBuf>::new());
            project("10002")
        };
        assert_eq!(
            backups_in(&s.state_dir()).len(),
            1,
            "rating last: {rating_last}: {crossing}"
        );
        let shown = ok(&s, &["runs", "show", &run]);
        let entries = shown["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 3, "{shown}");
        let preceded: Vec<bool> = entries
            .iter()
            .map(|entry| entry["command"]["args"]["large_run"]["backup"]["path"].is_string())
            .collect();
        assert_eq!(preceded, vec![false, false, true], "{shown}");
        assert_eq!(
            entries[2]["command"]["args"]["large_run"]["run_tracks"], 3,
            "{shown}"
        );
    }
}

/// Review round 2, item 3: staging hard-links the analysis and artwork
/// files and Stem audio, never copies them under the edit gate. When a link
/// cannot be made (forced here), the backup is refused, saying why, with
/// nothing changed and nothing left.
#[test]
fn a_hard_link_that_cannot_be_made_refuses_the_backup_with_nothing_changed() {
    let s = Sandbox::new();
    let tree = rbl_backup::analysis_dir(s.location())
        .join("P001")
        .join("0000AAAA");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("ANLZ0000.DAT"), vec![7_u8; 1024]).unwrap();
    three_to_project(&s, "[energy]\napply = 0.5\n\n[backup]\nabove_tracks = 2\n");
    stop_service(&s);
    let unchanged = ratings(&s);
    stop_service(&s);
    let mut service = serve_with(
        &s,
        &[(wreckordbox_service::large_run::FAULT_ENV, "link_fails")],
    );
    let error = refused(&s, &["project"]);
    assert_eq!(error["code"], "run_backup_failed", "{error}");
    let message = error["message"].as_str().unwrap();
    assert!(message.contains("hard link"), "{message}");
    assert!(message.contains("nothing was changed"), "{message}");
    assert_eq!(ratings(&s), unchanged);
    assert_eq!(backups_in(&s.state_dir()), Vec::<PathBuf>::new());
    let library_root = s.location().master_db.parent().unwrap().to_path_buf();
    let staging_left: Vec<_> = [s.state_dir(), library_root]
        .iter()
        .flat_map(|dir| std::fs::read_dir(dir).unwrap().flatten())
        .filter(|entry| entry.file_name().to_string_lossy().contains("run-backup"))
        .map(|entry| entry.path())
        .collect();
    assert!(staging_left.is_empty(), "{staging_left:?}");
    let _ = service.kill();
    let _ = service.wait();
}
