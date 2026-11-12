//! Preferences › Audio › Transport edge reaches the engine: before it is
//! built, while it plays, and not at all when the value is not one of the
//! three (wbx-45.16).
#![allow(clippy::unwrap_used)]
use rbl_deck::{NullSink, Sink, TransportEdge};
use rbxport_lib::{commands, player::Player, preview::Preview};
use std::sync::Arc;
use tauri::Manager;

fn null_sink() -> rbxport_lib::player::SinkOpener {
    Box::new(|render, _, _| Ok(Arc::new(NullSink::new(44_100, render)) as Arc<dyn Sink>))
}

fn set(app: &tauri::App<tauri::test::MockRuntime>, edge: &str) -> Result<(), String> {
    tauri::async_runtime::block_on(commands::set_transport_edge(
        app.state::<Arc<Player>>(),
        app.state::<Arc<Preview>>(),
        edge.to_owned(),
    ))
    .map_err(|e| format!("{e:?}"))
}

#[test]
fn the_transport_edge_is_kept_until_the_engine_exists_and_then_applied_live() {
    let app = tauri::test::mock_app();
    app.manage(Arc::new(Player::with_sink(null_sink())));
    app.manage(Arc::new(Preview::with_sink(null_sink())));

    // Before any engine: held, and the engine is built with it.
    set(&app, "2ms").unwrap();
    let engine = app.state::<Arc<Player>>().engine(app.handle()).unwrap();
    assert_eq!(engine.transport_edge(), TransportEdge::TwoMs);

    // With the engine up: applied to it at once, nothing rebuilt.
    set(&app, "1ms").unwrap();
    assert_eq!(engine.transport_edge(), TransportEdge::OneMs);
    let same = app.state::<Arc<Player>>().engine(app.handle()).unwrap();
    assert!(
        Arc::ptr_eq(&engine, &same),
        "changing the edge rebuilt the engine"
    );
    set(&app, "immediate").unwrap();
    assert_eq!(engine.transport_edge(), TransportEdge::Immediate);

    // Anything else is refused and changes nothing.
    assert!(set(&app, "3ms").is_err(), "an unknown edge was accepted");
    assert_eq!(engine.transport_edge(), TransportEdge::Immediate);
}
