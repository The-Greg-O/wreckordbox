//! Reviewing waiting Facts (wbx-27) in a service a front end hosts on its
//! own Library core: the window's accept is the DJ's, told to the host, and
//! ONE step of the window's Undo, which puts the Facts back in the queue;
//! the window's rejection must still name the DJ.
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
use std::time::Duration;

use rbl_db::fixture::{self, Shape};
use rbl_library::{LibraryCore, NoHost};
use serde_json::{json, Value};
use wreckordbox_service::stems::SIDECAR_DIR_ENV;
use wreckordbox_service::{Config, Service};

fn stars(core: &LibraryCore, track: &str) -> u8 {
    let index = core.library().unwrap();
    index.rating[index.row_of(track).unwrap() as usize]
}

#[test]
fn the_windows_accept_is_the_djs_and_one_undo_step_puts_the_facts_back_in_the_queue() {
    assert!(rbl_db::test_mode(), "run with RBXPORT_TEST=1");
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    let location = fixture::build(
        &library,
        Shape {
            tracks: 6,
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
    let events = service.events();
    let [a, b] = [1, 2].map(fixture::track_id);
    for (track, value) in [(&a, 4), (&b, 2)] {
        service
            .call(
                "facts.record",
                json!({"source": {"kind": "agent", "name": "tagger"},
                    "facts": [{"track": track, "kind": "energy", "value": value,
                    "confidence": 0.6}]}),
            )
            .unwrap();
    }
    let waiting = || -> usize {
        service.call("review.waiting", json!({})).unwrap()["count"]
            .as_u64()
            .unwrap() as usize
    };
    let standing_kind = |track: &str| -> Value {
        service
            .call("facts.standing", json!({"id": track, "kind": "energy"}))
            .unwrap()["standing"]["source"]["kind"]
            .clone()
    };
    while events.recv_timeout(Duration::from_millis(100)).is_ok() {}
    assert_eq!(waiting(), 2);

    // The window accepts everything, naming no Run: the DJ's own act.
    let accepted = service.call("review.accept", json!({"all": true})).unwrap();
    assert_eq!(accepted["count"], 2, "{accepted}");
    assert_eq!((stars(&core, &a), stars(&core, &b)), (4, 2));
    let told = events.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(told.event, "library_changed");
    assert_eq!(standing_kind(&a), "dj");
    assert_eq!(standing_kind(&b), "dj");
    assert_eq!(waiting(), 0);

    // One Undo takes the whole accept back, and the Facts wait again;
    // one Redo puts it back, the DJ's again.
    core.step_history(&NoHost, true).unwrap();
    assert_eq!((stars(&core, &a), stars(&core, &b)), (0, 0));
    assert_eq!(standing_kind(&a), "agent");
    assert_eq!(waiting(), 2);
    core.step_history(&NoHost, false).unwrap();
    assert_eq!((stars(&core, &a), stars(&core, &b)), (4, 2));
    assert_eq!(standing_kind(&a), "dj");
    assert_eq!(waiting(), 0);

    // The window's rejection names the DJ, or it is refused.
    service
        .call(
            "facts.record",
            json!({"source": {"kind": "agent", "name": "tagger"},
                "facts": [{"track": fixture::track_id(3), "kind": "energy", "value": 3,
                "confidence": 0.6}]}),
        )
        .unwrap();
    let refused = service
        .call("review.reject", json!({"all": true}))
        .unwrap_err();
    assert_eq!(refused.code, "review_not_the_djs");
    assert_eq!(waiting(), 1);
    service
        .call(
            "review.reject",
            json!({"all": true, "source": {"kind": "dj"}}),
        )
        .unwrap();
    assert_eq!(waiting(), 0);
}
