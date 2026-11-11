//! Wreckordbox never contacts upstream's infrastructure and never presents
//! upstream's services as its own: no update feed or signing key, no bug
//! report window or report host, and no crash-reporting client. What the
//! shell ships is its configuration, its capabilities and its dependencies,
//! so those are what is read here.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::Value;

const CONFIG: &str = include_str!("../tauri.conf.json");
const LINUX_CONFIG: &str = include_str!("../tauri.linux.conf.json");
const CAPABILITIES: &str = include_str!("../capabilities/default.json");
const MANIFEST: &str = include_str!("../Cargo.toml");

fn config() -> Value {
    serde_json::from_str(CONFIG).unwrap()
}

#[test]
fn the_configuration_and_capabilities_name_no_upstream_host() {
    for (name, text) in [("tauri.conf.json", CONFIG), ("tauri.linux.conf.json", LINUX_CONFIG), ("capabilities/default.json", CAPABILITIES)] {
        assert!(!text.contains("rbxport.com"), "{name} names an rbxport-owned host");
    }
}

/// The page may talk to the app shell and to nothing on the network.
#[test]
fn the_content_security_policy_allows_no_remote_host() {
    let csp = config()["app"]["security"]["csp"].as_str().unwrap().to_owned();
    for source in csp.split([' ', ';']).filter(|s| s.starts_with("http")) {
        assert!(source.starts_with("http://") && source.ends_with(".localhost"), "the policy allows {source}");
    }
    assert!(!csp.contains("frame-src"), "no window frames a remote page");
}

#[test]
fn there_is_no_update_feed_signing_key_or_update_artefact() {
    let config = config();
    assert!(config["plugins"].get("updater").is_none(), "an updater is configured");
    assert!(config["bundle"].get("createUpdaterArtifacts").is_none(), "update artefacts are still built");
    assert!(!CONFIG.contains("pubkey"), "a signing key is configured");
}

#[test]
fn no_window_is_granted_to_a_bug_reporter() {
    let capabilities: Value = serde_json::from_str(CAPABILITIES).unwrap();
    let windows: Vec<&str> = capabilities["windows"].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
    assert_eq!(windows, ["main", "preferences", "sync", "log"]);
}

#[test]
fn the_shell_depends_on_no_updater_or_crash_reporting_client() {
    let manifest: Vec<&str> = MANIFEST.lines().filter(|line| !line.trim_start().starts_with('#')).collect();
    for dependency in ["tauri-plugin-updater", "sentry", "minisign-verify"] {
        assert!(!manifest.iter().any(|line| line.starts_with(dependency)), "{dependency} is still a dependency");
    }
}
