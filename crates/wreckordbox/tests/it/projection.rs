//! Projecting Energy and Prep colour (wbx-26) through the real CLI and a
//! service on a fixture Library, with a second client watching.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
use crate::common;

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

fn fact(
    s: &Sandbox,
    track: &str,
    kind: &str,
    value: &str,
    source: &str,
    confidence: &str,
) -> Value {
    ok(
        s,
        &[
            "facts",
            "record",
            track,
            kind,
            value,
            "--source",
            source,
            "--confidence",
            confidence,
        ],
    )["facts"][0]
        .clone()
}

fn track(s: &Sandbox, id: &str) -> Value {
    ok(s, &["tracks", "show", id])
}

/// The changes of a report as `(track, field, before, after)`, sorted.
fn changes(report: &Value, key: &str) -> Vec<(String, String, Value, Value)> {
    let mut list: Vec<_> = report[key]
        .as_array()
        .unwrap_or_else(|| panic!("no {key} in {report}"))
        .iter()
        .map(|change| {
            (
                change["track"].as_str().unwrap().to_owned(),
                change["field"].as_str().unwrap().to_owned(),
                change["before"].clone(),
                change["after"].clone(),
            )
        })
        .collect();
    list.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    list
}

fn drain(watcher: &mut wreckordbox_service::Client) {
    while watcher
        .next_event(Duration::from_millis(300))
        .unwrap()
        .is_some()
    {}
}

#[test]
fn a_dry_run_lists_exactly_what_applying_then_does_as_one_run_and_twice_changes_nothing() {
    let s = Sandbox::new();
    s.dj_prep_colours();
    policy(&s, "[energy]\napply = 0.8\n\n[prep_colour]\napply = 0.5\n");
    ok(&s, &["status"]);
    let energy = fact(&s, "10001", "energy", "4", "agent:tagger", "0.9");
    fact(&s, "10002", "energy", "2", "tool:meter", "0.8");
    fact(&s, "10003", "prep_colour", "Green", "agent:tagger", "0.6");
    let mut watcher = s.client();
    watcher.subscribe().unwrap();
    let runs_before = ok(&s, &["runs", "list"]);

    let dry = ok(&s, &["project", "--dry-run"]);
    assert_eq!(dry["dry_run"], true);
    assert_eq!(
        changes(&dry, "changes"),
        vec![
            ("10001".into(), "energy".into(), json!(null), json!(4)),
            ("10002".into(), "energy".into(), json!(null), json!(2)),
            (
                "10003".into(),
                "prep_colour".into(),
                json!(null),
                json!({"id": 5, "colour": "Green", "label": "Clean"})
            ),
        ]
    );
    let first = dry["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|change| change["track"] == "10001")
        .unwrap();
    assert_eq!(first["fact"], energy["id"]);
    assert_eq!(first["source"], json!({"kind": "agent", "name": "tagger"}));
    // A dry run writes nothing: no Library change, no Run, no event.
    assert_eq!(track(&s, "10001")["rating"], 0);
    assert_eq!(ok(&s, &["runs", "list"]), runs_before);
    assert!(watcher
        .next_event(Duration::from_millis(300))
        .unwrap()
        .is_none());

    let applied = ok(&s, &["project"]);
    assert_eq!(applied["dry_run"], false);
    assert_eq!(changes(&applied, "changes"), changes(&dry, "changes"));
    assert_eq!(applied["plan"], dry["plan"]);
    assert_eq!(track(&s, "10001")["rating"], 4);
    assert_eq!(track(&s, "10002")["rating"], 2);
    assert_eq!(track(&s, "10003")["colour_id"], 5);
    // One edit, so the window hears of it once and live.
    let event = watcher.next_event(Duration::from_secs(2)).unwrap().unwrap();
    assert_eq!(event.event, "library_changed");
    // One Run, by a tool and never the DJ, holding exactly those changes.
    let run = &applied["run"];
    assert_eq!(run["source"]["kind"], "tool");
    assert_eq!(run["source"]["name"], "projection");
    // Which policy was in force is in the plan and in the Run.
    let hash = dry["policy_hash"].as_str().unwrap();
    assert_eq!(hash.len(), 16, "{dry}");
    assert_eq!(applied["policy_hash"], hash);
    assert!(run["description"].as_str().unwrap().contains(hash), "{run}");
    let shown = ok(&s, &["runs", "show", &run["id"].to_string()]);
    let library: Vec<&Value> = shown["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["store"] == "library")
        .collect();
    assert_eq!(library.len(), 1, "{shown}");
    assert_eq!(library[0]["reversible"], true);
    let mut values: Vec<(String, String)> = library[0]["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| (value["subject"].to_string(), value["after"].to_string()))
        .collect();
    values.sort();
    assert_eq!(values.len(), 3, "{shown}");
    // No DJ Fact was made by the Projection's edit.
    let standing = ok(&s, &["facts", "standing", "10001", "energy"]);
    assert_eq!(standing["standing"]["source"]["kind"], "agent");

    drain(&mut watcher);
    let runs_after = ok(&s, &["runs", "list"]);
    let again = ok(&s, &["project"]);
    assert_eq!(again["changes"], json!([]));
    assert!(
        again.get("run").is_none() || again["run"].is_null(),
        "{again}"
    );
    assert_eq!(ok(&s, &["runs", "list"]), runs_after);
    assert!(watcher
        .next_event(Duration::from_millis(300))
        .unwrap()
        .is_none());
}

fn reasons(report: &Value, key: &str) -> Vec<(String, String, String)> {
    let mut list: Vec<_> = report[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["track"].as_str().unwrap().to_owned(),
                item["field"].as_str().unwrap().to_owned(),
                item["reason"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    list.sort();
    list
}

#[test]
fn a_kind_set_to_wait_is_held_and_a_threshold_applies_only_facts_at_or_above_it() {
    let s = Sandbox::new();
    ok(&s, &["status"]);
    fact(&s, "10001", "energy", "3", "agent:tagger", "0.7");
    fact(&s, "10002", "energy", "5", "agent:tagger", "0.69");
    fact(&s, "10003", "prep_colour", "Green", "agent:tagger", "1");

    // No policy file: everything waits for the DJ.
    let strict = ok(&s, &["project"]);
    assert_eq!(strict["changes"], json!([]));
    assert_eq!(
        reasons(&strict, "held"),
        vec![
            ("10001".into(), "energy".into(), "waits_for_dj".into()),
            ("10002".into(), "energy".into(), "waits_for_dj".into()),
            ("10003".into(), "prep_colour".into(), "waits_for_dj".into()),
        ]
    );
    assert_eq!(track(&s, "10001")["rating"], 0);
    assert_eq!(track(&s, "10003")["colour_id"], 0);

    policy(
        &s,
        "[energy]\napply = 0.7\n\n[prep_colour]\napply = \"wait\"\n",
    );
    let applied = ok(&s, &["project"]);
    assert_eq!(
        changes(&applied, "changes"),
        vec![("10001".into(), "energy".into(), json!(null), json!(3))]
    );
    assert_eq!(
        reasons(&applied, "held"),
        vec![
            ("10002".into(), "energy".into(), "below_confidence".into()),
            ("10003".into(), "prep_colour".into(), "waits_for_dj".into()),
        ]
    );
    let below = applied["held"]
        .as_array()
        .unwrap()
        .iter()
        .find(|held| held["track"] == "10002")
        .unwrap();
    assert_eq!(below["threshold"], 0.7);
    assert_eq!(below["after"], 5);
    assert_eq!(track(&s, "10001")["rating"], 3);
    assert_eq!(track(&s, "10002")["rating"], 0);
    assert_eq!(track(&s, "10003")["colour_id"], 0);
}

#[test]
fn a_value_the_dj_set_by_hand_is_never_overwritten_and_a_disagreeing_fact_is_reported() {
    let s = Sandbox::new();
    s.dj_prep_colours();
    // Set before wreckordbox recorded anything (a migrated Library): no
    // Ledger entry and no DJ Fact; the value is the DJ's.
    s.edit(|writer| {
        writer.set_rating("10001", 2).unwrap();
        writer.set_color("10004", Some("2")).unwrap();
    });
    policy(&s, "[energy]\napply = 0\n\n[prep_colour]\napply = 0\n");
    ok(&s, &["status"]);
    // Set by hand through the CLI as the DJ: recorded as the DJ's Fact.
    ok(&s, &["tracks", "rate", "10002", "1", "--source", "dj"]);
    fact(&s, "10001", "energy", "5", "agent:tagger", "1");
    fact(&s, "10002", "energy", "4", "agent:tagger", "1");
    fact(&s, "10004", "prep_colour", "Green", "agent:tagger", "1");
    // The DJ's own Fact, for an unrated track: projected, by a tool.
    let djs = fact(&s, "10005", "energy", "3", "dj", "1");

    let dry = ok(&s, &["project", "--dry-run"]);
    assert_eq!(
        changes(&dry, "changes"),
        vec![("10005".into(), "energy".into(), json!(null), json!(3))],
        "{dry}"
    );
    assert_eq!(
        reasons(&dry, "kept"),
        vec![
            ("10001".into(), "energy".into(), "no_record".into()),
            ("10002".into(), "energy".into(), "dj_fact".into()),
            ("10004".into(), "prep_colour".into(), "no_record".into()),
        ]
    );
    let kept = dry["kept"]
        .as_array()
        .unwrap()
        .iter()
        .find(|kept| kept["track"] == "10004")
        .unwrap();
    assert_eq!(kept["value"]["label"], "Problem");
    assert_eq!(kept["fact_value"]["label"], "Clean");
    let applied = ok(&s, &["project"]);
    assert_eq!(changes(&applied, "changes"), changes(&dry, "changes"));
    assert_eq!(applied["kept"], dry["kept"]);
    assert_eq!(applied["run"]["source"]["kind"], "tool");
    assert_eq!(track(&s, "10001")["rating"], 2);
    assert_eq!(track(&s, "10002")["rating"], 1);
    assert_eq!(track(&s, "10004")["colour_id"], 2);
    assert_eq!(track(&s, "10005")["rating"], 3);
    // Writing the DJ's Fact made no new DJ Fact: the same one stands.
    let standing = ok(&s, &["facts", "standing", "10005", "energy"]);
    assert_eq!(standing["standing"]["id"], djs["id"]);
}

#[test]
fn the_djs_explicit_set_to_the_same_value_is_recorded_and_protects_it() {
    let s = Sandbox::new();
    policy(&s, "[energy]\napply = 0.5\n");
    ok(&s, &["status"]);
    fact(&s, "10001", "energy", "3", "agent:tagger", "0.6");
    ok(&s, &["project"]);
    assert_eq!(track(&s, "10001")["rating"], 3);
    let runs = ok(&s, &["runs", "list"])["total"].as_u64().unwrap();
    // The DJ says 3, which is what the Library already holds.
    let rated = ok(&s, &["tracks", "rate", "10001", "3", "--source", "dj"]);
    assert_eq!(rated["run"]["source"]["kind"], "dj", "{rated}");
    assert_eq!(
        ok(&s, &["runs", "list"])["total"].as_u64().unwrap(),
        runs + 1
    );
    let standing = ok(&s, &["facts", "standing", "10001", "energy"]);
    assert_eq!(standing["standing"]["source"]["kind"], "dj");
    assert_eq!(standing["standing"]["value"], 3);
    // A colour set by the DJ asserts nothing about Energy.
    ok(
        &s,
        &[
            "intake",
            "colour",
            "10002",
            "--colour-id",
            "5",
            "--source",
            "dj",
        ],
    );
    let other = ok(&s, &["facts", "standing", "10002", "energy"]);
    assert!(other["standing"].is_null(), "{other}");
    // A better Fact arrives: the DJ's 3 stays.
    fact(&s, "10001", "energy", "4", "agent:other", "0.9");
    let applied = ok(&s, &["project"]);
    assert_eq!(applied["changes"], json!([]));
    assert_eq!(
        reasons(&applied, "kept"),
        vec![("10001".into(), "energy".into(), "dj_fact".into())]
    );
    assert_eq!(track(&s, "10001")["rating"], 3);
}

#[test]
fn a_value_a_projection_wrote_follows_a_better_fact_but_not_once_another_writes_it() {
    let s = Sandbox::new();
    policy(&s, "[energy]\napply = 0.5\n");
    ok(&s, &["status"]);
    fact(&s, "10001", "energy", "3", "agent:tagger", "0.6");
    fact(&s, "10002", "energy", "3", "agent:tagger", "0.6");
    ok(&s, &["project"]);
    assert_eq!(track(&s, "10001")["rating"], 3);
    // Someone else (here a tool, not a projection) changes one.
    ok(&s, &["tracks", "rate", "10002", "1"]);
    fact(&s, "10001", "energy", "4", "agent:other", "0.9");
    fact(&s, "10002", "energy", "4", "agent:other", "0.9");
    let applied = ok(&s, &["project"]);
    assert_eq!(
        changes(&applied, "changes"),
        vec![("10001".into(), "energy".into(), json!(3), json!(4))]
    );
    assert_eq!(
        reasons(&applied, "kept"),
        vec![("10002".into(), "energy".into(), "set_by_other".into())]
    );
    assert_eq!(track(&s, "10001")["rating"], 4);
    assert_eq!(track(&s, "10002")["rating"], 1);
}

#[test]
fn prep_colour_facts_name_the_colour_not_the_djs_label() {
    let s = Sandbox::new();
    // The DJ calls colour 2 "Green" and colour 5 "Clean".
    s.name_colours(&[(2, "Green"), (5, "Clean")]);
    policy(&s, "[prep_colour]\napply = 0.5\n");
    ok(&s, &["status"]);
    fact(&s, "10001", "prep_colour", "Green", "agent:tagger", "0.9");
    fact(&s, "10002", "prep_colour", "Red", "agent:tagger", "0.9");
    let applied = ok(&s, &["project"]);
    assert_eq!(
        changes(&applied, "changes"),
        vec![
            (
                "10001".into(),
                "prep_colour".into(),
                json!(null),
                json!({"id": 5, "colour": "Green", "label": "Clean"})
            ),
            (
                "10002".into(),
                "prep_colour".into(),
                json!(null),
                json!({"id": 2, "colour": "Red", "label": "Green"})
            ),
        ]
    );
    assert_eq!(track(&s, "10001")["colour_id"], 5);
    assert_eq!(track(&s, "10002")["colour_id"], 2);
}

#[test]
fn only_the_energy_judgement_is_projected_never_a_measurement() {
    let s = Sandbox::new();
    std::fs::create_dir_all(s.sidecar_dir()).unwrap();
    std::fs::write(
        s.sidecar_dir().join("kinds.toml"),
        "[kinds.energy_estimate]\nnature = \"measurement\"\nvalue = \"integer\"\nmin = 1\nmax = 5\n",
    )
    .unwrap();
    let current = s.import_track("Current.wav", 1);
    policy(&s, "[energy]\napply = 0.5\nfrom = \"energy_estimate\"\n");
    ok(&s, &["status"]);
    // Projecting another kind into Energy is not allowed (wbx-33 turns
    // Intensity into an Energy Judgement).
    assert_eq!(
        refused(&s, &["project", "--dry-run"])["code"],
        "projection_policy_invalid"
    );
    policy(&s, "[energy]\napply = 0.5\n");
    fact(&s, &current, "energy_estimate", "5", "tool:meter", "1");
    let applied = ok(&s, &["project"]);
    assert_eq!(applied["changes"], json!([]));
    assert_eq!(track(&s, &current)["rating"], 0);
}

fn files(dir: &std::path::Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut found = std::collections::BTreeMap::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                found.extend(files(&path));
            } else {
                found.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    found
}

#[test]
fn a_dry_run_leaves_every_sidecar_file_byte_for_byte_and_makes_none() {
    let absent = Sandbox::new();
    ok(&absent, &["status"]);
    let dry = ok(&absent, &["project", "--dry-run"]);
    assert_eq!(dry["changes"], json!([]));
    assert!(!absent.sidecar_dir().exists(), "a dry run made a Sidecar");

    let s = Sandbox::new();
    policy(&s, "[energy]\napply = 0.5\n");
    ok(&s, &["status"]);
    fact(&s, "10001", "energy", "4", "agent:tagger", "0.9");
    // Let the service close its Sidecar's write-ahead log state settle:
    // the comparison is of the files as they are between two reads.
    let before = files(&s.sidecar_dir());
    let dry = ok(&s, &["project", "--dry-run"]);
    assert_eq!(dry["changes"].as_array().unwrap().len(), 1);
    assert_eq!(files(&s.sidecar_dir()), before);
    assert_eq!(track(&s, "10001")["rating"], 0);
}

#[test]
fn protection_a_changed_plan_and_the_djs_name_are_refused_with_nothing_written() {
    let s = Sandbox::new();
    policy(&s, "[energy]\napply = 0.5\n");
    ok(&s, &["status"]);
    fact(&s, "10001", "energy", "4", "agent:tagger", "0.9");
    let runs = ok(&s, &["runs", "list"]);

    s.protect(true);
    let dry = ok(&s, &["project", "--dry-run"]);
    assert_eq!(dry["library_protected"], true);
    assert_eq!(dry["changes"].as_array().unwrap().len(), 1);
    assert_eq!(refused(&s, &["project"])["code"], "library_protected");
    assert_eq!(track(&s, "10001")["rating"], 0);
    assert_eq!(ok(&s, &["runs", "list"]), runs);
    s.protect(false);

    assert_eq!(
        refused(&s, &["project", "--source", "dj"])["code"],
        "projection_not_the_djs"
    );
    // Nor can it join the DJ's Run, whatever source it names.
    let djs = ok(
        &s,
        &[
            "runs",
            "start",
            "--source",
            "dj",
            "--description",
            "By hand",
        ],
    );
    let djs = djs["id"].to_string();
    for args in [
        vec!["project", "--run", djs.as_str()],
        vec!["project", "--run", djs.as_str(), "--source", "agent:x"],
    ] {
        assert_eq!(
            refused(&s, &args)["code"],
            "projection_not_the_djs",
            "{args:?}"
        );
    }
    assert_eq!(track(&s, "10001")["rating"], 0);
    // A plan checked before a new Fact arrived is not the plan now.
    fact(&s, "10002", "energy", "2", "agent:tagger", "0.9");
    let runs = ok(&s, &["runs", "list"]);
    assert_eq!(
        refused(&s, &["project", "--expect", dry["plan"].as_str().unwrap()])["code"],
        "projection_changed"
    );
    assert_eq!(track(&s, "10001")["rating"], 0);
    assert_eq!(ok(&s, &["runs", "list"]), runs);

    // The plan checked now is the plan applied.
    let checked = ok(&s, &["project", "--dry-run"]);
    let applied = ok(
        &s,
        &[
            "project",
            "--expect",
            checked["plan"].as_str().unwrap(),
            "--source",
            "agent:energy-pass",
        ],
    );
    assert_eq!(changes(&applied, "changes"), changes(&checked, "changes"));
    assert_eq!(applied["run"]["source"]["name"], "energy-pass");
    assert_eq!(track(&s, "10002")["rating"], 2);

    std::fs::write(
        s.sidecar_dir().join("projection-policy.toml"),
        "[energy]\napply = 2\n",
    )
    .unwrap();
    assert_eq!(
        refused(&s, &["project", "--dry-run"])["code"],
        "projection_policy_invalid"
    );
}

/// The cost asked for: a Library of 2,000 tracks, each with an Energy and a
/// Prep colour Fact, projected (dry run, apply, and again).
#[test]
fn projecting_two_thousand_tracks_stays_cheap() {
    let s = Sandbox::with_shape(rbl_db::fixture::Shape {
        tracks: 2000,
        ..rbl_db::fixture::Shape::default()
    });
    policy(&s, "[energy]\napply = 0.5\n\n[prep_colour]\napply = 0.5\n");
    ok(&s, &["status"]);
    let lines: String = (0..2000)
        .flat_map(|i| {
            let id = rbl_db::fixture::track_id(i);
            [
                json!({"track": id, "kind": "energy", "value": 1 + i % 5, "confidence": 0.9}),
                json!({"track": id, "kind": "prep_colour", "value": "Green", "confidence": 0.9}),
            ]
        })
        .map(|line| line.to_string() + "\n")
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
    let time = |args: &[&str]| {
        let started = std::time::Instant::now();
        let report = ok(&s, args);
        (report, started.elapsed())
    };
    let (dry, dry_took) = time(&["project", "--dry-run"]);
    assert_eq!(dry["changes"].as_array().unwrap().len(), 4000);
    let (applied, apply_took) = time(&["project"]);
    assert_eq!(applied["plan"], dry["plan"]);
    let (again, again_took) = time(&["project"]);
    assert_eq!(again["changes"], json!([]));
    assert_eq!(again["unchanged"], 4000);
    eprintln!(
        "2,000 tracks, 4,000 Facts: dry run {dry_took:?}, apply {apply_took:?}, again {again_took:?}"
    );
    // Generous bounds for a shared debug build; a per-track scan or audio
    // read would be far over them.
    assert!(dry_took < Duration::from_secs(20), "{dry_took:?}");
    assert!(apply_took < Duration::from_secs(60), "{apply_took:?}");
}

/// Starts a service here with `faults` given to it, and waits until it is up.
fn serve_with_faults(s: &Sandbox, faults: &str) -> std::process::Child {
    let child = s
        .command()
        .env("WRECKORDBOX_LEDGER_FAULTS", faults)
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
}

/// The review's case: the DJ's assertion of a projected value is begun (its
/// intent saved) and the service dies before it is recorded; the next
/// service records it as unconfirmed. That track's value is the DJ's from
/// then on; another track's projected value is not affected.
#[test]
fn an_interrupted_assertion_makes_its_tracks_value_the_djs_and_no_other() {
    let s = Sandbox::new();
    policy(&s, "[energy]\napply = 0.5\n");
    ok(&s, &["status"]);
    fact(&s, "10001", "energy", "3", "agent:tagger", "0.6");
    fact(&s, "10002", "energy", "3", "agent:tagger", "0.6");
    ok(&s, &["project"]);
    stop_service(&s);

    let mut service = serve_with_faults(&s, "delay_after_begin_ms=4000");
    let rating = s
        .command()
        .args(["--json", "tracks", "rate", "10001", "3", "--source", "dj"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    service.kill().unwrap();
    service.wait().unwrap();
    let mut rating = rating;
    assert!(!rating.wait().unwrap().success(), "the rating was cut off");

    // The next service recovers the intent as an unconfirmed write.
    let runs = ok(&s, &["runs", "list"]);
    let mut unconfirmed = 0;
    for run in runs["runs"].as_array().unwrap() {
        let shown = ok(&s, &["runs", "show", &run["id"].to_string()]);
        unconfirmed += shown["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["kind"] == "unconfirmed")
            .count();
    }
    assert_eq!(unconfirmed, 1, "{runs}");

    fact(&s, "10001", "energy", "4", "agent:other", "0.9");
    fact(&s, "10002", "energy", "4", "agent:other", "0.9");
    let applied = ok(&s, &["project"]);
    assert_eq!(
        changes(&applied, "changes"),
        vec![("10002".into(), "energy".into(), json!(3), json!(4))],
        "{applied}"
    );
    assert_eq!(
        reasons(&applied, "kept"),
        vec![(
            "10001".into(),
            "energy".into(),
            "provenance_incomplete".into()
        )]
    );
    assert_eq!(track(&s, "10001")["rating"], 3);
}

/// The verification review's second case: the DJ asserts the value a
/// projection left (nothing changes, so the write tells the Ledger it wrote
/// nothing) and the service dies before the assertion is recorded. Its
/// intent must survive, as a barrier for that track only.
#[test]
fn an_assertion_cut_off_after_its_write_said_nothing_changed_still_protects_the_value() {
    let s = Sandbox::new();
    policy(&s, "[energy]\napply = 0.5\n");
    ok(&s, &["status"]);
    fact(&s, "10001", "energy", "3", "agent:tagger", "0.6");
    fact(&s, "10002", "energy", "3", "agent:tagger", "0.6");
    ok(&s, &["project"]);
    stop_service(&s);

    let mut service = serve_with_faults(&s, "abort_before_assert");
    let rated = s.cli(&["--json", "tracks", "rate", "10001", "3", "--source", "dj"]);
    assert!(!rated.status.success(), "the service should have died");
    service.wait().unwrap();

    fact(&s, "10001", "energy", "4", "agent:other", "0.9");
    fact(&s, "10002", "energy", "4", "agent:other", "0.9");
    let applied = ok(&s, &["project"]);
    assert_eq!(
        changes(&applied, "changes"),
        vec![("10002".into(), "energy".into(), json!(3), json!(4))],
        "{applied}"
    );
    assert_eq!(
        reasons(&applied, "kept"),
        vec![(
            "10001".into(),
            "energy".into(),
            "provenance_incomplete".into()
        )]
    );
    assert_eq!(track(&s, "10001")["rating"], 3);
}
