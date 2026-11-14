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
