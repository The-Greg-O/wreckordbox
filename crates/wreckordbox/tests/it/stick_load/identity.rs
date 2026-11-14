//! A Stick load's identity is the service's (wbx-45.14, from the wbx-72
//! review): a caller's `operation` is only a label, refused while another
//! running load carries it, and every event and answer carries the
//! service's own unique `load` id. The first load is held at `copying` by
//! the service's test-mode pause.

use super::*;

use std::collections::BTreeMap;

fn load_named(
    rig: &Rig,
    index: usize,
    plan: &str,
    operation: &str,
) -> Result<Value, wreckordbox_service::ServiceError> {
    rig.sandbox.client().call::<Value>(
        "sticks.load",
        serde_json::json!({
            "stick": rig.path(index), "playlists": [rig.set], "plan": plan, "operation": operation,
        }),
    )
}

fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                todo.push(path);
            } else {
                out.push((path.clone(), std::fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a_load_named_like_one_running_is_refused_and_every_load_has_its_own_id() {
    let rig = Rig::new(&["FRIDAY", "SATURDAY"]);
    rig.unlock(0);
    rig.unlock(1);
    let first_plan = rig.dry_run(0, &[&rig.set])["plan"]
        .as_str()
        .unwrap()
        .to_owned();
    let second_plan = rig.dry_run(1, &[&rig.set])["plan"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut listener = rig.sandbox.client();
    listener.subscribe().unwrap();

    // The first load, named "op-x", held while it copies.
    let state = rig.sandbox.state_dir();
    let pause = state.join(wreckordbox_service::stick_load::TEST_PAUSE_AT_FILE);
    std::fs::write(&pause, "copying").unwrap();
    let first = {
        let (socket_rig, plan) = (&rig, first_plan.clone());
        std::thread::scope(|scope| {
            let running = scope.spawn(move || load_named(socket_rig, 0, &plan, "op-x"));
            let paused = state.join(wreckordbox_service::stick_load::TEST_PAUSED_FILE);
            let until = std::time::Instant::now() + Duration::from_secs(60);
            while !paused.exists() {
                assert!(
                    std::time::Instant::now() < until,
                    "the first load never reached copying"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            // Another caller names its load the same, on the other Stick.
            let untouched = snapshot(&rig.paths[1]);
            let refused = load_named(&rig, 1, &second_plan, "op-x");
            std::fs::remove_file(&pause).unwrap();
            let error = refused.expect_err("a name another running load carries is refused");
            assert_eq!(error.code, "stick_load_duplicate", "{error:?}");
            assert_eq!(
                snapshot(&rig.paths[1]),
                untouched,
                "nothing was written to its Stick"
            );
            running.join().unwrap()
        })
    }
    .unwrap();
    assert_eq!(first["outcome"], "loaded", "{first}");
    let first_id = first["load"].as_str().unwrap().to_owned();
    assert!(!first_id.is_empty());

    // Once the first has answered, the name may be used again: it labels a
    // load with an id of its own.
    let again = load_named(&rig, 1, &second_plan, "op-x").unwrap();
    assert_eq!(again["outcome"], "loaded", "{again}");
    let again_id = again["load"].as_str().unwrap().to_owned();
    assert_ne!(again_id, first_id);

    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    while let Some(event) = listener.next_event(Duration::from_millis(500)).unwrap() {
        if event.event == "stick_load_progress" {
            assert_eq!(event.data["operation"], "op-x", "{}", event.data);
            let load = event.data["load"]
                .as_str()
                .unwrap_or_else(|| panic!("no load id: {}", event.data));
            let stick = event.data["stick"].as_str().unwrap().to_owned();
            let earlier = seen.insert(load.to_owned(), stick.clone());
            assert!(
                earlier.is_none_or(|s| s == stick),
                "one id, one load: {}",
                event.data
            );
        }
    }
    assert_eq!(
        seen.get(&first_id).map(String::as_str),
        Some(rig.path(0)),
        "{seen:?}"
    );
    assert_eq!(
        seen.get(&again_id).map(String::as_str),
        Some(rig.path(1)),
        "{seen:?}"
    );
    assert_eq!(seen.len(), 2, "no event of the refused load: {seen:?}");
}
