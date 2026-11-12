//! Projecting Energy and Prep colour (wbx-26) through the real CLI and a
//! service on a fixture Library, with a second client watching.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
mod common;

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
    std::fs::write(s.sidecar_dir().join("projection.toml"), text).unwrap();
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
    // Set by hand before wreckordbox knew of them: no DJ Fact at all
    // (the shape of the wbx-19 case, where a hand edit records none).
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
    // The DJ's own Fact, differing from the Library: a tool does not write it.
    fact(&s, "10005", "energy", "3", "dj", "1");

    let dry = ok(&s, &["project", "--dry-run"]);
    assert_eq!(dry["changes"], json!([]), "{dry}");
    assert_eq!(
        reasons(&dry, "kept"),
        vec![
            ("10001".into(), "energy".into(), "set_by_dj".into()),
            ("10002".into(), "energy".into(), "dj_fact".into()),
            ("10004".into(), "prep_colour".into(), "set_by_dj".into()),
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
    assert_eq!(
        reasons(&dry, "held"),
        vec![("10005".into(), "energy".into(), "dj_fact".into())]
    );
    let applied = ok(&s, &["project"]);
    assert_eq!(applied["changes"], json!([]));
    assert_eq!(applied["kept"], dry["kept"]);
    assert_eq!(track(&s, "10001")["rating"], 2);
    assert_eq!(track(&s, "10002")["rating"], 1);
    assert_eq!(track(&s, "10004")["colour_id"], 2);
    assert_eq!(track(&s, "10005")["rating"], 0);
}

#[test]
fn a_value_a_projection_wrote_follows_a_better_fact_but_not_once_the_dj_changes_it() {
    let s = Sandbox::new();
    policy(&s, "[energy]\napply = 0.5\n");
    ok(&s, &["status"]);
    fact(&s, "10001", "energy", "3", "agent:tagger", "0.6");
    fact(&s, "10002", "energy", "3", "agent:tagger", "0.6");
    ok(&s, &["project"]);
    assert_eq!(track(&s, "10001")["rating"], 3);
    // The DJ changes one by hand; a better Fact arrives for both.
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
        vec![("10002".into(), "energy".into(), "set_by_dj".into())]
    );
    assert_eq!(track(&s, "10001")["rating"], 4);
    assert_eq!(track(&s, "10002")["rating"], 1);
}

#[test]
fn stale_measurements_are_never_projected() {
    let s = Sandbox::new();
    let current = s.import_track("Current.wav", 1);
    let changed = s.import_track("Changed.wav", 2);
    std::fs::create_dir_all(s.sidecar_dir()).unwrap();
    std::fs::write(
        s.sidecar_dir().join("kinds.toml"),
        "[kinds.energy_estimate]\nnature = \"measurement\"\nvalue = \"integer\"\nmin = 1\nmax = 5\n",
    )
    .unwrap();
    policy(&s, "[energy]\napply = 0.5\nfrom = \"energy_estimate\"\n");
    ok(&s, &["status"]);
    fact(&s, &current, "energy_estimate", "4", "tool:meter", "0.9");
    let measured = fact(&s, &changed, "energy_estimate", "5", "tool:meter", "0.9");
    // The audio changes after it was measured; nothing has looked since.
    let path = s.path().join("audio/Changed.wav");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[44] ^= 0x55;
    std::fs::write(&path, bytes).unwrap();

    let dry = ok(&s, &["project", "--dry-run"]);
    assert_eq!(
        changes(&dry, "changes"),
        vec![(current.clone(), "energy".into(), json!(null), json!(4))]
    );
    assert_eq!(dry["stale"][0]["track"], changed.as_str());
    assert_eq!(dry["stale"][0]["fact"], measured["id"]);
    let applied = ok(&s, &["project"]);
    assert_eq!(changes(&applied, "changes"), changes(&dry, "changes"));
    assert_eq!(track(&s, &current)["rating"], 4);
    assert_eq!(track(&s, &changed)["rating"], 0);
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
        s.sidecar_dir().join("projection.toml"),
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
