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
use std::time::Duration;

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
            (
                "Mood".to_owned(),
                vec!["Dark".to_owned(), "Uplifting".to_owned()]
            ),
            (
                "Sub-genre".to_owned(),
                ["House", "> Melodic", "> Acid", "Techno", "> Melodic (T)"]
                    .map(str::to_owned)
                    .to_vec()
            ),
            // The category's own order first, then the rest as written.
            (
                "Vox".to_owned(),
                vec!["Full vocal".to_owned(), "Instrumental".to_owned()]
            ),
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

#[test]
fn a_schema_with_mistakes_is_refused_with_each_mistake_and_its_line() {
    let sandbox = Sandbox::new();

    // A child whose parent is not a term.
    let found = problems(
        &sandbox,
        r#"version = 1

[genre]
terms = [
  { id = "house", name = "House" },
  { id = "garage-speed", name = "Speed", parent = "garage" },
]
"#,
    );
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, Some(6));
    assert!(
        found[0].1.contains("garage-speed") && found[0].1.contains("child with no parent"),
        "{found:?}"
    );

    // Five projected categories: rekordbox has four.
    let found = problems(
        &sandbox,
        r#"version = 1

[dimensions.a]
terms = [{ id = "x", name = "X" }]
[dimensions.b]
terms = [{ id = "x", name = "X" }]
[dimensions.c]
terms = [{ id = "x", name = "X" }]
[dimensions.d]
terms = [{ id = "x", name = "X" }]
[dimensions.e]
terms = [{ id = "x", name = "X" }]

[deck]
categories = [
  { dimension = "a", label = "A" },
  { dimension = "b", label = "B" },
  { dimension = "c", label = "C" },
  { dimension = "d", label = "D" },
  { dimension = "e", label = "E" },
]
"#,
    );
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, Some(20));
    assert!(
        found[0].1.contains("5 categories") && found[0].1.contains("at most 4"),
        "{found:?}"
    );

    // The same name twice in one category; the same name under two parents
    // of the tree is refused only while the deck would show it twice.
    let found = problems(
        &sandbox,
        r#"version = 1

[genre]
terms = [
  { id = "house", name = "House" },
  { id = "house-deep", name = "Deep", parent = "house" },
  { id = "techno", name = "Techno" },
  { id = "techno-deep", name = "Deep", parent = "techno" },
]

[dimensions.mood]
terms = [
  { id = "dark", name = "Dark" },
  { id = "dark-2", name = "Dark" },
]

[deck]
categories = [
  { dimension = "genre", label = "Sub-genre" },
  { dimension = "mood", label = "Mood" },
]
"#,
    );
    let lines: Vec<Option<u64>> = found.iter().map(|(line, _)| *line).collect();
    assert_eq!(lines, [Some(8), Some(14), Some(14)], "{found:?}");
    assert!(
        found[0].1.contains("two MyTags called \"> Deep\"")
            && found[0].1.contains("house-deep")
            && found[0].1.contains("techno-deep"),
        "{found:?}"
    );
    assert!(
        found[1]
            .1
            .contains("mood: two terms in the dimension are called \"Dark\": dark and dark-2"),
        "{found:?}"
    );

    // Other mistakes a hand makes: an identifier twice, a key that is not
    // one, and text that is not TOML at all.
    let found = problems(
        &sandbox,
        "version = 1\n\n[dimensions.mood]\nterms = [\n  { id = \"dark\", name = \"Dark\" },\n  { id = \"dark\", name = \"Darker\" },\n]\n",
    );
    assert_eq!(found[0].0, Some(6));
    assert!(
        found[0].1.contains("two terms have the id dark"),
        "{found:?}"
    );
    let found = problems(
        &sandbox,
        "version = 1\n\n[dimensions.mood]\nterms = [\n  { id = \"dark\", nme = \"Dark\" },\n]\n",
    );
    assert_eq!(found[0].0, Some(5), "{found:?}");
    let found = problems(&sandbox, "version = 1\n\n[genre\n");
    assert_eq!(found[0].0, Some(3), "{found:?}");

    // By hand: the mistakes one to a line, and a failing exit status.
    let text = sandbox.cli(&["schema", "check"]);
    assert!(!text.status.success());
    assert!(
        text.stdout.contains("cannot be used: 1 problem"),
        "{}",
        text.stdout
    );
    assert!(text.stdout.contains("\n  line 3: "), "{}", text.stdout);

    // A Tag schema that cannot be used stops Facts being recorded until it
    // is fixed, as a kinds file with a mistake does.
    assert_eq!(
        refused(
            &sandbox,
            &["facts", "record", "10001", "energy", "3", "--source", "dj"]
        )["code"],
        "tag_schema_invalid"
    );
}

#[test]
fn checking_where_there_is_no_schema_says_so() {
    let sandbox = Sandbox::new();
    let error = refused(&sandbox, &["schema", "check"]);
    assert_eq!(error["code"], "tag_schema_missing");
    assert!(error["message"]
        .as_str()
        .unwrap()
        .contains("schema convert"));
}

#[test]
fn a_valid_schema_makes_its_terms_the_only_values_of_its_kinds() {
    let sandbox = Sandbox::new();
    write_schema(&sandbox, WRITTEN_BY_HAND);

    let record = |args: &[&str]| {
        let mut all = vec!["--json", "facts", "record", "10001"];
        all.extend_from_slice(args);
        all.extend_from_slice(&["--source", "dj"]);
        sandbox.cli(&all)
    };
    // A term's identifier is a value; its name, or anything else, is not.
    assert!(record(&["genre", "techno-melodic"]).status.success());
    assert!(!record(&["genre", "Melodic"]).status.success());
    // A dimension the code has never heard of is a kind of Fact.
    assert!(record(&["era", "nineties"]).status.success());
    assert!(!record(&["era", "eighties"]).status.success());
}

/// A Library with four MyTag categories as a DJ keeps them, every name made
/// up: a set-role category, a mood category, a mixed one, and a sub-genre
/// category that is a tree written as a flat list.
struct Deck {
    sandbox: Sandbox,
    sub_genre: String,
    /// The Library's categories as `mytags list` gives them.
    there: Vec<(String, Vec<String>)>,
}

const SUB_GENRES: [&str; 10] = [
    "House",
    "> Deep",
    ">  Acid",
    "> Tribal",
    "Techno",
    "> Deep (T)",
    "> Hypnotic",
    "-----",
    "Breaks",
    "> Nu Skool",
];

fn my_tags_there(sandbox: &Sandbox) -> Vec<(String, Vec<String>)> {
    ok(sandbox, &["mytags", "list"])["categories"]
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

fn converted_deck() -> Deck {
    let sandbox = Sandbox::new();
    let [_, mood, sound, sub_genre] = sandbox.four_categories();
    let dark = sandbox.create_my_tag_in(&mood, "Dark");
    let uplifting = sandbox.create_my_tag_in(&mood, "Uplifting");
    let vocal = sandbox.create_my_tag_in(&sound, "Vocal");
    sandbox.create_my_tag_in(&sound, "Dub Mix");
    let genres: Vec<String> = SUB_GENRES
        .iter()
        .map(|name| sandbox.create_my_tag_in(&sub_genre, name))
        .collect();
    let genre = |name: &str| genres[SUB_GENRES.iter().position(|at| *at == name).unwrap()].as_str();
    let peak = rbl_db::fixture::MY_TAG_PEAK;
    for (index, tags) in [
        // A child with its parent put on by hand, as most tracks have it.
        (
            1,
            vec![peak, dark.as_str(), genre("House"), genre("> Deep")],
        ),
        // A child without its parent.
        (2, vec![genre("> Deep (T)")]),
        // Two children of two parents, one parent put on.
        (
            3,
            vec![
                genre("Techno"),
                genre("> Hypnotic"),
                genre(">  Acid"),
                vocal.as_str(),
            ],
        ),
        // A parent alone.
        (4, vec![genre("House")]),
        // The entry that is not a tag, and two moods.
        (5, vec![genre("-----"), dark.as_str(), uplifting.as_str()]),
        (6, vec![genre("Breaks"), genre("> Nu Skool")]),
    ] {
        sandbox.detail_track(
            index,
            &common::Details {
                my_tags: &tags,
                ..common::Details::default()
            },
        );
    }
    let there = my_tags_there(&sandbox);
    Deck {
        sandbox,
        sub_genre,
        there,
    }
}

/// The DJ's live Facts about a track as `(kind, value)`, by kind.
fn dj_facts(sandbox: &Sandbox, track: &str) -> Vec<(String, Value)> {
    let mut facts: Vec<(String, Value)> = ok(sandbox, &["facts", "show", track])["facts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|fact| {
            assert_eq!(fact["source"], json!({"kind": "dj", "name": "dj"}));
            assert_eq!(fact["confidence"], 1.0);
            (
                fact["kind"].as_str().unwrap().to_owned(),
                fact["value"].clone(),
            )
        })
        .collect();
    facts.sort_by(|a, b| a.0.cmp(&b.0));
    facts
}

#[test]
fn conversion_writes_a_schema_whose_deck_view_is_the_mytags_there_are() {
    let Deck { sandbox, there, .. } = converted_deck();
    assert_eq!(there[3].0, "Sub-genre");
    assert_eq!(there[3].1, SUB_GENRES);

    let converted = ok(&sandbox, &["schema", "convert"]);

    assert_eq!(converted["file"], schema_file(&sandbox).to_str().unwrap());
    assert_eq!(converted["file_state"], "written");
    // The file is valid, and the deck it makes is the Library's categories,
    // labels, tags and order, to the character.
    let checked = ok(&sandbox, &["schema", "check"]);
    assert_eq!(checked["valid"], true);
    assert_eq!(deck(&checked), there);

    // The tree was found by its tags' prefixes, and from the list's order:
    // a tag with no prefix starts a group, the prefixed ones after it are
    // its children, whatever the spacing after the prefix.
    assert_eq!(
        converted["tree"],
        json!({"category": "Sub-genre", "told": false, "parents": 4, "children": 6, "child_prefix": "> "})
    );
    let text = std::fs::read_to_string(schema_file(&sandbox)).unwrap();
    let genre_lines: Vec<&str> = text
        .lines()
        .skip_while(|line| *line != "[genre]")
        .skip(2)
        .take_while(|line| *line != "]")
        .collect();
    assert_eq!(
        genre_lines,
        [
            r#"  { id = "house", name = "House" },"#,
            r#"  { id = "house-deep", name = "Deep", parent = "house" },"#,
            r#"  { id = "house-acid", name = "Acid", parent = "house", deck = ">  Acid" },"#,
            r#"  { id = "house-tribal", name = "Tribal", parent = "house" },"#,
            r#"  { id = "techno", name = "Techno" },"#,
            r#"  { id = "techno-deep", name = "Deep", parent = "techno", deck = "> Deep (T)" },"#,
            r#"  { id = "techno-hypnotic", name = "Hypnotic", parent = "techno" },"#,
            r#"  { id = "term", name = "-----" }, # odd: it has no letter or digit, so it looks like a divider, not a tag"#,
            r#"  { id = "breaks", name = "Breaks" },"#,
            r#"  { id = "breaks-nu-skool", name = "Nu Skool", parent = "breaks" },"#,
        ]
    );
    // The other categories are dimensions as they are, each a kind of Fact.
    let categories: Vec<(&str, &str, bool)> = converted["categories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|category| {
            (
                category["category"].as_str().unwrap(),
                category["dimension"].as_str().unwrap(),
                category["tree"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        categories,
        [
            ("Situation", "situation", false),
            ("Mood", "mood", false),
            ("Sound", "sound", false),
            ("Sub-genre", "genre", true),
        ]
    );

    // The same name under two parents: the hand-written marker is the
    // deck's text, not the term's name.
    assert_eq!(
        converted["disambiguated"],
        json!([{"id": "techno-deep", "name": "Deep", "parent": "techno", "tag": "> Deep (T)"}])
    );
    // What is not a tag is kept as a term, and pointed at.
    let odd = converted["odd"].as_array().unwrap();
    assert_eq!(odd.len(), 1, "{odd:?}");
    assert_eq!(odd[0]["tag"], "-----");
    assert_eq!(odd[0]["id"], "term");
    assert_eq!(odd[0]["category"], "Sub-genre");
    assert_eq!(odd[0]["tracks"], 1);

    // By hand, the same is said in words.
    let again = sandbox.cli(&["schema", "convert"]);
    assert!(again.status.success(), "{}", again.stderr);
    for said in [
        "is already what this conversion writes",
        "Sub-genre  ->  genre (10 terms, the genre tree)",
        "\"> Deep (T)\" is Deep under techno (techno-deep)",
        "1 odd entry kept as a term",
        "No Fact to record",
    ] {
        assert!(again.stdout.contains(said), "{said}: {}", again.stdout);
    }
}

#[test]
fn every_tag_on_a_track_becomes_a_fact_from_the_dj_in_one_run_and_a_child_implies_its_parent() {
    let Deck { sandbox, .. } = converted_deck();
    let mut watcher = sandbox.client();
    watcher.subscribe().unwrap();

    let converted = ok(&sandbox, &["schema", "convert"]);

    // One Fact per track and dimension, its value the terms' identifiers.
    // In the tree the Fact names the most specific terms: a parent put on
    // by hand beside its child is implied by the child, not recorded.
    assert_eq!(
        dj_facts(&sandbox, "10001"),
        [
            ("genre".to_owned(), json!(["house-deep"])),
            ("mood".to_owned(), json!(["dark"])),
            ("situation".to_owned(), json!(["peak"])),
        ]
    );
    assert_eq!(
        dj_facts(&sandbox, "10002"),
        [("genre".to_owned(), json!(["techno-deep"]))]
    );
    assert_eq!(
        dj_facts(&sandbox, "10003"),
        [
            ("genre".to_owned(), json!(["house-acid", "techno-hypnotic"])),
            ("sound".to_owned(), json!(["vocal"])),
        ]
    );
    // A parent alone is itself the most specific term.
    assert_eq!(
        dj_facts(&sandbox, "10004"),
        [("genre".to_owned(), json!(["house"]))]
    );
    assert_eq!(
        dj_facts(&sandbox, "10005"),
        [
            ("genre".to_owned(), json!(["term"])),
            ("mood".to_owned(), json!(["dark", "uplifting"])),
        ]
    );
    assert_eq!(dj_facts(&sandbox, "10007"), []);

    // Whether the deck shows a child's parent too is the schema's to say,
    // set from what most tracks have today.
    assert_eq!(
        converted["implied_parents"],
        json!({"with_parent": 3, "without_parent": 2, "tag_parents": true})
    );
    let text = std::fs::read_to_string(schema_file(&sandbox)).unwrap();
    assert!(text.contains("\ntag_parents = true\n"), "{text}");

    // Every one in the same Run, the DJ's, finished.
    assert_eq!(converted["tracks"], 6);
    assert_eq!(converted["track_tags"], 15);
    assert_eq!(converted["facts_recorded"], 10);
    let run = &converted["run"];
    assert_eq!(run["source"], json!({"kind": "dj", "name": "dj"}));
    assert_eq!(run["description"], "Convert the Library's MyTags to Facts");
    assert_eq!(run["outcome"], "recorded 10 Facts");
    assert!(run["finished_at"].is_i64());
    for track in ["10001", "10002", "10003", "10004", "10005", "10006"] {
        for fact in ok(&sandbox, &["facts", "show", track])["facts"]
            .as_array()
            .unwrap()
        {
            assert_eq!(fact["run"], run["id"], "{fact}");
        }
    }

    // Other clients hear of the Sidecar, never of the Library.
    let mut heard = Vec::new();
    while let Some(event) = watcher.next_event(Duration::from_millis(500)).unwrap() {
        heard.push(event.event);
    }
    assert_eq!(heard, vec!["sidecar_changed"; 10]);
}

/// The schema of the Library's database and how many rows each table has.
fn library_shape(sandbox: &Sandbox) -> Vec<(String, String, i64)> {
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
    shape
}

#[test]
fn conversion_changes_nothing_in_the_library() {
    let Deck { sandbox, there, .. } = converted_deck();
    let shape = library_shape(&sandbox);
    let bytes = std::fs::read(sandbox.master_db()).unwrap();
    let generation = ok(&sandbox, &["status"])["generation"].clone();
    let track = ok(&sandbox, &["mytags", "show", "10001"]);

    ok(&sandbox, &["schema", "convert"]);
    ok(&sandbox, &["schema", "convert", "--replace"]);

    assert_eq!(my_tags_there(&sandbox), there);
    assert_eq!(ok(&sandbox, &["mytags", "show", "10001"]), track);
    assert_eq!(ok(&sandbox, &["status"])["generation"], generation);
    assert_eq!(library_shape(&sandbox), shape);
    assert!(
        std::fs::read(sandbox.master_db()).unwrap() == bytes,
        "master.db changed"
    );
    // What it wrote is in the Sidecar's folder, which is not the Library's.
    assert!(!sandbox
        .sidecar_dir()
        .starts_with(sandbox.master_db().parent().unwrap()));
}

#[test]
fn converting_twice_records_nothing_twice_and_an_edited_schema_is_not_written_over() {
    let Deck { sandbox, .. } = converted_deck();
    let first = ok(&sandbox, &["schema", "convert"]);
    let text = std::fs::read_to_string(schema_file(&sandbox)).unwrap();
    let all = |track: &str| {
        ok(&sandbox, &["facts", "show", track, "--all"])["facts"]
            .as_array()
            .unwrap()
            .len()
    };
    assert_eq!(all("10001"), 3);

    // Again: the file is what it would write, and every Fact is there.
    let second = ok(&sandbox, &["schema", "convert"]);
    assert_eq!(second["file_state"], "unchanged");
    assert_eq!(second["facts_recorded"], 0);
    assert_eq!(second["facts_unchanged"], first["facts_recorded"]);
    assert_eq!(second["run"], Value::Null);
    assert_eq!(all("10001"), 3);

    // The DJ edits the schema: renames a term, keeping its identifier.
    let edited = text.replace(r#"name = "Tribal""#, r#"name = "Tribal & Afro""#);
    assert_ne!(edited, text);
    write_schema(&sandbox, &edited);
    assert_eq!(ok(&sandbox, &["schema", "check"])["valid"], true);

    let error = refused(&sandbox, &["schema", "convert"]);
    assert_eq!(error["code"], "tag_schema_exists");
    assert!(error["message"].as_str().unwrap().contains("--replace"));
    assert_eq!(
        std::fs::read_to_string(schema_file(&sandbox)).unwrap(),
        edited
    );
    assert_eq!(all("10001"), 3);

    // A dry run shows what it would write and touches nothing.
    let dry = ok(&sandbox, &["schema", "convert", "--dry-run"]);
    assert_eq!(dry["file_state"], "not_written");
    assert_eq!(dry["text"], text);
    assert_eq!(
        std::fs::read_to_string(schema_file(&sandbox)).unwrap(),
        edited
    );

    // Told to, it writes over the edit; still no Fact twice.
    let replaced = ok(&sandbox, &["schema", "convert", "--replace"]);
    assert_eq!(replaced["file_state"], "replaced");
    assert_eq!(replaced["facts_recorded"], 0);
    assert_eq!(
        std::fs::read_to_string(schema_file(&sandbox)).unwrap(),
        text
    );
    assert_eq!(all("10001"), 3);
}

#[test]
fn the_tree_is_the_category_the_dj_names_when_told() {
    let Deck {
        sandbox,
        sub_genre,
        there,
    } = converted_deck();

    // No tree: the sub-genre list is a dimension like the others, each tag
    // a term with its text as it is.
    let flat = ok(
        &sandbox,
        &["schema", "convert", "--tree", "none", "--dry-run"],
    );
    assert_eq!(flat["tree"], Value::Null);
    assert!(!flat["text"].as_str().unwrap().contains("[genre]"));

    // Another category as the tree, by name; the sub-genre list by id.
    let mood = ok(
        &sandbox,
        &["schema", "convert", "--tree", "Mood", "--dry-run"],
    );
    assert_eq!(mood["tree"]["category"], "Mood");
    assert_eq!(mood["tree"]["told"], true);
    assert_eq!(mood["tree"]["children"], 0);
    let told = ok(&sandbox, &["schema", "convert", "--tree", &sub_genre]);
    assert_eq!(told["tree"]["category"], "Sub-genre");
    assert_eq!(told["tree"]["told"], true);
    assert_eq!(deck(&ok(&sandbox, &["schema", "check"])), there);

    assert_eq!(
        refused(&sandbox, &["schema", "convert", "--tree", "Nonesuch"])["code"],
        "my_tag_category_not_found"
    );

    // A dimension is called what the DJ says, and its Facts are that kind.
    let named = ok(
        &sandbox,
        &[
            "schema",
            "convert",
            "--name",
            "Situation=set_role",
            "--replace",
        ],
    );
    assert_eq!(named["categories"][0]["dimension"], "set_role");
    assert_eq!(dj_facts(&sandbox, "10001")[2].0, "set_role");
}
