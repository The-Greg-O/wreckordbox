//! MyTag definitions in MyTag Projection (wbx-14.14), through the real CLI
//! and a service on a fixture Library, with a second client watching: the
//! MyTags the Tag schema has and the Library lacks are made, and a deck
//! tag's label and place follow the schema only for the MyTags a
//! projection made itself; everything the DJ made is kept and reported.
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

fn write(s: &Sandbox, file: &str, text: &str) {
    std::fs::create_dir_all(s.sidecar_dir()).unwrap();
    std::fs::write(s.sidecar_dir().join(file), text).unwrap();
}

fn fact(s: &Sandbox, track: &str, kind: &str, values: &[&str]) {
    let mut args = vec!["facts", "record", track, kind];
    args.extend_from_slice(values);
    args.extend_from_slice(&["--source", "agent:tagger", "--confidence", "0.9"]);
    ok(s, &args);
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

/// Each category's tags, by name, in the Library's order.
fn vocabulary(s: &Sandbox) -> Vec<(String, Vec<String>)> {
    ok(s, &["mytags", "list"])["categories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|category| {
            (
                category["name"].as_str().unwrap().to_owned(),
                category["tags"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|tag| tag["name"].as_str().unwrap().to_owned())
                    .collect(),
            )
        })
        .collect()
}

fn tags_in(s: &Sandbox, category: &str) -> Vec<String> {
    vocabulary(s)
        .into_iter()
        .find(|(name, _)| name == category)
        .map(|(_, tags)| tags)
        .unwrap_or_default()
}

/// The id of the MyTag named `name` in `category`.
fn tag_id(s: &Sandbox, category: &str, name: &str) -> String {
    ok(s, &["mytags", "list"])["categories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == category)
        .and_then(|c| {
            c["tags"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["name"] == name)
        })
        .map(|t| t["id"].as_str().unwrap().to_owned())
        .unwrap_or_else(|| panic!("no {category}: {name}"))
}

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

/// `(change, category, label, reason)` of each definition under `key`.
fn definitions(report: &Value, key: &str) -> Vec<(String, String, String, String)> {
    report[key]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|d| {
            (
                d["change"].as_str().unwrap().to_owned(),
                d["category"].as_str().unwrap().to_owned(),
                d["label"].as_str().unwrap().to_owned(),
                d["reason"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect()
}

fn d(change: &str, category: &str, label: &str, reason: &str) -> (String, String, String, String) {
    (change.into(), category.into(), label.into(), reason.into())
}

/// The DJ's MyTags: Mood with Dark and Uplifting, made by hand.
fn by_hand() -> Sandbox {
    let s = Sandbox::new();
    let [_, mood, _sound, _sub_genre] = s.four_categories();
    let dark = s.create_my_tag_in(&mood, "Dark");
    s.create_my_tag_in(&mood, "Uplifting");
    s.detail_track(
        1,
        &common::Details {
            my_tags: &[dark.as_str()],
            ..common::Details::default()
        },
    );
    s
}

/// A Mood dimension with `terms` (`id = name` pairs, in order) and the
/// deck's `order` for it.
fn schema(terms: &[(&str, &str)], order: &[&str]) -> String {
    let terms: String = terms
        .iter()
        .map(|(id, name)| format!("  {{ id = \"{id}\", name = \"{name}\" }},\n"))
        .collect();
    let order = if order.is_empty() {
        String::new()
    } else {
        let ids: Vec<String> = order.iter().map(|id| format!("\"{id}\"")).collect();
        format!(", order = [{}]", ids.join(", "))
    };
    format!(
        "version = 1\n\n[dimensions.mood]\nterms = [\n{terms}]\n\n[deck]\ncategories = [\n  {{ dimension = \"mood\", label = \"Mood\"{order} }},\n]\n"
    )
}

const MOOD: &[(&str, &str)] = &[
    ("dark", "Dark"),
    ("uplifting", "Uplifting"),
    ("euphoric", "Euphoric"),
];

/// A MyTag the schema has and the Library lacks is listed by the dry run
/// (which writes nothing) and made by the apply, with the assignments that
/// use it, in ONE Run, ONE Ledger entry and ONE `library_changed`; then
/// there is nothing left to do.
#[test]
fn projecting_makes_the_mytags_the_schema_has_with_their_assignments_as_one_run() {
    let s = by_hand();
    write(&s, "tag-schema.toml", &schema(MOOD, &[]));
    write(&s, "projection-policy.toml", "[mytags]\napply = 0.5\n");
    fact(&s, "10004", "mood", &["euphoric"]);
    let before = stored_bytes(&s);
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(stored_bytes(&s), before, "a dry run writes nothing");
    assert_eq!(
        definitions(&dry, "definitions"),
        [d("create_tag", "Mood", "Euphoric", "")],
        "{dry}"
    );
    let change = &dry["changes"][0];
    assert_eq!(change["label"], "Euphoric", "{dry}");
    assert_eq!(change["change"], "add");
    assert_eq!(tags_in(&s, "Mood"), ["Dark", "Uplifting"]);

    let runs = ok(&s, &["runs", "list"])["total"].as_i64().unwrap();
    let mut watcher = s.client();
    watcher.subscribe().unwrap();
    let applied = ok(
        &s,
        &[
            "project",
            "--mytags",
            "--expect",
            dry["plan"].as_str().unwrap(),
        ],
    );
    assert_eq!(applied["definitions"], dry["definitions"]);
    assert_eq!(tags_in(&s, "Mood"), ["Dark", "Uplifting", "Euphoric"]);
    assert_eq!(tags_of(&s, "10004"), ["Mood: Euphoric"]);
    assert_eq!(ok(&s, &["runs", "list"])["total"], runs + 1);
    let shown = ok(&s, &["runs", "show", &applied["run"]["id"].to_string()]);
    let entries = shown["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "one Ledger entry: {shown}");
    assert_eq!(entries[0]["reversible"], true);
    let mut heard = Vec::new();
    while let Some(event) = watcher.next_event(Duration::from_millis(500)).unwrap() {
        heard.push(event.event);
    }
    assert_eq!(heard, ["library_changed"]);

    let again = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(again["definitions"], json!([]), "{again}");
    assert_eq!(again["changes"], json!([]), "{again}");
}

/// Definitions are a Library edit like any other: refused under Library
/// Protection with nothing made.
#[test]
fn library_protection_refuses_making_a_mytag() {
    let s = by_hand();
    write(&s, "tag-schema.toml", &schema(MOOD, &[]));
    ok(&s, &["protection", "on"]);
    let error = refused(&s, &["project", "--mytags"]);
    assert_eq!(error["code"], "library_protected", "{error}");
    assert_eq!(tags_in(&s, "Mood"), ["Dark", "Uplifting"]);
}

/// A label changed in the schema renames the MyTag only when a projection
/// made it and nobody wrote it since; the DJ's MyTag of a relabelled term
/// is kept, reported, and nothing is made in its place.
#[test]
fn a_relabelled_term_renames_only_the_projections_own_mytag() {
    let s = by_hand();
    write(&s, "tag-schema.toml", &schema(MOOD, &[]));
    fact(&s, "10004", "mood", &["euphoric"]);
    write(&s, "projection-policy.toml", "[mytags]\napply = 0.5\n");
    ok(&s, &["project", "--mytags"]);
    let euphoric = tag_id(&s, "Mood", "Euphoric");

    write(
        &s,
        "tag-schema.toml",
        &schema(
            &[
                ("dark", "Darkest"),
                ("uplifting", "Uplifting"),
                ("euphoric", "Euphoria"),
            ],
            &[],
        ),
    );
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(
        definitions(&dry, "definitions"),
        [d("rename_tag", "Mood", "Euphoria", "")],
        "{dry}"
    );
    assert_eq!(
        definitions(&dry, "definitions_kept"),
        [d("rename_tag", "Mood", "Darkest", "set_by_dj")],
        "{dry}"
    );
    ok(&s, &["project", "--mytags"]);
    assert_eq!(tags_in(&s, "Mood"), ["Dark", "Uplifting", "Euphoria"]);
    assert_eq!(tag_id(&s, "Mood", "Euphoria"), euphoric, "the same MyTag");
    assert_eq!(tags_of(&s, "10004"), ["Mood: Euphoria"]);
    assert_eq!(tags_of(&s, "10001"), ["Mood: Dark"]);

    // The DJ renames the projection's MyTag: theirs from then on.
    ok(&s, &["mytags", "rename", &euphoric, "Joy"]);
    write(
        &s,
        "tag-schema.toml",
        &schema(
            &[
                ("dark", "Dark"),
                ("uplifting", "Uplifting"),
                ("euphoric", "Bliss"),
            ],
            &[],
        ),
    );
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(definitions(&dry, "definitions"), [], "{dry}");
    assert_eq!(
        definitions(&dry, "definitions_kept"),
        [d("rename_tag", "Mood", "Bliss", "written_since")],
        "{dry}"
    );
    ok(&s, &["project", "--mytags"]);
    assert_eq!(tags_in(&s, "Mood"), ["Dark", "Uplifting", "Joy"]);
}

/// The deck's order places the projection's own MyTags; the DJ's keep
/// their order among themselves, and an order that would move them is kept
/// and reported, as is a deck order of categories the Library does not
/// have.
#[test]
fn the_deck_order_places_only_the_projections_own_mytags() {
    let s = by_hand();
    write(&s, "tag-schema.toml", &schema(MOOD, &["euphoric"]));
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    let listed = definitions(&dry, "definitions");
    assert_eq!(
        listed,
        [
            d("create_tag", "Mood", "Euphoric", ""),
            d("order_tags", "Mood", "", "")
        ],
        "{dry}"
    );
    assert_eq!(
        dry["definitions"][1]["order"],
        json!(["Euphoric", "Dark", "Uplifting"])
    );
    ok(&s, &["project", "--mytags"]);
    assert_eq!(tags_in(&s, "Mood"), ["Euphoric", "Dark", "Uplifting"]);

    // Uplifting before Dark would move the DJ's own: kept.
    write(&s, "tag-schema.toml", &schema(MOOD, &["uplifting"]));
    let before = vocabulary(&s);
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(definitions(&dry, "definitions"), [], "{dry}");
    assert_eq!(
        definitions(&dry, "definitions_kept"),
        [d("order_tags", "Mood", "", "set_by_dj")],
        "{dry}"
    );
    ok(&s, &["project", "--mytags"]);
    assert_eq!(vocabulary(&s), before);
}
