//! `wreckordbox mutants` against a Library service on a throwaway Library
//! and Sidecar: the real binary, its output and exit status, and what a
//! second connected client is told.
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

/// Runs a command that must succeed and answers its JSON result.
fn ok(sandbox: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let run = sandbox.cli(&all);
    assert!(run.status.success(), "{args:?}: {}", run.stderr);
    assert_eq!(run.stderr, "", "{args:?}");
    run.json()
}

/// Runs a command that must fail and answers its error code.
fn refused(sandbox: &Sandbox, args: &[&str]) -> String {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let run = sandbox.cli(&all);
    assert!(!run.status.success(), "{args:?} succeeded: {}", run.stdout);
    assert_eq!(run.stdout, "", "{args:?}");
    run.error_json()["error"]["code"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// The entries of a playlist as `playlists tracks` lists them.
fn entries(sandbox: &Sandbox, playlist: &str) -> Vec<Value> {
    ok(sandbox, &["playlists", "tracks", playlist])["tracks"]
        .as_array()
        .unwrap()
        .clone()
}

/// The identifier of the entry at `position` (from 1) of a playlist.
fn entry_at(sandbox: &Sandbox, playlist: &str, position: usize) -> String {
    entries(sandbox, playlist)[position - 1]["entry_id"]
        .as_str()
        .expect("a playlist's track carries its entry's identifier")
        .to_owned()
}

/// Mutates an entry as the agent `preparer`.
fn mutate(sandbox: &Sandbox, entry: &str) -> Value {
    ok(
        sandbox,
        &["mutants", "create", entry, "--source", "agent:preparer"],
    )
}

/// The Mutants `mutants list` answers, for one playlist or them all.
fn listed(sandbox: &Sandbox, playlist: Option<&str>) -> Vec<Value> {
    let mut args = vec!["mutants", "list"];
    if let Some(playlist) = playlist {
        args.extend_from_slice(&["--playlist", playlist]);
    }
    ok(sandbox, &args)["mutants"].as_array().unwrap().clone()
}

fn ids(mutants: &[Value]) -> Vec<i64> {
    mutants
        .iter()
        .map(|mutant| mutant["id"].as_i64().unwrap())
        .collect()
}

#[test]
fn a_playlist_entry_is_mutated_and_the_mutant_read_back_with_its_parent_source_and_run() {
    let sandbox = Sandbox::new();
    sandbox.describe_track(3, "Midnight Drive", "Ada Lovelace", "8A", 0);
    let playlist = sandbox.create_playlist("Friday", &[2, 3, 4]);
    let entry = entry_at(&sandbox, &playlist, 2);

    let created = ok(
        &sandbox,
        &[
            "mutants",
            "create",
            &entry,
            "--source",
            "agent:preparer",
            "--description",
            "Prepare Friday",
        ],
    );

    let mutant = &created["mutant"];
    let id = mutant["id"].as_i64().unwrap();
    assert_eq!(mutant["entry_id"], entry.as_str());
    assert_eq!(mutant["position"], 2);
    assert_eq!(
        mutant["playlist"],
        json!({ "id": playlist, "name": "Friday" })
    );
    assert_eq!(
        mutant["parent"],
        json!({ "id": "10003", "title": "Midnight Drive" })
    );
    // Nothing is edited yet: the Mutant is its parent as it stands.
    assert_eq!(mutant["edits"], json!({}));
    assert_eq!(
        mutant["source"],
        json!({ "kind": "agent", "name": "preparer" })
    );
    assert_eq!(created["created_run"], true);
    assert_eq!(created["run"]["description"], "Prepare Friday");
    assert_eq!(mutant["run"], created["run"]["id"]);

    // Read back by its own identifier and by its entry's.
    let shown = ok(&sandbox, &["mutants", "show", &id.to_string()]);
    assert_eq!(shown["mutant"], *mutant);
    let by_entry = ok(&sandbox, &["mutants", "show", "--entry", &entry]);
    assert_eq!(by_entry["mutant"], *mutant);
    // What happened to it, each change with who made it and in which Run.
    let history = shown["history"].as_array().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["action"], "created");
    assert_eq!(history[0]["run"], created["run"]["id"]);
    assert_eq!(history[0]["source"], mutant["source"]);

    // As text, a person can read which entry it is.
    let text = sandbox.cli(&["mutants", "show", &id.to_string()]);
    assert!(text.status.success(), "{}", text.stderr);
    assert!(text.stdout.contains("Midnight Drive"), "{}", text.stdout);
    assert!(text.stdout.contains("Friday"), "{}", text.stdout);
}

/// Stops the service the sandbox's commands started and waits until it is
/// gone; the next command starts another.
fn stop_service(sandbox: &Sandbox) {
    let pid = sandbox.status()["pid"].as_u64().unwrap();
    sandbox.client().stop().unwrap();
    let gone = (0..400).any(|_| {
        std::thread::sleep(Duration::from_millis(25));
        !common::alive(pid)
    });
    assert!(gone, "the service did not stop");
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

/// Every row of the Library's playlist entries, deleted ones included.
fn entry_rows(sandbox: &Sandbox) -> Vec<String> {
    let mut found = Vec::new();
    sandbox.edit(|writer| {
        let db = writer.library().connection();
        let mut rows = db
            .prepare(
                "SELECT quote(ID) || quote(PlaylistID) || quote(ContentID) || quote(TrackNo)
                     || quote(rb_local_deleted) || quote(rb_local_usn) || quote(updated_at)
                 FROM djmdSongPlaylist ORDER BY ID",
            )
            .unwrap();
        found = rows
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
    });
    found
}

#[test]
fn mutants_are_listed_for_one_playlist_and_across_the_library() {
    let sandbox = Sandbox::new();
    let friday = sandbox.create_playlist("Friday", &[1, 2, 3]);
    let sunday = sandbox.create_playlist("Sunday", &[4, 5]);
    assert_eq!(listed(&sandbox, None), Vec::<Value>::new());

    let third = mutate(&sandbox, &entry_at(&sandbox, &friday, 3));
    let other = mutate(&sandbox, &entry_at(&sandbox, &sunday, 1));
    let first = mutate(&sandbox, &entry_at(&sandbox, &friday, 1));
    let id = |created: &Value| created["mutant"]["id"].as_i64().unwrap();

    // One playlist's, in the playlist's order.
    let of_friday = listed(&sandbox, Some(&friday));
    assert_eq!(ids(&of_friday), [id(&first), id(&third)]);
    assert_eq!(of_friday[0], first["mutant"]);
    assert_eq!(of_friday[1], third["mutant"]);
    assert_eq!(ids(&listed(&sandbox, Some(&sunday))), [id(&other)]);

    // The whole Library's, playlist by playlist.
    let all = ok(&sandbox, &["mutants", "list"]);
    assert_eq!(all["total"], 3);
    assert_eq!(
        ids(all["mutants"].as_array().unwrap()),
        [id(&first), id(&third), id(&other)]
    );

    // Those of one track.
    let of_track = ok(&sandbox, &["mutants", "list", "--track", "10004"]);
    assert_eq!(ids(of_track["mutants"].as_array().unwrap()), [id(&other)]);

    // A playlist with none, and one that is not there, list nothing.
    let empty = sandbox.cli(&["mutants", "list", "--playlist", "424242"]);
    assert!(empty.status.success(), "{}", empty.stderr);
    assert_eq!(empty.stdout.trim(), "No Mutants.");

    let table = sandbox.cli(&["mutants", "list"]);
    assert!(table.status.success(), "{}", table.stderr);
    let lines: Vec<&str> = table.stdout.lines().collect();
    assert_eq!(lines.len(), 4, "{}", table.stdout);
    assert!(lines[0].starts_with("MUTANT"), "{}", table.stdout);
    assert!(lines[1].contains("Friday"), "{}", table.stdout);
    assert!(lines[3].contains("Sunday"), "{}", table.stdout);
}

#[test]
fn the_same_track_in_two_playlists_or_twice_in_one_carries_different_mutants() {
    let sandbox = Sandbox::new();
    let friday = sandbox.create_playlist("Friday", &[7, 8]);
    let sunday = sandbox.create_playlist("Sunday", &[7]);
    // Track 7 a second time in Friday.
    sandbox.edit(|writer| {
        writer
            .add_tracks_allowing(&friday, &["10007".to_owned()], true)
            .unwrap();
    });
    let in_friday = entries(&sandbox, &friday);
    let tracks: Vec<&str> = in_friday
        .iter()
        .map(|entry| entry["id"].as_str().unwrap())
        .collect();
    assert_eq!(tracks, ["10007", "10008", "10007"]);
    let first = in_friday[0]["entry_id"].as_str().unwrap().to_owned();
    let again = in_friday[2]["entry_id"].as_str().unwrap().to_owned();
    let elsewhere = entry_at(&sandbox, &sunday, 1);
    assert_ne!(first, again);

    // Mutating one entry of the track leaves its other entries the core
    // track.
    let one = mutate(&sandbox, &first);
    assert_eq!(
        refused(&sandbox, &["mutants", "show", "--entry", &again]),
        "mutant_not_found"
    );
    assert_eq!(
        refused(&sandbox, &["mutants", "show", "--entry", &elsewhere]),
        "mutant_not_found"
    );

    // Each of the others takes a Mutant of its own.
    let two = mutate(&sandbox, &again);
    let three = ok(
        &sandbox,
        &[
            "mutants",
            "create",
            "--playlist",
            &sunday,
            "--position",
            "1",
            "--source",
            "dj",
        ],
    );
    let mutants = [&one["mutant"], &two["mutant"], &three["mutant"]];
    let mut distinct = ids(&mutants.map(Clone::clone));
    distinct.dedup();
    assert_eq!(distinct.len(), 3);
    for mutant in mutants {
        assert_eq!(mutant["parent"]["id"], "10007");
    }
    assert_eq!(one["mutant"]["position"], 1);
    assert_eq!(two["mutant"]["position"], 3);
    assert_eq!(three["mutant"]["entry_id"], elsewhere.as_str());
    assert_eq!(
        three["mutant"]["source"],
        json!({ "kind": "dj", "name": "dj" })
    );
    assert_eq!(
        ids(
            &ok(&sandbox, &["mutants", "list", "--track", "10007"])["mutants"]
                .as_array()
                .unwrap()
                .clone()
        )
        .len(),
        3
    );
}

#[test]
fn removing_a_mutant_returns_the_entry_to_the_core_track() {
    let sandbox = Sandbox::new();
    let playlist = sandbox.create_playlist("Friday", &[2, 3]);
    let entry = entry_at(&sandbox, &playlist, 2);
    let before = entries(&sandbox, &playlist);
    let created = mutate(&sandbox, &entry);
    let id = created["mutant"]["id"].as_i64().unwrap().to_string();

    let removed = ok(&sandbox, &["mutants", "remove", &id, "--source", "dj"]);

    assert_eq!(removed["mutant"]["id"], created["mutant"]["id"]);
    assert_eq!(removed["mutant"]["removed"], true);
    assert_eq!(removed["mutant"]["position"], 2);
    assert_eq!(
        removed["run"]["source"],
        json!({ "kind": "dj", "name": "dj" })
    );
    // The entry has no Mutant, no list holds one, and the playlist is as
    // it was: the entry is the track.
    assert_eq!(
        refused(&sandbox, &["mutants", "show", "--entry", &entry]),
        "mutant_not_found"
    );
    assert_eq!(listed(&sandbox, None), Vec::<Value>::new());
    assert_eq!(entries(&sandbox, &playlist), before);
    assert_eq!(before[1]["id"], "10003");

    // What was removed can still be read, with who removed it.
    let shown = ok(&sandbox, &["mutants", "show", &id]);
    assert_eq!(shown["mutant"]["removed"], true);
    let actions: Vec<&str> = shown["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|change| change["action"].as_str().unwrap())
        .collect();
    assert_eq!(actions, ["created", "removed"]);
    assert_eq!(
        shown["history"][1]["source"],
        json!({ "kind": "dj", "name": "dj" })
    );
    // It cannot be removed twice.
    assert_eq!(
        refused(&sandbox, &["mutants", "remove", &id, "--source", "dj"]),
        "mutant_not_found"
    );

    // The entry can be mutated afresh, and removed by naming the entry.
    let afresh = mutate(&sandbox, &entry);
    assert_ne!(afresh["mutant"]["id"], created["mutant"]["id"]);
    let text = sandbox.cli(&["mutants", "remove", "--entry", &entry, "--source", "dj"]);
    assert!(text.status.success(), "{}", text.stderr);
    assert!(
        text.stdout.starts_with("Removed Mutant "),
        "{}",
        text.stdout
    );
    assert_eq!(listed(&sandbox, None), Vec::<Value>::new());
}

#[test]
fn mutating_what_is_already_a_mutant_or_is_not_there_fails_with_a_stable_code_and_changes_nothing()
{
    let sandbox = Sandbox::new();
    let playlist = sandbox.create_playlist("Friday", &[2, 3]);
    let genre = sandbox.create_genre_playlist("Dub", "Dub");
    let entry = entry_at(&sandbox, &playlist, 1);
    let who = ["--source", "agent:preparer"];
    let create = |args: &[&str]| {
        let mut all = vec!["mutants", "create"];
        all.extend_from_slice(args);
        all.extend_from_slice(&who);
        refused(&sandbox, &all)
    };

    // No such entry: an identifier the Library does not have, a position
    // past the end, a playlist with no entries of its own.
    assert_eq!(create(&["no-such-entry"]), "playlist_entry_not_found");
    assert_eq!(
        create(&["--playlist", &playlist, "--position", "3"]),
        "playlist_entry_not_found"
    );
    assert_eq!(
        create(&["--playlist", &genre, "--position", "1"]),
        "playlist_entry_not_found"
    );
    assert_eq!(
        create(&["--playlist", "424242", "--position", "1"]),
        "playlist_not_found"
    );
    // Nobody to hold to account.
    assert_eq!(
        refused(&sandbox, &["mutants", "create", &entry]),
        "invalid_source"
    );
    assert_eq!(
        refused(&sandbox, &["mutants", "create", &entry, "--run", "99"]),
        "run_not_found"
    );
    // No Mutant to read or remove.
    assert_eq!(
        refused(&sandbox, &["mutants", "show", "99"]),
        "mutant_not_found"
    );
    assert_eq!(
        refused(&sandbox, &["mutants", "remove", "99", "--source", "dj"]),
        "mutant_not_found"
    );
    assert_eq!(listed(&sandbox, None), Vec::<Value>::new());

    // Already a Mutant, however the entry is named.
    let created = mutate(&sandbox, &entry);
    assert_eq!(create(&[&entry]), "already_a_mutant");
    assert_eq!(
        create(&["--playlist", &playlist, "--position", "1"]),
        "already_a_mutant"
    );
    let failure = sandbox.cli(&["mutants", "create", &entry, "--source", "dj"]);
    assert!(!failure.status.success());
    assert!(
        failure.stderr.contains("already a Mutant"),
        "{}",
        failure.stderr
    );
    let all = listed(&sandbox, None);
    assert_eq!(all, vec![created["mutant"].clone()]);

    // A refused command made no Run: the next Run follows the Mutant's.
    let next = ok(
        &sandbox,
        &["runs", "start", "--source", "dj", "--description", "Next"],
    );
    assert_eq!(
        next["id"].as_i64().unwrap(),
        created["run"]["id"].as_i64().unwrap() + 1
    );
}

#[test]
fn a_mutant_made_in_a_run_of_ones_own_joins_it_and_leaves_it_open() {
    let sandbox = Sandbox::new();
    let playlist = sandbox.create_playlist("Friday", &[2, 3]);
    let run = ok(
        &sandbox,
        &[
            "runs",
            "start",
            "--source",
            "agent:preparer",
            "--description",
            "Prepare Friday",
        ],
    );
    let run_id = run["id"].as_i64().unwrap().to_string();

    let first = ok(
        &sandbox,
        &[
            "mutants",
            "create",
            &entry_at(&sandbox, &playlist, 1),
            "--run",
            &run_id,
        ],
    );
    let second = ok(
        &sandbox,
        &[
            "mutants",
            "create",
            &entry_at(&sandbox, &playlist, 2),
            "--run",
            &run_id,
        ],
    );

    for created in [&first, &second] {
        assert_eq!(created["created_run"], false);
        assert_eq!(created["run"]["id"], run["id"]);
        assert_eq!(created["run"]["finished_at"], Value::Null);
        assert_eq!(created["mutant"]["run"], run["id"]);
        assert_eq!(
            created["mutant"]["source"],
            json!({ "kind": "agent", "name": "preparer" })
        );
    }
    ok(&sandbox, &["runs", "finish", &run_id]);
    let id = second["mutant"]["id"].as_i64().unwrap().to_string();
    assert_eq!(
        refused(&sandbox, &["mutants", "remove", &id, "--run", &run_id]),
        "run_finished"
    );
}

#[test]
fn a_mutant_survives_a_restart_of_the_service() {
    let sandbox = Sandbox::new();
    let playlist = sandbox.create_playlist("Friday", &[2, 3]);
    let created = mutate(&sandbox, &entry_at(&sandbox, &playlist, 2));
    let id = created["mutant"]["id"].as_i64().unwrap().to_string();
    let shown = ok(&sandbox, &["mutants", "show", &id]);
    let before = sandbox.status()["pid"].clone();

    stop_service(&sandbox);

    assert_eq!(ok(&sandbox, &["mutants", "show", &id]), shown);
    assert_eq!(
        listed(&sandbox, Some(&playlist)),
        vec![created["mutant"].clone()]
    );
    assert_ne!(sandbox.status()["pid"], before);
}

#[test]
fn a_mutant_stays_with_its_entry_when_the_playlist_is_reordered() {
    let sandbox = Sandbox::new();
    let playlist = sandbox.create_playlist("Friday", &[2, 3, 4]);
    let entry = entry_at(&sandbox, &playlist, 1);
    let created = mutate(&sandbox, &entry);
    let id = created["mutant"]["id"].as_i64().unwrap().to_string();
    assert_eq!(created["mutant"]["position"], 1);

    // The window reorders the playlist: track 10002 goes last.
    stop_service(&sandbox);
    sandbox.edit(|writer| {
        let order = ["10003", "10004", "10002"].map(str::to_owned);
        writer.reorder(&playlist, &order).unwrap();
    });

    assert_eq!(entry_at(&sandbox, &playlist, 3), entry);
    let moved = ok(&sandbox, &["mutants", "show", &id]);
    assert_eq!(moved["mutant"]["position"], 3);
    assert_eq!(moved["mutant"]["entry_id"], entry.as_str());
    assert_eq!(moved["mutant"]["parent"]["id"], "10002");
    // The entry that is now first is not a Mutant.
    assert_eq!(
        refused(
            &sandbox,
            &[
                "mutants",
                "show",
                "--entry",
                &entry_at(&sandbox, &playlist, 1)
            ]
        ),
        "mutant_not_found"
    );
}

#[test]
fn an_entry_keeps_its_identity_through_a_removal_undone_and_a_track_added_again_is_a_new_entry() {
    let sandbox = Sandbox::new();
    let playlist = sandbox.create_playlist("Friday", &[2, 3, 4]);
    let entry = entry_at(&sandbox, &playlist, 2);
    let created = mutate(&sandbox, &entry);
    let id = created["mutant"]["id"].as_i64().unwrap().to_string();

    // The entry is removed and the removal undone: the same entry is back,
    // and it is still the Mutant.
    stop_service(&sandbox);
    let track = ["10003".to_owned()];
    let mut removal = None;
    sandbox.edit(|writer| {
        removal = Some(writer.remove_tracks_with_undo(&playlist, &track).unwrap().1);
    });
    assert_eq!(entries(&sandbox, &playlist).len(), 2);
    let orphan = ok(&sandbox, &["mutants", "show", &id]);
    assert_eq!(orphan["mutant"]["position"], Value::Null);
    assert_eq!(
        refused(
            &sandbox,
            &["mutants", "create", &entry, "--source", "agent:preparer"]
        ),
        "playlist_entry_not_found"
    );
    stop_service(&sandbox);
    sandbox.edit(|writer| {
        writer
            .undo_track_removal(removal.as_ref().unwrap())
            .unwrap();
    });
    assert_eq!(entry_at(&sandbox, &playlist, 2), entry);
    assert_eq!(
        ok(&sandbox, &["mutants", "show", "--entry", &entry])["mutant"]["position"],
        2
    );

    // Removed and then added afresh, the track is a new entry, which is
    // not a Mutant.
    stop_service(&sandbox);
    sandbox.edit(|writer| {
        writer.remove_tracks(&playlist, &track).unwrap();
        writer.add_tracks(&playlist, &track).unwrap();
    });
    let again = entry_at(&sandbox, &playlist, 3);
    assert_eq!(entries(&sandbox, &playlist)[2]["id"], "10003");
    assert_ne!(again, entry);
    assert_eq!(
        refused(&sandbox, &["mutants", "show", "--entry", &again]),
        "mutant_not_found"
    );
}

#[test]
fn a_mutant_records_the_fingerprint_of_its_parents_audio() {
    let sandbox = Sandbox::new();
    let track = sandbox.import_track("Night Bus.wav", 1);
    let playlist = sandbox.create_playlist("Friday", &[2]);
    sandbox.edit(|writer| {
        writer
            .add_tracks(&playlist, std::slice::from_ref(&track))
            .unwrap();
    });

    // A fixture track has no audio file: it is mutated all the same, with
    // no fingerprint.
    let silent = mutate(&sandbox, &entry_at(&sandbox, &playlist, 1));
    assert_eq!(silent["mutant"]["fingerprint"], Value::Null);

    let created = mutate(&sandbox, &entry_at(&sandbox, &playlist, 2));
    let fingerprint = created["mutant"]["fingerprint"].as_str().unwrap();
    assert!(fingerprint.starts_with("a1:"), "{fingerprint}");
    // The fingerprint every other Sidecar command gives the same audio.
    let fact = ok(
        &sandbox,
        &["facts", "record", &track, "genre", "Dub", "--source", "dj"],
    );
    assert_eq!(fact["facts"][0]["fingerprint"], fingerprint);
}

#[test]
fn creating_and_removing_a_mutant_changes_nothing_in_the_library() {
    let sandbox = Sandbox::new();
    let track = sandbox.import_track("Night Bus.wav", 1);
    let playlist = sandbox.create_playlist("Friday", &[2, 3]);
    sandbox.edit(|writer| {
        writer
            .add_tracks(&playlist, std::slice::from_ref(&track))
            .unwrap();
    });
    let shape = library_shape(&sandbox);
    let rows = entry_rows(&sandbox);
    let listed_before = entries(&sandbox, &playlist);

    let entry = entry_at(&sandbox, &playlist, 3);
    let created = mutate(&sandbox, &entry);
    assert_eq!(created["mutant"]["parent"]["id"], track.as_str());
    // The Mutant is in the Sidecar's folder, which is not the Library's.
    assert!(sandbox.sidecar_dir().join("sidecar.db").is_file());

    // The playlist's entry is still the parent track, in the same place,
    // and no rekordbox-format table gained a table, a column or a row; the
    // entries' own rows are untouched.
    assert_eq!(entries(&sandbox, &playlist), listed_before);
    assert_eq!(listed_before[2]["id"], track.as_str());
    stop_service(&sandbox);
    assert_eq!(library_shape(&sandbox), shape);
    assert_eq!(entry_rows(&sandbox), rows);

    // Nor does removing it.
    let id = created["mutant"]["id"].as_i64().unwrap().to_string();
    let removed = ok(
        &sandbox,
        &["mutants", "remove", &id, "--source", "agent:preparer"],
    );
    assert_eq!(removed["mutant"]["removed"], true);
    assert_eq!(entries(&sandbox, &playlist), listed_before);
    stop_service(&sandbox);
    assert_eq!(library_shape(&sandbox), shape);
    assert_eq!(entry_rows(&sandbox), rows);
}

#[test]
fn a_second_client_is_told_of_mutant_changes_as_sidecar_changes_not_library_changes() {
    let sandbox = Sandbox::new();
    let playlist = sandbox.create_playlist("Friday", &[2, 3]);
    let entry = entry_at(&sandbox, &playlist, 2);
    let mut watcher = sandbox.client();
    watcher.subscribe().unwrap();

    let created = mutate(&sandbox, &entry);
    let id = created["mutant"]["id"].as_i64().unwrap();
    let removed = ok(
        &sandbox,
        &["mutants", "remove", &id.to_string(), "--source", "dj"],
    );
    assert_eq!(removed["mutant"]["removed"], true);

    let mut events = Vec::new();
    while let Some(event) = watcher.next_event(Duration::from_millis(500)).unwrap() {
        events.push(event);
    }
    let heard: Vec<(&str, Value)> = events
        .iter()
        .map(|event| (event.event.as_str(), event.data.clone()))
        .collect();
    let changed = |action: &str| {
        (
            "sidecar_changed",
            json!({
                "track_id": "10003", "kind": "mutant", "fact_id": null, "stale": [],
                "mutant_id": id, "playlist_id": playlist, "entry_id": entry, "action": action,
            }),
        )
    };
    assert_eq!(heard, [changed("created"), changed("removed")]);
}
