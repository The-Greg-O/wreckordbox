//! File publication effects; generated audio only.
use super::*;
use rbl_db::key_tag::prepared::{prepare, Outcome, Prepared};
fn ready(path: &Path, key: &str) -> Box<Prepared> {
    match prepare(path, None, Some(key)).unwrap() {
        Outcome::Ready(plan) => plan,
        Outcome::Skipped(other) => panic!("unexpected: {other:?}"),
    }
}
#[test]
fn preparation_preserves_the_original_and_rollback_restores_exact_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("song.mp3");
    write_mp3(&path, Some(&busy_tag(Some("Cm"))), false);
    let before = std::fs::read(&path).unwrap();
    let mut plan = ready(&path, "Am");
    assert_eq!(plan.before(), Some("Cm"));
    assert_eq!(plan.after(), Some("Am"));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    plan.publish().unwrap();
    assert_eq!(read_key(&path).unwrap().as_deref(), Some("Am"));
    plan.rollback().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    drop(plan);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
#[test]
fn a_prepared_tag_cannot_replace_a_concurrent_writer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("song.mp3");
    write_mp3(&path, Some(&busy_tag(Some("Cm"))), false);
    let mut first = ready(&path, "Am");
    let mut second = ready(&path, "Fm");
    first.publish().unwrap();
    assert!(second.publish().is_err());
    assert_eq!(read_key(&path).unwrap().as_deref(), Some("Am"));
    // An external in-place write after publication also prevents rollback.
    write_key(&path, "Gm").unwrap();
    assert!(first.rollback().is_err());
    assert_eq!(read_key(&path).unwrap().as_deref(), Some("Gm"));
}
#[cfg(unix)]
#[test]
fn a_retargeted_symlink_cannot_publish_to_either_file() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.mp3");
    let b = dir.path().join("b.mp3");
    let link = dir.path().join("link.mp3");
    write_mp3(&a, Some(&busy_tag(Some("Cm"))), false);
    write_mp3(&b, Some(&busy_tag(Some("Dm"))), false);
    std::os::unix::fs::symlink(&a, &link).unwrap();
    let mut plan = ready(&link, "Am");
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&b, &link).unwrap();
    assert!(plan.publish().is_err());
    assert_eq!(read_key(&a).unwrap().as_deref(), Some("Cm"));
    assert_eq!(read_key(&b).unwrap().as_deref(), Some("Dm"));
}
