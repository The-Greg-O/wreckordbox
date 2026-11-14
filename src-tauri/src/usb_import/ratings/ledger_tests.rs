//! Round-trip through a hosted native Library; the Ledger is read by the CLI.
#![allow(clippy::pedantic, clippy::unwrap_used, clippy::expect_used)]
use super::*;
use serde_json::{json, Value};
use std::{process::Command, sync::Arc};
use wreckordbox_service::{Config, Service};

fn cli(f: &Fixture, args: &[&str]) -> Value {
    let exe = std::env::current_exe().unwrap();
    let binary = exe.parent().unwrap().parent().unwrap().join("wreckordbox");
    assert!(
        binary.is_file(),
        "prebuild wreckordbox through the heavy queue"
    );
    let output = Command::new(binary)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", f._dir.path().join("home"))
        .env("RBXPORT_TEST", "1")
        .env("RBXPORT_OPTIONS", f._dir.path().join("options.json"))
        .env("RBXPORT_TEST_FIXTURE", f._dir.path())
        .env("RBXPORT_STATE_DIR", f.state.backup_dir())
        .env("WRECKORDBOX_RUNTIME_DIR", f._dir.path().join("run"))
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn player_cleared_rating_is_recorded_without_dj_facts_and_projection_keeps_it() {
    let f = Fixture::new();
    let location = f.state.location().unwrap();
    rbl_db::fixture::write_options_json(
        &f._dir.path().join("options.json"),
        location.master_db.to_str().unwrap(),
        &location.passphrase,
    )
    .unwrap();
    let config = Config {
        runtime_dir: f._dir.path().join("run"),
        idle_exit: None,
        recovery_dir: f.state.backup_dir().to_owned(),
    };
    let host: Arc<dyn rbl_library::Host + Send + Sync> = Arc::new(rbl_library::NoHost);
    let service = Service::host(&location, &config, &f.state.shared_core(), &host).unwrap();
    let id = rbl_db::fixture::track_id(0);
    f.one("UPDATE content SET rating=0 WHERE content_id=1");
    assert_eq!(f.import().unwrap().ratings, 1);
    assert_eq!(f.desktop(0).0, 0);
    let runs = cli(&f, &["runs", "list"]);
    assert_eq!(runs["total"], 1, "accepted player rating has its own Run");
    assert_eq!(
        runs["runs"][0]["source"],
        json!({"kind":"tool","name":"round-trip"})
    );
    let shown = cli(&f, &["runs", "show", &runs["runs"][0]["id"].to_string()]);
    assert_eq!(shown["total"], 1);
    assert_eq!(shown["entries"][0]["kind"], "track");
    assert_eq!(
        shown["entries"][0]["payload"]["edits"],
        json!([{"track":id,"column":"Rating","before":2,"after":0}])
    );
    assert_eq!(
        cli(&f, &["facts", "show", &id, "--all"])["facts"],
        json!([])
    );
    assert_eq!(f.import().unwrap().ratings, 0);
    assert_eq!(
        cli(&f, &["runs", "list"])["total"],
        1,
        "repeat is not another Run"
    );
    f.state
        .read_db(|db| {
            crate::commands::refresh_after_edit(
                &f.state,
                db,
                crate::commands::Touched::Metadata(vec![id.clone()]),
            )
        })
        .unwrap();
    service.call("facts.record", json!({"source":{"kind":"agent","name":"tagger"},"facts":[{"track":id,"kind":"energy","value":5,"confidence":1.0}]})).unwrap();
    std::fs::write(
        wreckordbox_sidecar::default_dir(&location.master_db).join("projection-policy.toml"),
        "[energy]\napply = 0.8\n",
    )
    .unwrap();
    let applied = service
        .call("projection.apply", json!({"tracks":[id]}))
        .unwrap();
    assert_eq!(
        applied["changes"],
        json!([]),
        "projection must not refill a rating deliberately cleared on a player: {applied}"
    );
    assert_eq!(f.desktop(0).0, 0);
}
