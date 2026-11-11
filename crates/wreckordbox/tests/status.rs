//! `wreckordbox status` against a Library service on a throwaway Library.
//!
//! Every test builds its own fixture Library in a temporary folder, with a
//! private runtime directory and home, and drives the real `wreckordbox`
//! binary. Nothing here can reach the installed Library.
#![allow(clippy::pedantic, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::Sandbox;

#[test]
fn status_starts_a_service_and_reports_the_library_it_has_open() {
    let sandbox = Sandbox::new();

    let run = sandbox.cli(&["status"]);

    assert!(run.status.success(), "{}", run.stderr);
    assert!(run.stdout.contains("running"), "{}", run.stdout);
    assert!(run.stdout.contains(sandbox.master_db().to_str().unwrap()), "{}", run.stdout);
    assert!(run.stdout.contains("40 tracks"), "{}", run.stdout);
    assert_eq!(run.stderr, "");
}

#[test]
fn status_connects_to_a_running_service_instead_of_starting_another() {
    let sandbox = Sandbox::new();

    let first = sandbox.status();
    let second = sandbox.status();

    assert_eq!(first["pid"], second["pid"]);
    assert!(common::alive(first["pid"].as_u64().unwrap()));
}

#[test]
fn a_second_client_sees_the_service_the_cli_started() {
    let sandbox = Sandbox::new();
    let reported = sandbox.status();

    let seen = sandbox.client().status().unwrap();

    assert_eq!(u64::from(seen.pid), reported["pid"].as_u64().unwrap());
    assert_eq!(seen.library.path, sandbox.master_db());
    assert_eq!(seen.library.tracks, 40);
}

#[test]
fn status_as_json_is_machine_readable() {
    let sandbox = Sandbox::new();

    let run = sandbox.cli(&["status", "--json"]);

    assert!(run.status.success(), "{}", run.stderr);
    let status = run.json();
    assert_eq!(status["running"], true);
    assert_eq!(status["library"]["path"], sandbox.master_db().to_str().unwrap());
    assert_eq!(status["library"]["tracks"], 40);
    assert_eq!(status["protocol"], 1);
    assert!(status["pid"].as_u64().unwrap() > 0);
    assert_ne!(status["pid"].as_u64().unwrap(), u64::from(std::process::id()));
    assert_eq!(run.stderr, "");
}

#[test]
fn a_second_service_for_the_same_library_refuses_to_run_and_says_why() {
    let sandbox = Sandbox::new();
    let running = sandbox.status();

    // Even from another runtime directory: the rule is one per Library.
    let elsewhere = sandbox.path().join("other-run");
    let second = sandbox.run(sandbox.command().env("WRECKORDBOX_RUNTIME_DIR", &elsewhere).arg("serve"));

    assert_eq!(second.status.code(), Some(1));
    assert!(second.stderr.contains("already running"), "{}", second.stderr);
    assert!(second.stderr.contains(&format!("pid {}", running["pid"])), "{}", second.stderr);
    assert_eq!(second.stdout, "");
    // The first is untouched.
    assert_eq!(sandbox.status()["pid"], running["pid"]);
}

#[test]
fn a_service_run_by_hand_is_the_one_the_cli_uses() {
    let sandbox = Sandbox::new();
    let mut service = sandbox.serve();

    let status = sandbox.status();

    assert_eq!(status["pid"].as_u64().unwrap(), u64::from(service.id()));
    service.kill().unwrap();
    service.wait().unwrap();
}

#[test]
fn the_socket_is_private_to_the_user() {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    let sandbox = Sandbox::new();

    let status = sandbox.status();

    let socket = std::path::PathBuf::from(status["socket"].as_str().unwrap());
    assert_eq!(socket.parent().unwrap(), sandbox.runtime_dir());
    let metadata = std::fs::metadata(&socket).unwrap();
    assert!(metadata.file_type().is_socket());
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    let dir = std::fs::metadata(sandbox.runtime_dir()).unwrap();
    assert_eq!(dir.permissions().mode() & 0o777, 0o700);
}

#[test]
fn the_socket_lives_in_the_users_runtime_directory_by_default() {
    let sandbox = Sandbox::new();
    let runtime = sandbox.path().join("xdg-runtime");
    std::fs::create_dir_all(&runtime).unwrap();

    let run = sandbox.run(
        sandbox.command().env_remove("WRECKORDBOX_RUNTIME_DIR").env("XDG_RUNTIME_DIR", &runtime).args(["--json", "status"]),
    );

    assert!(run.status.success(), "{}", run.stderr);
    let socket = std::path::PathBuf::from(run.json()["socket"].as_str().unwrap());
    assert_eq!(socket.parent().unwrap(), runtime.join("wreckordbox"));
}

#[test]
fn a_service_stops_when_nothing_has_used_it_for_a_while() {
    let mut sandbox = Sandbox::new();
    sandbox.idle_exit_secs = 1;
    let pid = sandbox.status()["pid"].as_u64().unwrap();

    let gone = (0..100).any(|_| {
        std::thread::sleep(std::time::Duration::from_millis(100));
        !common::alive(pid)
    });

    assert!(gone, "the service is still running");
    // And the next command starts a fresh one.
    assert_ne!(sandbox.status()["pid"].as_u64().unwrap(), pid);
}

#[test]
fn a_missing_library_is_reported_as_missing() {
    let sandbox = Sandbox::new();
    std::fs::remove_file(sandbox.master_db()).unwrap();

    let run = sandbox.cli(&["status"]);

    assert_eq!(run.status.code(), Some(1));
    assert!(run.stderr.contains("the Library is missing"), "{}", run.stderr);
    assert!(run.stderr.contains(sandbox.master_db().to_str().unwrap()), "{}", run.stderr);
    assert_eq!(run.stdout, "");
}

#[test]
fn a_damaged_library_is_reported_as_damaged() {
    let sandbox = Sandbox::new();
    std::fs::write(sandbox.master_db(), vec![0x5a_u8; 8192]).unwrap();

    let run = sandbox.cli(&["status"]);

    assert_eq!(run.status.code(), Some(1));
    assert!(run.stderr.contains("the Library is damaged"), "{}", run.stderr);
    assert_eq!(run.stdout, "");
}

#[test]
fn a_locked_library_is_reported_as_locked() {
    let sandbox = Sandbox::new();
    // Another program in the middle of writing the Library.
    let writer = rbl_db::Library::open(sandbox.location().clone(), rbl_db::OpenMode::ReadWrite).unwrap();
    writer.connection().execute_batch("BEGIN EXCLUSIVE").unwrap();

    let run = sandbox.cli(&["status"]);

    assert_eq!(run.status.code(), Some(1));
    assert!(run.stderr.contains("the Library is locked"), "{}", run.stderr);
    assert_eq!(run.stdout, "");
}

#[test]
fn errors_as_json_go_to_standard_error_with_a_stable_code() {
    let sandbox = Sandbox::new();
    std::fs::remove_file(sandbox.master_db()).unwrap();
    let missing = sandbox.cli(&["--json", "status"]);
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(missing.error_json()["error"]["code"], "library_missing");
    assert_eq!(missing.stdout, "");

    let sandbox = Sandbox::new();
    std::fs::write(sandbox.master_db(), vec![0x5a_u8; 8192]).unwrap();
    let damaged = sandbox.cli(&["--json", "status"]);
    assert_eq!(damaged.status.code(), Some(1));
    assert_eq!(damaged.error_json()["error"]["code"], "library_damaged");
    assert!(damaged.error_json()["error"]["message"].as_str().unwrap().contains("damaged"));
}

#[test]
fn a_failed_start_leaves_no_service_behind() {
    let sandbox = Sandbox::new();
    let good = std::fs::read(sandbox.master_db()).unwrap();
    std::fs::write(sandbox.master_db(), vec![0x5a_u8; 8192]).unwrap();
    assert_eq!(sandbox.cli(&["status"]).status.code(), Some(1));

    // Once the Library is back, a service starts: the failed one let go.
    std::fs::write(sandbox.master_db(), good).unwrap();

    assert_eq!(sandbox.status()["library"]["tracks"], 40);
}

#[test]
fn help_describes_the_subcommands_and_options() {
    let sandbox = Sandbox::new();

    let help = sandbox.cli(&["--help"]);

    assert!(help.status.success());
    assert!(help.stdout.contains("status"), "{}", help.stdout);
    assert!(help.stdout.contains("--json"), "{}", help.stdout);
    assert!(help.stdout.contains("Library service"), "{}", help.stdout);

    let status = sandbox.cli(&["status", "--help"]);
    assert!(status.status.success());
    assert!(status.stdout.contains("which Library it"), "{}", status.stdout);
    assert!(status.stdout.contains("--json"), "{}", status.stdout);
    // Asking for help starts nothing.
    assert!(!sandbox.runtime_dir().exists());
}

#[test]
fn an_unknown_subcommand_fails_without_starting_a_service() {
    let sandbox = Sandbox::new();

    let run = sandbox.cli(&["frobnicate"]);

    assert_eq!(run.status.code(), Some(2));
    assert!(run.stderr.contains("frobnicate"), "{}", run.stderr);
    assert!(!sandbox.runtime_dir().exists());
}

#[test]
fn an_idle_service_stops_even_when_its_socket_file_has_been_removed() {
    let mut sandbox = Sandbox::new();
    sandbox.idle_exit_secs = 1;
    let status = sandbox.status();
    let pid = status["pid"].as_u64().unwrap();
    std::fs::remove_file(status["socket"].as_str().unwrap()).unwrap();

    let gone = (0..100).any(|_| {
        std::thread::sleep(std::time::Duration::from_millis(100));
        !common::alive(pid)
    });

    assert!(gone, "the service is still running");
}
