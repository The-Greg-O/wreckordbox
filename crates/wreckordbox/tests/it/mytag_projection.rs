//! Projecting the Tag schema to native MyTags (wbx-29) through the real CLI
//! and a service on a fixture Library, with a second client watching.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
use crate::common;

use std::collections::BTreeMap;
use std::path::PathBuf;
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

fn write_schema(s: &Sandbox, text: &str) {
    std::fs::create_dir_all(s.sidecar_dir()).unwrap();
    std::fs::write(s.sidecar_dir().join("tag-schema.toml"), text).unwrap();
}

/// Records a Fact from `source` with `values` as its list of terms.
fn fact(s: &Sandbox, track: &str, kind: &str, values: &[&str], source: &str) -> i64 {
    let mut args = vec!["facts", "record", track, kind];
    args.extend_from_slice(values);
    args.extend_from_slice(&["--source", source, "--confidence", "0.9"]);
    ok(s, &args)["facts"][0]["id"].as_i64().unwrap()
}

/// The database files of the Library and the Sidecar, byte for byte.
fn stored_bytes(s: &Sandbox) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    for db in [
        s.master_db().to_path_buf(),
        s.sidecar_dir().join("sidecar.db"),
    ] {
        for suffix in ["", "-wal", "-journal"] {
            let path = PathBuf::from(format!("{}{suffix}", db.display()));
            if path.exists() {
                files.insert(path.clone(), std::fs::read(path).unwrap());
            }
        }
    }
    files
}

/// Every track's MyTags as `(track, [category: tag])`, from `mytags show`.
fn tags_of(s: &Sandbox, track: &str) -> Vec<String> {
    let mut tags: Vec<String> = ok(s, &["mytags", "show", track])["my_tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tag| {
            format!(
                "{}: {}",
                tag["category"].as_str().unwrap(),
                tag["name"].as_str().unwrap()
            )
        })
        .collect();
    tags.sort();
    tags
}

/// `(track, tag label, add|remove, implied)` of each change listed.
fn changes(report: &Value) -> Vec<(String, String, String, bool)> {
    report["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["track"].as_str().unwrap().to_owned(),
                c["label"].as_str().unwrap().to_owned(),
                c["change"].as_str().unwrap().to_owned(),
                c["implied"].as_bool().unwrap(),
            )
        })
        .collect()
}

/// `(track, tag label, add|remove, reason)` of each listed under `key`.
fn listed(report: &Value, key: &str) -> Vec<(String, String, String, String)> {
    report[key]
        .as_array()
        .unwrap_or_else(|| panic!("no {key}: {report}"))
        .iter()
        .map(|c| {
            (
                c["track"].as_str().unwrap().to_owned(),
                c["label"].as_str().unwrap().to_owned(),
                c["change"].as_str().unwrap().to_owned(),
                c["reason"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

struct Deck {
    s: Sandbox,
    mood: String,
    dark: String,
    uplifting: String,
}

/// Four categories as rekordbox always has them; Mood and a Sub-genre tree
/// in use, every child put on with its parent, as most DJs do it.
fn tagged_by_hand() -> Deck {
    let s = Sandbox::new();
    let [_, mood, _sound, sub_genre] = s.four_categories();
    let dark = s.create_my_tag_in(&mood, "Dark");
    let uplifting = s.create_my_tag_in(&mood, "Uplifting");
    let house = s.create_my_tag_in(&sub_genre, "House");
    let deep = s.create_my_tag_in(&sub_genre, "> Deep");
    let techno = s.create_my_tag_in(&sub_genre, "Techno");
    let hypnotic = s.create_my_tag_in(&sub_genre, "> Hypnotic");
    let peak = rbl_db::fixture::MY_TAG_PEAK;
    for (index, tags) in [
        (1, vec![peak, dark.as_str(), house.as_str(), deep.as_str()]),
        (
            2,
            vec![uplifting.as_str(), techno.as_str(), hypnotic.as_str()],
        ),
        (3, vec![dark.as_str(), uplifting.as_str()]),
    ] {
        s.detail_track(
            index,
            &common::Details {
                my_tags: &tags,
                ..common::Details::default()
            },
        );
    }
    Deck {
        s,
        mood,
        dark,
        uplifting,
    }
}

/// The acceptance test: convert today's MyTags to a schema, project back,
/// get identical MyTags. Converting makes the DJ's Facts and no native
/// write, so every existing category, tag and assignment stays the DJ's;
/// the projection wants exactly what is there, so it lists no change and
/// claims nothing. The dry run writes nothing anywhere and tells no one.
#[test]
fn converting_the_mytags_and_projecting_back_lists_no_change_and_writes_nothing() {
    let Deck { s, .. } = tagged_by_hand();
    policy(&s, "[mytags]\napply = 0.5\n");
    let converted = ok(&s, &["schema", "convert"]);
    assert_eq!(converted["implied_parents"]["tag_parents"], true);
    let before: Vec<Vec<String>> = ["10001", "10002", "10003"]
        .iter()
        .map(|t| tags_of(&s, t))
        .collect();
    let runs = ok(&s, &["runs", "list"])["total"].clone();
    let mut watcher = s.client();
    watcher.subscribe().unwrap();
    let bytes = stored_bytes(&s);

    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);

    assert_eq!(dry["dry_run"], true);
    assert_eq!(dry["changes"], json!([]), "{dry}");
    assert_eq!(dry["held"], json!([]));
    assert_eq!(dry["kept"], json!([]));
    assert_eq!(dry["blocked"], json!([]));
    assert_eq!(dry["definitions"], json!([]));
    assert_eq!(dry["invalid"], json!([]));
    // Every assignment there is is one the schema's Facts want: Peak, Dark,
    // House and Deep; Uplifting, Techno, Hypnotic; Dark and Uplifting.
    assert_eq!(dry["unchanged"], 9);
    let categories: Vec<(&str, &str, &str)> = dry["categories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["label"].as_str().unwrap(),
                c["dimension"].as_str().unwrap(),
                c["state"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        categories,
        [
            ("Situation", "situation", "matched"),
            ("Mood", "mood", "matched"),
            ("Sound", "sound", "matched"),
            ("Sub-genre", "genre", "matched"),
        ]
    );
    // Nothing written: not the Library, not the Sidecar; no Run, no event.
    assert!(stored_bytes(&s) == bytes, "a dry run wrote");
    assert_eq!(ok(&s, &["runs", "list"])["total"], runs);
    assert!(watcher
        .next_event(Duration::from_millis(400))
        .unwrap()
        .is_none());
    let after: Vec<Vec<String>> = ["10001", "10002", "10003"]
        .iter()
        .map(|t| tags_of(&s, t))
        .collect();
    assert_eq!(after, before);

    // Applying it is the same: nothing to do, nothing written, no Run.
    let applied = ok(&s, &["project", "--mytags"]);
    assert_eq!(applied["changes"], json!([]));
    assert_eq!(applied["plan"], dry["plan"]);
    assert_eq!(ok(&s, &["runs", "list"])["total"], runs);
}

const SCHEMA: &str = r#"version = 1

[genre]
terms = [
  { id = "house", name = "House" },
  { id = "house-deep", name = "Deep", parent = "house" },
  { id = "techno", name = "Techno" },
  { id = "techno-hypnotic", name = "Hypnotic", parent = "techno" },
]

[dimensions.mood]
terms = [
  { id = "dark", name = "Dark" },
  { id = "uplifting", name = "Uplifting" },
  { id = "euphoric", name = "Euphoric" },
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

/// A dry run lists every category, tag and assignment that would change,
/// and every one it would not touch and why: what the DJ set by hand (by a
/// recorded edit, or before any record) is kept; a dimension not on the
/// deck is not written at all; a child brings its parent with no Fact of
/// its own.
#[test]
fn a_dry_run_lists_each_change_and_keeps_what_the_dj_set_by_hand() {
    let Deck {
        s, uplifting, mood, ..
    } = tagged_by_hand();
    write_schema(&s, SCHEMA);
    // The DJ takes Uplifting off track 3 by hand: a recorded DJ edit.
    ok(
        &s,
        &["mytags", "unassign", &uplifting, "10003", "--source", "dj"],
    );
    let tagger = "agent:tagger";
    let fill = fact(&s, "10004", "mood", &["uplifting"], tagger);
    fact(&s, "10004", "genre", &["techno-hypnotic"], tagger);
    fact(&s, "10001", "mood", &["uplifting"], tagger);
    fact(&s, "10002", "mood", &["euphoric"], tagger);
    fact(&s, "10003", "mood", &["uplifting"], tagger);
    fact(&s, "10005", "era", &["nineties"], tagger);
    fact(&s, "10006", "situation", &["peak"], tagger);
    let bytes = stored_bytes(&s);

    // No policy: every change waits for the DJ.
    let waiting = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(waiting["changes"], json!([]));
    let mut held = listed(&waiting, "held");
    held.sort();
    assert_eq!(
        held,
        [
            (
                "10001".into(),
                "Uplifting".into(),
                "add".into(),
                "waits_for_dj".into()
            ),
            (
                "10004".into(),
                "> Hypnotic".into(),
                "add".into(),
                "waits_for_dj".into()
            ),
            (
                "10004".into(),
                "Techno".into(),
                "add".into(),
                "waits_for_dj".into()
            ),
            (
                "10004".into(),
                "Uplifting".into(),
                "add".into(),
                "waits_for_dj".into()
            ),
        ]
    );

    policy(&s, "[mytags]\napply = 0.5\n");
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    let mut planned = changes(&dry);
    planned.sort();
    assert_eq!(
        planned,
        [
            ("10001".into(), "Uplifting".into(), "add".into(), false),
            // The parent comes with its child, from the same Fact.
            ("10004".into(), "> Hypnotic".into(), "add".into(), false),
            ("10004".into(), "Techno".into(), "add".into(), true),
            ("10004".into(), "Uplifting".into(), "add".into(), false),
        ]
    );
    let first = dry["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["track"] == "10004" && c["label"] == "Uplifting")
        .unwrap();
    assert_eq!(first["fact"], fill);
    assert_eq!(first["category"], "Mood");
    assert_eq!(first["category_id"], mood.as_str());
    assert_eq!(first["tag"], uplifting.as_str());
    assert_eq!(first["source"], json!({"kind": "agent", "name": "tagger"}));
    let mut kept = listed(&dry, "kept");
    kept.sort();
    assert_eq!(
        kept,
        [
            // Dark was put on before anything was recorded: the DJ's.
            (
                "10001".into(),
                "Dark".into(),
                "remove".into(),
                "no_record".into()
            ),
            (
                "10002".into(),
                "Uplifting".into(),
                "remove".into(),
                "no_record".into()
            ),
            (
                "10003".into(),
                "Dark".into(),
                "remove".into(),
                "no_record".into()
            ),
            // Taken off by the DJ's recorded edit: never put back.
            (
                "10003".into(),
                "Uplifting".into(),
                "add".into(),
                "set_by_dj".into()
            ),
        ]
    );
    // A term with no MyTag yet, and a category the Library does not have.
    let mut blocked = listed(&dry, "blocked");
    blocked.sort();
    assert_eq!(
        blocked,
        [
            (
                "10002".into(),
                "Euphoric".into(),
                "add".into(),
                "no_my_tag".into()
            ),
            (
                "10005".into(),
                "90s".into(),
                "add".into(),
                "no_category".into()
            ),
        ]
    );
    assert_eq!(
        dry["definitions"],
        json!([{"change": "create_tag", "category": "Mood", "category_id": mood, "label": "Euphoric", "dimension": "mood", "term": "euphoric"}])
    );
    let era = dry["categories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["label"] == "Era")
        .unwrap()
        .clone();
    assert_eq!(era["state"], "missing");
    assert_eq!(era["category_id"], Value::Null);
    // Situation is in the Library but not on the deck: never written.
    let outside: Vec<&str> = dry["outside_deck"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["category"].as_str().unwrap())
        .collect();
    assert_eq!(outside, ["Situation", "Sound"]);
    for report in [&waiting, &dry] {
        assert!(!report.to_string().contains("10006"), "{report}");
    }
    assert!(stored_bytes(&s) == bytes, "a dry run wrote");
    assert_ne!(waiting["plan"], dry["plan"]);
}

/// A Fact naming a term the schema does not have, or a value that is not
/// a list of terms, is listed as unusable and projects nothing.
#[test]
fn a_fact_the_schema_cannot_place_is_listed_and_not_projected() {
    let Deck { s, .. } = tagged_by_hand();
    write_schema(&s, SCHEMA);
    policy(&s, "[mytags]\napply = 0.5\n");
    let id = fact(&s, "10004", "mood", &["dark"], "agent:tagger");
    // The schema loses the term afterwards.
    write_schema(
        &s,
        &SCHEMA.replace("  { id = \"dark\", name = \"Dark\" },\n", ""),
    );
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    let invalid = dry["invalid"].as_array().unwrap();
    assert_eq!(invalid.len(), 1, "{dry}");
    assert_eq!(invalid[0]["fact"], id);
    assert_eq!(invalid[0]["track"], "10004");
    assert_eq!(changes(&dry), []);
    // A broken policy refuses, naming the file.
    policy(&s, "[mytags]\napply = 2\n");
    let error = refused(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(error["code"], "projection_policy_invalid");
}

/// The cost asked for: a Library of 2,000 tracks, each with a mood and a
/// genre Fact, projected as a dry run, then applied, then again.
#[test]
fn a_dry_run_over_two_thousand_tracks_stays_cheap() {
    let s = Sandbox::with_shape(rbl_db::fixture::Shape {
        tracks: 2000,
        ..rbl_db::fixture::Shape::default()
    });
    let [_, mood, _, sub_genre] = s.four_categories();
    for name in ["Dark", "Uplifting", "Euphoric"] {
        s.create_my_tag_in(&mood, name);
    }
    for name in ["House", "> Deep", "Techno", "> Hypnotic"] {
        s.create_my_tag_in(&sub_genre, name);
    }
    write_schema(&s, SCHEMA);
    policy(&s, "[mytags]\napply = 0.5\n");
    ok(&s, &["status"]);
    let moods = ["dark", "uplifting", "euphoric"];
    let genres = ["house-deep", "techno-hypnotic", "techno"];
    let lines: String = (0..2000)
        .flat_map(|i| {
            let id = rbl_db::fixture::track_id(i);
            [
                json!({"track": id, "kind": "mood", "value": [moods[i % 3]], "confidence": 0.9}),
                json!({"track": id, "kind": "genre", "value": [genres[i % 3]], "confidence": 0.9}),
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
    let (dry, dry_took) = time(&["project", "--mytags", "--dry-run"]);
    // One mood each; a deep or hypnotic child brings its parent.
    let expected = 2000 + 2000 + (0..2000).filter(|i| i % 3 != 2).count();
    assert_eq!(dry["changes"].as_array().unwrap().len(), expected);
    let (applied, apply_took) = time(&["project", "--mytags"]);
    assert_eq!(applied["plan"], dry["plan"]);
    let (again, again_took) = time(&["project", "--mytags", "--dry-run"]);
    assert_eq!(again["changes"], json!([]));
    assert_eq!(again["unchanged"], expected);
    eprintln!(
        "2,000 tracks, 4,000 Facts, {expected} assignments: dry run {dry_took:?}, apply {apply_took:?}, dry run again {again_took:?}"
    );
    assert!(dry_took < Duration::from_secs(20), "{dry_took:?}");
    assert!(apply_took < Duration::from_secs(60), "{apply_took:?}");
}

/// The hand-written schema with every term a MyTag of the Library and every
/// category one of its own: nothing blocked, nothing to define.
fn placed_schema() -> String {
    SCHEMA
        .replace("  { id = \"euphoric\", name = \"Euphoric\" },\n", "")
        .replace("  { dimension = \"era\", label = \"Era\" },\n", "")
}

/// Stops the service here and waits until its process is gone.
fn stop_service(s: &Sandbox) {
    let Ok(mut client) =
        wreckordbox_service::Client::connect(&s.config().socket_path(s.location()))
    else {
        return;
    };
    let pid = client.status().ok().map(|status| u64::from(status.pid));
    let _ = client.stop();
    if let Some(pid) = pid {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while common::alive(pid) {
            assert!(
                std::time::Instant::now() < deadline,
                "the service did not stop"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// Runs a CLI command whose service, started by it, has `env` set.
fn with_env(s: &Sandbox, env: &[(&str, &str)], args: &[&str]) -> common::Run {
    let mut command = s.command();
    for (key, value) in env {
        command.env(key, value);
    }
    command.arg("--json").args(args);
    s.run(&mut command)
}

fn waits_for(file: &std::path::Path) {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !file.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "{} never appeared",
            file.display()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The DJ's Facts about `track`, by kind.
fn dj_facts(s: &Sandbox, track: &str) -> Vec<Value> {
    ok(s, &["facts", "show", track])["facts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|fact| fact["source"]["kind"] == "dj")
        .cloned()
        .collect()
}

/// Applying does exactly what the dry run listed, as ONE recorded edit in a
/// Run of its own by a tool: one Ledger entry, one `library_changed`, no
/// DJ Fact. It is never the DJ's, and a plan other than the one checked is
/// refused with nothing written.
#[test]
fn applying_is_one_run_by_a_tool_that_does_what_the_dry_run_listed_and_makes_no_dj_fact() {
    let Deck { s, .. } = tagged_by_hand();
    write_schema(&s, &placed_schema());
    policy(&s, "[mytags]\napply = 0.5\n");
    fact(&s, "10004", "mood", &["uplifting"], "agent:tagger");
    fact(&s, "10004", "genre", &["techno-hypnotic"], "agent:tagger");
    fact(&s, "10001", "mood", &["uplifting"], "agent:tagger");
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(dry["changes"].as_array().unwrap().len(), 4, "{dry}");
    let before = tags_of(&s, "10004");

    // Never the DJ's, and never another plan than the one checked.
    let error = refused(&s, &["project", "--mytags", "--source", "dj"]);
    assert_eq!(error["code"], "projection_not_the_djs");
    let error = refused(&s, &["project", "--mytags", "--expect", "0000000000000000"]);
    assert_eq!(error["code"], "projection_changed");
    assert_eq!(tags_of(&s, "10004"), before);
    let runs = ok(&s, &["runs", "list"])["total"].as_i64().unwrap();

    let mut watcher = s.client();
    watcher.subscribe().unwrap();
    let plan = dry["plan"].as_str().unwrap();
    let applied = ok(&s, &["project", "--mytags", "--expect", plan]);

    assert_eq!(applied["changes"], dry["changes"]);
    assert_eq!(
        tags_of(&s, "10004"),
        [
            "Mood: Uplifting",
            "Sub-genre: > Hypnotic",
            "Sub-genre: Techno"
        ]
    );
    assert_eq!(
        tags_of(&s, "10001"),
        [
            "Mood: Dark",
            "Mood: Uplifting",
            "Situation: Peak",
            "Sub-genre: > Deep",
            "Sub-genre: House",
        ]
    );
    let run = &applied["run"];
    assert_eq!(run["source"], json!({"kind": "tool", "name": "projection"}));
    assert!(
        run["description"]
            .as_str()
            .unwrap()
            .starts_with("MyTag projection under policy "),
        "{run}"
    );
    assert_eq!(ok(&s, &["runs", "list"])["total"], runs + 1);
    let shown = ok(&s, &["runs", "show", &run["id"].to_string()]);
    let entries = shown["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "one Ledger entry: {shown}");
    assert_eq!(entries[0]["kind"], "track_tags");
    assert_eq!(entries[0]["reversible"], true);
    let mut heard = Vec::new();
    while let Some(event) = watcher.next_event(Duration::from_millis(500)).unwrap() {
        heard.push(event.event);
    }
    assert_eq!(heard, ["library_changed"]);
    // A projection's write is a tool's: it made no DJ Fact.
    for track in ["10001", "10004"] {
        assert_eq!(dj_facts(&s, track), Vec::<Value>::new(), "{track}");
    }
    // And now there is nothing to do.
    let again = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(again["changes"], json!([]));
}

/// What a projection put on follows its Fact; once anyone else writes the
/// assignment (the DJ by a recorded edit, or rekordbox while wreckordbox
/// was closed, which leaves other rows) it is the DJ's and is kept.
#[test]
fn a_projected_mytag_follows_its_fact_until_anyone_else_writes_it() {
    let Deck { s, uplifting, .. } = tagged_by_hand();
    write_schema(&s, &placed_schema());
    policy(&s, "[mytags]\napply = 0.5\n");
    fact(&s, "10004", "mood", &["uplifting"], "agent:tagger");
    fact(&s, "10005", "mood", &["uplifting"], "agent:tagger");
    fact(&s, "10006", "mood", &["uplifting"], "agent:tagger");
    ok(&s, &["project", "--mytags"]);
    for track in ["10004", "10005", "10006"] {
        assert_eq!(tags_of(&s, track), ["Mood: Uplifting"], "{track}");
    }

    // The agent changes its mind: the projection's own tag comes off.
    fact(&s, "10004", "mood", &["dark"], "agent:tagger");
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    let mut planned = changes(&dry);
    planned.sort();
    assert_eq!(
        planned,
        [
            ("10004".into(), "Dark".into(), "add".into(), false),
            ("10004".into(), "Uplifting".into(), "remove".into(), false),
        ]
    );
    ok(&s, &["project", "--mytags"]);
    assert_eq!(tags_of(&s, "10004"), ["Mood: Dark"]);

    // The DJ puts Uplifting back by hand: theirs from now on.
    ok(
        &s,
        &["mytags", "assign", &uplifting, "10004", "--source", "dj"],
    );
    // On 10005, rekordbox takes it off and puts it on again while
    // wreckordbox is closed: nothing recorded, but other rows.
    stop_service(&s);
    s.edit(|writer| {
        writer.set_my_tags("10005", &[]).unwrap();
        writer
            .set_my_tags("10005", std::slice::from_ref(&uplifting))
            .unwrap();
    });
    for track in ["10004", "10005", "10006"] {
        fact(&s, track, "mood", &["dark"], "agent:tagger");
    }
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    let mut kept = listed(&dry, "kept");
    kept.sort();
    assert_eq!(
        kept,
        [
            (
                "10004".into(),
                "Uplifting".into(),
                "remove".into(),
                "set_by_dj".into()
            ),
            (
                "10005".into(),
                "Uplifting".into(),
                "remove".into(),
                "written_since".into()
            ),
        ]
    );
    // 10006 is still exactly as the projection left it.
    assert!(changes(&dry).contains(&("10006".into(), "Uplifting".into(), "remove".into(), false)));
    ok(&s, &["project", "--mytags"]);
    assert_eq!(tags_of(&s, "10004"), ["Mood: Dark", "Mood: Uplifting"]);
    assert_eq!(tags_of(&s, "10005"), ["Mood: Dark", "Mood: Uplifting"]);
    assert_eq!(tags_of(&s, "10006"), ["Mood: Dark"]);
}

#[test]
fn library_protection_refuses_the_apply_and_nothing_is_written() {
    let Deck { s, .. } = tagged_by_hand();
    write_schema(&s, &placed_schema());
    policy(&s, "[mytags]\napply = 0.5\n");
    fact(&s, "10004", "mood", &["uplifting"], "agent:tagger");
    ok(&s, &["protection", "on"]);
    let runs = ok(&s, &["runs", "list"])["total"].clone();
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(dry["library_protected"], true);
    let error = refused(&s, &["project", "--mytags"]);
    assert_eq!(error["code"], "library_protected", "{error}");
    assert_eq!(tags_of(&s, "10004"), Vec::<String>::new());
    assert_eq!(ok(&s, &["runs", "list"])["total"], runs);
    ok(&s, &["protection", "off"]);
    // Nothing was kept of the refused apply: the same plan stands.
    let after = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(after["plan"], dry["plan"]);
}

/// A tag the schema has and the Library does not is listed; applying
/// refuses, with nothing written, until it exists.
#[test]
fn an_apply_with_mytags_still_to_make_is_refused_and_writes_nothing() {
    let Deck { s, .. } = tagged_by_hand();
    write_schema(&s, SCHEMA);
    policy(&s, "[mytags]\napply = 0.5\n");
    fact(&s, "10004", "mood", &["uplifting"], "agent:tagger");
    let error = refused(&s, &["project", "--mytags"]);
    assert_eq!(error["code"], "projection_definitions_pending");
    assert_eq!(tags_of(&s, "10004"), Vec::<String>::new());
}

/// A write by another connection between the plan and the write (here
/// rekordbox's, through the Library's own writer) refuses the whole apply
/// with nothing written: the plan is checked again under the edit gate.
#[test]
fn a_write_between_the_plan_and_the_write_refuses_the_whole_apply() {
    let Deck { s, uplifting, .. } = tagged_by_hand();
    write_schema(&s, &placed_schema());
    policy(&s, "[mytags]\napply = 0.5\n");
    fact(&s, "10004", "mood", &["uplifting"], "agent:tagger");
    fact(&s, "10005", "mood", &["uplifting"], "agent:tagger");
    stop_service(&s);
    let gate = tempfile::tempdir().unwrap();
    let dir = gate.path().to_str().unwrap().to_owned();
    let applying = {
        let command = s.command();
        let dir = dir.clone();
        std::thread::spawn(move || {
            let mut command = command;
            command
                .env("WRECKORDBOX_PROJECTION_GATE", &dir)
                .args(["--json", "project", "--mytags"]);
            command.output().unwrap()
        })
    };
    waits_for(&gate.path().join("paused"));
    s.edit(|writer| {
        writer
            .set_my_tags("10005", std::slice::from_ref(&uplifting))
            .unwrap();
    });
    std::fs::write(gate.path().join("go"), b"").unwrap();
    let output = applying.join().unwrap();
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "projection_changed", "{error}");
    assert_eq!(stored_tags(&s, "10004"), Vec::<String>::new());
    assert_eq!(stored_tags(&s, "10005"), ["Uplifting"]);
}

/// Past the last check under the gate, the checked writer compares the
/// rows again in its own transaction: a write by another connection at
/// that moment is never overwritten, and nothing of the batch is written.
#[test]
fn a_write_after_the_last_check_is_never_overwritten() {
    let Deck { s, dark, .. } = tagged_by_hand();
    write_schema(&s, &placed_schema());
    policy(&s, "[mytags]\napply = 0.5\n");
    fact(&s, "10004", "mood", &["uplifting"], "agent:tagger");
    fact(&s, "10005", "mood", &["dark"], "agent:tagger");
    stop_service(&s);
    let gate = tempfile::tempdir().unwrap();
    let dir = gate.path().to_str().unwrap().to_owned();
    let applying = {
        let command = s.command();
        let dir = dir.clone();
        std::thread::spawn(move || {
            let mut command = command;
            command
                .env("WRECKORDBOX_MYTAG_PROJECTION_HOLD", &dir)
                .args(["--json", "project", "--mytags"]);
            command.output().unwrap()
        })
    };
    waits_for(&gate.path().join("checked"));
    // rekordbox puts Dark on 10005 itself, where the plan saw no row.
    s.edit(|writer| {
        writer
            .set_my_tags("10005", std::slice::from_ref(&dark))
            .unwrap();
    });
    std::fs::write(gate.path().join("write"), b"").unwrap();
    let output = applying.join().unwrap();
    assert!(
        !output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "projection_changed", "{error}");
    assert_eq!(stored_tags(&s, "10004"), Vec::<String>::new());
    assert_eq!(stored_tags(&s, "10005"), ["Dark"]);
}

/// A service killed after its write and before the Ledger entry leaves the
/// written MyTags the DJ's (no record) and a barrier bounded to exactly the
/// tracks the apply was writing: any other track is still projected.
#[test]
fn a_crash_after_the_write_leaves_a_barrier_bounded_to_the_written_tracks() {
    let Deck { s, .. } = tagged_by_hand();
    write_schema(&s, &placed_schema());
    policy(&s, "[mytags]\napply = 0.5\n");
    fact(&s, "10004", "mood", &["uplifting"], "agent:tagger");
    fact(&s, "10005", "mood", &["uplifting"], "agent:tagger");
    stop_service(&s);
    let crashed = with_env(
        &s,
        &[("WRECKORDBOX_LEDGER_FAULTS", "abort_after_write")],
        &["project", "--mytags"],
    );
    assert!(!crashed.status.success());
    // The next service recovers the unconfirmed write.
    let barriers = ok(&s, &["projection", "barriers"]);
    let listed = barriers["barriers"].as_array().unwrap();
    assert_eq!(listed.len(), 1, "{barriers}");
    assert_eq!(listed[0]["what"], "unconfirmed");
    assert_eq!(listed[0]["tracks"], json!(["10004", "10005"]), "{barriers}");
    assert_eq!(tags_of(&s, "10004"), ["Mood: Uplifting"]);
    // What was written is the DJ's now; another track is projected.
    fact(&s, "10004", "mood", &["dark"], "agent:tagger");
    fact(&s, "10006", "mood", &["dark"], "agent:tagger");
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert!(
        listed_has(&dry, "kept", "10004", "Uplifting", "no_record"),
        "{dry}"
    );
    assert!(
        listed_has(&dry, "kept", "10004", "Dark", "provenance_incomplete"),
        "{dry}"
    );
    assert_eq!(
        changes(&dry),
        [("10006".into(), "Dark".into(), "add".into(), false)]
    );
}

fn listed_has(report: &Value, key: &str, track: &str, label: &str, reason: &str) -> bool {
    listed(report, key)
        .iter()
        .any(|(t, l, _, r)| t == track && l == label && r == reason)
}

/// A track's live MyTags as the Library's database holds them, read on a
/// connection of its own (a write by another connection is not in the
/// service's index until it next reads the Library).
fn stored_tags(s: &Sandbox, track: &str) -> Vec<String> {
    let mut names = Vec::new();
    s.edit(|writer| {
        let db = writer.library().connection();
        let mut statement = db
            .prepare(
                "SELECT t.Name FROM djmdSongMyTag s JOIN djmdMyTag t ON t.ID = s.MyTagID
                 WHERE s.ContentID = ?1 AND s.rb_local_deleted = 0 ORDER BY t.Name",
            )
            .unwrap();
        names = statement
            .query_map([track], |row| row.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
    });
    names
}

/// `(track, tag label, add|remove, reason)` of what is kept, sorted.
fn kept_of(report: &Value) -> Vec<(String, String, String, String)> {
    let mut kept = listed(report, "kept");
    kept.sort();
    kept
}

fn row(track: &str, label: &str, change: &str, reason: &str) -> (String, String, String, String) {
    (track.into(), label.into(), change.into(), reason.into())
}

/// The DJ confirming a MyTag as it is (on, or off) is the DJ's choice,
/// recorded as an assertion of that one assignment: by `mytags assign` or
/// `unassign` as the DJ (a selection's unchanged members too, while only
/// its changes are the Undo step), and by an unchanged selection in
/// Intake's tag panel. A projection never changes it afterwards. The same
/// set by a client that is not the DJ asserts nothing.
#[test]
fn the_djs_confirmation_of_a_mytag_as_it_is_is_theirs_and_a_projection_keeps_it() {
    let Deck { s, uplifting, .. } = tagged_by_hand();
    write_schema(&s, &placed_schema());
    policy(&s, "[mytags]\napply = 0.5\n");
    for track in ["10004", "10005", "10007", "10008"] {
        fact(&s, track, "mood", &["uplifting"], "agent:tagger");
    }
    ok(&s, &["project", "--mytags"]);
    // 10007's agent changes its mind: the projection takes Uplifting off.
    fact(&s, "10007", "mood", &["dark"], "agent:tagger");
    ok(&s, &["project", "--mytags"]);
    assert_eq!(tags_of(&s, "10007"), ["Mood: Dark"]);

    // The DJ puts Uplifting on 10004 (it is on) and 10006 (it is not).
    let mixed = ok(
        &s,
        &[
            "mytags", "assign", &uplifting, "10004", "10006", "--source", "dj",
        ],
    );
    assert_eq!(mixed["changed"], 1, "{mixed}");
    let shown = ok(&s, &["runs", "show", &mixed["run"]["id"].to_string()]);
    let entries = shown["entries"].as_array().unwrap();
    let mut kinds: Vec<&str> = entries
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    kinds.sort_unstable();
    assert_eq!(kinds, ["my_tag_assertion", "track_tags"], "{shown}");
    let of = |kind: &str| entries.iter().find(|e| e["kind"] == kind).unwrap();
    // The Undo step holds the change only; the assertion is no step.
    let change = of("track_tags");
    assert_eq!(change["payload"]["tags"].as_array().unwrap().len(), 1);
    assert_eq!(change["payload"]["tags"][0]["track"], "10006");
    let assertion = of("my_tag_assertion");
    assert_eq!(assertion["reversible"], false);
    assert_eq!(
        assertion["payload"]["assertions"],
        json!([{"track": "10004", "tag": uplifting, "on": true}])
    );
    // The DJ takes Uplifting off 10007, where it already is off.
    ok(
        &s,
        &["mytags", "unassign", &uplifting, "10007", "--source", "dj"],
    );
    // The DJ's tag panel, Mood on 10005 as it is: Uplifting, not Dark.
    let panel = ok(
        &s,
        &[
            "intake",
            "tag",
            "10005",
            "mood",
            "uplifting",
            "--source",
            "dj",
        ],
    );
    assert_eq!(panel["changed"], false, "{panel}");
    // Someone other than the DJ "puts" Uplifting on 10008, where it is: no
    // assertion, nothing recorded.
    let runs = ok(&s, &["runs", "list"])["total"].clone();
    ok(&s, &["mytags", "assign", &uplifting, "10008"]);
    assert_eq!(ok(&s, &["runs", "list"])["total"], runs);

    for track in ["10004", "10005", "10008"] {
        fact(&s, track, "mood", &["dark"], "agent:tagger");
    }
    fact(&s, "10007", "mood", &["uplifting"], "agent:tagger");
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(
        kept_of(&dry),
        [
            row("10004", "Uplifting", "remove", "set_by_dj"),
            row("10005", "Dark", "add", "set_by_dj"),
            row("10005", "Uplifting", "remove", "set_by_dj"),
            row("10007", "Uplifting", "add", "set_by_dj"),
        ],
        "{dry}"
    );
    let mut planned = changes(&dry);
    planned.sort();
    assert_eq!(
        planned,
        [
            ("10004".into(), "Dark".into(), "add".into(), false),
            // The projection's own Dark, which the DJ did not confirm.
            ("10007".into(), "Dark".into(), "remove".into(), false),
            ("10008".into(), "Dark".into(), "add".into(), false),
            ("10008".into(), "Uplifting".into(), "remove".into(), false),
        ]
    );
}

/// A DJ's selection cut off after its change was recorded and before its
/// assertion was: the intent the assertion was to settle stays, and
/// recovery makes it a barrier bounded to the selection's tracks, so the
/// confirmed member is not taken over and other tracks still project.
#[test]
fn an_assertion_cut_off_after_its_change_leaves_a_barrier_over_its_tracks_only() {
    let Deck { s, uplifting, .. } = tagged_by_hand();
    write_schema(&s, &placed_schema());
    policy(&s, "[mytags]\napply = 0.5\n");
    for track in ["10004", "10005"] {
        fact(&s, track, "mood", &["uplifting"], "agent:tagger");
    }
    ok(&s, &["project", "--mytags"]);
    stop_service(&s);
    let cut = with_env(
        &s,
        &[("WRECKORDBOX_LEDGER_FAULTS", "abort_before_assert")],
        &[
            "mytags", "assign", &uplifting, "10004", "10006", "--source", "dj",
        ],
    );
    assert!(!cut.status.success());
    let barriers = ok(&s, &["projection", "barriers"]);
    let listed = barriers["barriers"].as_array().unwrap();
    assert_eq!(listed.len(), 1, "{barriers}");
    let tracks = listed[0]["tracks"].as_array().unwrap();
    assert!(
        tracks.contains(&json!("10004")) && tracks.contains(&json!("10006")),
        "{barriers}"
    );
    assert!(!tracks.contains(&json!("10005")), "{barriers}");
    for track in ["10004", "10005"] {
        fact(&s, track, "mood", &["dark"], "agent:tagger");
    }
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert!(
        listed_has(&dry, "kept", "10004", "Uplifting", "provenance_incomplete"),
        "{dry}"
    );
    assert!(changes(&dry).contains(&("10005".into(), "Uplifting".into(), "remove".into(), false)));
}

/// A second projection killed after its write and before its Ledger entry:
/// its mark is no evidence, because the write it names was never
/// confirmed. The assignment is not the projection's, before the DJ clears
/// the barrier and after (clearing never hands anything to a projection).
#[test]
fn a_crash_during_a_second_projection_leaves_its_assignments_the_djs_cleared_or_not() {
    let Deck { s, .. } = tagged_by_hand();
    write_schema(&s, &placed_schema());
    policy(&s, "[mytags]\napply = 0.5\n");
    fact(&s, "10004", "mood", &["uplifting"], "agent:tagger");
    ok(&s, &["project", "--mytags"]);
    fact(&s, "10004", "mood", &["dark"], "agent:tagger");
    stop_service(&s);
    let crashed = with_env(
        &s,
        &[("WRECKORDBOX_LEDGER_FAULTS", "abort_after_write")],
        &["project", "--mytags"],
    );
    assert!(!crashed.status.success());
    fact(&s, "10004", "mood", &["uplifting"], "agent:tagger");
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(changes(&dry), [], "{dry}");
    let kept = kept_of(&dry);
    assert_eq!(kept.len(), 2, "{dry}");
    assert!(kept
        .iter()
        .any(|(_, label, change, _)| label == "Uplifting" && change == "add"));

    let barriers = ok(&s, &["projection", "barriers"]);
    let barrier = barriers["barriers"][0]["seq"].to_string();
    ok(
        &s,
        &["projection", "acknowledge", &barrier, "--source", "dj"],
    );
    let cleared = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(changes(&cleared), [], "{cleared}");
    assert_eq!(kept_of(&cleared).len(), 2, "{cleared}");
    assert_eq!(tags_of(&s, "10004"), ["Mood: Dark"]);
}

/// A MyTag entry of the Ledger whose payload cannot be read is a barrier
/// like any other: `projection barriers` lists it, the DJ can clear it,
/// and then the empty assignments it concerned may be filled.
#[test]
fn an_unreadable_mytag_entry_is_listed_and_cleared_as_a_barrier() {
    let Deck { s, .. } = tagged_by_hand();
    write_schema(&s, &placed_schema());
    policy(&s, "[mytags]\napply = 0.5\n");
    fact(&s, "10004", "mood", &["uplifting"], "agent:tagger");
    stop_service(&s);
    {
        use wreckordbox_sidecar::ledger::{Action, NewEntry, Store};
        let sidecar = wreckordbox_sidecar::Sidecar::open(&s.sidecar_dir()).unwrap();
        let dj = wreckordbox_sidecar::Source::new(wreckordbox_sidecar::SourceKind::Dj, "dj");
        let run = sidecar.begin_run(&dj, "a damaged entry").unwrap();
        let command = json!({"name": "mytags.assign", "args": {"tracks": ["10004"]}});
        let payload = json!({"tags": "not a list"});
        sidecar
            .append(&NewEntry {
                id: None,
                run,
                store: Store::Library,
                source: &dj,
                at: None,
                action: Action::Edit,
                kind: "track_tags",
                label: "Track Edit",
                command: Some(&command),
                reverses: None,
                part_of: None,
                reversible: true,
                format: 1,
                payload: Some(&payload),
            })
            .unwrap();
    }
    let barriers = ok(&s, &["projection", "barriers"]);
    let listed = barriers["barriers"].as_array().unwrap();
    assert_eq!(listed.len(), 1, "{barriers}");
    assert_eq!(listed[0]["what"], "unreadable");
    assert_eq!(listed[0]["tracks"], json!(["10004"]));
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert!(
        listed_has(&dry, "kept", "10004", "Uplifting", "provenance_incomplete"),
        "{dry}"
    );
    let seq = listed[0]["seq"].to_string();
    ok(&s, &["projection", "acknowledge", &seq, "--source", "dj"]);
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(
        changes(&dry),
        [("10004".into(), "Uplifting".into(), "add".into(), false)]
    );
}

/// The round trip's one exception, shown and gated: where most tracks
/// carry a child with its parent, the schema says parents are shown, so a
/// track carrying a child alone gets its parent, listed as an implied
/// change; it waits for the DJ until the policy allows MyTags, then applies.
#[test]
fn converting_then_projecting_adds_a_missing_parent_only_as_a_listed_held_or_allowed_change() {
    let s = Sandbox::new();
    let [_, _, _, sub_genre] = s.four_categories();
    let house = s.create_my_tag_in(&sub_genre, "House");
    let deep = s.create_my_tag_in(&sub_genre, "> Deep");
    let techno = s.create_my_tag_in(&sub_genre, "Techno");
    let hypnotic = s.create_my_tag_in(&sub_genre, "> Hypnotic");
    for (index, tags) in [
        (1, vec![house.as_str(), deep.as_str()]),
        (2, vec![techno.as_str(), hypnotic.as_str()]),
        (3, vec![deep.as_str()]),
    ] {
        s.detail_track(
            index,
            &common::Details {
                my_tags: &tags,
                ..common::Details::default()
            },
        );
    }
    let converted = ok(&s, &["schema", "convert"]);
    assert_eq!(
        converted["implied_parents"],
        json!({"with_parent": 2, "without_parent": 1, "tag_parents": true})
    );
    let held = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(changes(&held), []);
    assert_eq!(
        listed(&held, "held"),
        [row("10003", "House", "add", "waits_for_dj")]
    );
    assert_eq!(held["held"][0]["implied"], true);
    let text = s.cli(&["project", "--mytags", "--dry-run"]);
    assert!(
        text.stdout
            .contains("Held, in this report only (MyTag changes do not reach Review yet): 1."),
        "{}",
        text.stdout
    );
    assert!(!text.stdout.contains("Held for review"), "{}", text.stdout);

    policy(&s, "[mytags]\napply = 0.5\n");
    let allowed = ok(&s, &["project", "--mytags"]);
    assert_eq!(
        changes(&allowed),
        [("10003".into(), "House".into(), "add".into(), true)]
    );
    assert_eq!(
        tags_of(&s, "10003"),
        ["Sub-genre: > Deep", "Sub-genre: House"]
    );
}
