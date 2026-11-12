//! `wreckordbox facts` and `wreckordbox runs` against a Library service on
//! a throwaway Library and Sidecar: the real binary, its output and exit
//! status, and what a second connected client is told.
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

/// The Facts `facts show` lists for a track.
fn facts_of(sandbox: &Sandbox, track: &str) -> Vec<Value> {
    ok(sandbox, &["facts", "show", track])["facts"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn a_fact_recorded_about_a_track_is_read_back_with_its_source_confidence_run_and_time() {
    let sandbox = Sandbox::new();
    sandbox.describe_track(3, "Midnight Drive", "Ada Lovelace", "8A", 0);

    let recorded = ok(
        &sandbox,
        &[
            "facts",
            "record",
            "10003",
            "genre",
            "Tech House",
            "--source",
            "agent:tagger",
            "--confidence",
            "0.8",
            "--description",
            "Tag the new arrivals",
        ],
    );

    let run = recorded["run"]["id"].as_i64().unwrap();
    assert_eq!(recorded["run"]["description"], "Tag the new arrivals");
    assert_eq!(
        recorded["run"]["source"],
        json!({ "kind": "agent", "name": "tagger" })
    );
    assert_eq!(recorded["facts"].as_array().unwrap().len(), 1);
    let fact = &recorded["facts"][0];
    assert_eq!(fact["track_id"], "10003");
    assert_eq!(fact["kind"], "genre");
    assert_eq!(fact["value"], json!(["Tech House"]));

    let shown = ok(&sandbox, &["facts", "show", "10003"]);
    assert_eq!(
        shown["track"],
        json!({ "id": "10003", "title": "Midnight Drive" })
    );
    let facts = shown["facts"].as_array().unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0], *fact);
    assert_eq!(facts[0]["nature"], "judgement");
    assert_eq!(
        facts[0]["source"],
        json!({ "kind": "agent", "name": "tagger" })
    );
    assert_eq!(facts[0]["confidence"], 0.8);
    assert_eq!(facts[0]["run"], run);
    // Milliseconds since the Unix epoch, and recent.
    assert!(facts[0]["recorded_at"].as_i64().unwrap() > 1_700_000_000_000);
    assert_eq!(facts[0]["stale"], false);
    assert_eq!(facts[0]["superseded"], false);

    // Another track has none.
    assert_eq!(facts_of(&sandbox, "10004"), Vec::<Value>::new());
}

/// Runs a command that must fail and answers its JSON error.
fn refused(sandbox: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let run = sandbox.cli(&all);
    assert!(!run.status.success(), "{args:?}: {}", run.stdout);
    assert_eq!(run.stdout, "", "{args:?}");
    run.error_json()["error"].clone()
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

/// The identifiers of the Facts, in the order given.
fn ids(facts: &[Value]) -> Vec<i64> {
    facts
        .iter()
        .map(|fact| fact["id"].as_i64().unwrap())
        .collect()
}

#[test]
fn many_facts_read_from_standard_input_are_recorded_in_one_run() {
    let sandbox = Sandbox::new();
    let lines = concat!(
        r#"{"track": "10001", "kind": "genre", "value": ["Techno"], "confidence": 0.9}"#,
        "\n",
        r#"{"track": "10001", "kind": "mood", "value": ["Dark", "Hypnotic"]}"#,
        "\n\n",
        r#"{"track": "10002", "kind": "energy", "value": 4, "source": {"kind": "tool", "name": "calibration"}}"#,
        "\n",
    );

    let run = with_input(
        &sandbox,
        &[
            "--json",
            "facts",
            "record-many",
            "--source",
            "agent:tagger",
            "--confidence",
            "0.6",
            "--description",
            "First pass over the crate",
        ],
        lines,
    );

    assert!(run.status.success(), "{}", run.stderr);
    let recorded = run.json();
    assert_eq!(recorded["created_run"], true);
    assert_eq!(recorded["run"]["description"], "First pass over the crate");
    assert_eq!(recorded["run"]["outcome"], "recorded 3 Facts");
    let run_id = recorded["run"]["id"].clone();
    let facts = recorded["facts"].as_array().unwrap();
    assert_eq!(facts.len(), 3);
    assert!(facts.iter().all(|fact| fact["run"] == run_id));
    // A line's own confidence and source win; the command's fill the gaps.
    assert_eq!(facts[0]["confidence"], 0.9);
    assert_eq!(facts[1]["confidence"], 0.6);
    assert_eq!(facts[1]["value"], json!(["Dark", "Hypnotic"]));
    assert_eq!(
        facts[1]["source"],
        json!({ "kind": "agent", "name": "tagger" })
    );
    assert_eq!(
        facts[2]["source"],
        json!({ "kind": "tool", "name": "calibration" })
    );

    let first = facts_of(&sandbox, "10001");
    assert_eq!(ids(&first), [ids(facts)[1], ids(facts)[0]]);
    assert_eq!(facts_of(&sandbox, "10002"), [facts[2].clone()]);
}

#[test]
fn a_run_is_started_recorded_in_by_several_commands_and_finished() {
    let sandbox = Sandbox::new();

    let started = ok(
        &sandbox,
        &[
            "runs",
            "start",
            "--source",
            "agent:tagger",
            "--description",
            "Tag the backlog",
        ],
    );
    let run = started["id"].as_i64().unwrap();
    let run_arg = run.to_string();
    assert_eq!(started["description"], "Tag the backlog");
    assert_eq!(
        started["source"],
        json!({ "kind": "agent", "name": "tagger" })
    );
    assert_eq!(started["finished_at"], Value::Null);

    // With a Run, the source is the Run's unless the command says another.
    let one = ok(
        &sandbox,
        &[
            "facts",
            "record",
            "10001",
            "genre",
            "Techno",
            "--run",
            &run_arg,
            "--confidence",
            "0.7",
        ],
    );
    assert_eq!(one["created_run"], false);
    assert_eq!(one["facts"][0]["run"], run);
    assert_eq!(
        one["facts"][0]["source"],
        json!({ "kind": "agent", "name": "tagger" })
    );
    let two = with_input(
        &sandbox,
        &["--json", "facts", "record-many", "--run", &run_arg],
        r#"[{"track": "10002", "kind": "vocals", "value": "Female", "confidence": 0.5,
             "source": {"kind": "web", "name": "https://example.com/release/1"}}]"#,
    );
    assert!(two.status.success(), "{}", two.stderr);
    assert_eq!(two.json()["facts"][0]["run"], run);
    assert_eq!(two.json()["run"]["finished_at"], Value::Null);

    let finished = ok(&sandbox, &["runs", "finish", &run_arg, "--outcome", "done"]);
    assert_eq!(finished["id"], run);
    assert_eq!(finished["outcome"], "done");
    assert!(finished["finished_at"].as_i64().unwrap() >= started["started_at"].as_i64().unwrap());

    // A finished Run takes nothing more, and neither does one that never was.
    let late = refused(
        &sandbox,
        &[
            "facts",
            "record",
            "10003",
            "genre",
            "House",
            "--run",
            &run_arg,
            "--confidence",
            "0.7",
        ],
    );
    assert_eq!(late["code"], "run_finished");
    let nowhere = refused(
        &sandbox,
        &[
            "facts",
            "record",
            "10003",
            "genre",
            "House",
            "--run",
            "999",
            "--confidence",
            "0.7",
        ],
    );
    assert_eq!(nowhere["code"], "run_not_found");
    assert_eq!(
        refused(&sandbox, &["runs", "finish", &run_arg])["code"],
        "run_finished"
    );
    assert_eq!(facts_of(&sandbox, "10003"), Vec::<Value>::new());
}

#[test]
fn an_undeclared_kind_or_a_value_of_the_wrong_type_is_refused_and_nothing_is_recorded() {
    let sandbox = Sandbox::new();
    let agent = ["--source", "agent:tagger", "--confidence", "0.8"];
    let record = |words: &[&str]| {
        let mut args = vec!["facts", "record", "10003"];
        args.extend_from_slice(words);
        args.extend_from_slice(&agent);
        refused(&sandbox, &args)
    };

    let unknown = record(&["tempo_feel", "Rolling"]);
    assert_eq!(unknown["code"], "kind_not_declared");
    assert!(
        unknown["message"].as_str().unwrap().contains("tempo_feel"),
        "{unknown}"
    );

    // Energy is a whole number from 1 to 5.
    assert_eq!(record(&["energy", "high"])["code"], "wrong_value_type");
    assert_eq!(record(&["energy", "9"])["code"], "wrong_value_type");
    assert_eq!(record(&["energy", "3.5"])["code"], "wrong_value_type");
    // Vocals is one text.
    assert_eq!(
        record(&["vocals", "Full", "Instrumental"])["code"],
        "wrong_value_type"
    );
    // Prep colour is one of the colour labels.
    assert_eq!(
        record(&["prep_colour", "Mauve"])["code"],
        "wrong_value_type"
    );
    // Stems are recorded by a separation, not by hand.
    assert_eq!(record(&["stems", "{}"])["code"], "kind_reserved");

    // In a batch, one bad Fact refuses them all, and says which.
    let batch = with_input(
        &sandbox,
        &[
            "--json",
            "facts",
            "record-many",
            "--source",
            "agent:tagger",
            "--confidence",
            "0.8",
        ],
        concat!(
            r#"{"track": "10003", "kind": "genre", "value": ["Techno"]}"#,
            "\n",
            r#"{"track": "10003", "kind": "genre", "value": 7}"#,
            "\n",
        ),
    );
    assert!(!batch.status.success());
    let error = batch.error_json()["error"].clone();
    assert_eq!(error["code"], "wrong_value_type");
    assert!(
        error["message"].as_str().unwrap().starts_with("Fact 2: "),
        "{error}"
    );
    // A track that is not in the Library, likewise.
    let lost = with_input(
        &sandbox,
        &[
            "--json",
            "facts",
            "record-many",
            "--source",
            "agent:tagger",
            "--confidence",
            "0.8",
        ],
        concat!(
            r#"{"track": "10003", "kind": "genre", "value": ["Techno"]}"#,
            "\n",
            r#"{"track": "77", "kind": "genre", "value": ["Techno"]}"#,
            "\n",
        ),
    );
    assert_eq!(lost.error_json()["error"]["code"], "track_not_found");

    assert_eq!(facts_of(&sandbox, "10003"), Vec::<Value>::new());

    // The right types are taken.
    let args = |words: &[&'static str]| {
        let mut all = vec!["facts", "record", "10003"];
        all.extend_from_slice(words);
        all.extend_from_slice(&agent);
        all
    };
    assert_eq!(
        ok(&sandbox, &args(&["energy", "4"]))["facts"][0]["value"],
        4
    );
    assert_eq!(
        ok(&sandbox, &args(&["situation", "Opener", "Peak time"]))["facts"][0]["value"],
        json!(["Opener", "Peak time"])
    );
    assert_eq!(
        ok(&sandbox, &args(&["prep_colour", "Green"]))["facts"][0]["value"],
        "Green"
    );
}

#[test]
fn a_fact_needs_a_source_and_a_confidence_unless_the_dj_says_it() {
    let sandbox = Sandbox::new();

    let nobody = refused(
        &sandbox,
        &[
            "facts",
            "record",
            "10003",
            "genre",
            "Techno",
            "--confidence",
            "0.8",
        ],
    );
    assert_eq!(nobody["code"], "invalid_source");
    let nameless = refused(
        &sandbox,
        &[
            "facts",
            "record",
            "10003",
            "genre",
            "Techno",
            "--source",
            "agent",
            "--confidence",
            "0.8",
        ],
    );
    assert_eq!(nameless["code"], "invalid_source");
    let unsure = refused(
        &sandbox,
        &[
            "facts",
            "record",
            "10003",
            "genre",
            "Techno",
            "--source",
            "agent:tagger",
        ],
    );
    assert_eq!(unsure["code"], "invalid_confidence");
    let too_sure = refused(
        &sandbox,
        &[
            "facts",
            "record",
            "10003",
            "genre",
            "Techno",
            "--source",
            "agent:tagger",
            "--confidence",
            "1.5",
        ],
    );
    assert_eq!(too_sure["code"], "invalid_confidence");
    assert_eq!(facts_of(&sandbox, "10003"), Vec::<Value>::new());

    // The DJ is sure, and needs no name.
    let mine = ok(
        &sandbox,
        &[
            "facts", "record", "10003", "genre", "Techno", "--source", "dj",
        ],
    );
    assert_eq!(mine["facts"][0]["confidence"], 1.0);
    assert_eq!(
        mine["facts"][0]["source"],
        json!({ "kind": "dj", "name": "dj" })
    );
    // With no description, the Run made for it says what it was.
    assert_eq!(mine["run"]["description"], "Record genre for track 10003");
    assert_eq!(
        refused(&sandbox, &["facts", "show", "77"])["code"],
        "track_not_found"
    );
}

#[test]
fn every_declared_kind_says_whether_it_is_a_judgement_or_a_measurement_and_its_value_type() {
    let sandbox = Sandbox::new();

    let kinds = ok(&sandbox, &["facts", "kinds"]);

    let declared: Vec<(&str, &str, &str)> = kinds["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|kind| {
            (
                kind["name"].as_str().unwrap(),
                kind["nature"].as_str().unwrap(),
                kind["value"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        declared,
        [
            ("stems", "measurement", "object"),
            ("genre", "judgement", "texts"),
            ("mood", "judgement", "texts"),
            ("situation", "judgement", "texts"),
            ("sound", "judgement", "texts"),
            ("vocals", "judgement", "text"),
            ("version", "judgement", "text"),
            ("energy", "judgement", "integer"),
            ("prep_colour", "judgement", "text"),
            ("intensity", "measurement", "object"),
        ]
    );
    assert!(kinds["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .all(|kind| kind["built_in"] == true));
    let energy = &kinds["kinds"][7];
    assert_eq!(
        (energy["min"].clone(), energy["max"].clone()),
        (json!(1.0), json!(5.0))
    );

    let text = sandbox.cli(&["facts", "kinds"]);
    assert!(text.status.success(), "{}", text.stderr);
    let lines: Vec<&str> = text.stdout.lines().collect();
    assert_eq!(
        lines[0].split_whitespace().collect::<Vec<_>>(),
        ["KIND", "NATURE", "VALUE", "DECLARED"]
    );
    assert!(
        lines.iter().any(|line| line.starts_with("energy ")
            && line.contains("Judgement")
            && line.contains("a whole number from 1 to 5")
            && line.ends_with("built in")),
        "{}",
        text.stdout
    );
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("intensity ") && line.contains("Measurement")),
        "{}",
        text.stdout
    );
}

#[test]
fn a_kind_declared_in_the_kinds_file_is_usable_at_once_and_a_bad_file_is_refused() {
    let sandbox = Sandbox::new();
    let file = PathBuf::from(ok(&sandbox, &["facts", "kinds"])["file"].as_str().unwrap());
    assert_eq!(
        file,
        sandbox
            .sidecar_dir()
            .canonicalize()
            .unwrap()
            .join("kinds.toml")
    );
    let era = ["facts", "record", "10003", "era", "90s", "--source", "dj"];
    assert_eq!(refused(&sandbox, &era)["code"], "kind_not_declared");

    // The service is already running; the file is read when it is needed.
    std::fs::write(
        &file,
        r#"
[kinds.era]
nature = "judgement"
value = "text"
choices = ["90s", "00s", "10s"]
description = "The decade the track sounds like."

[kinds.danceability]
nature = "measurement"
value = "number"
min = 0
max = 1
"#,
    )
    .unwrap();

    let kinds = ok(&sandbox, &["facts", "kinds"]);
    let added: Vec<Value> = kinds["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|kind| kind["built_in"] == false)
        .cloned()
        .collect();
    assert_eq!(added.len(), 2);
    assert_eq!(added[0]["name"], "danceability");
    assert_eq!(added[0]["nature"], "measurement");
    assert_eq!(added[0]["value"], "number");
    assert_eq!(added[1]["name"], "era");
    assert_eq!(added[1]["choices"], json!(["90s", "00s", "10s"]));

    let recorded = ok(&sandbox, &era);
    assert_eq!(recorded["facts"][0]["kind"], "era");
    assert_eq!(recorded["facts"][0]["nature"], "judgement");
    assert_eq!(
        refused(
            &sandbox,
            &["facts", "record", "10003", "era", "80s", "--source", "dj"]
        )["code"],
        "wrong_value_type"
    );

    // A file that declares a kind badly is refused whole, and says where.
    for bad in [
        "[kinds.era]\nnature = \"hunch\"\nvalue = \"text\"\n",
        "[kinds.era]\nnature = \"judgement\"\nvalue = \"text\"\nmin = 1\n",
        "[kinds.genre]\nnature = \"measurement\"\nvalue = \"number\"\n",
        "[kinds.\"Era!\"]\nnature = \"judgement\"\nvalue = \"text\"\n",
        "not toml at all [",
    ] {
        std::fs::write(&file, bad).unwrap();
        let error = refused(
            &sandbox,
            &[
                "facts", "record", "10003", "genre", "House", "--source", "dj",
            ],
        );
        assert_eq!(error["code"], "kinds_file_invalid", "{bad}");
        assert!(
            error["message"].as_str().unwrap().contains("kinds.toml"),
            "{error}"
        );
        assert_eq!(
            refused(&sandbox, &["facts", "kinds"])["code"],
            "kinds_file_invalid"
        );
    }
    assert_eq!(
        ids(&facts_of(&sandbox, "10003")),
        ids(recorded["facts"].as_array().unwrap())
    );
}

#[test]
fn facts_from_different_sources_about_the_same_track_and_kind_stand_side_by_side() {
    let sandbox = Sandbox::new();
    let record = |value: &'static str, source: &'static str| {
        ok(
            &sandbox,
            &[
                "facts",
                "record",
                "10003",
                "genre",
                value,
                "--source",
                source,
                "--confidence",
                "0.7",
            ],
        )["facts"][0]
            .clone()
    };

    let agent = record("Tech House", "agent:tagger");
    let other_agent = record("Minimal", "agent:second-opinion");
    let web = record("House", "web:https://example.com/release/1");
    let dj = record("Techno", "dj");

    let standing = ok(&sandbox, &["facts", "show", "10003", "--kind", "genre"]);
    let facts = standing["facts"].as_array().unwrap();
    assert_eq!(
        ids(facts),
        ids(&[dj.clone(), web.clone(), other_agent.clone(), agent.clone()])
    );
    assert!(facts.iter().all(|fact| fact["superseded"] == false));
    let said: Vec<(&str, &str)> = facts
        .iter()
        .map(|fact| {
            (
                fact["source"]["kind"].as_str().unwrap(),
                fact["value"][0].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        said,
        [
            ("dj", "Techno"),
            ("web", "House"),
            ("agent", "Minimal"),
            ("agent", "Tech House")
        ]
    );

    // The same source saying it again replaces only its own earlier Fact,
    // which is kept.
    let again = record("Deep House", "agent:tagger");
    let now = facts_of(&sandbox, "10003");
    assert_eq!(ids(&now), ids(&[again.clone(), dj, web, other_agent]));
    let everything = ok(&sandbox, &["facts", "show", "10003", "--all"]);
    let kept = everything["facts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|fact| fact["id"] == agent["id"])
        .unwrap();
    assert_eq!(kept["superseded"], true);
    assert_eq!(kept["superseded_by"], again["id"]);
    assert_eq!(kept["value"], json!(["Tech House"]));

    // Asking for a kind that is not declared is a mistake, not an empty list.
    assert_eq!(
        refused(&sandbox, &["facts", "show", "10003", "--kind", "genres"])["code"],
        "kind_not_declared"
    );
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
fn recording_facts_changes_nothing_in_the_library_and_other_clients_hear_only_of_the_sidecar() {
    let sandbox = Sandbox::new();
    sandbox.describe_track(3, "Midnight Drive", "Ada Lovelace", "8A", 2);
    let before = library_shape(&sandbox);
    let track_before = ok(&sandbox, &["tracks", "show", "10003"]);
    let mut watcher = sandbox.client();
    watcher.subscribe().unwrap();

    let one = ok(
        &sandbox,
        &[
            "facts",
            "record",
            "10003",
            "energy",
            "5",
            "--source",
            "agent:tagger",
            "--confidence",
            "0.9",
        ],
    );
    let many = with_input(
        &sandbox,
        &["--json", "facts", "record-many", "--source", "dj"],
        concat!(
            r#"{"track": "10003", "kind": "genre", "value": ["Techno"]}"#,
            "\n",
            r#"{"track": "10004", "kind": "prep_colour", "value": "Green"}"#,
            "\n",
        ),
    );
    assert!(many.status.success(), "{}", many.stderr);
    let many = many.json();
    ok(
        &sandbox,
        &[
            "runs",
            "start",
            "--source",
            "dj",
            "--description",
            "By hand",
        ],
    );
    ok(&sandbox, &["facts", "show", "10003"]);
    ok(&sandbox, &["facts", "kinds"]);

    // Each recorded Fact is announced as a change to the Sidecar, and
    // nothing is announced as a change to the Library.
    let mut events = Vec::new();
    while let Some(event) = watcher.next_event(Duration::from_millis(500)).unwrap() {
        events.push(event);
    }
    let heard: Vec<(&str, Value)> = events
        .iter()
        .map(|event| (event.event.as_str(), event.data.clone()))
        .collect();
    let changed = |fact: &Value| {
        (
            "sidecar_changed",
            json!({ "track_id": fact["track_id"], "kind": fact["kind"], "fact_id": fact["id"], "stale": [] }),
        )
    };
    assert_eq!(
        heard,
        [
            changed(&one["facts"][0]),
            changed(&many["facts"][0]),
            changed(&many["facts"][1]),
        ]
    );

    // The track is as it was: the Energy recorded as a Fact is not the
    // track's rating, and the Prep colour Fact is not its colour.
    assert_eq!(ok(&sandbox, &["tracks", "show", "10003"]), track_before);
    assert_eq!(track_before["rating"], 2);
    assert_eq!(
        ok(&sandbox, &["tracks", "show", "10004"])["prep_colour"],
        Value::Null
    );

    // The Facts are in the Sidecar's own database, in its own folder, and
    // no rekordbox-format file gained a table, a column or a row.
    let sidecar = sandbox.sidecar_dir();
    assert!(sidecar.join("sidecar.db").is_file());
    assert!(!sidecar.starts_with(sandbox.master_db().parent().unwrap()));
    assert_eq!(library_shape(&sandbox), before);
}

/// Changes the audio in a WAV file the sandbox imported: one sample of it.
fn change_audio(path: &std::path::Path) {
    let mut bytes = std::fs::read(path).unwrap();
    bytes[44] ^= 0x55;
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn a_measurement_is_tied_to_the_audio_it_was_made_from_and_a_judgement_is_not() {
    let sandbox = Sandbox::new();
    let track = sandbox.import_track("Night Bus.wav", 1);
    let audio = sandbox.path().join("audio/Night Bus.wav");
    let tool = ["--source", "tool:meter", "--confidence", "1"];
    let measure = |track: &str, value: &'static str| {
        let mut args = vec!["facts", "record", track, "intensity", value];
        args.extend_from_slice(&tool);
        args.into_iter().map(str::to_owned).collect::<Vec<_>>()
    };
    fn run(args: &[String]) -> Vec<&str> {
        args.iter().map(String::as_str).collect()
    }

    // A fixture track has no audio file: a Judgement can still be recorded
    // about it, a Measurement cannot.
    let no_audio = refused(&sandbox, &run(&measure("10003", r#"{"summary": 0.4}"#)));
    assert_eq!(no_audio["code"], "track_file_missing");
    let judged = ok(
        &sandbox,
        &[
            "facts", "record", "10003", "genre", "Techno", "--source", "dj",
        ],
    );
    assert_eq!(judged["facts"][0]["fingerprint"], Value::Null);

    // With the audio at hand, every Fact carries its fingerprint.
    let measured = ok(
        &sandbox,
        &run(&measure(
            &track,
            r#"{"summary": 0.62, "curve": [0.2, 0.9]}"#,
        )),
    );
    let fact = &measured["facts"][0];
    assert_eq!(fact["nature"], "measurement");
    assert_eq!(
        fact["value"],
        json!({ "summary": 0.62, "curve": [0.2, 0.9] })
    );
    let fingerprint = fact["fingerprint"].as_str().unwrap().to_owned();
    assert!(fingerprint.starts_with("a1:"), "{fingerprint}");
    let genre = ok(
        &sandbox,
        &[
            "facts",
            "record",
            &track,
            "genre",
            "Dub Techno",
            "--source",
            "dj",
        ],
    );
    assert_eq!(genre["facts"][0]["fingerprint"], fingerprint.as_str());
    assert_eq!(genre["marked_stale"], json!([]));

    // The audio changes. Until something looks, nothing is known of it.
    change_audio(&audio);
    assert!(facts_of(&sandbox, &track)
        .iter()
        .all(|fact| fact["stale"] == false));

    // Looking marks the Measurement stale and leaves the Judgement alone.
    let mut watcher = sandbox.client();
    watcher.subscribe().unwrap();
    let checked = ok(&sandbox, &["facts", "show", &track, "--check-audio"]);
    let by_kind = |facts: &Value, kind: &str| {
        facts["facts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|fact| fact["kind"] == kind)
            .unwrap()
            .clone()
    };
    assert_eq!(by_kind(&checked, "intensity")["stale"], true);
    assert_eq!(by_kind(&checked, "genre")["stale"], false);
    let event = watcher.next_event(Duration::from_secs(2)).unwrap().unwrap();
    assert_eq!(event.event, "sidecar_changed");
    assert_eq!(
        event.data,
        json!({ "track_id": track, "kind": "intensity", "fact_id": null, "stale": [fact["id"]] })
    );

    // Measuring the new audio records a Fact with the new fingerprint.
    let again = ok(&sandbox, &run(&measure(&track, r#"{"summary": 0.7}"#)));
    assert_ne!(again["facts"][0]["fingerprint"], fingerprint.as_str());
    assert_eq!(again["facts"][0]["stale"], false);
}

#[test]
fn recording_finds_out_measurements_made_from_other_audio() {
    let sandbox = Sandbox::new();
    let track = sandbox.import_track("Night Bus.wav", 1);
    let measured = ok(
        &sandbox,
        &[
            "facts",
            "record",
            &track,
            "intensity",
            r#"{"summary": 0.62}"#,
            "--source",
            "tool:meter",
            "--confidence",
            "1",
        ],
    );
    change_audio(&sandbox.path().join("audio/Night Bus.wav"));

    let judged = ok(
        &sandbox,
        &[
            "facts",
            "record",
            &track,
            "genre",
            "Dub Techno",
            "--source",
            "dj",
        ],
    );

    assert_eq!(judged["marked_stale"], json!([measured["facts"][0]["id"]]));
    let text = sandbox.cli(&["facts", "show", &track]);
    assert!(text.status.success(), "{}", text.stderr);
    let stale_line = text
        .stdout
        .lines()
        .find(|line| line.contains("intensity"))
        .unwrap();
    assert!(stale_line.ends_with("stale"), "{}", text.stdout);
}

#[test]
fn by_hand_the_results_are_readable_lines_and_a_table() {
    let sandbox = Sandbox::new();
    sandbox.describe_track(3, "Midnight Drive", "Ada Lovelace", "8A", 0);

    let nothing = sandbox.cli(&["facts", "show", "10003"]);
    assert_eq!(
        nothing.stdout,
        "Track 10003 \"Midnight Drive\" has no Facts.\n"
    );

    let one = sandbox.cli(&[
        "facts",
        "record",
        "10003",
        "mood",
        "Dark",
        "Hypnotic",
        "--source",
        "agent:tagger",
        "--confidence",
        "0.75",
    ]);
    assert!(one.status.success(), "{}", one.stderr);
    assert_eq!(
        one.stdout,
        "Recorded Fact 1 in Run 1: track 10003 mood = [\"Dark\",\"Hypnotic\"] (agent:tagger, confidence 0.75).\n"
    );
    let many = with_input(
        &sandbox,
        &["facts", "record-many", "--source", "dj"],
        concat!(
            r#"{"track": "10003", "kind": "genre", "value": ["Techno"]}"#,
            "\n",
            r#"{"track": "10004", "kind": "energy", "value": 4}"#,
            "\n",
        ),
    );
    assert_eq!(many.stdout, "Recorded 2 Facts about 2 tracks in Run 2.\n");
    let started = sandbox.cli(&[
        "runs",
        "start",
        "--source",
        "agent:tagger",
        "--description",
        "Tag the backlog",
    ]);
    assert_eq!(
        started.stdout,
        "Started Run 3: Tag the backlog (agent:tagger).\n"
    );
    let finished = sandbox.cli(&["runs", "finish", "3"]);
    assert_eq!(
        finished.stdout,
        "Finished Run 3: Tag the backlog (agent:tagger).\n"
    );

    let shown = sandbox.cli(&["facts", "show", "10003"]);
    assert!(shown.status.success(), "{}", shown.stderr);
    let lines: Vec<Vec<&str>> = shown
        .stdout
        .lines()
        .map(|line| {
            line.split("  ")
                .filter(|cell| !cell.is_empty())
                .map(str::trim)
                .collect()
        })
        .collect();
    assert_eq!(lines[0], ["Track 10003 \"Midnight Drive\""]);
    assert_eq!(
        lines[1],
        [
            "FACT",
            "KIND",
            "VALUE",
            "SOURCE",
            "CONFIDENCE",
            "RUN",
            "RECORDED",
            "NOTE"
        ]
    );
    // Newest first, each with who said it, how sure, in which Run and when.
    assert_eq!(lines[2][..6], ["2", "genre", "Techno", "dj", "1", "2"]);
    assert_eq!(
        lines[3][..6],
        ["1", "mood", "Dark, Hypnotic", "agent:tagger", "0.75", "1"]
    );
    let when = lines[3][6];
    assert!(
        when.len() == 20 && when.starts_with("20") && when.ends_with('Z') && &when[10..11] == " ",
        "{when}"
    );
    assert_eq!(lines.len(), 4);

    // An error by hand is one line on standard error.
    let error = sandbox.cli(&["facts", "record", "10003", "energy", "11", "--source", "dj"]);
    assert!(!error.status.success());
    assert_eq!(
        error.stderr,
        "wreckordbox: energy takes a whole number from 1 to 5, not 11\n"
    );
}
