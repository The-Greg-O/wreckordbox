//! The Ledger through `wreckordbox`: every edit to the Library and every
//! change to the Sidecar is recorded with its source, its Run and its
//! before and after values, and Runs can be listed and shown, also after
//! the service has restarted. The real binary against a service on a
//! throwaway Library and Sidecar.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod common;

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

/// The entries `runs show` lists for a Run.
fn entries(sandbox: &Sandbox, run: i64) -> Vec<Value> {
    ok(sandbox, &["runs", "show", &run.to_string()])["entries"]
        .as_array()
        .unwrap()
        .clone()
}

/// Stops the service a command started here and waits until it is gone, so
/// the next command starts another.
fn restart_service(sandbox: &Sandbox) {
    let pid = sandbox.status()["pid"].as_u64().unwrap();
    sandbox.client().stop().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while common::alive(pid) {
        assert!(
            std::time::Instant::now() < deadline,
            "the service did not stop"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn a_rating_set_from_the_command_is_a_ledger_entry_with_its_source_run_and_before_and_after() {
    let sandbox = Sandbox::new();
    sandbox.describe_track(3, "Midnight Drive", "Ada Lovelace", "8A", 2);

    let rated = ok(
        &sandbox,
        &[
            "tracks",
            "rate",
            "10003",
            "4",
            "--source",
            "agent:tagger",
            "--description",
            "Rate the warm-up tracks",
        ],
    );

    // The command reports the Run it made.
    let run = rated["run"]["id"]
        .as_i64()
        .expect("the command names its Run");
    assert_eq!(rated["run"]["description"], "Rate the warm-up tracks");
    assert_eq!(
        rated["run"]["source"],
        json!({"kind": "agent", "name": "tagger"})
    );

    let shown = ok(&sandbox, &["runs", "show", &run.to_string()]);
    assert_eq!(shown["run"]["id"], run);
    assert_eq!(shown["total"], 1);
    let entry = &shown["entries"][0];
    assert_eq!(entry["store"], "library");
    assert_eq!(entry["action"], "edit");
    assert_eq!(entry["kind"], "track");
    assert_eq!(entry["label"], "Track Edit");
    assert_eq!(entry["source"], json!({"kind": "agent", "name": "tagger"}));
    assert_eq!(entry["command"]["name"], "tracks.rate");
    assert_eq!(entry["reversible"], true);
    assert_eq!(
        entry["values"],
        json!([{"subject": "track 10003", "field": "Rating", "before": 2, "after": 4}])
    );
    assert!(entry["at"].as_i64().unwrap() > 1_600_000_000_000);
}

/// The Runs `runs list` lists, newest first.
fn runs(sandbox: &Sandbox) -> Vec<Value> {
    ok(sandbox, &["runs", "list"])["runs"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn an_edit_whose_caller_names_no_source_is_the_tools_and_never_the_djs() {
    let sandbox = Sandbox::new();

    let rated = ok(&sandbox, &["tracks", "rate", "10001", "3"]);

    assert_eq!(
        rated["run"]["source"],
        json!({"kind": "tool", "name": "wreckordbox"})
    );
    // A Run of the command's own, ended with it and saying what it was.
    assert!(rated["run"]["finished_at"].is_i64());
    assert_eq!(rated["run"]["outcome"], "1 change");
    assert!(rated["run"]["description"]
        .as_str()
        .unwrap()
        .starts_with("tracks.rate "));
    let run = rated["run"]["id"].as_i64().unwrap();
    assert_eq!(
        entries(&sandbox, run)[0]["source"],
        json!({"kind": "tool", "name": "wreckordbox"})
    );

    // The DJ at the command line says so.
    let djs = ok(
        &sandbox,
        &["tracks", "rate", "10001", "4", "--source", "dj"],
    );
    assert_eq!(djs["run"]["source"], json!({"kind": "dj", "name": "dj"}));
}

#[test]
fn edits_and_facts_made_in_one_run_are_listed_and_shown_together_also_after_a_restart() {
    let sandbox = Sandbox::new();
    sandbox.describe_track(3, "Midnight Drive", "Ada Lovelace", "8A", 2);
    sandbox.dj_prep_colours();
    let acid = sandbox.create_my_tag("Acid");
    let started = ok(
        &sandbox,
        &[
            "runs",
            "start",
            "--source",
            "agent:tagger",
            "--description",
            "Tag the warm-up",
        ],
    );
    let run = started["id"].as_i64().unwrap();
    let id = run.to_string();

    let rated = ok(&sandbox, &["tracks", "rate", "10003", "4", "--run", &id]);
    assert_eq!(rated["run"]["id"], run);
    assert!(rated["run"]["finished_at"].is_null(), "the Run stays open");
    let assigned = ok(
        &sandbox,
        &["mytags", "assign", &acid, "10003", "10004", "--run", &id],
    );
    assert_eq!(assigned["run"]["id"], run);
    let coloured = ok(
        &sandbox,
        &["intake", "colour", "10003", "Clean", "--run", &id],
    );
    assert_eq!(coloured["run"]["id"], run);
    ok(
        &sandbox,
        &[
            "facts",
            "record",
            "10003",
            "genre",
            "House",
            "--confidence",
            "0.8",
            "--run",
            &id,
        ],
    );
    ok(
        &sandbox,
        &[
            "facts",
            "record",
            "10003",
            "genre",
            "Tech House",
            "--confidence",
            "0.8",
            "--run",
            &id,
        ],
    );
    ok(&sandbox, &["runs", "finish", &id, "--outcome", "done"]);

    let check = |sandbox: &Sandbox| {
        let listed = runs(sandbox);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["id"], run);
        assert_eq!(
            listed[0]["source"],
            json!({"kind": "agent", "name": "tagger"})
        );
        assert_eq!(listed[0]["description"], "Tag the warm-up");
        assert_eq!(listed[0]["outcome"], "done");
        assert!(listed[0]["started_at"].as_i64().unwrap() > 1_600_000_000_000);
        assert_eq!(listed[0]["changes"], 5);
        assert_eq!(
            listed[0]["counts"],
            json!([
                {"store": "library", "kind": "track", "count": 2},
                {"store": "library", "kind": "track_tags", "count": 1},
                {"store": "sidecar", "kind": "genre", "count": 2},
            ])
        );

        let shown = entries(sandbox, run);
        let kinds: Vec<(&str, &str, &str)> = shown
            .iter()
            .map(|entry| {
                (
                    entry["store"].as_str().unwrap(),
                    entry["action"].as_str().unwrap(),
                    entry["kind"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            kinds,
            [
                ("library", "edit", "track"),
                ("library", "edit", "track_tags"),
                ("library", "edit", "track"),
                ("sidecar", "record", "genre"),
                ("sidecar", "record", "genre"),
            ]
        );
        assert_eq!(shown[1]["command"]["name"], "mytags.assign");
        assert_eq!(
            shown[1]["values"],
            json!([
                {"subject": "track 10003", "field": "MyTags", "before": [], "after": [acid]},
                {"subject": "track 10004", "field": "MyTags", "before": [], "after": [acid]},
            ])
        );
        assert_eq!(shown[2]["command"]["name"], "tracks.set_prep_colour");
        assert_eq!(shown[2]["values"][0]["field"], "ColorID");
        // A Fact's entry is the Fact, with the one it took the place of.
        assert_eq!(
            shown[3]["values"],
            json!([{"subject": "track 10003", "field": "genre", "before": null, "after": ["House"]}])
        );
        assert_eq!(
            shown[4]["values"],
            json!([{"subject": "track 10003", "field": "genre", "before": ["House"], "after": ["Tech House"]}])
        );
        assert_eq!(shown[4]["replaced"]["id"], shown[3]["fact"]["id"]);
        assert!(shown
            .iter()
            .all(|entry| entry["source"] == json!({"kind": "agent", "name": "tagger"})));
    };
    check(&sandbox);

    restart_service(&sandbox);
    check(&sandbox);

    // A Run that has ended takes no more changes, and nothing is changed.
    let refused = sandbox.cli(&["--json", "tracks", "rate", "10003", "1", "--run", &id]);
    assert!(!refused.status.success());
    assert_eq!(refused.error_json()["error"]["code"], "run_finished");
    let missing = sandbox.cli(&["--json", "tracks", "rate", "10003", "1", "--run", "999"]);
    assert_eq!(missing.error_json()["error"]["code"], "run_not_found");
    assert_eq!(ok(&sandbox, &["tracks", "show", "10003"])["rating"], 4);
    assert_eq!(runs(&sandbox).len(), 1);
}

#[test]
fn an_edit_the_undo_history_does_not_keep_is_still_an_entry_naming_it() {
    let sandbox = Sandbox::new();
    let categories = sandbox.four_categories();

    let created = ok(
        &sandbox,
        &[
            "mytags",
            "create",
            &categories[1],
            "Dark",
            "--source",
            "agent:tagger",
        ],
    );
    let made = entries(&sandbox, created["run"]["id"].as_i64().unwrap());
    assert_eq!(made.len(), 1);
    assert_eq!(made[0]["action"], "edit");
    assert_eq!(made[0]["kind"], "unrecorded");
    assert_eq!(made[0]["label"], "mytags.create");
    assert_eq!(made[0]["command"]["args"]["name"], "Dark");
    assert_eq!(made[0]["reversible"], false);
    assert_eq!(made[0]["payload"], Value::Null);

    let tag = created["my_tag"]["id"].as_str().unwrap();
    let deleted = ok(
        &sandbox,
        &["mytags", "delete", tag, "--source", "agent:tagger"],
    );
    let gone = entries(&sandbox, deleted["run"]["id"].as_i64().unwrap());
    assert_eq!(gone.len(), 1);
    assert_eq!(gone[0]["kind"], "permanent");
    assert_eq!(gone[0]["label"], "mytags.delete");
    assert_eq!(gone[0]["reversible"], false);
}

#[test]
fn a_command_that_changes_nothing_makes_no_run_and_one_that_fails_makes_none() {
    let sandbox = Sandbox::new();
    sandbox.describe_track(3, "Midnight Drive", "Ada Lovelace", "8A", 2);

    let same = ok(&sandbox, &["tracks", "rate", "10003", "2"]);
    assert!(same.get("run").is_none(), "{same}");
    let failed = sandbox.cli(&["--json", "tracks", "rate", "424242", "2"]);
    assert_eq!(failed.error_json()["error"]["code"], "track_not_found");
    // A command that edits nothing takes no Run.
    let reading = sandbox.cli(&["--json", "tracks", "list", "--source", "agent:tagger"]);
    assert!(!reading.status.success());
    assert_eq!(reading.error_json()["error"]["code"], "bad_request");

    let listed = ok(&sandbox, &["runs", "list"]);
    assert_eq!(listed["total"], 0);
    assert_eq!(listed["runs"], json!([]));
    let text = sandbox.cli(&["runs", "list"]);
    assert_eq!(text.stdout, "No Runs.\n");
    let none = sandbox.cli(&["--json", "runs", "show", "1"]);
    assert_eq!(none.error_json()["error"]["code"], "run_not_found");
}

#[test]
fn runs_and_a_runs_entries_are_paged_and_read_as_tables() {
    let sandbox = Sandbox::new();
    for (track, stars) in [("10001", "1"), ("10002", "2"), ("10003", "3")] {
        ok(
            &sandbox,
            &[
                "tracks",
                "rate",
                track,
                stars,
                "--source",
                "agent:tagger",
                "--description",
                "Rate one",
            ],
        );
    }
    let started = ok(
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
    let run = started["id"].as_i64().unwrap().to_string();
    for stars in ["4", "5", "1"] {
        ok(&sandbox, &["tracks", "rate", "10004", stars, "--run", &run]);
    }

    let first = ok(&sandbox, &["runs", "list", "--limit", "3"]);
    assert_eq!(first["total"], 4);
    assert_eq!(first["runs"].as_array().unwrap().len(), 3);
    assert_eq!(first["runs"][0]["id"].to_string(), run, "newest first");
    assert_eq!(first["next_offset"], 3);
    let rest = ok(&sandbox, &["runs", "list", "--limit", "3", "--offset", "3"]);
    assert_eq!(rest["runs"].as_array().unwrap().len(), 1);
    assert_eq!(rest["next_offset"], Value::Null);

    let page = ok(
        &sandbox,
        &["runs", "show", &run, "--limit", "2", "--offset", "1"],
    );
    assert_eq!(page["total"], 3);
    assert_eq!(page["next_offset"], Value::Null);
    let stars: Vec<&Value> = page["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| &entry["values"][0]["after"])
        .collect();
    assert_eq!(stars, [&json!(5), &json!(1)]);

    let table = sandbox.cli(&["runs", "list", "--limit", "2"]);
    assert!(table.status.success(), "{}", table.stderr);
    let lines: Vec<&str> = table.stdout.lines().collect();
    assert!(lines[0].starts_with("RUN  SOURCE"), "{}", table.stdout);
    assert!(
        lines[1].contains("dj")
            && lines[1].contains("open")
            && lines[1].contains("3 library track")
    );
    assert!(lines[2].contains("agent:tagger") && lines[2].ends_with("Rate one"));
    assert_eq!(lines[3], "Runs 1 to 2 of 4; next page: --offset 2");

    let shown = sandbox.cli(&["runs", "show", &run]);
    assert!(shown.status.success(), "{}", shown.stderr);
    assert!(
        shown
            .stdout
            .starts_with(&format!("Run {run}: By hand (dj)\n")),
        "{}",
        shown.stdout
    );
    assert!(shown.stdout.contains("still open"));
    assert!(
        shown.stdout.contains("track 10004 Rating: 0 -> 4"),
        "{}",
        shown.stdout
    );

    // In text, every edit says which Run it is in: one made by hand with
    // none names the Run made for it.
    let plain = sandbox.cli(&["tracks", "rate", "10005", "1"]);
    assert_eq!(plain.stdout.lines().count(), 2, "{}", plain.stdout);
    assert!(
        plain.stdout.lines().last().unwrap().starts_with("Run "),
        "{}",
        plain.stdout
    );
    let rated = sandbox.cli(&["tracks", "rate", "10005", "2", "--run", &run]);
    assert!(
        rated
            .stdout
            .ends_with(&format!("\nRun {run}: By hand (dj).\n")),
        "{}",
        rated.stdout
    );
}

#[test]
fn an_edit_whose_entry_could_not_be_written_is_recorded_before_anything_else_changes() {
    let sandbox = Sandbox::new();
    ok(&sandbox, &["tracks", "rate", "10001", "1"]);
    restart_service(&sandbox);
    // What a service leaves when the Ledger refused an entry: the entry,
    // on disk beside it.
    std::fs::write(
        sandbox.sidecar_dir().join("ledger-spill.jsonl"),
        concat!(
            r#"{"run":null,"source":{"kind":"agent","name":"tagger"},"description":"Rate late","at":1700000000000,"#,
            r#""action":"edit","kind":"track","label":"Track Edit","command":null,"reverses":null,"reversible":true,"#,
            r#""format":1,"payload":{"edits":[{"track":"10002","column":"Rating","before":0,"after":5}]}}"#,
            "\n"
        ),
    )
    .unwrap();

    ok(&sandbox, &["tracks", "rate", "10001", "2"]);

    let listed = runs(&sandbox);
    assert_eq!(listed.len(), 3);
    // Written first, in a Run of its own, with the time it was made.
    assert_eq!(listed[1]["description"], "Rate late");
    let late = entries(&sandbox, listed[1]["id"].as_i64().unwrap());
    assert_eq!(late[0]["at"], 1_700_000_000_000_i64);
    assert_eq!(late[0]["values"][0]["after"], 5);
    assert!(!sandbox.sidecar_dir().join("ledger-spill.jsonl").exists());
}

#[test]
fn nothing_is_changed_while_the_ledger_cannot_be_written() {
    let sandbox = Sandbox::new();
    sandbox.describe_track(3, "Midnight Drive", "Ada Lovelace", "8A", 2);
    // A Sidecar that cannot be opened: its database is not one.
    std::fs::create_dir_all(sandbox.sidecar_dir()).unwrap();
    std::fs::write(
        sandbox.sidecar_dir().join("sidecar.db"),
        "this is not a database, and is long enough not to pass for an empty one".repeat(40),
    )
    .unwrap();

    let refused = sandbox.cli(&["--json", "tracks", "rate", "10003", "5"]);

    assert!(!refused.status.success());
    let error = refused.error_json();
    assert_eq!(error["error"]["code"], "write_refused");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Ledger"),
        "{error}"
    );
    assert_eq!(ok(&sandbox, &["tracks", "show", "10003"])["rating"], 2);
}
