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
    assert_eq!(
        dj_facts(&sandbox, "10006"),
        [("genre".to_owned(), json!(["breaks-nu-skool"]))]
    );
    // A track with no tag has no Fact.
    for track in ["10007", "10008", "10000"] {
        assert_eq!(dj_facts(&sandbox, track), [], "{track}");
    }

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

/// Runs a command with `input` on its standard input.
fn with_input(sandbox: &Sandbox, args: &[&str], input: &str) -> common::Run {
    use std::io::Write as _;
    let mut child = sandbox
        .command()
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    common::Run {
        status: output.status,
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

/// Records one Fact from the DJ; the run, so a refusal can be looked at.
fn dj_records(sandbox: &Sandbox, track: &str, kind: &str, values: &[&str]) -> common::Run {
    let mut all = vec!["--json", "facts", "record", track, kind];
    all.extend_from_slice(values);
    all.extend_from_slice(&["--source", "dj"]);
    sandbox.cli(&all)
}

/// The code a refused run reports.
fn code(run: &common::Run) -> String {
    assert!(!run.status.success(), "succeeded: {}", run.stdout);
    run.error_json()["error"]["code"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Every file in the Sidecar's folder whose name starts like the Tag
/// schema's.
fn schema_files(sandbox: &Sandbox) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(sandbox.sidecar_dir())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.starts_with("tag-schema"))
        .collect();
    names.sort();
    names
}

#[test]
fn converting_again_never_overwrites_what_the_dj_has_said_since_unless_told_to_reimport() {
    let Deck { sandbox, .. } = converted_deck();
    ok(&sandbox, &["schema", "convert"]);

    // The DJ corrects a Fact, and says something of a track with no MyTag.
    assert!(dj_records(&sandbox, "10001", "genre", &["house-acid"])
        .status
        .success());
    assert!(dj_records(&sandbox, "10007", "mood", &["dark"])
        .status
        .success());

    let again = ok(&sandbox, &["schema", "convert"]);
    assert_eq!(again["facts_recorded"], 0);
    assert_eq!(again["facts_unchanged"], 9);
    assert_eq!(again["facts_conflicting"], 2);
    assert_eq!(again["run"], Value::Null);
    assert_eq!(
        again["conflicts"],
        json!([
            {"track_id": "10001", "dimension": "genre", "fact": ["house-acid"], "library": ["house-deep"]},
            {"track_id": "10007", "dimension": "mood", "fact": ["dark"], "library": []},
        ])
    );
    // What the DJ said stands, live.
    assert_eq!(
        dj_facts(&sandbox, "10001")[0],
        ("genre".to_owned(), json!(["house-acid"]))
    );
    assert_eq!(
        dj_facts(&sandbox, "10007"),
        [("mood".to_owned(), json!(["dark"]))]
    );
    // A dry run says the same, and by hand the way out is named.
    let dry = ok(&sandbox, &["schema", "convert", "--dry-run"]);
    assert_eq!(dry["facts_conflicting"], 2);
    assert_eq!(dry["facts_recorded"], 0);
    let text = sandbox.cli(&["schema", "convert"]);
    assert!(
        text.stdout.contains("2 Facts from the DJ say otherwise"),
        "{}",
        text.stdout
    );
    assert!(text.stdout.contains("--reimport"), "{}", text.stdout);
    assert!(text.stdout.contains("10001  genre"), "{}", text.stdout);

    // Told to import again, the Library's MyTags are the DJ's word once
    // more; the corrections are superseded, not lost.
    let dry = ok(&sandbox, &["schema", "convert", "--reimport", "--dry-run"]);
    assert_eq!(dry["facts_recorded"], 2);
    assert_eq!(dj_facts(&sandbox, "10001")[0].1, json!(["house-acid"]));
    let reimported = ok(&sandbox, &["schema", "convert", "--reimport"]);
    assert_eq!(reimported["facts_recorded"], 2);
    assert_eq!(reimported["facts_conflicting"], 0);
    assert_eq!(reimported["conflicts"], json!([]));
    assert_eq!(
        dj_facts(&sandbox, "10001")[0],
        ("genre".to_owned(), json!(["house-deep"]))
    );
    assert_eq!(
        dj_facts(&sandbox, "10007"),
        [("mood".to_owned(), json!([]))]
    );
    let history = ok(
        &sandbox,
        &["facts", "show", "10001", "--kind", "genre", "--all"],
    );
    let values: Vec<&Value> = history["facts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|fact| &fact["value"])
        .collect();
    assert_eq!(
        values,
        [
            &json!(["house-deep"]),
            &json!(["house-acid"]),
            &json!(["house-deep"])
        ]
    );
}

#[test]
fn a_first_conversion_leaves_the_facts_the_dj_already_recorded_as_they_are() {
    let Deck { sandbox, .. } = converted_deck();
    // Before there is a schema, any text is a mood.
    assert!(dj_records(&sandbox, "10007", "mood", &["Wistful"])
        .status
        .success());
    assert!(dj_records(&sandbox, "10001", "mood", &["Brooding", "Dark"])
        .status
        .success());

    let converted = ok(&sandbox, &["schema", "convert"]);

    assert_eq!(converted["facts_recorded"], 9);
    assert_eq!(converted["facts_conflicting"], 2);
    assert_eq!(
        converted["conflicts"],
        json!([
            {"track_id": "10001", "dimension": "mood", "fact": ["Brooding", "Dark"], "library": ["dark"]},
            {"track_id": "10007", "dimension": "mood", "fact": ["Wistful"], "library": []},
        ])
    );
    assert_eq!(
        dj_facts(&sandbox, "10007"),
        [("mood".to_owned(), json!(["Wistful"]))]
    );
    assert_eq!(
        dj_facts(&sandbox, "10001")[1],
        ("mood".to_owned(), json!(["Brooding", "Dark"]))
    );
    assert_eq!(
        dj_facts(&sandbox, "10001")[0],
        ("genre".to_owned(), json!(["house-deep"]))
    );
}

#[test]
fn a_genre_fact_recorded_as_one_text_before_genre_was_a_list_reads_as_a_list_of_one() {
    use wreckordbox_sidecar::{Kind, NewFact, Sidecar, Source, SourceKind};
    let Deck { sandbox, .. } = converted_deck();
    // A store from before: genre Facts are single texts.
    let (dj_fact, agent_fact, run) = {
        let store = Sidecar::open(&sandbox.sidecar_dir()).unwrap();
        let dj = Source::new(SourceKind::Dj, "dj");
        let agent = Source::new(SourceKind::Agent, "tagger");
        let run = store.begin_run(&agent, "An earlier pass").unwrap();
        let record = |track: &str, value: &str, source: &Source, confidence: f64| {
            store
                .record(&NewFact {
                    track_id: track,
                    fingerprint: None,
                    kind: Kind::judgement("genre"),
                    value: json!(value),
                    source,
                    confidence,
                    run,
                })
                .unwrap()
        };
        let first = record("10007", "Old Techno", &dj, 1.0);
        let second = record("10007", "Techno", &dj, 1.0);
        assert_eq!(
            store.fact(first.id).unwrap().unwrap().superseded_by,
            Some(second.id)
        );
        let agent_fact = record("10007", "Hard Techno", &agent, 0.7);
        store.finish_run(run, "done").unwrap();
        (second, agent_fact, run)
    };

    // Read: a list of one, with who said it, how surely, when and in which
    // Run exactly as stored, and the history intact.
    let shown = ok(
        &sandbox,
        &["facts", "show", "10007", "--kind", "genre", "--all"],
    );
    let facts = shown["facts"].as_array().unwrap();
    assert_eq!(facts.len(), 3);
    let by_id = |id: i64| facts.iter().find(|fact| fact["id"] == id).unwrap();
    let read = by_id(dj_fact.id);
    assert_eq!(read["value"], json!(["Techno"]));
    assert_eq!(read["source"], json!({"kind": "dj", "name": "dj"}));
    assert_eq!(read["run"], run.0);
    assert_eq!(read["recorded_at"], dj_fact.recorded_at);
    assert_eq!(read["superseded"], false);
    let read = by_id(agent_fact.id);
    assert_eq!(read["value"], json!(["Hard Techno"]));
    assert_eq!(read["confidence"], 0.7);
    assert_eq!(read["source"], json!({"kind": "agent", "name": "tagger"}));
    let old = facts
        .iter()
        .find(|fact| fact["superseded"] == true)
        .unwrap();
    assert_eq!(old["value"], json!(["Old Techno"]));
    assert_eq!(old["superseded_by"], dj_fact.id);
    // Without the filter too.
    assert_eq!(dj_facts(&sandbox, "10008"), []);
    let all = ok(&sandbox, &["facts", "show", "10007"]);
    for fact in all["facts"].as_array().unwrap() {
        assert!(fact["value"].is_array(), "{fact}");
    }

    // Written the old way, one text is still taken, as a list of one.
    let many = with_input(
        &sandbox,
        &[
            "--json",
            "facts",
            "record-many",
            "--source",
            "agent:tagger",
            "--confidence",
            "0.5",
        ],
        "{\"track\": \"10008\", \"kind\": \"genre\", \"value\": \"Electro\"}\n",
    );
    assert!(many.status.success(), "{}", many.stderr);
    assert_eq!(many.json()["facts"][0]["value"], json!(["Electro"]));

    // The conversion sees the old Fact for what it says: the DJ's word on
    // a track with no MyTag, left alone and reported.
    let converted = ok(&sandbox, &["schema", "convert"]);
    assert_eq!(
        converted["conflicts"],
        json!([{"track_id": "10007", "dimension": "genre", "fact": ["Techno"], "library": []}])
    );
    let after = ok(&sandbox, &["facts", "show", "10007", "--kind", "genre"]);
    assert_eq!(after["facts"].as_array().unwrap().len(), 2);
}

#[test]
fn a_kind_with_its_own_fixed_values_cannot_be_a_dimension() {
    let sandbox = Sandbox::new();
    let found = problems(
        &sandbox,
        "version = 1\n\n[dimensions.prep_colour]\nterms = [{ id = \"banana\", name = \"Banana\" }]\n\n[dimensions.energy]\nterms = [{ id = \"high\", name = \"High\" }]\n",
    );
    let lines: Vec<Option<u64>> = found.iter().map(|(line, _)| *line).collect();
    assert_eq!(lines, [Some(3), Some(6)], "{found:?}");
    assert!(
        found[0].1.contains("prep_colour")
            && found[0].1.contains("Call the dimension something else"),
        "{found:?}"
    );
    assert!(found[1].1.contains("energy"), "{found:?}");
    // Prep colour's values are still the colours, never the schema's.
    assert_eq!(
        code(&dj_records(&sandbox, "10001", "prep_colour", &["banana"])),
        "tag_schema_invalid"
    );
    write_schema(
        &sandbox,
        "version = 1\n\n[dimensions.mood]\nterms = [{ id = \"dark\", name = \"Dark\" }]\n",
    );
    assert_eq!(
        code(&dj_records(&sandbox, "10001", "prep_colour", &["banana"])),
        "wrong_value_type"
    );
    assert!(dj_records(&sandbox, "10001", "prep_colour", &["Green"])
        .status
        .success());

    // So is a kind of the kinds file that has choices of its own.
    std::fs::write(
        sandbox.sidecar_dir().join("kinds.toml"),
        "[kinds.era]\nnature = \"judgement\"\nvalue = \"text\"\nchoices = [\"90s\", \"00s\"]\n",
    )
    .unwrap();
    let found = problems(
        &sandbox,
        "version = 1\n\n[dimensions.era]\nterms = [{ id = \"nineties\", name = \"90s\" }]\n",
    );
    assert_eq!(found[0].0, Some(3), "{found:?}");
}

#[test]
fn a_dimension_with_no_terms_takes_no_term() {
    let sandbox = Sandbox::new();
    write_schema(
        &sandbox,
        "version = 1\n\n[dimensions.mood]\nterms = []\n\n[dimensions.era]\n",
    );
    assert_eq!(ok(&sandbox, &["schema", "check"])["valid"], true);
    for kind in ["mood", "era"] {
        let refused = dj_records(&sandbox, "10001", kind, &["invented"]);
        assert_eq!(code(&refused), "wrong_value_type", "{kind}");
        let none = with_input(
            &sandbox,
            &["--json", "facts", "record-many", "--source", "dj"],
            &format!("{{\"track\": \"10001\", \"kind\": \"{kind}\", \"value\": []}}\n"),
        );
        assert!(none.status.success(), "{kind}: {}", none.stderr);
    }

    // A category the conversion finds empty is such a dimension.
    let sandbox = Sandbox::new();
    sandbox.four_categories();
    ok(&sandbox, &["schema", "convert"]);
    assert_eq!(
        code(&dj_records(&sandbox, "10001", "mood", &["invented"])),
        "wrong_value_type"
    );
    assert!(dj_records(&sandbox, "10001", "situation", &["peak"])
        .status
        .success());
}

#[test]
fn a_conversion_that_cannot_record_its_facts_writes_nothing() {
    use wreckordbox_sidecar::{
        fingerprint_file, Kind, NewFact, RunId, Sidecar, Source, SourceKind,
    };
    let Deck { sandbox, .. } = converted_deck();

    // A kind that holds one text cannot hold a category's several tags:
    // refused before anything is written, in a dry run too.
    for extra in [&[][..], &["--dry-run"][..]] {
        let mut args = vec!["schema", "convert", "--name", "Sound=vocals"];
        args.extend_from_slice(extra);
        let error = refused(&sandbox, &args);
        assert_eq!(error["code"], "bad_request", "{error}");
        assert!(
            error["message"].as_str().unwrap().contains("vocals"),
            "{error}"
        );
    }
    assert_eq!(schema_files(&sandbox), Vec::<String>::new());
    assert_eq!(dj_facts(&sandbox, "10001"), []);

    // The store refuses the Facts themselves: it knows `set_role` as a
    // Measurement from before, and a dimension's Facts are Judgements.
    let seeded = {
        let store = Sidecar::open(&sandbox.sidecar_dir()).unwrap();
        let tool = Source::new(SourceKind::Tool, "old");
        let run = store.begin_run(&tool, "Before").unwrap();
        let fingerprint = fingerprint_file(sandbox.master_db()).unwrap();
        store
            .record(&NewFact {
                track_id: "10009",
                fingerprint: Some(&fingerprint),
                kind: Kind::measurement("set_role"),
                value: json!(["x"]),
                source: &tool,
                confidence: 1.0,
                run,
            })
            .unwrap();
        store.finish_run(run, "done").unwrap();
        run
    };
    let error = refused(
        &sandbox,
        &["schema", "convert", "--name", "Situation=set_role"],
    );
    assert_eq!(error["code"], "sidecar_failed", "{error}");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("no Tag schema was written"),
        "{error}"
    );
    // No schema is left behind, nor a working file, nor a Fact.
    assert_eq!(schema_files(&sandbox), Vec::<String>::new());
    assert_eq!(dj_facts(&sandbox, "10001"), []);
    assert_eq!(
        code(&sandbox.cli(&["--json", "schema", "check"])),
        "tag_schema_missing"
    );

    // Where there was a schema, it is the one there again.
    let earlier = ok(&sandbox, &["schema", "convert", "--dry-run"])["text"]
        .as_str()
        .unwrap()
        .replace("name = \"Tribal\"", "name = \"Tribal & Afro\"");
    write_schema(&sandbox, &earlier);
    let error = refused(
        &sandbox,
        &[
            "schema",
            "convert",
            "--name",
            "Situation=set_role",
            "--replace",
        ],
    );
    assert_eq!(error["code"], "sidecar_failed", "{error}");
    assert!(
        error["message"].as_str().unwrap().contains("as it was"),
        "{error}"
    );
    assert_eq!(
        std::fs::read_to_string(schema_file(&sandbox)).unwrap(),
        earlier
    );
    assert_eq!(schema_files(&sandbox), ["tag-schema.toml"]);
    assert_eq!(dj_facts(&sandbox, "10001"), []);

    // Every Run the failed conversions began is finished.
    let store = Sidecar::open(&sandbox.sidecar_dir()).unwrap();
    let mut began = 0;
    for id in (seeded.0 + 1).. {
        let Some(run) = store.run(RunId(id)).unwrap() else {
            break;
        };
        began += 1;
        assert!(run.is_finished(), "{run:?}");
        assert!(run.outcome.unwrap().starts_with("nothing recorded"));
    }
    assert_eq!(began, 2);
}

#[test]
fn two_conversions_at_once_are_one_after_the_other() {
    for _ in 0..4 {
        let Deck { sandbox, .. } = converted_deck();
        // One service for both.
        ok(&sandbox, &["status"]);
        let start = std::sync::Barrier::new(2);
        let (plain, named) = std::thread::scope(|scope| {
            let plain = scope.spawn(|| {
                start.wait();
                sandbox.cli(&["--json", "schema", "convert"])
            });
            let named = scope.spawn(|| {
                start.wait();
                sandbox.cli(&[
                    "--json",
                    "schema",
                    "convert",
                    "--name",
                    "Situation=set_role",
                ])
            });
            (plain.join().unwrap(), named.join().unwrap())
        });

        // One wrote its schema and its Facts; the other found a schema that
        // is not its own and did nothing.
        let (won, lost, kind, other) = match (plain.status.success(), named.status.success()) {
            (true, false) => (plain, named, "situation", "set_role"),
            (false, true) => (named, plain, "set_role", "situation"),
            both => panic!("{both:?}: {} {}", plain.stderr, named.stderr),
        };
        assert_eq!(won.json()["file_state"], "written");
        assert_eq!(won.json()["facts_recorded"], 10);
        assert_eq!(code(&lost), "tag_schema_exists");
        assert_eq!(schema_files(&sandbox), ["tag-schema.toml"]);
        let text = std::fs::read_to_string(schema_file(&sandbox)).unwrap();
        assert!(text.contains(&format!("[dimensions.{kind}]")), "{text}");
        assert!(!text.contains(&format!("[dimensions.{other}]")), "{text}");
        let kinds: Vec<String> = dj_facts(&sandbox, "10001")
            .into_iter()
            .map(|fact| fact.0)
            .collect();
        assert_eq!(kinds, ["genre", "mood", kind]);
        assert_eq!(
            ok(&sandbox, &["schema", "convert", "--dry-run"])["text"]
                .as_str()
                .is_some(),
            true
        );
    }
}

#[test]
fn a_schema_that_cannot_be_read_is_reported_and_kept() {
    use std::os::unix::fs::PermissionsExt as _;
    let Deck { sandbox, .. } = converted_deck();
    std::fs::create_dir_all(sandbox.sidecar_dir()).unwrap();

    // Not text at all: it is a file that is there, and not the one the
    // conversion writes.
    let bytes = [0xff, 0xfe, b'v', 0x80];
    std::fs::write(schema_file(&sandbox), bytes).unwrap();
    assert_eq!(
        refused(&sandbox, &["schema", "convert"])["code"],
        "tag_schema_exists"
    );
    assert_eq!(std::fs::read(schema_file(&sandbox)).unwrap(), bytes);
    assert_eq!(dj_facts(&sandbox, "10001"), []);

    // There, but not to be read: said so, and left.
    std::fs::write(schema_file(&sandbox), "version = 1\n").unwrap();
    std::fs::set_permissions(
        schema_file(&sandbox),
        std::fs::Permissions::from_mode(0o000),
    )
    .unwrap();
    for args in [
        &["schema", "convert"][..],
        &["schema", "convert", "--replace"][..],
    ] {
        let error = refused(&sandbox, args);
        assert_eq!(error["code"], "tag_schema_unreadable", "{error}");
    }
    std::fs::set_permissions(
        schema_file(&sandbox),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(schema_file(&sandbox)).unwrap(),
        "version = 1\n"
    );
    assert_eq!(schema_files(&sandbox), ["tag-schema.toml"]);
    assert_eq!(dj_facts(&sandbox, "10001"), []);
}

#[test]
fn a_schema_with_a_mistake_stops_only_the_recording_of_the_kinds_it_governs() {
    let Deck { sandbox, .. } = converted_deck();
    ok(&sandbox, &["schema", "convert"]);
    let good = std::fs::read_to_string(schema_file(&sandbox)).unwrap();

    // A mistake in a file that still says which dimensions it has.
    let broken = good.replace("parent = \"breaks\"", "parent = \"brakes\"");
    assert_ne!(broken, good);
    write_schema(&sandbox, &broken);
    assert_eq!(ok_or_invalid(&sandbox), false);

    // What is stored is still read, whatever the kind.
    assert_eq!(dj_facts(&sandbox, "10001").len(), 3);
    for kind in ["genre", "mood", "energy"] {
        let shown = sandbox.cli(&["--json", "facts", "show", "10001", "--kind", kind]);
        assert!(shown.status.success(), "{kind}: {}", shown.stderr);
    }
    assert!(sandbox.cli(&["--json", "facts", "kinds"]).status.success());
    // A kind the schema does not govern is recorded as ever.
    assert!(dj_records(&sandbox, "10001", "energy", &["3"])
        .status
        .success());
    assert!(dj_records(&sandbox, "10001", "version", &["Dub"])
        .status
        .success());
    // A kind it governs is not, until the file is fixed.
    for kind in ["genre", "mood", "situation", "sound"] {
        let refused = dj_records(&sandbox, "10001", kind, &["dark"]);
        assert_eq!(code(&refused), "tag_schema_invalid", "{kind}");
        let message = refused.error_json()["error"]["message"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            message.contains("tag-schema.toml") && message.contains("line "),
            "{message}"
        );
    }
    // One governed Fact refuses the whole batch.
    let many = with_input(
        &sandbox,
        &["--json", "facts", "record-many", "--source", "dj"],
        "{\"track\": \"10002\", \"kind\": \"energy\", \"value\": 4}\n{\"track\": \"10002\", \"kind\": \"mood\", \"value\": [\"dark\"]}\n",
    );
    assert_eq!(code(&many), "tag_schema_invalid");
    assert_eq!(dj_facts(&sandbox, "10002").len(), 1);

    // A file that is not TOML says nothing of its dimensions: every kind a
    // schema could govern is held, the others are not.
    write_schema(&sandbox, "version = 1\n[genre\n");
    assert_eq!(dj_facts(&sandbox, "10001").len(), 5);
    assert!(dj_records(&sandbox, "10001", "energy", &["4"])
        .status
        .success());
    assert!(dj_records(&sandbox, "10001", "prep_colour", &["Green"])
        .status
        .success());
    for kind in ["genre", "mood", "version", "never_heard_of"] {
        assert_eq!(
            code(&dj_records(&sandbox, "10001", kind, &["x"])),
            "tag_schema_invalid",
            "{kind}"
        );
    }

    // Fixed, everything is as it was.
    write_schema(&sandbox, &good);
    assert!(dj_records(&sandbox, "10001", "mood", &["dark"])
        .status
        .success());
    assert_eq!(
        code(&dj_records(&sandbox, "10001", "never_heard_of", &["x"])),
        "kind_not_declared"
    );
}

/// Whether `schema check` finds the Library's schema valid.
fn ok_or_invalid(sandbox: &Sandbox) -> bool {
    sandbox.cli(&["--json", "schema", "check"]).json()["valid"]
        .as_bool()
        .unwrap()
}

#[test]
fn a_tree_with_grandchildren_orphans_jumps_and_names_that_collide_converts_exactly() {
    let sandbox = Sandbox::new();
    let [_, _, _, sub_genre] = sandbox.four_categories();
    let texts = [
        "> Orphan",
        "Bass",
        "> House",
        "> Dubstep",
        "> > Deep Dubstep",
        "> Garage",
        "Bass House",
        "> > Skipped",
        "Bassline",
        // The Library refuses two MyTags with one text in a category, so
        // the same name under another parent is not written here.
        "> Speed",
    ];
    let tags: Vec<String> = texts
        .iter()
        .map(|text| sandbox.create_my_tag_in(&sub_genre, text))
        .collect();
    for (index, carried) in [
        // A grandchild with its parent and grandparent put on by hand.
        (1, vec![1, 3, 4]),
        // A grandchild alone, and a child of another group.
        (2, vec![4, 9]),
        // A top term whose name begins as another's does: not its child.
        (3, vec![6]),
        // The orphan and the entry written too deep.
        (4, vec![0, 7]),
        // A child and its uncle's group's parent.
        (5, vec![2, 8]),
    ] {
        let carried: Vec<&str> = carried.iter().map(|at| tags[*at].as_str()).collect();
        sandbox.detail_track(
            index,
            &common::Details {
                my_tags: &carried,
                ..common::Details::default()
            },
        );
    }
    let there = my_tags_there(&sandbox);

    let converted = ok(&sandbox, &["schema", "convert"]);

    assert_eq!(deck(&ok(&sandbox, &["schema", "check"])), there);
    let text = std::fs::read_to_string(schema_file(&sandbox)).unwrap();
    let genre_lines: Vec<&str> = text
        .lines()
        .skip_while(|line| *line != "[genre]")
        .skip(2)
        .take_while(|line| *line != "]")
        .map(|line| line.split(" # ").next().unwrap())
        .collect();
    assert_eq!(
        genre_lines,
        [
            // A child before any parent stands at the top, its text kept.
            r#"  { id = "orphan", name = "Orphan", deck = "> Orphan" },"#,
            r#"  { id = "bass", name = "Bass" },"#,
            r#"  { id = "bass-house", name = "House", parent = "bass" },"#,
            r#"  { id = "bass-dubstep", name = "Dubstep", parent = "bass" },"#,
            // A child of a child.
            r#"  { id = "bass-dubstep-deep-dubstep", name = "Deep Dubstep", parent = "bass-dubstep" },"#,
            // Back up a level: a child of the group's parent again.
            r#"  { id = "bass-garage", name = "Garage", parent = "bass" },"#,
            // Its slug is taken by Bass's child House: a number tells them
            // apart, and sharing a name's beginning makes no one a child.
            r#"  { id = "bass-house-2", name = "Bass House" },"#,
            // Two levels down under a top term: its child, its text kept.
            r#"  { id = "bass-house-2-skipped", name = "Skipped", parent = "bass-house-2", deck = "> > Skipped" },"#,
            r#"  { id = "bassline", name = "Bassline" },"#,
            r#"  { id = "bassline-speed", name = "Speed", parent = "bassline" },"#,
        ]
    );
    let odd: Vec<(&str, &str)> = converted["odd"]
        .as_array()
        .unwrap()
        .iter()
        .map(|odd| (odd["id"].as_str().unwrap(), odd["why"].as_str().unwrap()))
        .collect();
    assert_eq!(
        odd,
        [
            (
                "orphan",
                "it is written as a child, but no parent comes before it"
            ),
            (
                "bass-house-2-skipped",
                "it is written 2 levels down, under a tag that is only 0 down"
            ),
        ]
    );
    assert_eq!(converted["tree"]["parents"], 4);
    assert_eq!(converted["tree"]["children"], 6);

    // Each track's genre Fact, whole.
    let genre = |track: &str| dj_facts(&sandbox, track);
    assert_eq!(
        genre("10001"),
        [("genre".to_owned(), json!(["bass-dubstep-deep-dubstep"]))]
    );
    assert_eq!(
        genre("10002"),
        [(
            "genre".to_owned(),
            json!(["bass-dubstep-deep-dubstep", "bassline-speed"])
        )]
    );
    assert_eq!(
        genre("10003"),
        [("genre".to_owned(), json!(["bass-house-2"]))]
    );
    assert_eq!(
        genre("10004"),
        [(
            "genre".to_owned(),
            json!(["orphan", "bass-house-2-skipped"])
        )]
    );
    assert_eq!(
        genre("10005"),
        [("genre".to_owned(), json!(["bass-house", "bassline"]))]
    );
    assert_eq!(genre("10006"), []);
    // A grandchild with both above it, one without either, a child without
    // its parent twice, and one written under a parent it is not with.
    assert_eq!(
        converted["implied_parents"],
        json!({"with_parent": 2, "without_parent": 4, "tag_parents": false})
    );
}
