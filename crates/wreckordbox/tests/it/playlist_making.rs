//! Making and arranging playlists from the command line (wbx-uia):
//! `playlists create | create-folder | add | rename | move`, beside the
//! older `remove | reorder | delete`. The real binary against a service on a
//! throwaway Library: what the read commands then show, what the Ledger
//! records, what a second client is told, and that Library Protection and
//! intelligent playlists refuse with nothing written.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::common;

use common::Sandbox;
use rbl_db::fixture::track_id;
use serde_json::{json, Value};
use wreckordbox_service::Client;

/// A successful run's JSON.
fn ok(sandbox: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let run = sandbox.cli(&all);
    assert!(run.status.success(), "{args:?}: {}", run.stderr);
    assert_eq!(run.stderr, "", "{args:?}");
    run.json()
}

/// The code a refused run reports, having printed nothing.
fn refused(sandbox: &Sandbox, args: &[&str]) -> (String, String) {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let run = sandbox.cli(&all);
    assert!(!run.status.success(), "{args:?}: {}", run.stdout);
    assert_eq!(common::without_run(&run.stdout), "", "{args:?}");
    let error = run.error_json();
    (
        error["error"]["code"]
            .as_str()
            .expect(&run.stderr)
            .to_owned(),
        error["error"]["message"].as_str().unwrap_or("").to_owned(),
    )
}

/// The tree as `(id, name, kind, parent)`, in the window's order.
fn tree(sandbox: &Sandbox) -> Vec<(String, String, String, Option<String>)> {
    ok(sandbox, &["playlists", "tree"])["playlists"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| {
            (
                node["id"].as_str().unwrap().to_owned(),
                node["name"].as_str().unwrap().to_owned(),
                node["kind"].as_str().unwrap().to_owned(),
                node["parent_id"].as_str().map(str::to_owned),
            )
        })
        .collect()
}

fn node(sandbox: &Sandbox, id: &str) -> Option<(String, String, Option<String>)> {
    tree(sandbox)
        .into_iter()
        .find(|(node, ..)| node == id)
        .map(|(_, name, kind, parent)| (name, kind, parent))
}

/// The children of `parent` (`None`: the top level), in order, by name.
fn children(sandbox: &Sandbox, parent: Option<&str>) -> Vec<String> {
    tree(sandbox)
        .into_iter()
        .filter(|(.., of)| of.as_deref() == parent)
        .map(|(_, name, ..)| name)
        .collect()
}

/// A playlist's tracks, in its order.
fn tracks_of(sandbox: &Sandbox, playlist: &str) -> Vec<String> {
    ok(
        sandbox,
        &["playlists", "tracks", playlist, "--limit", "500"],
    )["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|track| track["id"].as_str().unwrap().to_owned())
        .collect()
}

fn ids(indexes: &[usize]) -> Vec<String> {
    indexes.iter().map(|&i| track_id(i)).collect()
}

fn collection_size(sandbox: &Sandbox) -> u64 {
    ok(sandbox, &["tracks", "list", "--limit", "1"])["total"]
        .as_u64()
        .unwrap()
}

fn watcher(sandbox: &Sandbox) -> Client {
    sandbox.status();
    let mut watcher = sandbox.client();
    watcher.subscribe().unwrap();
    common::barrier::settle(&mut watcher);
    watcher
}

#[test]
fn a_playlist_is_made_with_its_tracks_in_the_order_given_and_answers_its_id() {
    let sandbox = Sandbox::new();
    let before = tree(&sandbox).len();

    let made = ok(
        &sandbox,
        &[
            "playlists",
            "create",
            "Warm-up",
            &track_id(4),
            &track_id(1),
            &track_id(3),
        ],
    );

    let id = made["playlist"].as_str().expect("the new playlist's id");
    assert_eq!(made["name"], "Warm-up");
    assert_eq!(made["kind"], "playlist");
    assert_eq!(made["parent"], "root");
    assert_eq!(made["added"].as_array().unwrap().len(), 3);
    assert_eq!(tree(&sandbox).len(), before + 1);
    assert_eq!(
        node(&sandbox, id),
        Some(("Warm-up".into(), "playlist".into(), None))
    );
    assert_eq!(tracks_of(&sandbox, id), ids(&[4, 1, 3]));
    // Chained: the id it answered is what the next command takes.
    ok(&sandbox, &["playlists", "add", id, &track_id(9)]);
    assert_eq!(tracks_of(&sandbox, id), ids(&[4, 1, 3, 9]));
}

#[test]
fn an_empty_playlist_and_a_folder_are_made_and_a_playlist_goes_inside_the_folder() {
    let sandbox = Sandbox::new();

    let folder = ok(&sandbox, &["playlists", "create-folder", "Gigs"]);
    let folder_id = folder["playlist"].as_str().unwrap().to_owned();
    assert_eq!(folder["kind"], "folder");
    let first = ok(
        &sandbox,
        &["playlists", "create", "Friday", "--parent", &folder_id],
    );
    let second = ok(
        &sandbox,
        &[
            "playlists",
            "create",
            "Thursday",
            "--parent",
            &folder_id,
            "--at",
            "1",
        ],
    );

    assert_eq!(first["added"], json!([]));
    assert_eq!(
        node(&sandbox, first["playlist"].as_str().unwrap()),
        Some(("Friday".into(), "playlist".into(), Some(folder_id.clone())))
    );
    assert_eq!(
        children(&sandbox, Some(&folder_id)),
        ["Thursday", "Friday"],
        "--at 1 puts it first"
    );
    assert!(tracks_of(&sandbox, second["playlist"].as_str().unwrap()).is_empty());
    let sub = ok(
        &sandbox,
        &["playlists", "create-folder", "2026", "--parent", &folder_id],
    );
    assert_eq!(
        node(&sandbox, sub["playlist"].as_str().unwrap()).unwrap().2,
        Some(folder_id)
    );
}

#[test]
fn nothing_is_made_inside_a_playlist_or_a_folder_that_is_not_there_nor_with_a_track_that_is_not() {
    let sandbox = Sandbox::new();
    let playlist = sandbox.create_playlist("Set", &[1, 2]);
    let before = tree(&sandbox);

    let (code, _) = refused(
        &sandbox,
        &["playlists", "create", "Inner", "--parent", &playlist],
    );
    assert_eq!(code, "not_a_folder");
    let (code, _) = refused(
        &sandbox,
        &["playlists", "create-folder", "Inner", "--parent", "999999"],
    );
    assert_eq!(code, "playlist_not_found");
    let (code, _) = refused(&sandbox, &["playlists", "create", "Odd", "99999999"]);
    assert_eq!(code, "track_not_found");
    let (code, _) = refused(&sandbox, &["playlists", "create", "  "]);
    assert_eq!(code, "bad_request");

    assert_eq!(tree(&sandbox), before, "nothing made");
}

#[test]
fn tracks_are_added_at_the_end_or_at_a_position_and_a_track_already_there_is_skipped() {
    let sandbox = Sandbox::new();
    let playlist = sandbox.create_playlist("Set", &[1, 2]);

    let added = ok(&sandbox, &["playlists", "add", &playlist, &track_id(5)]);
    assert_eq!(added["playlist"], playlist.as_str());
    assert_eq!(added["added"].as_array().unwrap().len(), 1);
    assert_eq!(tracks_of(&sandbox, &playlist), ids(&[1, 2, 5]));

    ok(
        &sandbox,
        &[
            "playlists",
            "add",
            &playlist,
            &track_id(7),
            &track_id(8),
            "--at",
            "2",
        ],
    );
    assert_eq!(tracks_of(&sandbox, &playlist), ids(&[1, 7, 8, 2, 5]));

    let again = ok(&sandbox, &["playlists", "add", &playlist, &track_id(1)]);
    assert_eq!(again["added"], json!([]));
    assert_eq!(again["skipped"], json!([track_id(1)]));
    assert_eq!(tracks_of(&sandbox, &playlist), ids(&[1, 7, 8, 2, 5]));

    let twice = ok(
        &sandbox,
        &[
            "playlists",
            "add",
            &playlist,
            &track_id(1),
            "--allow-duplicates",
        ],
    );
    assert_eq!(twice["added"].as_array().unwrap().len(), 1);
    assert_eq!(tracks_of(&sandbox, &playlist), ids(&[1, 7, 8, 2, 5, 1]));

    let (code, _) = refused(&sandbox, &["playlists", "add", &playlist, "99999999"]);
    assert_eq!(code, "track_not_found");
    assert_eq!(tracks_of(&sandbox, &playlist), ids(&[1, 7, 8, 2, 5, 1]));
}

#[test]
fn a_playlist_is_renamed_and_moved_into_a_folder_and_a_folder_never_into_itself() {
    let sandbox = Sandbox::new();
    let playlist = sandbox.create_playlist("Set", &[1]);
    let folder = ok(&sandbox, &["playlists", "create-folder", "Gigs"])["playlist"]
        .as_str()
        .unwrap()
        .to_owned();
    let inner = ok(
        &sandbox,
        &["playlists", "create-folder", "Old", "--parent", &folder],
    )["playlist"]
        .as_str()
        .unwrap()
        .to_owned();

    let renamed = ok(&sandbox, &["playlists", "rename", &playlist, "Closing"]);
    assert_eq!(renamed["playlist"], playlist.as_str());
    assert_eq!(node(&sandbox, &playlist).unwrap().0, "Closing");

    ok(
        &sandbox,
        &[
            "playlists",
            "move",
            &playlist,
            "--parent",
            &folder,
            "--at",
            "1",
        ],
    );
    assert_eq!(children(&sandbox, Some(&folder)), ["Closing", "Old"]);
    assert_eq!(
        tracks_of(&sandbox, &playlist),
        ids(&[1]),
        "its tracks go with it"
    );

    ok(&sandbox, &["playlists", "move", &playlist]);
    assert_eq!(
        node(&sandbox, &playlist).unwrap().2,
        None,
        "back to the top"
    );

    let before = tree(&sandbox);
    let (code, _) = refused(
        &sandbox,
        &["playlists", "move", &folder, "--parent", &inner],
    );
    assert_eq!(code, "write_refused");
    let (code, _) = refused(&sandbox, &["playlists", "rename", "999999", "X"]);
    assert_eq!(code, "playlist_not_found");
    let (code, _) = refused(&sandbox, &["playlists", "rename", &playlist, ""]);
    assert_eq!(code, "bad_request");
    assert_eq!(tree(&sandbox), before);
}

#[test]
fn deleting_a_playlist_or_a_folder_never_deletes_a_track() {
    let sandbox = Sandbox::new();
    let tracks = collection_size(&sandbox);
    let folder = ok(&sandbox, &["playlists", "create-folder", "Gigs"])["playlist"]
        .as_str()
        .unwrap()
        .to_owned();
    let inside = ok(
        &sandbox,
        &[
            "playlists",
            "create",
            "Friday",
            "--parent",
            &folder,
            &track_id(1),
            &track_id(2),
        ],
    )["playlist"]
        .as_str()
        .unwrap()
        .to_owned();

    ok(&sandbox, &["playlists", "delete", &folder]);

    assert_eq!(node(&sandbox, &folder), None);
    assert_eq!(node(&sandbox, &inside), None);
    assert_eq!(collection_size(&sandbox), tracks);
    let shown = ok(&sandbox, &["tracks", "show", &track_id(1)]);
    assert_eq!(shown["id"], track_id(1).as_str());
}

#[test]
fn an_intelligent_playlists_entries_are_refused_clearly_and_nothing_changes() {
    let sandbox = Sandbox::new();
    let smart = sandbox.create_genre_playlist("House", "House");
    let before = tracks_of(&sandbox, &smart);

    for edit in [
        vec!["playlists", "add", smart.as_str(), "10001"],
        vec!["playlists", "remove", smart.as_str(), "10001"],
    ] {
        let (code, message) = refused(&sandbox, &edit);
        assert_eq!(code, "not_a_playlist", "{edit:?}");
        assert!(message.contains("intelligent"), "{edit:?}: {message}");
    }
    let (code, _) = refused(&sandbox, &["playlists", "create", "In", "--parent", &smart]);
    assert_eq!(code, "not_a_folder");
    assert_eq!(tracks_of(&sandbox, &smart), before);
    // It can still be renamed, as in the window.
    ok(&sandbox, &["playlists", "rename", &smart, "Deep House"]);
    assert_eq!(node(&sandbox, &smart).unwrap().0, "Deep House");
}

#[test]
fn every_new_edit_is_refused_while_protection_is_on_and_nothing_is_written() {
    let sandbox = Sandbox::new();
    let playlist = sandbox.create_playlist("Set", &[1, 2]);
    let folder = {
        let mut id = String::new();
        sandbox.edit(|writer| {
            id = writer
                .create_with_undo("Gigs", "root", rbl_db::write::ATTRIBUTE_FOLDER, None)
                .unwrap()
                .0;
        });
        id
    };
    sandbox.protect(true);
    let tree_before = tree(&sandbox);
    let tracks_before = tracks_of(&sandbox, &playlist);

    for edit in [
        vec!["playlists", "create", "New", "10003"],
        vec!["playlists", "create-folder", "New"],
        vec!["playlists", "add", playlist.as_str(), "10003"],
        vec!["playlists", "add", playlist.as_str(), "10003", "--at", "1"],
        vec!["playlists", "rename", playlist.as_str(), "Renamed"],
        vec![
            "playlists",
            "move",
            playlist.as_str(),
            "--parent",
            folder.as_str(),
        ],
    ] {
        let (code, _) = refused(&sandbox, &edit);
        assert_eq!(code, "library_protected", "{edit:?}");
    }

    assert_eq!(tree(&sandbox), tree_before);
    assert_eq!(tracks_of(&sandbox, &playlist), tracks_before);
    assert_eq!(ok(&sandbox, &["runs", "list"])["runs"], json!([]), "no Run");
}

#[test]
fn each_edit_is_a_ledger_entry_under_the_callers_source_and_inside_a_named_run() {
    let sandbox = Sandbox::new();
    let made = ok(
        &sandbox,
        &[
            "playlists",
            "create",
            "Warm-up",
            &track_id(1),
            &track_id(2),
            "--source",
            "agent:builder",
            "--description",
            "Make the warm-up",
        ],
    );
    let run = made["run"]["id"].as_i64().expect("its Run");
    assert_eq!(
        made["run"]["source"],
        json!({"kind": "agent", "name": "builder"})
    );
    let shown = ok(&sandbox, &["runs", "show", &run.to_string()]);
    let kinds: Vec<&str> = shown["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["create_playlist", "add_playlist_tracks"]);
    for entry in shown["entries"].as_array().unwrap() {
        assert_eq!(entry["source"], json!({"kind": "agent", "name": "builder"}));
        assert_eq!(entry["command"]["name"], "playlists.create");
    }

    let started = ok(
        &sandbox,
        &[
            "runs",
            "start",
            "--source",
            "agent:builder",
            "--description",
            "Tidy",
        ],
    );
    let named = started["id"].as_i64().unwrap();
    let id = named.to_string();
    let playlist = made["playlist"].as_str().unwrap();
    for edit in [
        vec!["playlists", "rename", playlist, "Opening", "--run", &id],
        vec!["playlists", "create-folder", "Gigs", "--run", &id],
        vec!["playlists", "add", playlist, &track_id(3), "--run", &id],
    ] {
        assert_eq!(ok(&sandbox, &edit)["run"]["id"], named, "{edit:?}");
    }
    let folder = tree(&sandbox)
        .into_iter()
        .find(|(_, name, ..)| name == "Gigs")
        .unwrap()
        .0;
    ok(
        &sandbox,
        &[
            "playlists",
            "move",
            playlist,
            "--parent",
            &folder,
            "--run",
            &id,
        ],
    );
    let kinds: Vec<String> = ok(&sandbox, &["runs", "show", &id])["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["kind"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        kinds,
        [
            "rename_playlist",
            "create_playlist",
            "add_playlist_tracks",
            "move_playlist"
        ]
    );
}

#[test]
fn a_second_client_is_told_of_each_edit_and_of_nothing_refused() {
    let sandbox = Sandbox::new();
    let playlist = sandbox.create_playlist("Set", &[1, 2]);
    let mut watcher = watcher(&sandbox);

    let made = ok(&sandbox, &["playlists", "create-folder", "Gigs"]);
    assert_eq!(
        common::barrier::one_library_change(&mut watcher),
        made["generation"].as_u64().unwrap()
    );
    let renamed = ok(&sandbox, &["playlists", "rename", &playlist, "Closing"]);
    assert_eq!(
        common::barrier::one_library_change(&mut watcher),
        renamed["generation"].as_u64().unwrap()
    );
    // Made with tracks: the last change it tells is the answer's.
    let filled = ok(&sandbox, &["playlists", "create", "Full", &track_id(4)]);
    let heard = common::barrier::library_changes(&mut watcher);
    assert!(!heard.is_empty());
    assert_eq!(
        heard.last().unwrap().data["generation"].as_u64(),
        filled["generation"].as_u64()
    );

    refused(&sandbox, &["playlists", "add", &playlist, "99999999"]);
    common::barrier::told_nothing(&mut watcher);
}
