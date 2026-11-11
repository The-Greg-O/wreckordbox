//! `wreckordbox tracks list` against a Library service on a throwaway
//! Library: the real binary, its output and its exit status.
#![allow(clippy::pedantic, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::Sandbox;

/// The fixture's 40 tracks, three of them given the details a DJ would
/// search by.
fn described() -> Sandbox {
    let sandbox = Sandbox::new();
    sandbox.describe_track(3, "Midnight Drive", "Ada Lovelace", "8A", 4);
    sandbox.describe_track(7, "Sunrise", "Ada Lovelace", "11B", 0);
    sandbox.describe_track(12, "Midnight Oil", "Grace Hopper", "8A", 5);
    sandbox
}

#[test]
fn tracks_list_prints_a_table_of_the_tracks_in_the_library() {
    let sandbox = described();

    let run = sandbox.cli(&["tracks", "list"]);

    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(run.stderr, "");
    let lines: Vec<&str> = run.stdout.lines().collect();
    let header: Vec<&str> = lines[0].split_whitespace().collect();
    assert_eq!(header, ["ID", "TITLE", "ARTIST", "BPM", "KEY", "RATING"]);
    // Fixture track 3 is id 10003 at 128.03 BPM.
    let row = lines.iter().find(|line| line.starts_with("10003")).expect(&run.stdout);
    let cells: Vec<&str> = row.split("  ").map(str::trim).filter(|cell| !cell.is_empty()).collect();
    assert_eq!(cells, ["10003", "Midnight Drive", "Ada Lovelace", "128.03", "8A", "4"]);
    // A track nobody described still has a row.
    assert!(lines.iter().any(|line| line.starts_with("10000") && line.contains("Track 000")), "{}", run.stdout);
}

#[test]
fn tracks_list_as_json_has_the_same_rows_under_stable_names() {
    let sandbox = described();

    let run = sandbox.cli(&["tracks", "list", "--json"]);

    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(run.stderr, "");
    let list = run.json();
    assert_eq!(list["total"], 40);
    let tracks = list["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 40);
    assert_eq!(
        tracks[3],
        serde_json::json!({
            "id": "10003", "title": "Midnight Drive", "artist": "Ada Lovelace",
            "bpm": 128.03, "key": "8A", "rating": 4
        })
    );
    assert_eq!(
        tracks[0],
        serde_json::json!({ "id": "10000", "title": "Track 000", "artist": "", "bpm": 128.0, "key": "", "rating": 0 })
    );
}

#[test]
fn a_text_filter_narrows_the_list_as_the_windows_search_does() {
    let sandbox = described();
    let ids = |args: &[&str]| -> Vec<String> {
        let run = sandbox.cli(args);
        assert!(run.status.success(), "{}", run.stderr);
        run.json()["tracks"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap().to_owned()).collect()
    };

    // A title, whatever its case.
    assert_eq!(ids(&["--json", "tracks", "list", "--filter", "MIDNIGHT"]), ["10003", "10012"]);
    // An artist.
    assert_eq!(ids(&["--json", "tracks", "list", "--filter", "lovelace"]), ["10003", "10007"]);
    // Every word must match, in any field.
    assert_eq!(ids(&["--json", "tracks", "list", "--filter", "midnight hopper"]), ["10012"]);

    let table = sandbox.cli(&["tracks", "list", "--filter", "sunrise"]);
    assert!(table.status.success(), "{}", table.stderr);
    let lines: Vec<&str> = table.stdout.lines().collect();
    assert_eq!(lines.len(), 2, "{}", table.stdout);
    assert!(lines[1].starts_with("10007"), "{}", table.stdout);
}

#[test]
fn a_limit_caps_the_rows_and_says_how_many_there_were() {
    let sandbox = described();

    let run = sandbox.cli(&["--json", "tracks", "list", "--limit", "3"]);

    assert!(run.status.success(), "{}", run.stderr);
    let list = run.json();
    let ids: Vec<&str> = list["tracks"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["10000", "10001", "10002"]);
    assert_eq!(list["total"], 40);

    // The limit applies after the filter.
    let filtered = sandbox.cli(&["--json", "tracks", "list", "--filter", "midnight", "--limit", "1"]).json();
    assert_eq!(filtered["tracks"].as_array().unwrap().len(), 1);
    assert_eq!(filtered["tracks"][0]["id"], "10003");
    assert_eq!(filtered["total"], 2);

    // By hand, the table says it was cut short.
    let table = sandbox.cli(&["tracks", "list", "--limit", "3"]);
    assert!(table.status.success(), "{}", table.stderr);
    let lines: Vec<&str> = table.stdout.lines().collect();
    assert_eq!(lines.len(), 5, "{}", table.stdout);
    assert_eq!(lines[4], "Showing 3 of 40 tracks; raise --limit to see more.");
}

#[test]
fn without_a_limit_a_large_library_is_cut_to_fifty_rows() {
    let sandbox = Sandbox::with_shape(rbl_db::fixture::Shape { tracks: 60, ..Default::default() });

    let list = sandbox.cli(&["--json", "tracks", "list"]).json();

    assert_eq!(list["tracks"].as_array().unwrap().len(), 50);
    assert_eq!(list["total"], 60);
    // A client that leaves the limit out of its request gets the same cap.
    let direct: serde_json::Value = sandbox.client().call("tracks.list", serde_json::json!({})).unwrap();
    assert_eq!(direct["tracks"].as_array().unwrap().len(), 50);
}

#[test]
fn an_empty_library_lists_no_tracks_and_succeeds() {
    let sandbox = Sandbox::empty();

    let json = sandbox.cli(&["--json", "tracks", "list"]);
    assert!(json.status.success(), "{}", json.stderr);
    assert_eq!(json.json(), serde_json::json!({ "tracks": [], "total": 0 }));
    assert_eq!(json.stderr, "");

    let table = sandbox.cli(&["tracks", "list"]);
    assert!(table.status.success(), "{}", table.stderr);
    assert_eq!(table.stdout, "No tracks.\n");
    assert_eq!(table.stderr, "");
}

#[test]
fn a_filter_that_matches_nothing_lists_no_tracks_and_succeeds() {
    let sandbox = described();

    let json = sandbox.cli(&["--json", "tracks", "list", "--filter", "no such track anywhere"]);
    assert!(json.status.success(), "{}", json.stderr);
    assert_eq!(json.json(), serde_json::json!({ "tracks": [], "total": 0 }));

    let table = sandbox.cli(&["tracks", "list", "--filter", "no such track anywhere"]);
    assert!(table.status.success(), "{}", table.stderr);
    assert_eq!(table.stdout, "No tracks.\n");
    assert_eq!(table.stderr, "");
}
