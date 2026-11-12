//! The Sidecar travels with backups: what a backup holds of the folder
//! beside the Library, and what a restore puts back.
#![allow(clippy::unwrap_used)]

use rbl_backup::testing;
use std::{fs, path::Path};

/// The names of an archive's entries.
fn entries(archive: &Path) -> Vec<String> {
    let zip = rbl_backup::archive::open(archive).unwrap();
    let mut names = rbl_backup::archive::names(&zip);
    names.sort();
    names
}

#[test]
fn a_backup_holds_the_sidecar_that_is_beside_the_library() {
    let dir = tempfile::tempdir().unwrap();
    let location = testing::library(dir.path());
    let sidecar_dir = wreckordbox_sidecar::default_dir(&location.master_db);
    drop(wreckordbox_sidecar::Sidecar::open(&sidecar_dir).unwrap());
    fs::write(sidecar_dir.join(wreckordbox_sidecar::KINDS_FILE), "[kinds.era]\nnature = \"judgement\"\nvalue = \"text\"\n").unwrap();
    let archive = dir.path().join("backup.zip");
    testing::archive(&location, &archive, None);
    let sidecar: Vec<String> = entries(&archive).into_iter().filter(|name| name.contains("sidecar")).collect();
    // Today: a fixed list of entries, so nothing of the Sidecar.
    assert_eq!(sidecar, [] as [String; 0]);
}
