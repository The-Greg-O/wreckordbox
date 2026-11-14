//! Player payloads survive bounded spill/replay with their original owner.
use super::*;

#[test]
fn ratings_survive_failed_recording_and_reads_do_not_replay_them() {
    if std::env::var_os("WBX1411_PLAYER_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "player::ratings_survive_failed_recording_and_reads_do_not_replay_them",
                "--test-threads=1",
            ])
            .env("WBX1411_PLAYER_CHILD", "1")
            .env("WRECKORDBOX_LEDGER_FAULTS", "fail_append=999")
            .env("RBXPORT_TEST", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let f = Fixture::new(300);
    let operation = Operation::new(&f.core, Kind::RoundTrip);
    let db = f.core.open_read_only().unwrap();
    let changes: Vec<_> = (0..300)
        .map(|i| {
            let id = fixture::track_id(i);
            let before: u8 = db
                .connection()
                .query_row("SELECT Rating FROM djmdContent WHERE ID=?1", [&id], |r| {
                    r.get(0)
                })
                .unwrap();
            (id, (before + 1) % 6)
        })
        .collect();
    for chunk in changes.chunks(128) {
        operation.run_many(json!({"tracks":chunk.len()}), chunk.len(), || {
            f.core.write_then(&NoHost, History::Outside, |w| {
                chunk.iter().map(|(id, stars)| {
                    let (_, edit) = w.set_rating_with_undo(id, *stars)?;
                    let payload = wreckordbox_service::ledger::stored::store(&rbl_library::history::LibraryEdit::Track(vec![edit]));
                    Ok(json!({"track":id,"operation":"rating","track_edit":payload}))
                }).collect::<Result<Vec<_>, rbl_db::DbError>>()
            }, |_, v| Ok(v))
        }, Clone::clone).unwrap();
    }
    assert!(operation.finish().unwrap().contains("accepted"));
    for (id, stars) in &changes {
        let actual: u8 = db
            .connection()
            .query_row("SELECT Rating FROM djmdContent WHERE ID=?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(actual, *stars, "recording must not undo {id}");
    }
    let runs = f.runs();
    assert_eq!(runs["total"], 1);
    assert_eq!(
        runs["runs"][0]["source"],
        json!({"kind":"tool","name":"round-trip"})
    );
    let args = json!({"id":runs["runs"][0]["id"],"limit":1000});
    let pending = f.service.call("runs.show", args.clone()).unwrap();
    assert_eq!(pending["pending_recording"]["total"], 300);
    assert_eq!(f.service.call("runs.show", args.clone()).unwrap(), pending);
    assert_eq!(
        f.service
            .call(
                "facts.of_track",
                json!({"id":fixture::track_id(1),"all":true})
            )
            .unwrap()["facts"],
        json!([])
    );
    // An explicit new write recovers the completed spill, never a read.
    f.service
        .call("tracks.rate", json!({"id":fixture::track_id(0),"rating":4}))
        .unwrap();
    let shown = f.service.call("runs.show", args).unwrap();
    assert_eq!(shown["total"], 300);
    let entries = shown["entries"].as_array().unwrap();
    let named: std::collections::BTreeSet<_> = entries
        .iter()
        .map(|entry| {
            assert_eq!(entry["source"], json!({"kind":"tool","name":"round-trip"}));
            assert_eq!(entry["action"], "edit");
            assert_eq!(entry["kind"], "track");
            assert_eq!(entry["reversible"], true);
            let edit = &entry["payload"]["edits"][0];
            assert_ne!(edit["before"], edit["after"]);
            edit["track"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(named, changes.iter().map(|(id, _)| id.clone()).collect());
}

#[test]
fn a_partial_player_chunk_bars_only_the_named_tracks() {
    for kind in [Kind::Link, Kind::RoundTrip] {
        let f = Fixture::new(3);
        let id = fixture::track_id(1);
        let other = fixture::track_id(2);
        let operation = Operation::new(&f.core, kind);
        let result = operation.run_many(
            json!({"tracks":[id]}),
            2,
            || {
                f.core.write_then(
                    &NoHost,
                    History::Outside,
                    |w| {
                        w.set_rating(&id, 0)?;
                        w.set_rating("missing-player-track", 6)
                    },
                    |_, v| Ok(v),
                )
            },
            |_| vec![],
        );
        assert!(result.is_err());
        assert_eq!(operation.finish(), None);
        let barriers = f.service.call("projection.barriers", json!({})).unwrap();
        assert_eq!(
            barriers["barriers"].as_array().unwrap().len(),
            1,
            "{barriers}"
        );
        assert_eq!(barriers["barriers"][0]["tracks"], json!([id]), "{barriers}");
        assert_ne!(barriers["barriers"][0]["source"]["kind"], "dj");
        assert!(!barriers["barriers"][0]["tracks"]
            .as_array()
            .unwrap()
            .contains(&json!(other)));
    }
}

#[test]
fn same_value_player_fields_keep_ratings_and_colours_from_projection() {
    for kind in [Kind::Link, Kind::RoundTrip] {
        let f = Fixture::new(3);
        let id = fixture::track_id(1);
        f.core
            .write_then(
                &NoHost,
                History::Outside,
                |w| {
                    w.set_rating(&id, 0)?;
                    w.set_color(&id, None)
                },
                |db, value| {
                    f.core
                        .refresh_metadata(db, std::slice::from_ref(&id), false)?;
                    Ok(value)
                },
            )
            .unwrap();
        let location = f.core.location().unwrap();
        let folder = wreckordbox_sidecar::default_dir(&location.master_db);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("projection-policy.toml"),
            "[energy]\napply = 0.8\n[prep_colour]\napply = 0.8\n",
        )
        .unwrap();
        let fact = |energy, colour| {
            f.service
                .call(
                    "facts.record",
                    json!({"source":{"kind":"agent","name":"fixture"},"facts":[
                        {"track":id,"kind":"energy","value":energy,"confidence":1.0},
                        {"track":id,"kind":"prep_colour","value":colour,"confidence":1.0}
                    ]}),
                )
                .unwrap();
        };
        fact(3, "Green");
        let first = f
            .service
            .call("projection.apply", json!({"tracks":[id]}))
            .unwrap();
        assert_eq!(first["changes"].as_array().unwrap().len(), 2, "{first}");
        let colour = f.core.library().unwrap().color
            [f.core.library().unwrap().row_of(&id).unwrap() as usize];
        let operation = Operation::new(&f.core, kind);
        operation
            .run(
                json!({"track":id}),
                || {
                    f.core.write_then(
                        &NoHost,
                        History::Outside,
                        |w| {
                            let (_, rating) = w.set_rating_with_undo(&id, 3)?;
                            let (_, colour) =
                                w.set_color_with_undo(&id, Some(&colour.to_string()))?;
                            operation.assert_player_fields();
                            Ok(wreckordbox_service::ledger::stored::store(
                                &rbl_library::history::LibraryEdit::Track(vec![rating, colour]),
                            ))
                        },
                        |_, payload| Ok(payload),
                    )
                },
                |payload| vec![json!({"track":id,"track_edit":payload})],
            )
            .unwrap();
        assert_eq!(operation.finish(), None);
        fact(4, "Red");
        let applied = f
            .service
            .call("projection.apply", json!({"tracks":[id]}))
            .unwrap();
        assert_eq!(applied["changes"], json!([]), "{kind:?}: {applied}");
        assert!(f
            .service
            .call("facts.of_track", json!({"id":id,"all":true}))
            .unwrap()["facts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|fact| fact["source"]["kind"] != "dj"));
    }
}
