//! MyTag definitions (wbx-14.14) in a service a front end hosts on its own
//! Library core: a projection that makes a MyTag and puts it on tracks is
//! ONE step of the window's Undo, which takes the tag and its assignments
//! away together and Redo brings both back, as often as the DJ likes; a
//! Redo does not hand the MyTag back to the projection (a relabelled term
//! then keeps it as it is), no DJ Fact is ever made, and a MyTag the DJ
//! undid is never made again.
//!
//! A second test makes each half of an Undo and a Redo fail in turn (a
//! SQLite trigger refusing one row) and checks that nothing at all was
//! changed or recorded (review round 1, finding 4).
//!
//! In a test binary of its own: the Sidecar's folder comes from the
//! environment, the process's, so the tests take turns (`TURN`).
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

static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn a_projection_that_makes_a_mytag_undoes_and_redoes_in_one_step() {
    let _turn = TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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

/// Every `djmdSongMyTag` and `djmdMyTag` row, live or not, with its update
/// number: what "nothing changed" means for the Library's MyTags.
fn native_rows(location: &rbl_db::LibraryLocation) -> Vec<String> {
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let conn = db.connection();
    let mut rows = Vec::new();
    for sql in [
        "SELECT ID || ' ' || MyTagID || ' ' || ContentID || ' ' || rb_local_deleted || ' ' || COALESCE(rb_local_usn, 0) FROM djmdSongMyTag ORDER BY ID",
        "SELECT ID || ' ' || COALESCE(Name, '') || ' ' || COALESCE(Seq, 0) || ' ' || rb_local_deleted || ' ' || COALESCE(rb_local_usn, 0) FROM djmdMyTag ORDER BY ID",
    ] {
        let mut statement = conn.prepare(sql).unwrap();
        rows.extend(
            statement
                .query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .map(Result::unwrap),
        );
    }
    rows
}

fn on_library(location: &rbl_db::LibraryLocation, dir: &std::path::Path, sql: &str) {
    let mut writer = rbl_db::write::Writer::open(location.clone(), dir.join("backups")).unwrap();
    writer.disable_automatic_backups();
    writer.library().connection().execute_batch(sql).unwrap();
}

fn entries(sidecar: &std::path::Path) -> i64 {
    rusqlite::Connection::open_with_flags(
        sidecar.join("sidecar.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap()
    .query_row("SELECT COUNT(*) FROM ledger", [], |r| r.get(0))
    .unwrap()
}

#[test]
fn undo_and_redo_of_a_projection_that_made_a_mytag_are_all_or_nothing() {
    let _turn = TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
    core.set_library(&NoHost, index, false, db.schema().db_version, 0, location.clone());
    drop(db);
    let host: Arc<dyn rbl_library::Host + Send + Sync> = Arc::new(NoHost);
    let service = Service::host(&location, &config, &core, &host).unwrap();
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
    service.call("projection.mytags.apply", json!({})).unwrap();
    let made: String = {
        let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
        db.connection()
            .query_row(
                "SELECT ID FROM djmdMyTag WHERE Name = 'Afterhours' AND rb_local_deleted = 0",
                [],
                |r| r.get(0),
            )
            .unwrap()
    };
    let carried = |live: bool| -> i64 {
        let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
        db.connection()
            .query_row(
                "SELECT COUNT(*) FROM djmdSongMyTag WHERE MyTagID = ?1 AND rb_local_deleted = ?2",
                rusqlite::params![made, i64::from(!live)],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(carried(true), 2);
    let label = wreckordbox_service::mytag_projection::UNDO_LABEL;

    // A step refused part-way changes nothing and records nothing.
    let refused_whole = |undo: bool, trigger: &str| {
        let before = (native_rows(&location), entries(&sidecar));
        on_library(&location, dir.path(), trigger);
        assert!(
            core.step_history(&NoHost, undo).is_err(),
            "{trigger}: the step should fail"
        );
        on_library(&location, dir.path(), "DROP TRIGGER injected;");
        assert_eq!(
            (native_rows(&location), entries(&sidecar)),
            before,
            "{trigger}: a refused step changed or recorded something"
        );
        let history = core.history().lock();
        let next = if undo { history.undo_label() } else { history.redo_label() };
        assert_eq!(next, Some(label), "{trigger}: the step stays to be taken");
    };
    let refuse = |what: &str, when: &str| {
        format!(
            "CREATE TRIGGER injected BEFORE {what} {when} BEGIN SELECT RAISE(ABORT, 'injected'); END;"
        )
    };
    // Undo takes the tracks back last first: refuse the second one (a),
    // then the definition once both assignments are off.
    refused_whole(
        true,
        &refuse("UPDATE ON djmdSongMyTag", &format!("WHEN NEW.ContentID = '{a}'")),
    );
    refused_whole(true, &refuse("UPDATE ON djmdMyTag", ""));
    core.step_history(&NoHost, true).unwrap();
    assert_eq!(carried(true), 0, "no live assignment of the made MyTag after Undo");

    // Redo makes the MyTag, then puts it on a, then b: refuse b, then the
    // definition.
    refused_whole(
        false,
        &refuse("INSERT ON djmdSongMyTag", &format!("WHEN NEW.ContentID = '{b}'")),
    );
    refused_whole(false, &refuse("UPDATE ON djmdMyTag", ""));
    core.step_history(&NoHost, false).unwrap();
    assert_eq!(carried(true), 2);
}
