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

#[test]
fn unsupported_exchange_falls_back_without_losing_rollback_or_checks() {
    for reason in [
        std::io::ErrorKind::InvalidInput,
        std::io::ErrorKind::Unsupported,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fallback.mp3");
        write_mp3(&path, Some(&busy_tag(Some("Cm"))), false);
        let before = std::fs::read(&path).unwrap();
        let mut plan = ready(&path, "Am");
        plan.force_exchange_error_for_test(reason);
        plan.publish().unwrap();
        assert_eq!(read_key(&path).unwrap().as_deref(), Some("Am"));
        plan.rollback().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
        drop(plan);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        let mut stale = ready(&path, "Fm");
        stale.force_exchange_error_for_test(reason);
        write_key(&path, "Gm").unwrap();
        assert!(stale.publish().is_err());
        assert_eq!(read_key(&path).unwrap().as_deref(), Some("Gm"));
    }
}

#[test]
fn preparing_sweeps_only_stale_owned_temporaries_from_dead_processes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cleanup.mp3");
    write_mp3(&path, Some(&busy_tag(Some("Cm"))), false);
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
    let names = [
        ".cleanup.mp3.rbx-key-4294967295-1.tmp".to_owned(),
        format!(".cleanup.mp3.rbx-key-{}-1.tmp", std::process::id()),
        ".cleanup.mp3.rbx-key-user-copy.tmp".to_owned(),
        ".other.mp3.rbx-key-4294967295-1.tmp".to_owned(),
    ];
    for name in &names {
        let file = std::fs::File::create(dir.path().join(name)).unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(old))
            .unwrap();
    }
    drop(ready(&path, "Am"));
    assert!(
        !dir.path().join(&names[0]).exists(),
        "prepare must clean the crashed writer's copy"
    );
    for name in &names[1..] {
        assert!(dir.path().join(name).exists(), "leave {name}");
    }
}

#[test]
fn prepared_aiff_and_flac_preserve_audio_other_tags_and_exact_rollback_bytes() {
    for fallback in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let aiff = dir.path().join("prepared.aiff");
        write_aiff(&aiff, Some(&busy_tag(Some("Cm"))));
        let before = std::fs::read(&aiff).unwrap();
        let tags = others(&read_id3(&aiff));
        let mut plan = ready(&aiff, "Am");
        if fallback {
            plan.force_exchange_error_for_test(std::io::ErrorKind::Unsupported);
        }
        plan.publish().unwrap();
        assert_eq!(read_key(&aiff).unwrap().as_deref(), Some("Am"));
        assert_eq!(others(&read_id3(&aiff)), tags);
        let after = std::fs::read(&aiff).unwrap();
        let ssnd = after.windows(4).position(|w| w == b"SSND").unwrap();
        assert_eq!(&after[ssnd + 16..ssnd + 16 + 8820], &noise(8820, 3));
        assert_eq!(
            aiff_chunks_except_id3(&after),
            aiff_chunks_except_id3(&before)
        );
        plan.rollback().unwrap();
        assert_eq!(std::fs::read(&aiff).unwrap(), before);
        let flac = dir.path().join("prepared.flac");
        write_flac(
            &flac,
            Some(&["title=kept", "INITIALKEY=Cm", "encoder=kept"]),
            8192,
        );
        let before = std::fs::read(&flac).unwrap();
        let (_, offset) = flac_comments(&flac);
        let mut plan = ready(&flac, "Am");
        if fallback {
            plan.force_exchange_error_for_test(std::io::ErrorKind::Unsupported);
        }
        plan.publish().unwrap();
        let (tags, after_offset) = flac_comments(&flac);
        assert_eq!(tags, ["title=kept", "INITIALKEY=Am", "encoder=kept"]);
        let after = std::fs::read(&flac).unwrap();
        assert_eq!(&after[after_offset..], &before[offset..]);
        assert_eq!(
            flac_non_comment_blocks(&after),
            flac_non_comment_blocks(&before)
        );
        plan.rollback().unwrap();
        assert_eq!(std::fs::read(&flac).unwrap(), before);
    }
}

#[test]
fn stale_cleanup_rechecks_authorization_and_has_an_explicit_entry_point() {
    use std::time::{Duration, SystemTime};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("authorized.mp3");
    write_mp3(&path, Some(&busy_tag(Some("Cm"))), false);
    let original = std::fs::read(&path).unwrap();
    let stale = dir.path().join(".authorized.mp3.rbx-key-4294967295-1.tmp");
    let copy = std::fs::File::create(&stale).unwrap();
    copy.set_modified(SystemTime::now() - Duration::from_secs(7200))
        .unwrap();
    let checks = std::cell::Cell::new(0);
    let result = rbl_db::key_tag::prepared::prepare_checked(&path, None, Some("Am"), || {
        let previous = checks.replace(checks.get() + 1);
        if previous == 0 {
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "authorization lost",
            ))
        }
    });
    assert!(result.is_err());
    assert!(stale.exists(), "refused cleanup leaves the copy alone");
    assert_eq!(std::fs::read(&path).unwrap(), original);
    // An explicitly authorized ordinary fixture folder: no real Stick involved.
    assert_eq!(rbl_db::key_tag::prepared::cleanup_stale(&path).unwrap(), 1);
    assert!(!stale.exists());
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

fn aiff_chunks_except_id3(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut at = 12;
    let mut chunks = Vec::new();
    while at + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let end = at + 8 + size + size % 2;
        if &bytes[at..at + 4] != b"ID3 " {
            chunks.push(bytes[at..end].to_vec());
        }
        at = end;
    }
    chunks
}
fn flac_non_comment_blocks(bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut at = 4;
    let mut blocks = Vec::new();
    loop {
        let header = &bytes[at..at + 4];
        let kind = header[0] & 0x7f;
        let size = u32::from_be_bytes([0, header[1], header[2], header[3]]) as usize;
        if !matches!(kind, 1 | 4) {
            blocks.push((kind, bytes[at + 4..at + 4 + size].to_vec()));
        }
        at += 4 + size;
        if header[0] & 0x80 != 0 {
            break;
        }
    }
    blocks
}

#[test]
fn staging_keeps_its_copies_if_authorization_is_lost_before_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lost.mp3");
    write_mp3(&path, Some(&busy_tag(Some("Cm"))), false);
    let before = std::fs::read(&path).unwrap();
    let result = rbl_db::key_tag::prepared::prepare_checked(&path, None, Some("Am"), || {
        if std::fs::read_dir(dir.path())?.count() > 1 {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Stick replaced during staging",
            ))
        } else {
            Ok(())
        }
    });
    assert!(result.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        2,
        "no unauthorized deletion of the staged copy"
    );
}
