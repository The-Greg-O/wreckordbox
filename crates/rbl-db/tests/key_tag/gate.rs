//! Concurrent publication effects on generated fixture files.
use super::*;
use rbl_db::key_tag::prepared::{prepare, Outcome, Prepared};
use std::sync::{mpsc, Arc, Barrier};
fn ready(path: &Path, key: &str) -> Box<Prepared> {
    match prepare(path, None, Some(key)).unwrap() {
        Outcome::Ready(plan) => plan,
        Outcome::Skipped(other) => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn gate_different_files_publish_in_parallel_while_another_publication_is_held() {
    let dir = tempfile::tempdir().unwrap();
    let mut plans = Vec::new();
    for n in 0..9 {
        let path = dir.path().join(format!("parallel-{n}.mp3"));
        write_mp3(&path, Some(&busy_tag(Some("Cm"))), false);
        plans.push((path.clone(), ready(&path, "Fm")));
    }
    let (path, mut held) = plans.pop().unwrap();
    let (reached_tx, reached_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let first = std::thread::spawn(move || {
        held.publish_held_for_test(|| {
            reached_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        })
    });
    reached_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    let start = Arc::new(Barrier::new(plans.len()));
    let others: Vec<_> = plans
        .into_iter()
        .map(|(path, mut plan)| {
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                let result = plan.publish();
                (path, result)
            })
        })
        .collect();
    // They must all finish while the unrelated first publication is held.
    let results: Vec<_> = others.into_iter().map(|t| t.join().unwrap()).collect();
    release_tx.send(()).unwrap();
    first.join().unwrap().unwrap();
    assert_eq!(read_key(&path).unwrap().as_deref(), Some("Fm"));
    for (path, result) in results {
        result.unwrap();
        assert_eq!(read_key(&path).unwrap().as_deref(), Some("Fm"));
    }
}

#[test]
fn gate_same_file_has_one_winner_and_a_metadata_refusal_without_damage() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shared.mp3");
    write_mp3(&path, Some(&busy_tag(Some("Cm"))), false);
    let original = std::fs::read(&path).unwrap();
    let plans = [ready(&path, "Am"), ready(&path, "Fm")];
    let start = Arc::new(Barrier::new(2));
    let workers: Vec<_> = plans
        .into_iter()
        .map(|mut plan| {
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                let result = plan.publish();
                (plan, result)
            })
        })
        .collect();
    let mut results: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|(_, r)| r.is_ok()).count(), 1);
    let refused = results
        .iter()
        .find_map(|(_, r)| r.as_ref().err())
        .unwrap()
        .to_string();
    assert!(refused.contains("changed while"), "{refused}");
    let (winner, _) = results.iter_mut().find(|(_, r)| r.is_ok()).unwrap();
    assert_eq!(read_key(&path).unwrap().as_deref(), winner.after());
    winner.rollback().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

#[test]
fn gate_exchange_prepares_only_one_audio_copy_and_still_restores_exact_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("one-copy.mp3");
    write_mp3(&path, Some(&busy_tag(Some("Cm"))), false);
    let original = std::fs::read(&path).unwrap();
    let peak = std::cell::Cell::new(0);
    let prepared = rbl_db::key_tag::prepared::prepare_checked(&path, None, Some("Fm"), || {
        let copies = std::fs::read_dir(dir.path())?
            .filter_map(std::result::Result::ok)
            .filter(|e| e.path() != path && e.metadata().is_ok_and(|m| m.len() > 0))
            .count();
        peak.set(peak.get().max(copies));
        Ok(())
    })
    .unwrap();
    let Outcome::Ready(mut plan) = prepared else {
        panic!("fixture must be writable")
    };
    assert!(
        peak.get() <= 1,
        "exchange staging held {} full copies",
        peak.get()
    );
    let copies: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p != &path && std::fs::metadata(p).unwrap().len() > 0)
        .collect();
    assert_eq!(
        copies.len(),
        1,
        "exchange must not stage a second full copy: {copies:?}"
    );
    plan.publish().unwrap();
    plan.rollback().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

#[test]
fn gate_legacy_write_cannot_sweep_a_live_plans_original_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("retained.mp3");
    write_mp3(&path, Some(&busy_tag(Some("Cm"))), false);
    let original = std::fs::read(&path).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(7200))
        .unwrap();
    let mut plan = ready(&path, "Fm");
    plan.publish().unwrap();
    let originals: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p != &path && std::fs::read(p).unwrap() == original)
        .collect();
    assert!(!originals.is_empty());
    write_key(&path, "Am").unwrap();
    for kept in originals {
        assert_eq!(std::fs::read(kept).unwrap(), original);
    }
}

#[test]
fn gate_cleanup_recognizes_both_legacy_and_counter_names() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("counter.mp3");
    write_mp3(&path, Some(&busy_tag(Some("Cm"))), false);
    let names = [
        ".counter.mp3.rbx-key-4294967295-1.tmp",
        ".counter.mp3.rbx-key-4294967295-1-9.tmp",
    ];
    for name in names {
        let file = std::fs::File::create(dir.path().join(name)).unwrap();
        file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(7200))
            .unwrap();
    }
    drop(ready(&path, "Fm"));
    for name in names {
        assert!(!dir.path().join(name).exists(), "sweep {name}");
    }
}
