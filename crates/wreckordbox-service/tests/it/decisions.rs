//! An answer that produces a Fact is one change to the Sidecar: the answer,
//! the Fact, what the Fact supersedes and the Run are all recorded or none
//! is, and clients are told only once they are. Asked of a service in the
//! test's own process, with clients on its socket. The Library is a
//! throwaway fixture.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::sync::{Arc, Barrier};
use std::time::Duration;

use rbl_db::fixture::{self, Shape};
use rbl_db::LibraryLocation;
use serde_json::{json, Value};
use wreckordbox_service::{Client, Config, Service};

struct Hosted {
    dir: tempfile::TempDir,
    location: LibraryLocation,
    config: Config,
    service: Service,
}

fn start() -> Hosted {
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
    let config = Config {
        runtime_dir: dir.path().join("run"),
        idle_exit: None,
        recovery_dir: dir.path().join("recovery"),
    };
    let service = Service::start(location.clone(), &config).unwrap();
    Hosted {
        dir,
        location,
        config,
        service,
    }
}

impl Hosted {
    fn call(&self, command: &str, args: Value) -> Value {
        self.service
            .call(command, args)
            .unwrap_or_else(|e| panic!("{command} failed: {} {}", e.code, e.message))
    }

    fn client(&self) -> Client {
        Client::connect(&self.config.socket_path(&self.location)).unwrap()
    }

    /// The Sidecar's database, which exists once a command has used it.
    fn sidecar_db(&self) -> rusqlite::Connection {
        let path = wreckordbox_sidecar::default_dir(&self.location.master_db).join("sidecar.db");
        assert!(
            path.starts_with(self.dir.path()),
            "the Sidecar is not in the test's folder: {}",
            path.display()
        );
        let db = rusqlite::Connection::open(path).unwrap();
        db.busy_timeout(Duration::from_secs(5)).unwrap();
        db
    }

    fn ask_genre(&self) -> i64 {
        self.call(
            "decisions.create",
            json!({
                "question_type": "choice",
                "question": "Which genre is this track?",
                "candidates": ["Techno", "Tech House"],
                "track": fixture::track_id(3),
            }),
        )["id"]
            .as_i64()
            .unwrap()
    }

    /// Every Fact of the track, superseded ones included, newest first.
    fn facts(&self) -> Vec<Value> {
        self.call(
            "facts.of_track",
            json!({ "id": fixture::track_id(3), "all": true }),
        )["facts"]
            .as_array()
            .unwrap()
            .clone()
    }
}

fn answer_with_fact(id: i64, choice: &str) -> Value {
    json!({
        "id": id,
        "decider": { "kind": "agent", "name": "tagger" },
        "choice": choice,
        "probability": 0.8,
        "fact": { "kind": "genre" },
    })
}

fn events(client: &mut Client) -> Vec<Value> {
    let mut seen = Vec::new();
    while let Some(event) = client.next_event(Duration::from_millis(300)).unwrap() {
        seen.push(json!({ "event": event.event, "data": event.data }));
    }
    seen
}

/// The next `count` events, each waited for as long as it takes, then
/// nothing more for the window [`events`] has always allowed (wbx-377): an
/// exact count that does not end at the first quiet moment.
fn exactly(client: &mut Client, count: usize) -> Vec<Value> {
    let mut seen = Vec::new();
    while seen.len() < count {
        let event = client
            .next_event(Duration::from_secs(30))
            .unwrap()
            .unwrap_or_else(|| panic!("{count} events were due: {seen:?}"));
        seen.push(json!({ "event": event.event, "data": event.data }));
    }
    seen.extend(events(client));
    seen
}

fn expect_sidecar_change(client: &mut Client, data: Value) {
    let event = client
        .next_event(Duration::from_secs(5))
        .unwrap()
        .expect("the committed setup change must reach the watcher");
    assert_eq!(event.event, "sidecar_changed");
    assert_eq!(event.data, data);
}

#[test]
fn when_the_answer_cannot_be_written_its_fact_is_not_recorded_and_no_client_is_told() {
    let hosted = start();
    // A command's response does not flush the asynchronous Sidecar forwarder.
    // Subscribe before setup and account for its exact events before the
    // failed answer; a quiet interval cannot establish that boundary.
    let mut watcher = hosted.client();
    watcher.subscribe().unwrap();
    let id = hosted.ask_genre();
    // The Decider already holds a view, which a new Fact would supersede.
    hosted.call(
        "facts.record",
        json!({
            "source": { "kind": "agent", "name": "tagger" },
            "confidence": 0.7,
            "facts": [{ "track": fixture::track_id(3), "kind": "genre", "value": "Techno" }],
        }),
    );
    let before = hosted.facts();
    assert_eq!(before.len(), 1);
    expect_sidecar_change(
        &mut watcher,
        json!({
            "track_id": fixture::track_id(3), "kind": "decision",
            "fact_id": null, "stale": [], "decision_id": id, "answer_id": null,
        }),
    );
    expect_sidecar_change(
        &mut watcher,
        json!({
            "track_id": fixture::track_id(3), "kind": "genre",
            "fact_id": before[0]["id"], "stale": [],
        }),
    );
    let db = hosted.sidecar_db();
    let runs = |db: &rusqlite::Connection| -> i64 {
        db.query_row("SELECT count(*) FROM run", [], |row| row.get(0))
            .unwrap()
    };
    let runs_before = runs(&db);
    // The store refuses the answer's row.
    db.execute_batch(
        "CREATE TRIGGER injected_failure BEFORE INSERT ON decision_answer
         BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
    )
    .unwrap();
    let failed = hosted
        .service
        .call("decisions.answer", answer_with_fact(id, "Tech House"))
        .unwrap_err();

    assert_eq!(failed.code, "sidecar_failed", "{}", failed.message);
    assert_eq!(
        hosted.facts(),
        before,
        "no Fact was recorded and the earlier one was not superseded"
    );
    assert_eq!(runs(&db), runs_before, "no Run was made for the Fact");
    assert_eq!(
        hosted.call("decisions.get", json!({ "id": id }))["pending"],
        true
    );
    assert_eq!(events(&mut watcher), Vec::<Value>::new());

    // Once the store takes the row again, the same answer goes through
    // whole, and the Fact's event comes before the answer's.
    db.execute_batch("DROP TRIGGER injected_failure").unwrap();
    let answered = hosted.call("decisions.answer", answer_with_fact(id, "Tech House"));
    let facts = hosted.facts();
    assert_eq!(facts.len(), 2);
    assert_eq!(facts[0]["id"], answered["fact"]["id"]);
    assert_eq!(facts[1]["superseded_by"], answered["fact"]["id"]);
    assert_eq!(answered["answer"]["fact_id"], answered["fact"]["id"]);
    assert_eq!(answered["answer"]["run"], answered["fact"]["run"]);
    assert!(answered["run"]["finished_at"].is_i64());
    let told = exactly(&mut watcher, 2);
    let kinds: Vec<&str> = told
        .iter()
        .map(|event| event["data"]["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["genre", "decision"], "{told:?}");
}

#[test]
fn two_clients_answering_as_one_decider_at_once_leave_the_later_answer_holding_the_live_fact() {
    let hosted = start();
    let barrier = Arc::new(Barrier::new(2));

    for round in 0..150 {
        let id = hosted.ask_genre();
        let threads: Vec<_> = ["Techno", "Tech House"]
            .into_iter()
            .map(|choice| {
                let mut client = hosted.client();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    client
                        .call::<Value>("decisions.answer", answer_with_fact(id, choice))
                        .unwrap()
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }

        let decision = hosted.call("decisions.get", json!({ "id": id }));
        let answers = decision["answers"].as_array().unwrap();
        assert_eq!(answers.len(), 2, "both answers are kept");
        let (earlier, later) = (&answers[0], &answers[1]);
        let facts = hosted.facts();
        let fact = |id: &Value| facts.iter().find(|fact| fact["id"] == *id).unwrap();
        let (earlier_fact, later_fact) = (fact(&earlier["fact_id"]), fact(&later["fact_id"]));
        assert_eq!(
            (
                later_fact["superseded"].clone(),
                later_fact["value"].clone()
            ),
            (json!(false), json!([later["choice"].clone()])),
            "round {round}: the later answer's Fact is the Decider's live one"
        );
        assert_eq!(
            earlier_fact["superseded_by"], later_fact["id"],
            "round {round}: the earlier answer's Fact gave way to the later answer's"
        );
    }
}
