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
