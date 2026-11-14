//! An ordered barrier for what a watching client is sent (wbx-377), in
//! place of "N ms of quiet".
//!
//! A service delivers `library_changed` (and `protection_changed`) from one
//! ordered queue, on its own thread, to the clients subscribed at the
//! moment of delivery. So an edit's event can still reach a client that
//! subscribed after the edit answered, and a fixed wait can end before a
//! late event arrives. The barrier is Library Protection turned on and off
//! again through the watcher's own connection: both turns travel the same
//! queue behind every Library change made before them, so every such event
//! has arrived once the barrier's last event has. It writes nothing to the
//! Library; it leaves protection off, as it must have been.
//!
//! The barrier orders only that queue. Events about the Sidecar (Facts,
//! Mutants, progress) go out from another queue, and a check of those
//! subscribes before its setup and consumes the setup's own events instead.

use std::time::Duration;

use serde_json::{json, Value};
use wreckordbox_service::{Client, Event};

/// Every event `watcher` is sent before the barrier, in order, the
/// barrier's own `protection_changed` events left out. Protection must be
/// off.
pub fn events_before(watcher: &mut Client) -> Vec<Event> {
    let on: Value = watcher
        .call("protection.set", json!({ "protect_library": true }))
        .unwrap();
    assert_eq!(
        on["previous"], false,
        "the barrier needs protection off: {on}"
    );
    let off: Value = watcher
        .call("protection.set", json!({ "protect_library": false }))
        .unwrap();
    let barrier = |event: &Event, set: &Value| {
        event.event == "protection_changed"
            && event.data["protect_library"] == set["protect_library"]
            && event.data["revision"] == set["revision"]
    };
    let mut before = Vec::new();
    loop {
        let event = watcher
            .next_event(Duration::from_secs(30))
            .unwrap()
            .expect("the barrier never arrived");
        if barrier(&event, &off) {
            return before;
        }
        if !barrier(&event, &on) {
            before.push(event);
        }
    }
}

/// The `library_changed` events `watcher` is sent before the barrier.
pub fn library_changes(watcher: &mut Client) -> Vec<Event> {
    events_before(watcher)
        .into_iter()
        .filter(|event| event.event == "library_changed")
        .collect()
}

/// Consumes whatever Library changes made so far still have to tell
/// `watcher`: after this, what it hears comes from what follows.
pub fn settle(watcher: &mut Client) {
    events_before(watcher);
}

/// The generation of the one `library_changed` event `watcher` is sent
/// before the barrier; fails when it is sent none or more than one.
/// Any other event before the barrier fails it too.
pub fn one_library_change(watcher: &mut Client) -> u64 {
    let changes = events_before(watcher);
    assert_eq!(changes.len(), 1, "one edit, one event: {changes:?}");
    assert_eq!(changes[0].event, "library_changed", "{changes:?}");
    assert_eq!(changes[0].data["kind"], "library", "{changes:?}");
    changes[0].data["generation"].as_u64().unwrap()
}

/// Fails when `watcher` is sent any event before the barrier.
pub fn told_nothing(watcher: &mut Client) {
    let heard = events_before(watcher);
    assert!(heard.is_empty(), "no event was due: {heard:?}");
}

/// Consumes the one `sidecar_changed` event each Fact in `facts` was
/// announced with, from a client subscribed before they were recorded, in
/// any order; fails on any other event (wbx-377: the setup's own events,
/// identified, instead of waiting for a moment of quiet).
pub fn facts_announced(watcher: &mut Client, facts: &[i64]) {
    let mut left: Vec<i64> = facts.to_vec();
    while !left.is_empty() {
        let event = watcher
            .next_event(Duration::from_secs(30))
            .unwrap()
            .unwrap_or_else(|| panic!("Facts {left:?} were not announced"));
        let fact = event.data["fact_id"].as_i64();
        assert!(
            event.event == "sidecar_changed" && fact.is_some_and(|id| left.contains(&id)),
            "expected the announcement of one of Facts {left:?}: {event:?}"
        );
        left.retain(|id| Some(*id) != fact);
    }
}

/// The next `count` events `watcher` is sent, each waited for as long as
/// it takes (up to 30 s), and then nothing more for `quiet` (wbx-377): an
/// exact count no longer ends at the first gap of `quiet`, so a late
/// expected event is not taken for a missing one. The check that nothing
/// more comes keeps the window the test always had; on the Sidecar's
/// queue there is no ordered barrier to stand in for it.
pub fn heard_exactly(watcher: &mut Client, count: usize, quiet: Duration) -> Vec<Event> {
    let mut heard = Vec::with_capacity(count);
    while heard.len() < count {
        match watcher.next_event(Duration::from_secs(30)).unwrap() {
            Some(event) => heard.push(event),
            None => panic!("{count} events were due, {} came: {heard:?}", heard.len()),
        }
    }
    if let Some(more) = watcher.next_event(quiet).unwrap() {
        panic!("only {count} events were due, then {more:?}: {heard:?}");
    }
    heard
}
