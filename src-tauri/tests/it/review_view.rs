//! The Review view's commands (wbx-74): the window lists the Facts waiting
//! for the DJ through the Library service it hosts, with each track's deck
//! row and each Fact's Run, and passes the DJ's accept, reject and reopen
//! to the service exactly as wbx-27 made them, declaring the DJ every time.
//! A refusal or a part-way accept comes back as an answer the view shows,
//! never as success.
//!
//! A fixture Library in a temporary folder and a mock app in place of the
//! window; nothing here can reach the installed Library.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rbl_db::fixture::{self, track_id, Shape};
use rbxport_lib::review_view::{self, REVIEW_CHANGED_EVENT};
use rbxport_lib::service_host;
use rbxport_lib::state::AppState;
use serde_json::{json, Value};
use tauri::test::MockRuntime;
use tauri::{Listener, Manager};
use wreckordbox_service::Config;

struct Window {
    dir: tempfile::TempDir,
    app: tauri::App<MockRuntime>,
    location: rbl_db::LibraryLocation,
}

impl Window {
    fn state(&self) -> Arc<AppState> {
        Arc::clone(self.app.state::<Arc<AppState>>().inner())
    }

    fn protect(&self, on: bool) {
        let state = self.app.state::<Arc<AppState>>();
        let now = tauri::async_runtime::block_on(service_host::set_library_protection(state, on))
            .unwrap();
        assert_eq!(now.protect_library, on);
    }

    fn config(&self) -> Config {
        Config {
            runtime_dir: self.dir.path().join("run"),
            idle_exit: None,
            recovery_dir: self.dir.path().join("backups"),
        }
    }

    fn call(&self, command: &str, args: Value) -> Value {
        self.state()
            .with_service(|service| service.call(command, args))
            .unwrap()
            .unwrap()
    }

    /// Records an agent's Energy proposal; with no policy file it waits.
    fn propose(&self, track: usize, energy: i64, confidence: f64) -> i64 {
        self.call(
            "facts.record",
            json!({
                "source": {"kind": "agent", "name": "tagger"},
                "facts": [{"track": track_id(track), "kind": "energy", "value": energy, "confidence": confidence}],
            }),
        )["facts"][0]["id"]
            .as_i64()
            .unwrap()
    }

    fn rating(&self, track: usize) -> Value {
        self.call("tracks.get", json!({ "id": track_id(track) }))["rating"].clone()
    }
}

fn hosting_window() -> Window {
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    let location = fixture::build(
        &library,
        Shape {
            tracks: 5,
            ..Shape::default()
        },
    )
    .unwrap();
    let state = AppState::with_backups(dir.path().join("backups"));
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (index, _) = rbl_index::load(&db).unwrap();
    state.set_library(index, false, db.schema().db_version, 0, location.clone());
    let app = tauri::test::mock_app();
    app.manage(Arc::new(state));
    let window = Window { dir, app, location };
    service_host::host_with(window.app.handle(), &window.location, &window.config()).unwrap();
    window
}

fn facts_of(listing: &Value) -> Vec<i64> {
    listing["waiting"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["fact"].as_i64().unwrap())
        .collect()
}

#[test]
fn the_waiting_facts_come_with_their_runs_and_each_tracks_deck_row() {
    let window = hosting_window();
    let a = window.propose(1, 4, 0.9);
    let b = window.propose(2, 2, 0.5);

    let shown = review_view::waiting(&window.state()).unwrap();

    assert_eq!(facts_of(&shown.listing), vec![a, b]);
    assert!(shown.listing["token"].as_str().is_some());
    let runs = shown.listing["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2, "each recording was its own Run: {runs:?}");
    // The deck's row of every listed track, once each.
    let mut ids: Vec<&str> = shown.rows.iter().map(|r| r.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![track_id(1).as_str(), track_id(2).as_str()]);
}

#[test]
fn an_accept_with_the_shown_token_sets_the_value_and_one_with_an_older_listing_is_refused() {
    let window = hosting_window();
    let state = window.state();
    let a = window.propose(1, 4, 0.9);
    let shown = review_view::waiting(&state).unwrap();
    let token = shown.listing["token"].as_str().unwrap().to_owned();

    // Something arrives after the DJ's listing: the accept is refused,
    // nothing written, and the answer says so (the view shows the change).
    let late = window.propose(2, 3, 0.8);
    let refused = review_view::accept(&state, &[a], &token).unwrap();
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "review_changed");
    assert_eq!(window.rating(1), 0);
    assert_eq!(
        facts_of(&review_view::waiting(&state).unwrap().listing),
        vec![a, late]
    );

    // Library Protection: refused, still waiting.
    let shown = review_view::waiting(&state).unwrap();
    let token = shown.listing["token"].as_str().unwrap().to_owned();
    window.protect(true);
    let protected = review_view::accept(&state, &[a], &token).unwrap();
    assert_eq!(protected["ok"], false, "{protected}");
    assert_eq!(protected["error"]["code"], "library_protected");
    assert_eq!(window.rating(1), 0);
    window.protect(false);

    // The listing the DJ sees: accepted, the value is the Library's now and
    // the Fact no longer waits.
    let accepted = review_view::accept(&state, &[a], &token).unwrap();
    assert_eq!(accepted["ok"], true, "{accepted}");
    assert_eq!(accepted["count"], 1);
    assert_eq!(window.rating(1), 4);
    assert_eq!(
        facts_of(&review_view::waiting(&state).unwrap().listing),
        vec![late]
    );
}

#[test]
fn a_rejection_is_the_djs_and_is_reopened_from_the_window() {
    let window = hosting_window();
    let state = window.state();
    let a = window.propose(1, 4, 0.9);
    let b = window.propose(2, 2, 0.5);

    let rejected = review_view::reject(&state, &[a, b]).unwrap();
    assert_eq!(rejected["ok"], true, "{rejected}");
    assert_eq!(rejected["count"], 2);
    assert!(facts_of(&review_view::waiting(&state).unwrap().listing).is_empty());
    let standing = review_view::rejected(&state).unwrap();
    assert_eq!(standing["count"], 2);
    // Recorded as the DJ's own decision.
    let entry = rejected["rejection"]["entry"].as_i64().unwrap();
    let run = rejected["rejection"]["run"].as_i64().unwrap();
    let shown_run = window.call("runs.show", json!({ "id": run }));
    assert_eq!(shown_run["run"]["source"]["kind"], "dj", "{shown_run}");
    // Nothing written to the Library.
    assert_eq!(window.rating(1), 0);

    // One mistaken rejection reopened as a whole: both wait again.
    let reopened = review_view::reopen(&state, &[], Some(entry)).unwrap();
    assert_eq!(reopened["ok"], true, "{reopened}");
    assert_eq!(reopened["count"], 2);
    assert_eq!(
        facts_of(&review_view::waiting(&state).unwrap().listing),
        vec![a, b]
    );

    // Reopening a Fact that is not rejected is refused, and said.
    let refused = review_view::reopen(&state, &[a], None).unwrap();
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "review_not_waiting");
}

#[test]
fn the_window_is_told_when_the_queue_changes_coalesced() {
    let window = hosting_window();
    let state = window.state();
    let told = Arc::new(AtomicUsize::new(0));
    {
        let told = Arc::clone(&told);
        window.app.listen_any(REVIEW_CHANGED_EVENT, move |_| {
            told.fetch_add(1, Ordering::SeqCst);
        });
    }
    review_view::watch(window.app.handle(), &state);
    let settle = |told: &AtomicUsize| {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut last = told.load(Ordering::SeqCst);
        let mut quiet_since = Instant::now();
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
            let now = told.load(Ordering::SeqCst);
            if now != last {
                last = now;
                quiet_since = Instant::now();
            } else if quiet_since.elapsed() > Duration::from_millis(600) && now > 0 {
                break;
            }
        }
        told.swap(0, Ordering::SeqCst)
    };

    // A burst of proposals while a Run works: told, a few times at most.
    let mut proposed = Vec::new();
    for n in 0..40 {
        proposed.push(window.propose(1 + n % 4, 1 + (n % 5) as i64, 0.7));
    }
    let burst = settle(&told);
    assert!(burst >= 1, "a burst is told");
    assert!(burst <= 8, "a burst is coalesced: told {burst} times");

    // Another client's rejection (no Library change) is told too.
    window.call(
        "review.reject",
        json!({ "facts": [proposed[39]], "source": {"kind": "dj"} }),
    );
    assert!(settle(&told) >= 1, "a rejection is told");

    // Watching twice does not tell twice.
    review_view::watch(window.app.handle(), &state);
    window.propose(1, 5, 0.6);
    let once = settle(&told);
    assert_eq!(once, 1, "one watcher");
}
