//! The fingerprint names a track's audio, not its file.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::common;

use common::write_audio;
use wreckordbox_sidecar::fingerprint_file;

#[test]
fn moving_or_renaming_a_file_does_not_change_its_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("Night Bus.wav");
    write_audio(&original, 1_000, 7, None);
    let before = fingerprint_file(&original).unwrap();

    let elsewhere = dir.path().join("moved");
    std::fs::create_dir_all(&elsewhere).unwrap();
    // A new folder, a new name and an extension that says nothing true.
    let moved = elsewhere.join("01 - night bus (final).mp3");
    std::fs::rename(&original, &moved).unwrap();

    assert_eq!(fingerprint_file(&moved).unwrap(), before);
    assert!(before.as_str().starts_with("a1:"), "{before}");
}

#[test]
fn rewriting_a_files_tags_does_not_change_its_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("plain.wav");
    let tagged = dir.path().join("tagged.wav");
    write_audio(&plain, 1_000, 7, None);
    write_audio(&tagged, 1_000, 7, Some("a comment that is not audio"));
    assert_ne!(
        std::fs::read(&plain).unwrap(),
        std::fs::read(&tagged).unwrap()
    );

    assert_eq!(
        fingerprint_file(&tagged).unwrap(),
        fingerprint_file(&plain).unwrap()
    );
}

#[test]
fn any_change_to_the_audio_changes_the_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("track.wav");
    write_audio(&path, 1_000, 7, None);
    let original = fingerprint_file(&path).unwrap();

    // Different samples, same length.
    write_audio(&path, 1_000, 8, None);
    let louder = fingerprint_file(&path).unwrap();
    // Same samples, one frame shorter.
    write_audio(&path, 999, 7, None);
    let shorter = fingerprint_file(&path).unwrap();

    assert_ne!(louder, original);
    assert_ne!(shorter, original);
    assert_ne!(shorter, louder);
}

#[test]
fn a_file_that_is_no_known_container_is_fingerprinted_whole() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mystery.bin");
    std::fs::write(&path, b"not audio that any reader here knows").unwrap();
    let first = fingerprint_file(&path).unwrap();
    assert!(first.as_str().starts_with("f1:"), "{first}");

    let renamed = dir.path().join("renamed.bin");
    std::fs::rename(&path, &renamed).unwrap();
    assert_eq!(fingerprint_file(&renamed).unwrap(), first);

    std::fs::write(&renamed, b"not audio that any reader here knows!").unwrap();
    assert_ne!(fingerprint_file(&renamed).unwrap(), first);
}

#[test]
fn a_missing_file_has_no_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    assert!(fingerprint_file(&dir.path().join("gone.wav")).is_err());
}

#[test]
fn cached_fingerprints_only_survive_matching_file_metadata() {
    use wreckordbox_sidecar::{fingerprint_cached, known_fingerprint};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("track.wav");
    write_audio(&path, 1000, 7, None);
    assert_eq!(known_fingerprint(&path), None);
    let verified = fingerprint_file(&path).unwrap();
    assert_eq!(known_fingerprint(&path), Some(verified.clone()));
    assert_eq!(fingerprint_cached(&path).unwrap(), verified);
    let previous = std::fs::metadata(&path).unwrap();
    let replacement = dir.path().join("replacement.wav");
    write_audio(&replacement, 1000, 8, None);
    std::fs::File::options()
        .write(true)
        .open(&replacement)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(previous.modified().unwrap()))
        .unwrap();
    std::fs::rename(replacement, &path).unwrap();
    // Same size and mtime; replacement inode must invalidate the verdict.
    assert_eq!(known_fingerprint(&path), None);
    assert_ne!(fingerprint_cached(&path).unwrap(), verified);
    std::fs::remove_file(&path).unwrap();
    assert_eq!(known_fingerprint(&path), None);
}
