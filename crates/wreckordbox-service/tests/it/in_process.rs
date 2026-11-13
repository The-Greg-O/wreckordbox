//! A service hosted in the test's own process, as the window will host it,
//! seen by clients on its socket. The Library is a throwaway fixture.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::sync::Arc;
use std::time::Duration;

use rbl_db::fixture::{self, Shape};
use rbl_library::{History, LibraryCore, NoHost, Touched};
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
        recovery_dir: dir.path().join("recovery"),
    };
    // As on the machine of a DJ who edits: Library Protection is off.
    wreckordbox_service::protection::store(&config.recovery_dir, false).unwrap();
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
        recovery_dir: socket.parent().unwrap().join("recovery"),
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

#[test]
fn the_hosting_process_runs_commands_and_hears_changes_without_the_socket() {
    let hosted = host();
    let events = hosted.service.events();
    let mut client = Client::connect(&hosted.socket).unwrap();
    client.subscribe().unwrap();
    let id = fixture::track_id(1);

    let rated = hosted
        .service
        .call("tracks.rate", json!({ "id": id, "rating": 3 }))
        .unwrap();

    assert_eq!(rated["previous_rating"], 0);
    assert_eq!(rated["rating"], 3);
    // The host and a client on the socket are told the same thing.
    let heard = events.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(heard.event, "library_changed");
    assert_eq!(
        heard.data,
        json!({ "kind": "library", "generation": rated["generation"] })
    );
    assert_eq!(
        client.next_event(Duration::from_secs(5)).unwrap(),
        Some(heard)
    );
    // And the host's edit is what a client then lists.
    let listed = client.tracks(&Default::default()).unwrap();
    assert_eq!(
        listed
            .tracks
            .iter()
            .find(|track| track.id == id)
            .unwrap()
            .rating,
        3
    );
    // A client's edit reaches the host too.
    client.rate_track(&id, 5).unwrap();
    assert_eq!(
        events.recv_timeout(Duration::from_secs(5)).unwrap().event,
        "library_changed"
    );

    let unknown = hosted
        .service
        .call("no.such.command", json!({}))
        .unwrap_err();
    assert_eq!(unknown.code, codes::UNKNOWN_COMMAND);
}

/// A front end's own Library core over a fixture, loaded as the window
/// loads its own, with the service hosted on it.
struct SharedCore {
    _dir: tempfile::TempDir,
    core: Arc<LibraryCore>,
    location: rbl_db::LibraryLocation,
    config: Config,
}

fn shared_core() -> SharedCore {
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
        recovery_dir: dir.path().join("recovery"),
    };
    // As on the machine of a DJ who edits: Library Protection is off.
    wreckordbox_service::protection::store(&config.recovery_dir, false).unwrap();
    let core = Arc::new(LibraryCore::new(config.recovery_dir.clone()));
    SharedCore {
        _dir: dir,
        core,
        location,
        config,
    }
}

fn no_host() -> Arc<dyn rbl_library::Host + Send + Sync> {
    Arc::new(NoHost)
}

impl SharedCore {
    fn load(&self) {
        let db = rbl_db::Library::open(self.location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
        let (index, _) = rbl_index::load(&db).unwrap();
        self.core.set_library(
            &NoHost,
            index,
            false,
            db.schema().db_version,
            0,
            self.location.clone(),
        );
    }

    fn host(&self) -> Service {
        Service::host(&self.location, &self.config, &self.core, &no_host()).unwrap()
    }

    fn client(&self) -> Client {
        Client::connect(&self.config.socket_path(&self.location)).unwrap()
    }

    fn rating(&self, id: &str) -> u8 {
        let index = self.core.library().unwrap();
        index.rating[index.row_of(id).unwrap() as usize]
    }
}

#[test]
fn a_service_hosted_on_a_front_ends_core_shares_its_library_and_generation() {
    let front = shared_core();
    front.load();
    let _service = front.host();
    let mut client = front.client();
    let id = fixture::track_id(1);

    // A client's edit is an edit of the front end's own Library.
    let rated = client.rate_track(&id, 4).unwrap();
    assert_eq!(front.rating(&id), 4);
    assert_eq!(rated.generation, front.core.generation());

    // The front end's edit is what a client then lists, one generation on.
    let (_, change) = front
        .core
        .edit(
            &NoHost,
            History::Outside,
            Touched::Metadata(vec![id.clone()]),
            |writer| writer.set_rating(&id, 2),
        )
        .unwrap();
    assert_eq!(change.generation, rated.generation + 1);
    let listed = client.tracks(&Default::default()).unwrap();
    assert_eq!(listed.tracks.iter().find(|t| t.id == id).unwrap().rating, 2);
    assert_eq!(client.status().unwrap().library.tracks, 3);
}

#[test]
fn a_hosted_service_answers_loading_until_the_front_end_has_read_the_library() {
    let front = shared_core();
    let _service = front.host();
    let mut client = front.client();

    assert_eq!(
        client.tracks(&Default::default()).unwrap_err().code,
        codes::LIBRARY_LOADING
    );

    front.load();
    assert_eq!(client.tracks(&Default::default()).unwrap().total, 3);
}

#[test]
fn a_service_by_itself_stops_when_asked_and_a_front_end_takes_the_library_over() {
    let front = shared_core();
    let alone = Service::start(front.location.clone(), &front.config).unwrap();
    let mut other = front.client();
    other.status().unwrap();

    // Hosting finds the running service, asks it to stop, and takes its place.
    front.load();
    let hosted = front.host();

    alone.wait();
    assert_eq!(other.status().unwrap_err().code, codes::SERVICE_FAILED);
    let mut client = front.client();
    assert_eq!(
        client.status().unwrap().pid,
        hosted.status().unwrap().pid,
        "the socket now answers for the front end's service"
    );
    client.rate_track(&fixture::track_id(0), 5).unwrap();
    assert_eq!(front.rating(&fixture::track_id(0)), 5);
}

#[test]
fn a_hosted_service_refuses_to_stop_for_a_client_so_its_front_end_stays_the_writer() {
    let front = shared_core();
    front.load();
    let _hosted = front.host();
    let mut client = front.client();

    assert_eq!(client.stop().unwrap_err().code, codes::STOP_REFUSED);
    assert!(client.status().unwrap().running);

    // A second front end cannot take the Library from the first.
    let second = Service::host(
        &front.location,
        &front.config,
        &Arc::new(LibraryCore::new(front.config.recovery_dir.clone())),
        &no_host(),
    )
    .map(|_| ())
    .unwrap_err();
    assert_eq!(second.code, codes::STOP_REFUSED);
    assert!(client.status().unwrap().running);
}

#[test]
fn a_front_end_is_told_when_its_service_stops() {
    let front = shared_core();
    front.load();
    let service = front.host();
    let stopped = service.stopped();
    let (told, hears) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        stopped.wait();
        let _ = told.send(());
    });
    assert!(hears.recv_timeout(Duration::from_millis(200)).is_err());

    service.stop();

    hears.recv_timeout(Duration::from_secs(5)).unwrap();
}

#[test]
fn a_front_ends_own_change_is_sent_to_clients_and_not_back_to_the_front_end() {
    let front = shared_core();
    front.load();
    let service = front.host();
    let events = service.events();
    let mut client = front.client();
    client.subscribe().unwrap();
    let id = fixture::track_id(1);

    // The front end edits through its own core, as the window's commands do.
    let (_, change) = front
        .core
        .edit(
            &NoHost,
            History::Outside,
            Touched::Metadata(vec![id.clone()]),
            |writer| writer.set_rating(&id, 2),
        )
        .unwrap();

    assert_eq!(
        client
            .next_event(Duration::from_secs(5))
            .unwrap()
            .unwrap()
            .data,
        json!({ "kind": "library", "generation": change.generation })
    );
    assert!(events.recv_timeout(Duration::from_millis(200)).is_err());

    // A client's change is sent to clients once, and to the front end.
    let rated = client.rate_track(&id, 4).unwrap();
    assert_eq!(
        client
            .next_event(Duration::from_secs(5))
            .unwrap()
            .unwrap()
            .data,
        json!({ "kind": "library", "generation": rated.generation })
    );
    assert_eq!(client.next_event(Duration::from_millis(200)).unwrap(), None);
    assert_eq!(
        events.recv_timeout(Duration::from_secs(5)).unwrap().data,
        json!({ "kind": "library", "generation": rated.generation })
    );
}

#[test]
fn turning_protection_tells_subscribed_clients_in_order_with_a_rising_revision() {
    let hosted = host();
    let mut client = Client::connect(&hosted.socket).unwrap();
    client.subscribe().unwrap();

    let on: serde_json::Value = client
        .call("protection.set", json!({ "protect_library": true }))
        .unwrap();
    let off: serde_json::Value = client
        .call("protection.set", json!({ "protect_library": false }))
        .unwrap();
    let again: serde_json::Value = client
        .call("protection.set", json!({ "protect_library": true }))
        .unwrap();

    assert_eq!(on["protect_library"], true);
    assert_eq!(on["previous"], false);
    assert_eq!(off["previous"], true);
    let revisions: Vec<u64> = [&on, &off, &again]
        .iter()
        .map(|set| set["revision"].as_u64().unwrap())
        .collect();
    assert!(
        revisions.windows(2).all(|pair| pair[0] < pair[1]),
        "{revisions:?}"
    );
    for (set, on) in [(&on, true), (&off, false), (&again, true)] {
        assert_eq!(
            client.next_event(Duration::from_secs(5)).unwrap(),
            Some(Event {
                event: "protection_changed".into(),
                data: json!({ "protect_library": on, "revision": set["revision"] }),
            })
        );
    }
    // The hosting process's own commands are held to the switch as well.
    let refused = hosted
        .service
        .call(
            "tracks.rate",
            json!({ "id": fixture::track_id(1), "rating": 3 }),
        )
        .unwrap_err();
    assert_eq!(refused.code, codes::LIBRARY_PROTECTED);
}
