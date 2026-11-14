//! MyTag Projection (wbx-29) in a service a front end hosts on its own
//! Library core: a projection the window asks for is a tool's, in its own
//! Run, and makes no DJ Fact; the host hears of it; the window's Undo and
//! Redo take it back and put it back in ONE step across several tracks, as
//! often as the DJ likes, without ever making a DJ Fact or handing the
//! assignment back to the projection; an absence an Undo left stays the
//! DJ's, also after rekordbox wrote the tag on and off again; and an
//! in-process hand set asserts only when it says so.
//!
//! One test, in a test binary of its own: the Sidecar's folder comes from
//! the environment, the process's.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::sync::Arc;
use std::time::Duration;

use rbl_db::fixture::{self, Shape, MY_TAG_PEAK};
use rbl_library::{LibraryCore, NoHost};
use serde_json::{json, Value};
use wreckordbox_service::stems::SIDECAR_DIR_ENV;
use wreckordbox_service::{Config, Service};

const SCHEMA: &str = r#"version = 1

[dimensions.situation]
terms = [
  { id = "peak", name = "Peak" },
  { id = "warm-up", name = "Warm-up" },
]

[deck]
categories = [
  { dimension = "situation", label = "Situation" },
]
"#;

fn has_peak(core: &LibraryCore, track: &str) -> bool {
    let index = core.library().unwrap();
    let row = index.row_of(track).unwrap() as usize;
    index
        .my_tag_keys(row)
        .contains(&rbl_index::smart::my_tag_key(MY_TAG_PEAK))
}

#[test]
fn a_mytag_projection_the_window_asks_for_is_a_tools_and_undoes_in_one_step() {
    assert!(rbl_db::test_mode(), "run with RBXPORT_TEST=1");
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    let location = fixture::build(
        &library,
        Shape {
            tracks: 10,
            ..Shape::default()
        },
    )
    .unwrap();
    let sidecar = dir.path().join("sidecar");
    std::fs::create_dir_all(&sidecar).unwrap();
    std::fs::write(
        sidecar.join("projection-policy.toml"),
        "[mytags]\napply = 0.5\n",
    )
    .unwrap();
    std::fs::write(sidecar.join("tag-schema.toml"), SCHEMA).unwrap();
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
    let [a, b, c, d, e, f] = [1, 2, 3, 4, 5, 6].map(fixture::track_id);
    let record = |track: &str, term: &str| {
        service
            .call(
                "facts.record",
                json!({"source": {"kind": "agent", "name": "tagger"},
                    "facts": [{"track": track, "kind": "situation", "value": [term],
                    "confidence": 0.9}]}),
            )
            .unwrap();
    };
    let project = || service.call("projection.mytags.apply", json!({})).unwrap();
    let plan = || service.call("projection.mytags.plan", json!({})).unwrap();
    let reason = |plan: &Value, track: &str| -> Option<String> {
        plan["kept"]
            .as_array()
            .unwrap()
            .iter()
            .find(|k| k["track"] == track && k["label"] == "Peak")
            .and_then(|k| k["reason"].as_str().map(str::to_owned))
    };
    let no_dj_fact = |track: &str| {
        let standing = service
            .call("facts.standing", json!({"id": track, "kind": "situation"}))
            .unwrap();
        assert_eq!(
            standing["standing"]["source"]["kind"], "agent",
            "{track}: {standing}"
        );
    };
    let drain = || while events.recv_timeout(Duration::from_millis(100)).is_ok() {};

    // The window asks for a projection, with nothing said about a Run.
    for track in [&a, &b, &c] {
        record(track, "peak");
    }
    drain();
    let applied = project();
    assert_eq!(applied["changes"].as_array().unwrap().len(), 3, "{applied}");
    let told = events.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(told.event, "library_changed");
    let runs = service.call("runs.list", json!({})).unwrap();
    assert_eq!(
        runs["runs"][0]["source"],
        json!({"kind": "tool", "name": "projection"}),
        "{runs}"
    );
    assert_eq!(runs["runs"][0]["changes"], 1, "one entry: {runs}");
    for track in [&a, &b, &c] {
        assert!(has_peak(&core, track), "{track}");
        no_dj_fact(track);
    }

    // One Undo takes all three back, one Redo puts all three back, as often
    // as the DJ goes back and forth; never a DJ Fact.
    for _ in 0..3 {
        core.step_history(&NoHost, true).unwrap();
        for track in [&a, &b, &c] {
            assert!(!has_peak(&core, track), "{track} after Undo");
            no_dj_fact(track);
        }
        core.step_history(&NoHost, false).unwrap();
        for track in [&a, &b, &c] {
            assert!(has_peak(&core, track), "{track} after Redo");
            no_dj_fact(track);
        }
    }
    // A Redo is not the projection's own write: the tag is the DJ's now.
    record(&a, "warm-up");
    assert_eq!(reason(&plan(), &a), Some("written_since".into()));

    // An absence an Undo left stays the DJ's, here after rekordbox put the
    // tag on and took it off again while wreckordbox was closed.
    record(&d, "peak");
    project();
    assert!(has_peak(&core, &d));
    core.step_history(&NoHost, true).unwrap();
    assert!(!has_peak(&core, &d));
    {
        let mut writer =
            rbl_db::write::Writer::open(location.clone(), dir.path().join("backups")).unwrap();
        writer.disable_automatic_backups();
        writer.set_my_tags(&d, &[MY_TAG_PEAK.to_owned()]).unwrap();
        writer.set_my_tags(&d, &[]).unwrap();
    }
    let planned = plan();
    assert_eq!(reason(&planned, &d), Some("undone".into()), "{planned}");
    assert!(
        !planned["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["track"] == d),
        "{planned}"
    );

    // An in-process hand set says whether it asserts: a resend without
    // `assert` asserts nothing; with it, the DJ's choice is kept.
    record(&e, "peak");
    record(&f, "peak");
    project();
    service
        .call("mytags.assign", json!({"id": MY_TAG_PEAK, "tracks": [e]}))
        .unwrap();
    service
        .call(
            "mytags.assign",
            json!({"id": MY_TAG_PEAK, "tracks": [f], "assert": true}),
        )
        .unwrap();
    record(&e, "warm-up");
    record(&f, "warm-up");
    let planned = plan();
    assert_eq!(reason(&planned, &f), Some("set_by_dj".into()), "{planned}");
    assert!(
        planned["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["track"] == e && c["label"] == "Peak" && c["change"] == "remove"),
        "{planned}"
    );
}
