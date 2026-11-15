//! The Sidecar's write admission (wbx-8yh): one check every write of the
//! handle passes, asked at the moment the write would be kept, so a handle
//! obtained while writing was allowed cannot write once it is not.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::json;
use wreckordbox_sidecar::{Kind, NewFact, Sidecar, SidecarError, Source, SourceKind};

const MOOD: Kind = Kind::judgement("mood");

/// Every file of the folder and its bytes; SQLite's shared-memory index is
/// left out (a read through an open connection touches it).
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                out.insert(path.clone(), Vec::new());
                todo.push(path);
            } else if !path.to_string_lossy().ends_with("-shm") {
                out.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    out
}

fn dj() -> Source {
    Source::new(SourceKind::Dj, "dj")
}

fn refused<T: std::fmt::Debug>(result: Result<T, SidecarError>) -> String {
    match result {
        Err(SidecarError::Refused(why)) => why,
        other => panic!("expected the write admission's refusal, got {other:?}"),
    }
}

#[test]
fn a_handle_obtained_while_writing_was_allowed_cannot_write_once_it_is_not() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("sidecar");
    let sidecar = Sidecar::open(&folder).unwrap();
    let allowed = Arc::new(AtomicBool::new(true));
    let gate = Arc::clone(&allowed);
    sidecar.set_write_admission(Arc::new(move || {
        if gate.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err("the Sidecar is read-only here".to_owned())
        }
    }));

    // Allowed: the handle writes.
    let run = sidecar.begin_run(&dj(), "before").unwrap();
    let fact = sidecar
        .record(&NewFact {
            track_id: "10003",
            fingerprint: None,
            kind: MOOD,
            value: json!("Dark"),
            source: &dj(),
            confidence: 1.0,
            run,
        })
        .unwrap();
    sidecar.finish_run(run, "done").unwrap();

    // The same handle, after writing stopped being allowed.
    allowed.store(false, Ordering::Release);
    let before = snapshot(&folder);
    assert_eq!(
        refused(sidecar.begin_run(&dj(), "after")),
        "the Sidecar is read-only here"
    );
    refused(sidecar.record_many(&[NewFact {
        track_id: "10003",
        fingerprint: None,
        kind: MOOD,
        value: json!("Bright"),
        source: &dj(),
        confidence: 1.0,
        run,
    }]));
    refused(sidecar.finish_run(run, "again"));
    refused(sidecar.workspace(run));
    assert_eq!(
        snapshot(&folder),
        before,
        "not a byte of the Sidecar changed"
    );

    // Reading still answers, from what was kept.
    let facts = sidecar.facts("10003", "mood").unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].id, fact.id);
    assert_eq!(facts[0].value, json!("Dark"));
}
