//! Competing proposed titles (wbx-66 review): two sources propose different
//! titles for one track. An accept naming both is refused before anything
//! is written (the DJ must choose one); accepting one is ONE step of the
//! window's Undo, which puts the title back and both proposals in the
//! queue again.
//!
//! In a test binary of its own: the Sidecar's folder comes from the
//! environment, the process's.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::sync::Arc;

use rbl_db::fixture::{self, Shape};
use rbl_library::{LibraryCore, NoHost};
use serde_json::{json, Value};
use wreckordbox_service::stems::SIDECAR_DIR_ENV;
use wreckordbox_service::{Config, Service};

fn title(core: &LibraryCore, track: &str) -> String {
    let index = core.library().unwrap();
    index
        .title
        .get(index.row_of(track).unwrap() as usize)
        .to_owned()
}

#[test]
fn an_accept_of_two_competing_titles_is_refused_and_one_accepted_is_one_undo_step() {
    assert!(rbl_db::test_mode(), "run with RBXPORT_TEST=1");
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    let location = fixture::build(
        &library,
        Shape {
            tracks: 3,
            ..Shape::default()
        },
    )
    .unwrap();
    let sidecar = dir.path().join("sidecar");
    std::fs::create_dir_all(&sidecar).unwrap();
    std::env::set_var(SIDECAR_DIR_ENV, &sidecar);
    let config = Config {
        runtime_dir: dir.path().join("run"),
        idle_exit: None,
        recovery_dir: dir.path().join("recovery"),
    };
    let core = Arc::new(LibraryCore::new(config.recovery_dir.clone()));
    core.set_protection(false).unwrap();
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (index, _) = rbl_index::load(&db).unwrap();
    core.set_library(
        &NoHost,
        index,
        false,
        db.schema().db_version,
        0,
        location.clone(),
    );
    let host: Arc<dyn rbl_library::Host + Send + Sync> = Arc::new(NoHost);
    let service = Service::host(&location, &config, &core, &host).unwrap();
    let track = fixture::track_id(1);
    let original = title(&core, &track);
    for (source, value) in [("tagger", "Title From Tagger"), ("web", "Title From Web")] {
        service
            .call(
                "facts.record",
                json!({"source": {"kind": "agent", "name": source},
                    "facts": [{"track": track, "kind": "title", "value": value,
                    "confidence": 0.9}]}),
            )
            .unwrap();
    }
    let listing = || {
        service
            .call("review.waiting", json!({"field": "title"}))
            .unwrap()
    };
    let rows = |listing: &Value| -> Vec<Value> {
        let tokens = &listing["row_tokens"];
        listing["waiting"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| json!({"fact": h["fact"], "token": tokens[h["fact"].to_string()]}))
            .collect()
    };
    let shown = listing();
    assert_eq!(shown["count"], 2, "{shown}");

    // Both at once: refused before anything is written, naming the choice.
    let both = service
        .call(
            "review.accept",
            json!({"rows": rows(&shown), "source": {"kind": "dj"}}),
        )
        .unwrap_err();
    assert_eq!(both.code, "review_competing", "{both:?}");
    assert!(both.message.contains("choose one"), "{both:?}");
    assert_eq!(title(&core, &track), original);
    assert_eq!(listing()["count"], 2);

    // One of them: written, the DJ's, and the other no longer waits.
    let shown = listing();
    let web = shown["waiting"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["after"] == "Title From Web")
        .unwrap()["fact"]
        .clone();
    let chosen: Vec<Value> = rows(&shown)
        .into_iter()
        .filter(|r| r["fact"] == web)
        .collect();
    let accepted = service
        .call(
            "review.accept",
            json!({"rows": chosen, "source": {"kind": "dj"}}),
        )
        .unwrap();
    assert_eq!(accepted["count"], 1, "{accepted}");
    assert_eq!(title(&core, &track), "Title From Web");
    assert_eq!(listing()["count"], 0);

    // One Undo puts the title back and both proposals wait again; Redo
    // applies the accepted one again.
    core.step_history(&NoHost, true).unwrap();
    assert_eq!(title(&core, &track), original);
    assert_eq!(listing()["count"], 2);
    core.step_history(&NoHost, false).unwrap();
    assert_eq!(title(&core, &track), "Title From Web");
    assert_eq!(listing()["count"], 0);
}
