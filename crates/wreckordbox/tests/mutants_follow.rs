//! Playlist edits keep Mutants consistent (wbx-80): `wreckordbox playlists
//! remove | delete | reorder | copy` and `wreckordbox tracks remove`
//! against a Library service on a throwaway Library and Sidecar, the real
//! binary, its output, and what a second client is told.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod common;

use std::time::Duration;

use common::Sandbox;
use serde_json::Value;

fn ok(sandbox: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let run = sandbox.cli(&all);
    assert!(run.status.success(), "{args:?}: {}", run.stderr);
    assert_eq!(run.stderr, "", "{args:?}");
    run.json()
}

fn refused(sandbox: &Sandbox, args: &[&str]) -> String {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let run = sandbox.cli(&all);
    assert!(!run.status.success(), "{args:?} succeeded: {}", run.stdout);
    run.error_json()["error"]["code"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn entries(sandbox: &Sandbox, playlist: &str) -> Vec<String> {
    ok(sandbox, &["playlists", "tracks", playlist])["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|track| track["entry_id"].as_str().unwrap().to_owned())
        .collect()
}

fn mutate(sandbox: &Sandbox, entry: &str) -> i64 {
    ok(
        sandbox,
        &["mutants", "create", entry, "--source", "agent:preparer"],
    )["mutant"]["id"]
        .as_i64()
        .unwrap()
}

fn mutant(sandbox: &Sandbox, id: i64) -> Value {
    ok(sandbox, &["mutants", "show", &id.to_string()])["mutant"].clone()
}

/// The live Mutants as `(id, entry)`, optionally of one playlist.
fn live(sandbox: &Sandbox, playlist: Option<&str>) -> Vec<(i64, String)> {
    let mut args = vec!["mutants", "list"];
    if let Some(playlist) = playlist {
        args.extend_from_slice(&["--playlist", playlist]);
    }
    ok(sandbox, &args)["mutants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["id"].as_i64().unwrap(),
                m["entry_id"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn track_ids(sandbox: &Sandbox, args: &[&str]) -> Vec<String> {
    let answer = ok(sandbox, args);
    let rows = answer["tracks"]
        .as_array()
        .or_else(|| answer["rows"].as_array())
        .unwrap_or_else(|| panic!("{args:?}: {answer}"));
    rows.iter()
        .map(|row| {
            row["id"]
                .as_str()
                .or_else(|| row["track"]["id"].as_str())
                .unwrap()
                .to_owned()
        })
        .collect()
}

#[test]
fn removing_a_mutated_entry_removes_its_mutant_and_reports_it() {
    let sandbox = Sandbox::new();
    let playlist = sandbox.create_playlist("Friday", &[1, 2, 3]);
    let playlist_two = sandbox.create_playlist("Saturday", &[1, 2]);
    let entries = entries(&sandbox, &playlist);
    let removed = mutate(&sandbox, &entries[1]);
    let kept = mutate(&sandbox, &entries[2]);
    let mut watcher = sandbox.client();
    watcher.subscribe().unwrap();

    let answer = ok(
        &sandbox,
        &[
            "playlists",
            "remove",
            &playlist,
            "10002",
            "--source",
            "agent:tidier",
        ],
    );
    assert_eq!(answer["changed"], true);
    assert_eq!(answer["mutants"][0]["id"], removed);
    assert_eq!(answer["mutants"][0]["removed"], true);
    assert_eq!(answer["run"]["source"]["name"], "tidier");

    assert_eq!(live(&sandbox, None), vec![(kept, entries[2].clone())]);
    let shown = mutant(&sandbox, removed);
    assert_eq!(shown["removed"], true);
    // The Mutant's change is in the same Run as the Library edit, by its
    // source.
    let history = ok(&sandbox, &["mutants", "show", &removed.to_string()])["history"].clone();
    assert_eq!(history[1]["action"], "detached");
    assert_eq!(history[1]["run"], answer["run"]["id"]);
    assert_eq!(history[1]["source"]["name"], "tidier");

    let mut heard = Vec::new();
    while let Some(event) = watcher.next_event(Duration::from_millis(500)).unwrap() {
        heard.push((event.event.clone(), event.data["action"].clone()));
    }
    assert!(
        heard.contains(&("library_changed".to_owned(), Value::Null)),
        "{heard:?}"
    );
    assert!(
        heard.contains(&("sidecar_changed".to_owned(), Value::from("detached"))),
        "{heard:?}"
    );

    // Text output names the Mutant that went.
    let second = entries_of(&sandbox, &playlist_two);
    let other = mutate(&sandbox, &second[0]);
    let run = sandbox.cli(&["playlists", "remove", &playlist_two, "10001"]);
    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(
        common::without_run(&run.stdout).trim_end(),
        format!(
            "Took 1 track out of playlist {playlist_two}.\nMutant {other} (an entry of playlist {playlist_two} \"Saturday\", track 10001 \"Track 001\"): removed with its entry."
        )
    );
}

fn entries_of(sandbox: &Sandbox, playlist: &str) -> Vec<String> {
    entries(sandbox, playlist)
}

#[test]
fn deleting_a_playlist_removes_its_mutants() {
    let sandbox = Sandbox::new();
    let playlist = sandbox.create_playlist("Friday", &[1, 2, 3]);
    let other = sandbox.create_playlist("Saturday", &[1]);
    let entries = entries(&sandbox, &playlist);
    let first = mutate(&sandbox, &entries[0]);
    let third = mutate(&sandbox, &entries[2]);
    let elsewhere = mutate(&sandbox, &entries_of(&sandbox, &other)[0]);

    let answer = ok(
        &sandbox,
        &["playlists", "delete", &playlist, "--source", "dj"],
    );
    let mut gone: Vec<i64> = answer["mutants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_i64().unwrap())
        .collect();
    gone.sort_unstable();
    assert_eq!(gone, vec![first, third]);
    assert_eq!(
        live(&sandbox, None)
            .into_iter()
            .map(|(id, _)| id)
            .collect::<Vec<_>>(),
        vec![elsewhere]
    );
    assert_eq!(
        refused(&sandbox, &["playlists", "tracks", &playlist]),
        "playlist_not_found"
    );
}

#[test]
fn reordering_a_playlist_leaves_every_mutant_on_the_same_entry() {
    let sandbox = Sandbox::new();
    let playlist = sandbox.create_playlist("Friday", &[1, 2, 3]);
    let entries = entries(&sandbox, &playlist);
    let first = mutate(&sandbox, &entries[0]);
    let second = mutate(&sandbox, &entries[1]);

    let order = [
        entries[2].as_str(),
        entries[0].as_str(),
        entries[1].as_str(),
    ];
    let mut args = vec!["playlists", "reorder", &playlist];
    args.extend_from_slice(&order);
    let answer = ok(&sandbox, &args);
    assert_eq!(answer["changed"], true);

    assert_eq!(
        self::entries(&sandbox, &playlist),
        vec![entries[2].clone(), entries[0].clone(), entries[1].clone()]
    );
    let first_now = mutant(&sandbox, first);
    let second_now = mutant(&sandbox, second);
    assert_eq!(
        (
            first_now["entry_id"].as_str().unwrap(),
            &first_now["position"]
        ),
        (entries[0].as_str(), &Value::from(2))
    );
    assert_eq!(
        (
            second_now["entry_id"].as_str().unwrap(),
            &second_now["position"]
        ),
        (entries[1].as_str(), &Value::from(3))
    );
    assert_eq!(first_now["removed"], false);

    // A stale order changes nothing.
    assert_eq!(
        refused(&sandbox, &["playlists", "reorder", &playlist, &entries[0]]),
        "invalid_entry_order"
    );
}

/// Gives a Mutant edits, as an agent would, and answers them as stored.
fn edit(sandbox: &Sandbox, id: i64, edits: &str) -> Value {
    ok(
        sandbox,
        &[
            "mutants",
            "edit",
            &id.to_string(),
            "--edits",
            edits,
            "--source",
            "agent:preparer",
        ],
    );
    mutant(sandbox, id)["edits"].clone()
}

#[test]
fn copying_a_playlist_copies_its_mutants_with_their_edits_as_versions_of_their_own() {
    let sandbox = Sandbox::new();
    // Track 1 twice: two entries, each its own Mutant.
    let playlist = sandbox.create_playlist("Friday", &[1, 2]);
    sandbox.edit(|writer| {
        writer
            .add_tracks_allowing(&playlist, &["10001".to_owned()], true)
            .unwrap();
    });
    let entries = entries(&sandbox, &playlist);
    assert_eq!(entries.len(), 3);
    let second = mutate(&sandbox, &entries[1]);
    let again = mutate(&sandbox, &entries[2]);
    let second_edits = edit(&sandbox, second, r#"{"key_shift":2}"#);
    let again_edits = edit(&sandbox, again, r#"{"tempo":1.04}"#);

    let copied = ok(
        &sandbox,
        &["playlists", "copy", &playlist, "--name", "Friday 2"],
    );
    let copy = copied["playlist"].as_str().unwrap().to_owned();
    assert_eq!(copied["name"], "Friday 2");
    assert_eq!(copied["tracks"], 3);

    let copy_entries = self::entries(&sandbox, &copy);
    assert_eq!(copy_entries.len(), 3);
    let in_copy = live(&sandbox, Some(&copy));
    assert_eq!(
        in_copy
            .iter()
            .map(|(_, entry)| entry.clone())
            .collect::<Vec<_>>(),
        vec![copy_entries[1].clone(), copy_entries[2].clone()]
    );
    let (copy_second, copy_again) = (in_copy[0].0, in_copy[1].0);
    assert!(![second, again].contains(&copy_second) && ![second, again].contains(&copy_again));
    let shown = mutant(&sandbox, copy_second);
    assert_eq!(shown["parent"]["id"], "10002");
    assert_eq!(shown["position"], 2);
    assert_eq!(shown["edits"], second_edits);
    let shown = mutant(&sandbox, copy_again);
    assert_eq!(shown["parent"]["id"], "10001");
    assert_eq!(shown["position"], 3);
    assert_eq!(shown["edits"], again_edits);
    assert_eq!(
        ok(&sandbox, &["mutants", "show", &copy_second.to_string()])["history"][0]["action"],
        "copied"
    );

    // Each version is edited on its own.
    let copy_now = edit(&sandbox, copy_second, r#"{"key_shift":-4}"#);
    assert_eq!(mutant(&sandbox, second)["edits"], second_edits);
    let source_now = edit(&sandbox, second, r#"{"key_shift":5}"#);
    assert_eq!(mutant(&sandbox, copy_second)["edits"], copy_now);
    assert_ne!(copy_now, source_now);
    assert_eq!(live(&sandbox, Some(&playlist)).len(), 2);
}

#[test]
fn a_mutant_is_never_a_track_in_all_tracks_or_a_search() {
    let sandbox = Sandbox::new();
    let playlist = sandbox.create_playlist("Friday", &[1, 2]);
    let before_all = track_ids(&sandbox, &["tracks", "list", "--limit", "1000"]);
    let before_found = track_ids(&sandbox, &["tracks", "query", "--title", "Track 002"]);
    let entries = entries(&sandbox, &playlist);
    mutate(&sandbox, &entries[1]);

    assert_eq!(
        track_ids(&sandbox, &["tracks", "list", "--limit", "1000"]),
        before_all
    );
    assert_eq!(
        track_ids(&sandbox, &["tracks", "query", "--title", "Track 002"]),
        before_found
    );
    assert_eq!(
        track_ids(&sandbox, &["tracks", "list", "--filter", "Track 002"]),
        vec!["10002".to_owned()]
    );
    // The playlist still lists the parent track at the entry.
    assert_eq!(
        ok(&sandbox, &["playlists", "tracks", &playlist])["total"],
        2
    );
}

#[test]
fn removing_a_track_from_the_collection_reports_its_mutants_first() {
    let sandbox = Sandbox::new();
    let friday = sandbox.create_playlist("Friday", &[1, 2]);
    let saturday = sandbox.create_playlist("Saturday", &[2, 3]);
    let on_friday = mutate(&sandbox, &entries(&sandbox, &friday)[1]);
    let on_saturday = mutate(&sandbox, &entries(&sandbox, &saturday)[0]);
    let unrelated = mutate(&sandbox, &entries(&sandbox, &saturday)[1]);

    // The Mutants a track has.
    let of_track = ok(&sandbox, &["mutants", "list", "--track", "10002"])["mutants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_i64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(of_track, vec![on_friday, on_saturday]);

    // Asked without --confirm: the report, and nothing removed.
    let report = ok(&sandbox, &["tracks", "remove", "10002"]);
    assert_eq!(report["removed"], false);
    let reported: Vec<i64> = report["mutants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_i64().unwrap())
        .collect();
    assert_eq!(reported, vec![on_friday, on_saturday]);
    assert_eq!(live(&sandbox, None).len(), 3);
    assert!(
        track_ids(&sandbox, &["tracks", "list", "--limit", "1000"]).contains(&"10002".to_owned())
    );
    let text = sandbox.cli(&["tracks", "remove", "10002"]);
    assert!(text.status.success());
    assert!(
        text.stdout
            .contains("Mutants that would be removed with them:")
            && text.stdout.contains(&format!(
                "Mutant {on_friday} (entry 2 of playlist {friday} \"Friday\""
            ))
            && text.stdout.contains("Nothing was removed."),
        "{}",
        text.stdout
    );

    let removed = ok(
        &sandbox,
        &["tracks", "remove", "10002", "--confirm", "--source", "dj"],
    );
    assert_eq!(removed["removed"], true);
    assert_eq!(removed["mutants"][0]["removed"], true);
    assert_eq!(
        live(&sandbox, None)
            .into_iter()
            .map(|(id, _)| id)
            .collect::<Vec<_>>(),
        vec![unrelated]
    );
    assert!(
        !track_ids(&sandbox, &["tracks", "list", "--limit", "1000"]).contains(&"10002".to_owned())
    );
}

#[test]
fn library_protection_refuses_a_playlist_edit_and_keeps_every_mutant() {
    let sandbox = Sandbox::new();
    let playlist = sandbox.create_playlist("Friday", &[1, 2]);
    let entries = entries(&sandbox, &playlist);
    let id = mutate(&sandbox, &entries[1]);
    sandbox.protect(true);

    for args in [
        vec!["playlists", "remove", playlist.as_str(), "10002"],
        vec!["playlists", "delete", playlist.as_str()],
        vec!["playlists", "copy", playlist.as_str()],
        vec!["tracks", "remove", "10002", "--confirm"],
    ] {
        assert_eq!(refused(&sandbox, &args), "library_protected", "{args:?}");
    }
    assert_eq!(live(&sandbox, None), vec![(id, entries[1].clone())]);
    assert_eq!(self::entries(&sandbox, &playlist), entries);
    assert_eq!(
        ok(&sandbox, &["playlists", "tracks", &playlist])["total"],
        2
    );
    // Reading never writes: a report changes nothing either.
    assert_eq!(
        ok(&sandbox, &["tracks", "remove", "10002"])["removed"],
        false
    );
}

const FAULTS: &str = "WRECKORDBOX_LEDGER_FAULTS";

/// Stops the service running here and waits until it is gone.
fn stop_service(sandbox: &Sandbox) {
    let Ok(mut client) =
        wreckordbox_service::Client::connect(&sandbox.config().socket_path(sandbox.location()))
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

fn with_faults(sandbox: &Sandbox, faults: &str, args: &[&str]) -> common::Run {
    let mut command = sandbox.command();
    command.env(FAULTS, faults).arg("--json").args(args);
    sandbox.run(&mut command)
}

#[test]
fn a_service_killed_between_a_removal_and_its_entry_still_removes_the_mutant_when_it_recovers() {
    let sandbox = Sandbox::new();
    let playlist = sandbox.create_playlist("Friday", &[1, 2]);
    let entries = entries(&sandbox, &playlist);
    let removed = mutate(&sandbox, &entries[1]);
    let kept = mutate(&sandbox, &entries[0]);
    stop_service(&sandbox);

    // The service dies the moment the entry is out of the Library, before
    // the Ledger records the change or its Mutant's consequence.
    let killed = with_faults(
        &sandbox,
        "abort_after_write",
        &["playlists", "remove", &playlist, "10002"],
    );
    assert!(!killed.status.success());
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while wreckordbox_service::Client::connect(&sandbox.config().socket_path(sandbox.location()))
        .and_then(|mut client| client.status())
        .is_ok()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the service did not die"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    // The next service records the change as unconfirmed and brings the
    // Mutants into line with the Library as it is, in that entry's Run.
    assert_eq!(live(&sandbox, None), vec![(kept, entries[0].clone())]);
    let history = ok(&sandbox, &["mutants", "show", &removed.to_string()])["history"].clone();
    assert_eq!(history[1]["action"], "detached");
    let run = ok(&sandbox, &["runs", "show", &history[1]["run"].to_string()]);
    assert_eq!(run["entries"][0]["kind"], "unconfirmed", "{run}");
}

#[test]
fn a_removal_whose_entry_had_to_wait_removes_its_mutant_when_the_entry_is_written() {
    let sandbox = Sandbox::new();
    let playlist = sandbox.create_playlist("Friday", &[1, 2]);
    let entries = entries(&sandbox, &playlist);
    let removed = mutate(&sandbox, &entries[1]);
    stop_service(&sandbox);

    // The Sidecar refuses the entry: it is kept, with its consequence.
    let run = with_faults(
        &sandbox,
        "fail_append=1",
        &["playlists", "remove", &playlist, "10002"],
    );
    assert!(run.status.success(), "{}", run.stderr);
    assert!(sandbox.sidecar_dir().join("ledger-spill.jsonl").is_file());
    stop_service(&sandbox);

    // The next service writes the entry and the Mutant's change together.
    assert!(live(&sandbox, None).is_empty());
    let history = ok(&sandbox, &["mutants", "show", &removed.to_string()])["history"].clone();
    assert_eq!(history.as_array().unwrap().len(), 2, "{history}");
    assert_eq!(history[1]["action"], "detached");
    assert!(!sandbox.sidecar_dir().join("ledger-spill.jsonl").exists());
}

#[test]
fn a_service_killed_between_a_copy_and_its_entry_still_copies_the_mutants_when_it_recovers() {
    let sandbox = Sandbox::new();
    let playlist = sandbox.create_playlist("Friday", &[1, 2]);
    let entries = entries(&sandbox, &playlist);
    let source = mutate(&sandbox, &entries[1]);
    let edits = edit(&sandbox, source, r#"{"key_shift":2}"#);
    stop_service(&sandbox);

    // The service dies the moment the copy is in the Library, before the
    // Ledger records it or copies its Mutants.
    let killed = with_faults(
        &sandbox,
        "abort_after_write",
        &["playlists", "copy", &playlist, "--name", "Saved"],
    );
    assert!(!killed.status.success());
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while wreckordbox_service::Client::connect(&sandbox.config().socket_path(sandbox.location()))
        .and_then(|mut client| client.status())
        .is_ok()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the service did not die"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    // The next service finds the copy and gives it its Mutant, with the
    // edits the source had, on the copy's entry of the same place.
    let tree = ok(&sandbox, &["playlists", "tree"]);
    let copy = tree["playlists"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["name"] == "Saved")
        .unwrap_or_else(|| panic!("no copy in {tree}"))["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let copy_entries = self::entries(&sandbox, &copy);
    let in_copy = live(&sandbox, Some(&copy));
    assert_eq!(in_copy.len(), 1, "{in_copy:?}");
    assert_eq!(in_copy[0].1, copy_entries[1]);
    let copied = mutant(&sandbox, in_copy[0].0);
    assert_eq!(copied["edits"], edits);
    assert_eq!(
        ok(&sandbox, &["mutants", "show", &in_copy[0].0.to_string()])["history"][0]["action"],
        "copied"
    );
    // Recovered once: a restart changes nothing more.
    stop_service(&sandbox);
    assert_eq!(live(&sandbox, Some(&copy)).len(), 1);
}
