//! A stopped service has let go of the Sidecar, as it lets go of the
//! Library, before another service or a reader may have it (wbx-45.7). The
//! service's own threads may still hold its state for a moment after it
//! stops; the Sidecar must not be held with it. The Library is a throwaway
//! fixture.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use rbl_db::fixture::{self, Shape};
use serde_json::json;
use wreckordbox_service::{Config, Service};
use wreckordbox_sidecar::FactsSnapshot;

#[test]
fn a_stopped_service_has_closed_the_sidecar_even_while_its_state_is_still_held() {
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    let location = fixture::build(
        &library,
        Shape {
            tracks: 1,
            ..Shape::default()
        },
    )
    .unwrap();
    let sidecar = library.join("wreckordbox-sidecar");
    std::fs::create_dir_all(&sidecar).unwrap();
    std::fs::write(
        sidecar.join("kinds.toml"),
        "[kinds.title]\nnature = \"judgement\"\nvalue = \"text\"\n",
    )
    .unwrap();
    let config = Config {
        runtime_dir: dir.path().join("run"),
        idle_exit: None,
        recovery_dir: dir.path().join("recovery"),
    };
    wreckordbox_service::protection::store(&config.recovery_dir, false).unwrap();

    let service = Service::start(location, &config).unwrap();
    let track = fixture::track_id(0);
    service
        .call(
            "facts.record",
            json!({
                "facts": [{ "track": track, "kind": "title", "value": "A Better Title" }],
                "source": { "kind": "agent", "name": "tagger" },
                "confidence": 0.8,
            }),
        )
        .unwrap();
    // As the service's own threads do for a moment after it stops.
    let still_held = service.stopped();
    drop(service);

    // The writer is closed: its journal is gone and a read-only reader
    // finds what was recorded.
    for leftover in ["sidecar.db-wal", "sidecar.db-journal"] {
        assert!(
            !sidecar.join(leftover).exists(),
            "{leftover} is still beside the Sidecar after the service stopped"
        );
    }
    let FactsSnapshot::Open(reader) = FactsSnapshot::open(&sidecar).unwrap() else {
        panic!("the Sidecar cannot be read after the service stopped");
    };
    let facts = reader.facts(&track, "title").unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].value, json!("A Better Title"));
    drop(still_held);
}
