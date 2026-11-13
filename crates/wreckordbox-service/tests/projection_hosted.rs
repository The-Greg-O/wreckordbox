//! Projection (wbx-26) in a service a front end hosts on its own Library
//! core: a projection the window asks for is still a tool's, in its own
//! Run, and makes no DJ Fact; the host hears of it; the window's Undo and
//! Redo take it back and put it back in one step; ownership follows Undo,
//! Redo and unrecorded writes; and a change made while a projection is
//! being applied refuses it whole.
//!
//! One test, in a test binary of its own: the Sidecar's folder and the
//! projection's test pause come from the environment, the process's.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::sync::Arc;
use std::time::Duration;

use rbl_db::fixture::{self, Shape};
use rbl_library::history::{LibraryEdit, TRACK_EDIT};
use rbl_library::{History, LibraryCore, NoHost, Touched};
use serde_json::{json, Value};
use wreckordbox_service::projection::PAUSE_ENV;
use wreckordbox_service::stems::SIDECAR_DIR_ENV;
use wreckordbox_service::{Client, Config, Service};

fn rate(core: &LibraryCore, track: &str, stars: u8) {
    core.recorded_edit(
        &NoHost,
        Touched::Metadata(vec![track.to_owned()]),
        TRACK_EDIT,
        |writer| {
            let (_, recorded) = writer.set_rating_with_undo(track, stars)?;
            Ok(((), LibraryEdit::Track(vec![recorded])))
        },
    )
    .unwrap();
}

/// A write no one names: a player's, an import's (no Ledger entry).
fn unrecorded(core: &LibraryCore, track: &str, stars: u8) {
    core.edit(
        &NoHost,
        History::Outside,
        Touched::Metadata(vec![track.to_owned()]),
        |writer| writer.set_rating(track, stars),
    )
    .unwrap();
}

fn stars(core: &LibraryCore, track: &str) -> u8 {
    let index = core.library().unwrap();
    index.rating[index.row_of(track).unwrap() as usize]
}

#[test]
fn a_projection_the_window_asks_for_is_a_tools_and_undoes_in_one_step() {
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
    std::fs::write(
        sidecar.join("projection-policy.toml"),
        "[energy]\napply = 0.5\n",
    )
    .unwrap();
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
    let [a, b, c, d] = [1, 2, 3, 4].map(fixture::track_id);
    let record = |track: &str, value: u8, confidence: f64| {
        service
            .call(
                "facts.record",
                json!({"source": {"kind": "agent", "name": "tagger"},
                    "facts": [{"track": track, "kind": "energy", "value": value,
                    "confidence": confidence}]}),
            )
            .unwrap();
    };
    let project = || service.call("projection.apply", json!({}));
    let drain = || while events.recv_timeout(Duration::from_millis(100)).is_ok() {};

    // The window asks for a projection, with nothing said about a Run.
    record(&a, 3, 0.6);
    drain();
    let applied = project().unwrap();
    assert_eq!(applied["changes"].as_array().unwrap().len(), 1, "{applied}");
    assert_eq!(stars(&core, &a), 3);
    // The host is told, as of any change it did not make itself.
    let told = events.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(told.event, "library_changed");
    // A tool's Run of its own, not the DJ's window Run.
    let runs = service.call("runs.list", json!({})).unwrap();
    assert_eq!(
        runs["runs"][0]["source"],
        json!({"kind": "tool", "name": "projection"}),
        "{runs}"
    );
    // No DJ Fact: the agent's answer still stands.
    let standing = service
        .call("facts.standing", json!({"id": a, "kind": "energy"}))
        .unwrap();
    assert_eq!(
        standing["standing"]["source"]["kind"], "agent",
        "{standing}"
    );

    // One Undo takes the whole projection back; one Redo puts it back.
    core.step_history(&NoHost, true).unwrap();
    assert_eq!(stars(&core, &a), 0);
    core.step_history(&NoHost, false).unwrap();
    assert_eq!(stars(&core, &a), 3);
    // Still the projection's value: a better Fact replaces it.
    record(&a, 4, 0.7);
    let applied = project().unwrap();
    assert_eq!(applied["changes"][0]["after"], 4, "{applied}");
    // Undo that second projection: the first one's 3 is back, and still
    // the projection's own, so a better Fact replaces it again.
    core.step_history(&NoHost, true).unwrap();
    assert_eq!(stars(&core, &a), 3);
    record(&a, 5, 0.8);
    let applied = project().unwrap();
    assert_eq!(applied["changes"][0]["before"], 3, "{applied}");
    assert_eq!(applied["changes"][0]["after"], 5, "{applied}");

    // A write no Ledger entry records (a player's) takes the field from the
    // projection, even when it puts the projected value back.
    record(&b, 2, 0.6);
    project().unwrap();
    assert_eq!(stars(&core, &b), 2);
    std::thread::sleep(Duration::from_millis(5));
    unrecorded(&core, &b, 1);
    unrecorded(&core, &b, 2);
    record(&b, 4, 0.9);
    let plan = service.call("projection.plan", json!({})).unwrap();
    let kept = plan["kept"].as_array().unwrap();
    assert!(
        kept.iter()
            .any(|k| k["track"] == b.as_str() && k["reason"] == "unrecorded_write"),
        "{plan}"
    );
    project().unwrap();
    assert_eq!(stars(&core, &b), 2);

    // The window's edit made while a projection is being applied: the DJ
    // rating the track to the value it already has (no row changes, but
    // an assertion) refuses the whole apply, and nothing is written.
    record(&c, 2, 0.6);
    project().unwrap();
    record(&c, 4, 0.9);
    record(&d, 3, 0.9);
    std::env::set_var(PAUSE_ENV, "600");
    let mut client = Client::connect(&config.socket_path(&location)).unwrap();
    let applying = std::thread::spawn(move || client.call::<Value>("projection.apply", json!({})));
    std::thread::sleep(Duration::from_millis(250));
    rate(&core, &c, 2);
    let refused = applying.join().unwrap().unwrap_err();
    std::env::remove_var(PAUSE_ENV);
    assert_eq!(refused.code, "projection_changed", "{refused:?}");
    assert_eq!(stars(&core, &c), 2);
    assert_eq!(stars(&core, &d), 0);
    // And afterwards the DJ's 2 is kept: the assertion made it the DJ's
    // Fact, which stands.
    let standing = service
        .call("facts.standing", json!({"id": c, "kind": "energy"}))
        .unwrap();
    assert_eq!(standing["standing"]["source"]["kind"], "dj", "{standing}");
    assert_eq!(standing["standing"]["value"], 2, "{standing}");
    let plan = service.call("projection.plan", json!({})).unwrap();
    assert!(
        plan["kept"]
            .as_array()
            .unwrap()
            .iter()
            .any(|k| k["track"] == c.as_str() && k["reason"] == "dj_fact"),
        "{plan}"
    );

    // Undo and Redo, three times over: a projection's value never becomes
    // the DJ's Fact, however long the chain; a DJ's edit stays the DJ's.
    let (e, f) = (fixture::track_id(5), fixture::track_id(0));
    let source_of = |track: &str| -> Value {
        service
            .call("facts.standing", json!({"id": track, "kind": "energy"}))
            .unwrap()["standing"]["source"]["kind"]
            .clone()
    };
    record(&e, 4, 0.9);
    project().unwrap();
    assert_eq!(stars(&core, &e), 4);
    for cycle in 1..=3 {
        core.step_history(&NoHost, true).unwrap();
        assert_eq!(stars(&core, &e), 0, "undo {cycle}");
        assert_eq!(source_of(&e), "agent", "after undo {cycle}");
        core.step_history(&NoHost, false).unwrap();
        assert_eq!(stars(&core, &e), 4, "redo {cycle}");
        assert_eq!(source_of(&e), "agent", "after redo {cycle}");
    }
    rate(&core, &f, 2);
    assert_eq!(source_of(&f), "dj");
    for cycle in 1..=3 {
        core.step_history(&NoHost, true).unwrap();
        assert!(source_of(&f).is_null(), "the DJ's Fact withdrawn by undo {cycle}");
        core.step_history(&NoHost, false).unwrap();
        assert_eq!(source_of(&f), "dj", "the DJ's Fact back after redo {cycle}");
    }
    drop(service);
}
