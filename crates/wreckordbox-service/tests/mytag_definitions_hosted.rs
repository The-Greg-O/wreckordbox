//! MyTag definitions (wbx-14.14) in a service a front end hosts on its own
//! Library core: a projection that makes a MyTag and puts it on tracks is
//! ONE step of the window's Undo, which takes the tag and its assignments
//! away together and Redo brings both back, as often as the DJ likes; a
//! Redo does not hand the MyTag back to the projection (a relabelled term
//! then keeps it as it is), no DJ Fact is ever made, and a MyTag the DJ
//! undid is never made again.
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

use rbl_db::fixture::{self, Shape};
use rbl_library::{LibraryCore, NoHost};
use serde_json::{json, Value};
use wreckordbox_service::stems::SIDECAR_DIR_ENV;
use wreckordbox_service::{Config, Service};

fn schema(label: &str) -> String {
    format!(
        r#"version = 1

[dimensions.situation]
terms = [
  {{ id = "peak", name = "Peak" }},
  {{ id = "afterhours", name = "{label}" }},
]

[deck]
categories = [
  {{ dimension = "situation", label = "Situation" }},
]
"#
    )
}

#[test]
fn a_projection_that_makes_a_mytag_undoes_and_redoes_in_one_step() {
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
    std::fs::write(sidecar.join("tag-schema.toml"), schema("Afterhours")).unwrap();
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
    for track in [&a, &b] {
        service
            .call(
                "facts.record",
                json!({"source": {"kind": "agent", "name": "tagger"},
                    "facts": [{"track": track, "kind": "situation", "value": ["afterhours"],
                    "confidence": 0.9}]}),
            )
            .unwrap();
    }
    // The Situation category's tags: `(id, name, tracks carrying it)`.
    let situation = || -> Vec<(String, String, i64)> {
        let listed = service.call("mytags.list", json!({})).unwrap();
        listed["categories"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "Situation")
            .unwrap()["tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                (
                    t["id"].as_str().unwrap().to_owned(),
                    t["name"].as_str().unwrap().to_owned(),
                    t["tracks"].as_i64().unwrap_or(-1),
                )
            })
            .collect()
    };
    let afterhours = || {
        situation()
            .into_iter()
            .find(|(_, name, _)| name == "Afterhours")
    };
    let no_dj_fact = |track: &str| {
        let standing = service
            .call("facts.standing", json!({"id": track, "kind": "situation"}))
            .unwrap();
        assert_eq!(
            standing["standing"]["source"]["kind"], "agent",
            "{standing}"
        );
    };
    let drain = || while events.recv_timeout(Duration::from_millis(100)).is_ok() {};
    let plan = || service.call("projection.mytags.plan", json!({})).unwrap();
    let kept = |plan: &Value| -> Vec<(String, String)> {
        plan["definitions_kept"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(|d| {
                (
                    d["change"].as_str().unwrap().to_owned(),
                    d["reason"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    };

    drain();
    let applied = service.call("projection.mytags.apply", json!({})).unwrap();
    assert_eq!(
        applied["definitions"][0]["change"], "create_tag",
        "{applied}"
    );
    assert_eq!(applied["changes"].as_array().unwrap().len(), 2, "{applied}");
    assert_eq!(
        events.recv_timeout(Duration::from_secs(2)).unwrap().event,
        "library_changed"
    );
    let (made, _, carried) = afterhours().expect("made");
    assert_eq!(carried, 2);
    assert_eq!(
        core.history().lock().undo_label(),
        Some(wreckordbox_service::mytag_projection::UNDO_LABEL)
    );

    // One Undo takes the MyTag and both assignments away; one Redo puts
    // them back, the same MyTag; as often as the DJ goes back and forth.
    for _ in 0..3 {
        core.step_history(&NoHost, true).unwrap();
        assert_eq!(afterhours(), None, "after Undo: {:?}", situation());
        core.step_history(&NoHost, false).unwrap();
        assert_eq!(afterhours(), Some((made.clone(), "Afterhours".into(), 2)));
        for track in [&a, &b] {
            no_dj_fact(track);
        }
    }
    // Nothing more to do; and the Redo is not the projection's own write:
    // relabelling the term keeps the MyTag as it is.
    let planned = plan();
    assert_eq!(planned["definitions"], json!([]), "{planned}");
    std::fs::write(sidecar.join("tag-schema.toml"), schema("After Hours")).unwrap();
    let planned = plan();
    assert_eq!(planned["definitions"], json!([]), "{planned}");
    assert_eq!(
        kept(&planned),
        [("rename_tag".into(), "written_since".into())]
    );

    // A MyTag the DJ undid is not made again.
    std::fs::write(sidecar.join("tag-schema.toml"), schema("Afterhours")).unwrap();
    core.step_history(&NoHost, true).unwrap();
    assert_eq!(afterhours(), None);
    let planned = plan();
    assert_eq!(planned["definitions"], json!([]), "{planned}");
    assert_eq!(kept(&planned), [("create_tag".into(), "removed".into())]);
    for track in [&a, &b] {
        no_dj_fact(track);
    }
}
