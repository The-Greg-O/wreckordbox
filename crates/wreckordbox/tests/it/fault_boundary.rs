#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
use crate::common::Sandbox;
#[test]
#[ignore = "requires WRECKORDBOX_NORMAL_BINARY built without the test-faults feature"]
fn a_normal_binary_ignores_test_fault_environment() {
    let binary =
        std::env::var_os("WRECKORDBOX_NORMAL_BINARY").expect("prebuild the normal CLI separately");
    let sandbox = Sandbox::new();
    let template = sandbox.command();
    let mut command = std::process::Command::new(binary);
    command.env_clear();
    command.args(template.get_args());
    for (key, value) in template.get_envs() {
        if let Some(value) = value {
            command.env(key, value);
        } else {
            command.env_remove(key);
        }
    }
    let output = command
        .env(
            "WRECKORDBOX_LEDGER_FAULTS",
            "abort_after_write,fail_append=999",
        )
        .args(["--json", "tracks", "rate", "10003", "4"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "normal binaries must not accept test fault switches: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let changed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let run = changed["run"]["id"].as_i64().unwrap();
    let shown: serde_json::Value = sandbox
        .client()
        .call("runs.show", serde_json::json!({"id":run}))
        .unwrap();
    assert_eq!(shown["total"], 1);
    assert_eq!(shown["entries"][0]["values"][0]["after"], 4);
    sandbox.client().stop().unwrap();
}
