//! A service hosted in the test's own process, as the window will host it,
//! seen by clients on its socket. The Library is a throwaway fixture.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::time::Duration;

use rbl_db::fixture::{self, Shape};
use serde_json::json;
use wreckordbox_service::{codes, Client, Config, Event, Service};

struct Hosted {
    _dir: tempfile::TempDir,
    service: Service,
    socket: std::path::PathBuf,
}

fn host() -> Hosted {
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
    let config = Config {
        runtime_dir: dir.path().join("run"),
        idle_exit: None,
    };
    let socket = config.socket_path(&location);
    let service = Service::start(location, &config).unwrap();
    Hosted {
        _dir: dir,
        service,
        socket,
    }
}

#[test]
fn a_subscribed_client_is_sent_events_and_an_unsubscribed_one_is_not() {
    let hosted = host();
    let mut subscribed = Client::connect(&hosted.socket).unwrap();
    let mut other = Client::connect(&hosted.socket).unwrap();
    subscribed.subscribe().unwrap();
    // Both connections are known to the service once each has been answered.
    other.status().unwrap();

    let event = Event {
        event: "library_changed".into(),
        data: json!({ "kind": "tracks", "generation": 7 }),
    };
    hosted.service.publish(&event);

    assert_eq!(
        subscribed.next_event(Duration::from_secs(5)).unwrap(),
        Some(event)
    );
    assert_eq!(other.next_event(Duration::from_millis(200)).unwrap(), None);
    // A subscribed connection still answers commands.
    assert_eq!(subscribed.status().unwrap().library.tracks, 3);
}

#[test]
fn an_event_that_arrives_before_a_commands_answer_is_kept() {
    let hosted = host();
    let mut client = Client::connect(&hosted.socket).unwrap();
    client.subscribe().unwrap();
    let event = Event {
        event: "library_changed".into(),
        data: json!({ "generation": 1 }),
    };
    hosted.service.publish(&event);

    // The event is already on the wire, ahead of this answer.
    assert_eq!(client.status().unwrap().library.tracks, 3);

    assert_eq!(
        client.next_event(Duration::from_secs(5)).unwrap(),
        Some(event)
    );
}

#[test]
fn an_unknown_command_is_refused_by_code_and_the_connection_survives() {
    let hosted = host();
    let mut client = Client::connect(&hosted.socket).unwrap();

    let refused = client
        .call::<serde_json::Value>("frobnicate", json!({}))
        .unwrap_err();

    assert_eq!(refused.code, codes::UNKNOWN_COMMAND);
    assert!(client.status().unwrap().running);
}

#[test]
fn a_stopped_service_lets_go_of_the_library() {
    let hosted = host();
    let socket = hosted.socket.clone();
    let mut client = Client::connect(&socket).unwrap();
    let location = rbl_db::LibraryLocation {
        master_db: client.status().unwrap().library.path,
        share_root: std::path::PathBuf::new(),
        passphrase: fixture::FIXTURE_PASSPHRASE.into(),
        is_real_install: false,
    };
    let config = Config {
        runtime_dir: socket.parent().unwrap().to_path_buf(),
        idle_exit: None,
    };
    let second = Service::start(location.clone(), &config)
        .map(|_| ())
        .unwrap_err();
    assert_eq!(second.code, codes::SERVICE_ALREADY_RUNNING);

    drop(hosted.service);

    assert_eq!(client.status().unwrap_err().code, codes::SERVICE_FAILED);
    assert_eq!(
        Client::connect(&socket).unwrap_err().code,
        codes::SERVICE_NOT_RUNNING
    );
    let again = Service::start(location, &config).unwrap();
    assert_eq!(again.status().unwrap().library.tracks, 3);
}
