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
