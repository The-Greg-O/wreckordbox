//! The Ledger of a service a front end hosts on its own Library core: an
//! edit made through the core by the front end, and a command the front
//! end asks of the service, are the DJ's, in the window's Run, which ends
//! when the DJ pauses; a client's edit is its own Run, and the Ledger
//! stops recording the core when the service stops.
//!
//! One test, in a test binary of its own: the Sidecar's folder and the
//! pause come from the environment, which is the process's.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::sync::Arc;

use rbl_db::fixture::{self, Shape};
use rbl_library::history::{LibraryEdit, TRACK_EDIT};
use rbl_library::{LibraryCore, NoHost, Touched};
use serde_json::json;
use wreckordbox_service::ledger::RunArg;
use wreckordbox_service::stems::SIDECAR_DIR_ENV;
use wreckordbox_service::{Client, Config, Service};

fn rate(core: &LibraryCore, track: &str, stars: u8) {
    core.recorded_edit(
        &NoHost,
        Touched::Metadata(vec![track.to_owned()]),
        TRACK_EDIT,
        |writer| {
            let (_, recorded) = writer.set_rating_with_undo(track, stars)?;
            Ok(((), LibraryEdit::Track(vec![recorded])))
        },
    )
    .unwrap();
}

#[test]
#[ignore = "explicit fixture-only edit latency measurement"]
fn native_edit_latency_at_twenty_thousand_tracks() {
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    let location = fixture::build(
        &library,
        Shape {
            tracks: 20_000,
            ..Shape::default()
        },
    )
    .unwrap();
    std::env::set_var(SIDECAR_DIR_ENV, dir.path().join("sidecar"));
    let config = Config {
        runtime_dir: dir.path().join("run"),
        idle_exit: None,
        recovery_dir: dir.path().join("recovery"),
    };
    let core = Arc::new(LibraryCore::new(config.recovery_dir.clone()));
    // As on the machine of a DJ who edits: Library Protection is off.
    core.set_protection(false).unwrap();
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (index, _) = rbl_index::load(&db).unwrap();
    core.set_library(
        &NoHost,
        index,
        false,
        db.schema().db_version,
        0,
        location.clone(),
    );
    let host: Arc<dyn rbl_library::Host + Send + Sync> = Arc::new(NoHost);
    let (first, _second, _third) = (
        fixture::track_id(1),
        fixture::track_id(2),
        fixture::track_id(3),
    );

    let service = Service::host(&location, &config, &core, &host).unwrap();
    let mut client = Client::connect(&config.socket_path(&location)).unwrap();
    for who in ["agent", "dj", "window", "outside"] {
        let mut elapsed = Vec::new();
        for n in 0..7_u8 {
            let rating = 1 + n % 5;
            let begin = std::time::Instant::now();
            match who {
                "window" => rate(&core, &first, rating),
                "outside" => {
                    core.edit(
                        &NoHost,
                        rbl_library::History::Outside,
                        Touched::Metadata(vec![first.clone()]),
                        |writer| writer.set_rating(&first, rating),
                    )
                    .unwrap();
                }
                _ => {
                    client.in_run(Some(RunArg {
                        source: Some(
                            serde_json::from_value(
                                json!({"kind":who,"name":if who=="dj" {"dj"} else {"timing"}}),
                            )
                            .unwrap(),
                        ),
                        ..RunArg::default()
                    }));
                    client.rate_track(&first, rating).unwrap();
                }
            }
            elapsed.push(begin.elapsed().as_secs_f64() * 1000.0);
        }
        elapsed.sort_by(f64::total_cmp);
        eprintln!(
            "WBX19_TIMING {who} median_ms={:.2} samples={elapsed:?}",
            elapsed[3]
        );
    }
    service.stop();
}
