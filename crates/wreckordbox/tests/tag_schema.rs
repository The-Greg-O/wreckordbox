//! `wreckordbox schema check` and `wreckordbox schema convert` against a
//! Library service on a throwaway Library and Sidecar: the real binary, its
//! output and exit status, the Tag schema file it writes, the Facts it
//! records and what a second connected client is told.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod common;

use std::path::PathBuf;

use common::Sandbox;
use serde_json::{json, Value};

/// Runs a command that must succeed and answers its JSON result.
fn ok(sandbox: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let run = sandbox.cli(&all);
    assert!(run.status.success(), "{args:?}: {}", run.stderr);
    assert_eq!(run.stderr, "", "{args:?}");
    run.json()
}

/// Runs a command that must be refused and answers its JSON error.
fn refused(sandbox: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let run = sandbox.cli(&all);
    assert!(!run.status.success(), "{args:?} succeeded: {}", run.stdout);
    assert_eq!(run.stdout, "", "{args:?}");
    run.error_json()["error"].clone()
}

/// Where the Tag schema of the sandbox's Library lives: beside the kinds
/// file, in the Sidecar's folder.
fn schema_file(sandbox: &Sandbox) -> PathBuf {
    sandbox.sidecar_dir().join("tag-schema.toml")
}

/// Puts `text` where the Tag schema lives.
fn write_schema(sandbox: &Sandbox, text: &str) {
    std::fs::create_dir_all(sandbox.sidecar_dir()).unwrap();
    std::fs::write(schema_file(sandbox), text).unwrap();
}

/// What `schema check` says is wrong with `text`: each problem's line and
/// message.
fn problems(sandbox: &Sandbox, text: &str) -> Vec<(Option<u64>, String)> {
    write_schema(sandbox, text);
    let run = sandbox.cli(&["--json", "schema", "check"]);
    assert!(!run.status.success(), "accepted: {}", run.stdout);
    let checked = run.json();
    assert_eq!(checked["valid"], false);
    checked["problems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|problem| {
            (
                problem["line"].as_u64(),
                problem["message"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// The deck view as `[(label, [tag text])]`.
fn deck(checked: &Value) -> Vec<(String, Vec<String>)> {
    checked["deck"]
        .as_array()
        .unwrap()
        .iter()
        .map(|category| {
            (
                category["label"].as_str().unwrap().to_owned(),
                category["tags"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|tag| tag["label"].as_str().unwrap().to_owned())
                    .collect(),
            )
        })
        .collect()
}

const WRITTEN_BY_HAND: &str = r#"version = 1

[genre]
terms = [
  { id = "house", name = "House" },
  { id = "house-melodic", name = "Melodic", parent = "house" },
  { id = "house-acid", name = "Acid", parent = "house" },
  { id = "techno", name = "Techno" },
  { id = "techno-melodic", name = "Melodic", parent = "techno", deck = "> Melodic (T)" },
]

[dimensions.mood]
terms = [
  { id = "dark", name = "Dark" },
  { id = "uplifting", name = "Uplifting" },
]

[dimensions.vocals]
terms = [
  { id = "none", name = "Instrumental" },
  { id = "full", name = "Full vocal" },
]

[dimensions.era]
terms = [
  { id = "nineties", name = "90s" },
]

[deck]
categories = [
  { dimension = "mood", label = "Mood" },
  { dimension = "genre", label = "Sub-genre" },
  { dimension = "vocals", label = "Vox", order = ["full"] },
]

[deck.genre]
child_prefix = "> "
"#;

#[test]
fn a_schema_written_by_hand_is_accepted_and_shown_as_the_deck_would_list_it() {
    let sandbox = Sandbox::new();
    write_schema(&sandbox, WRITTEN_BY_HAND);

    let checked = ok(&sandbox, &["schema", "check"]);

    assert_eq!(checked["valid"], true);
    assert_eq!(checked["problems"], json!([]));
    assert_eq!(checked["file"], schema_file(&sandbox).to_str().unwrap());
    // The same name under two parents is two terms; the deck tells them
    // apart by the label one of them is given there.
    assert_eq!(
        deck(&checked),
        [
            ("Mood".to_owned(), vec!["Dark".to_owned(), "Uplifting".to_owned()]),
            (
                "Sub-genre".to_owned(),
                ["House", "> Melodic", "> Acid", "Techno", "> Melodic (T)"]
                    .map(str::to_owned)
                    .to_vec()
            ),
            // The category's own order first, then the rest as written.
            ("Vox".to_owned(), vec!["Full vocal".to_owned(), "Instrumental".to_owned()]),
        ]
    );
    assert_eq!(checked["deck"][1]["tags"][4]["id"], "techno-melodic");
    // Every dimension is there with its Fact kind, projected or not.
    let dimensions: Vec<(&str, &str, u64, bool)> = checked["dimensions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|dimension| {
            (
                dimension["name"].as_str().unwrap(),
                dimension["kind"].as_str().unwrap(),
                dimension["terms"].as_u64().unwrap(),
                dimension["tree"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        dimensions,
        [
            ("genre", "genre", 5, true),
            ("era", "era", 1, false),
            ("mood", "mood", 2, false),
            ("vocals", "vocals", 2, false),
        ]
    );

    let text = sandbox.cli(&["schema", "check"]);
    assert!(text.status.success(), "{}", text.stderr);
    assert!(
        text.stdout.starts_with(&format!(
            "The Tag schema {} is valid.",
            schema_file(&sandbox).display()
        )),
        "{}",
        text.stdout
    );
    assert!(text.stdout.contains("> Melodic (T)"), "{}", text.stdout);
}
