//! Restore actual foundation-produced rows, without computing any legacy digest.
#![allow(
    dead_code,
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
pub fn seed(root: &Path, tracks: &[(&str, String)]) {
    std::fs::create_dir_all(root).unwrap();
    let db = rusqlite::Connection::open(root.join("sidecar.db")).unwrap();
    db.pragma_update(None, "foreign_keys", false).unwrap();
    db.execute_batch(include_str!("foundation.sql")).unwrap();
    for (format, id) in tracks {
        db.execute(
            "UPDATE fact_track SET track_id=?1 WHERE track_id=?2",
            (id, format),
        )
        .unwrap();
    }
    let mut stmt = db
        .prepare("SELECT value FROM fact WHERE kind='stems'")
        .unwrap();
    for value in stmt.query_map([], |r| r.get::<_, String>(0)).unwrap() {
        let value: serde_json::Value = serde_json::from_str(&value.unwrap()).unwrap();
        let audio = super::fixture::wav(value["samples"].as_u64().unwrap() as u32);
        for stem in value["stems"].as_array().unwrap() {
            let path = root.join(stem["file"].as_str().unwrap());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, &audio).unwrap();
        }
    }
}
pub fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                todo.push(p)
            } else if !p.to_string_lossy().ends_with("-shm") {
                out.insert(p.clone(), std::fs::read(p).unwrap());
            }
        }
    }
    out
}
pub fn golden() -> serde_json::Value {
    serde_json::from_str(include_str!("golden.json")).unwrap()
}
