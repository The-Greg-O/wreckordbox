//! `wreckordbox backup restore` and `backup list`: the restore a DJ runs
//! from a terminal, and the one the scripts in every backup hand a backup
//! holding the Sidecar to. The real binary against a throwaway Library and
//! its Sidecar; never the real Library or real backups.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod common;

use std::path::{Path, PathBuf};

use common::{Run, Sandbox};
use rbl_backup::testing;
use wreckordbox_sidecar::ledger::Action;
use wreckordbox_sidecar::{Fact, Kind, NewFact, Sidecar, Source, SourceKind};

const TRACK: &str = "10001";

/// The Sidecar where a program given no `WRECKORDBOX_SIDECAR_DIR` keeps it,
/// beside the Library: where this test's own restores and archives look.
fn sidecar_dir(sandbox: &Sandbox) -> PathBuf {
    wreckordbox_sidecar::default_dir(sandbox.master_db())
}

/// The real CLI, using the Sidecar beside the Library.
fn cli(sandbox: &Sandbox, args: &[&str]) -> Run {
    sandbox.run(
        sandbox
            .command()
            .env("WRECKORDBOX_SIDECAR_DIR", sidecar_dir(sandbox))
            .args(args),
    )
}

fn genre(sandbox: &Sandbox, value: &str) {
    let sidecar = Sidecar::open(&sidecar_dir(sandbox)).unwrap();
    let dj = Source::new(SourceKind::Dj, "dj");
    let run = sidecar.begin_run(&dj, "Tag by hand").unwrap();
    sidecar
        .record(&NewFact {
            track_id: TRACK,
            fingerprint: None,
            kind: Kind::judgement("genre"),
            value: serde_json::json!(value),
            source: &dj,
            confidence: 1.0,
            run,
        })
        .unwrap();
    sidecar.finish_run(run, "recorded").unwrap();
}

fn facts(sandbox: &Sandbox) -> Vec<Fact> {
    Sidecar::open(&sidecar_dir(sandbox))
        .unwrap()
        .facts_of_track(TRACK)
        .unwrap()
}

/// A Library rated 4 with a genre in its Sidecar, and a backup of both in
/// the backup folder; then both changed after it.
fn backed_up() -> (Sandbox, PathBuf, Vec<Fact>) {
    let sandbox = Sandbox::new();
    testing::set_rating(sandbox.location(), 4);
    genre(&sandbox, "house");
    let before = facts(&sandbox);
    let archive = sandbox.state_dir().join("rbexport-20261112-1200.zip");
    testing::archive(sandbox.location(), &archive, None);
    testing::set_rating(sandbox.location(), 1);
    genre(&sandbox, "techno");
    assert_ne!(facts(&sandbox), before);
    (sandbox, archive, before)
}

fn restore(sandbox: &Sandbox, archive: &Path) -> Run {
    cli(
        sandbox,
        &["--json", "backup", "restore", archive.to_str().unwrap(), "--yes"],
    )
}

#[test]
fn a_backup_restored_from_the_command_line_brings_back_the_library_with_its_sidecar_and_the_ledger_says_so() {
    let (sandbox, archive, before) = backed_up();
    let changed = facts(&sandbox);

    let listed = cli(&sandbox, &["--json", "backup", "list"]);
    assert!(listed.status.success(), "{}", listed.stderr);
    let backups = listed.json()["backups"].as_array().unwrap().clone();
    assert_eq!(backups.len(), 1, "{backups:?}");
    assert_eq!(backups[0]["path"], archive.to_str().unwrap());
    assert_eq!(backups[0]["sidecar"], true);
    assert_eq!(backups[0]["stems"], false);

    // Nothing is done without the DJ saying so.
    let asked = cli(&sandbox, &["backup", "restore", archive.to_str().unwrap()]);
    assert!(!asked.status.success());
    assert!(asked.stderr.contains("--yes"), "{}", asked.stderr);
    assert_eq!(testing::rating(sandbox.location()), 1);
    assert_eq!(facts(&sandbox), changed);

    let restored = restore(&sandbox, &archive);
    assert!(restored.status.success(), "{}", restored.stderr);
    let answer = restored.json();
    assert_eq!(answer["sidecar"], sidecar_dir(&sandbox).canonicalize().unwrap().to_str().unwrap());
    assert_eq!(testing::rating(sandbox.location()), 4);
    assert_eq!(facts(&sandbox), before);

    let entry = Sidecar::open(&sidecar_dir(&sandbox))
        .unwrap()
        .ledger_entry(answer["ledger_entry"].as_i64().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!((entry.action, entry.kind.as_str()), (Action::Write, "restore"));
    assert!(entry.label.contains("Library and the Sidecar were restored"), "{}", entry.label);
}

#[test]
fn a_backup_made_before_backups_carried_the_sidecar_restores_the_library_and_the_ledger_warns_of_entries_it_lacks() {
    let sandbox = Sandbox::new();
    testing::set_rating(sandbox.location(), 4);
    let archive = sandbox.state_dir().join("rbexport-20261112-1100.zip");
    testing::archive(sandbox.location(), &archive, None);
    testing::set_rating(sandbox.location(), 1);
    genre(&sandbox, "techno");
    let changed = facts(&sandbox);

    let restored = restore(&sandbox, &archive);
    assert!(restored.status.success(), "{}", restored.stderr);
    let answer = restored.json();
    assert_eq!(answer["sidecar"], serde_json::Value::Null);
    assert_eq!(testing::rating(sandbox.location()), 4);
    assert_eq!(facts(&sandbox), changed);
    let entry = Sidecar::open(&sidecar_dir(&sandbox))
        .unwrap()
        .ledger_entry(answer["ledger_entry"].as_i64().unwrap())
        .unwrap()
        .unwrap();
    assert!(entry.label.contains("holds no Sidecar"), "{}", entry.label);
}

#[test]
fn a_restore_is_refused_while_a_service_has_the_library_and_goes_ahead_once_it_is_stopped() {
    let (sandbox, archive, before) = backed_up();
    let changed = facts(&sandbox);
    let started = cli(&sandbox, &["--json", "status"]);
    assert!(started.status.success(), "{}", started.stderr);

    let refused = restore(&sandbox, &archive);
    assert!(!refused.status.success());
    let error = refused.error_json()["error"].clone();
    assert_eq!(error["code"], "restore_refused", "{error}");
    assert!(error["message"].as_str().unwrap().contains("in use"), "{error}");
    assert_eq!(testing::rating(sandbox.location()), 1);
    assert_eq!(facts(&sandbox), changed);
    assert!(!rbl_backup::journal::pending(&sandbox.state_dir()));

    let stopped = cli(&sandbox, &["stop"]);
    assert!(stopped.status.success(), "{}", stopped.stderr);
    let restored = restore(&sandbox, &archive);
    assert!(restored.status.success(), "{}", restored.stderr);
    assert_eq!(testing::rating(sandbox.location()), 4);
    assert_eq!(facts(&sandbox), before);
}

#[test]
fn a_restore_that_stopped_between_the_library_and_the_sidecar_is_put_right_by_the_next_service_before_it_reads_either() {
    let (sandbox, archive, _) = backed_up();
    let changed = facts(&sandbox);
    // Dies once the Library's database, WAL and SHM are replaced and before
    // anything of the Sidecar is.
    testing::stop_before_swap(3, true);
    let crashed = rbl_backup::restore::restore(
        &rbl_backup::restore::Request {
            archive: &archive,
            parts: rbl_backup::restore::Parts::ALL,
            location: sandbox.location(),
            state_dir: &sandbox.state_dir(),
            sizes: None,
        },
        &|| Ok(()),
        &mut |_, _, _, _| Ok(()),
    );
    assert!(crashed.is_err());
    assert!(rbl_backup::journal::pending(&sandbox.state_dir()));
    assert_eq!(testing::rating(sandbox.location()), 4);
    assert_eq!(facts(&sandbox), changed);

    // A command starts a service by itself, which recovers first.
    let status = cli(&sandbox, &["--json", "status"]);
    assert!(status.status.success(), "{}", status.stderr);
    assert!(!rbl_backup::journal::pending(&sandbox.state_dir()));
    assert_eq!(testing::rating(sandbox.location()), 1);
    assert_eq!(facts(&sandbox), changed);
    let leftovers: Vec<_> = std::fs::read_dir(&sidecar_dir(&sandbox))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(rbl_backup::journal::PREFIX))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}
