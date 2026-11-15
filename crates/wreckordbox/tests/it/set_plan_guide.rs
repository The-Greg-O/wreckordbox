//! `wreckordbox set-plan guide` (wbx-wjl): the agents' set planning guide,
//! printed by the real CLI with no service, no Library and no writes.
use crate::common;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use common::Sandbox;

const GUIDE: &str = include_str!("../../../../docs/agents/set-planning.md");

/// Every file and folder under `dir` with its size and modification time.
fn tree(dir: &Path) -> Vec<(PathBuf, u64, Option<SystemTime>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.map(Result::unwrap) {
            let meta = entry.metadata().unwrap();
            out.push((entry.path(), meta.len(), meta.modified().ok()));
            if meta.is_dir() {
                stack.push(entry.path());
            }
        }
    }
    out.sort();
    out
}

#[test]
fn prints_the_document_in_docs_and_starts_nothing() {
    let sandbox = Sandbox::new();
    let before = tree(sandbox.path());

    let run = sandbox.cli(&["set-plan", "guide"]);

    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(run.stdout.trim_end(), GUIDE.trim_end());
    assert_eq!(
        tree(sandbox.path()),
        before,
        "the guide wrote or started something"
    );
    assert!(
        wreckordbox_service::Client::connect(&sandbox.config().socket_path(sandbox.location()))
            .is_err(),
        "no service may be running"
    );
}

#[test]
fn needs_no_library() {
    let sandbox = Sandbox::new();
    let run = sandbox.run(
        sandbox
            .command()
            .env_remove("RBXPORT_TEST_FIXTURE")
            .args(["--json", "set-plan", "guide"]),
    );
    assert!(run.status.success(), "{}", run.stderr);
}

#[test]
fn json_gives_the_same_document_in_sections() {
    let sandbox = Sandbox::new();
    let before = tree(sandbox.path());

    let run = sandbox.cli(&["--json", "set-plan", "guide"]);

    assert!(run.status.success(), "{}", run.stderr);
    let guide = run.json();
    assert_eq!(guide["markdown"], GUIDE);
    assert_eq!(guide["title"], "Set planning: a guide for agents");
    assert_eq!(guide["source"], "docs/agents/set-planning.md");
    let sections = guide["sections"].as_array().unwrap();
    let headings: Vec<&str> = sections
        .iter()
        .map(|s| s["heading"].as_str().unwrap())
        .collect();
    assert_eq!(
        headings,
        [
            "1. Keys",
            "2. Tempo",
            "3. Energy and Intensity",
            "4. How this app sequences: the Recipe and the scores",
            "5. Limits and honesty",
        ]
    );
    let ids: Vec<&str> = sections.iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert_eq!(ids[0], "keys");
    assert_eq!(ids[3], "how-this-app-sequences-the-recipe-and-the-scores");
    // Nothing lost or reordered: the introduction and the sections are the
    // document.
    let mut whole = guide["introduction"].as_str().unwrap().to_owned();
    for section in sections {
        whole.push_str(section["markdown"].as_str().unwrap());
    }
    assert_eq!(whole, GUIDE);
    assert!(sections[0]["markdown"]
        .as_str()
        .unwrap()
        .contains("<!-- table: camelot -->"));
    assert_eq!(tree(sandbox.path()), before);
}
