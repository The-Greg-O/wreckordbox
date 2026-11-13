//! Public track records preserve the Library's unsigned MyTag identifiers.
use crate::common::Sandbox;
use rbl_db::fixture::{MY_TAG_CATEGORY, MY_TAG_PEAK};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::PathBuf};

fn ok(s: &Sandbox, args: &[&str]) -> Value {
    let output = s.cli(&[&["--json"][..], args].concat());
    assert!(output.status.success(), "{args:?}: {}", output.stderr);
    output.json()
}

// Compare persisted data, including WALs, but not SQLite's shared-memory locks.
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

#[test]
fn unsigned_mytag_ids_round_trip_through_assignment_tracks_and_mutants_without_read_writes() {
    let s = Sandbox::new();
    let ids = [MY_TAG_PEAK, "2147483648", "3000750867", "4294967295"];
    for id in &ids[1..] {
        let generated = s.create_my_tag_in(MY_TAG_CATEGORY, &format!("Tag {id}"));
        s.edit(|writer| {
            writer
                .library()
                .connection()
                .execute(
                    "UPDATE djmdMyTag SET ID = ?1 WHERE ID = ?2",
                    [*id, generated.as_str()],
                )
                .unwrap();
        });
    }
    let playlist = s.create_playlist("Unsigned tags", &[2]);
    for id in ids {
        let assigned = ok(&s, &["mytags", "assign", id, "10002"]);
        assert_eq!(assigned["my_tag"]["id"], id);
    }
    let mutant = ok(
        &s,
        &[
            "mutants",
            "create",
            "--playlist",
            &playlist,
            "--position",
            "1",
            "--source",
            "dj",
        ],
    )["mutant"]["id"]
        .to_string();
    ok(&s, &["protection", "on"]);
    let before = stored_bytes(&s);
    let refused = s.cli(&["--json", "mytags", "unassign", ids[2], "10002"]);
    assert!(!refused.status.success());
    assert_eq!(refused.error_json()["error"]["code"], "library_protected");

    let shown = ok(&s, &["tracks", "show", "10002"]);
    let queried = ok(&s, &["tracks", "query", "--playlist", &playlist]);
    assert_eq!(queried["total"], 1);
    let inherited = ok(&s, &["mutants", "show", &mutant]);
    let listed = ok(&s, &["mutants", "list"]);
    for record in [
        &shown,
        &queried["tracks"][0],
        &inherited["mutant"]["inherited"],
        &listed["mutants"][0]["inherited"],
    ] {
        let actual: Vec<_> = record["my_tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tag| tag["id"].clone())
            .collect();
        assert_eq!(actual, json!(ids).as_array().unwrap().clone());
    }
    assert_eq!(
        stored_bytes(&s),
        before,
        "reads or refused edit wrote stored data"
    );
}
