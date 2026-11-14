//! Real prepared/published key tags and external tags, on synthetic files only.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
#[path = "../../wreckordbox-sidecar/tests/fingerprint_support/mod.rs"]
mod fixture;
use lofty::{
    config::WriteOptions,
    picture::{MimeType, Picture, PictureType},
    prelude::{TagExt, TaggedFileExt},
    tag::{ItemKey, Tag, TagType},
};
use std::path::Path;
use wreckordbox_sidecar::fingerprint_file;
fn key(path: &Path, key: Option<&str>, supported: bool) {
    use rbl_db::key_tag::prepared::{prepare, Outcome};
    let before = std::fs::read(path).unwrap();
    match prepare(path, None, key).unwrap() {
        Outcome::Ready(mut staged) => {
            assert!(supported);
            assert_eq!(
                std::fs::read(path).unwrap(),
                before,
                "prepare does not publish"
            );
            staged.publish().unwrap();
            drop(staged);
            assert_eq!(
                rbl_db::key_tag::read_key(path)
                    .unwrap()
                    .as_deref()
                    .unwrap_or(""),
                key.unwrap_or("")
            );
        }
        Outcome::Skipped(_) => {
            assert!(!supported);
            assert_eq!(std::fs::read(path).unwrap(), before);
        }
    }
}
fn metadata(path: &Path, format: &str, add: bool) {
    let ty = match format {
        "flac" => TagType::VorbisComments,
        "m4a" => TagType::Mp4Ilst,
        _ => TagType::Id3v2,
    };
    let mut tag = Tag::new(ty);
    if add {
        tag.insert_text(ItemKey::TrackTitle, "Synthetic retag fixture".into());
        tag.insert_text(ItemKey::Comment, "A comment unrelated to samples".into());
        let png="89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000b49444154789c636000020000050001a5f645400000000049454e44ae426082";
        let bytes = (0..png.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&png[i..i + 2], 16).unwrap())
            .collect();
        tag.push_picture(Picture::new_unchecked(
            PictureType::CoverFront,
            Some(MimeType::Png),
            None,
            bytes,
        ));
    }
    tag.save_to_path(path, WriteOptions::default()).unwrap();
    let tagged = lofty::read_from_path(path).unwrap();
    let actual = tagged.tag(ty);
    assert_eq!(
        actual.and_then(|t| t.get_string(&ItemKey::TrackTitle)),
        add.then_some("Synthetic retag fixture")
    );
    assert_eq!(actual.map_or(0, |t| t.pictures().len()), usize::from(add));
}
fn exercise(format: &str) {
    let d = tempfile::tempdir().unwrap();
    let p = fixture::write(d.path(), format);
    let original = std::fs::read(&p).unwrap();
    let expected = fingerprint_file(&p).unwrap();
    let same = |stage: &str| {
        let got = fingerprint_file(&p).unwrap();
        assert_eq!(
            got.as_str(),
            expected.as_str(),
            "{format}: {stage}; legacy {} -> {}",
            expected.legacy(),
            got.legacy()
        );
    };
    let supported = !matches!(format, "wav" | "m4a");
    key(&p, Some("Am"), supported);
    same("app key publication");
    if format == "aiff" {
        // The app places ID3 after SSND. Foundation included its first bytes;
        // a1 must keep that old behavior, while a2 is confined to sound data.
        let retagged = fingerprint_file(&p).unwrap();
        assert_ne!(retagged.legacy(), expected.legacy());
        let golden: serde_json::Value = serde_json::from_str(include_str!(
            "../../wreckordbox-sidecar/tests/fingerprint_support/golden.json"
        ))
        .unwrap();
        assert_eq!(
            retagged.legacy().as_str(),
            golden["aiff"]["key_legacy"].as_str().unwrap()
        );
        assert!(!retagged.matches_stored(&expected.legacy()));
        let bytes = std::fs::read(&p).unwrap();
        let sound = bytes.windows(4).position(|b| b == b"SSND").unwrap();
        let tag = bytes.windows(4).position(|b| b == b"ID3 ").unwrap();
        assert!(tag > sound);
    }
    key(&p, Some("Dbm"), supported);
    same("app key replacement");
    key(&p, None, supported);
    same("app key removal");
    metadata(&p, format, true);
    assert_ne!(std::fs::read(&p).unwrap(), original);
    same("title/comment/artwork added");
    metadata(&p, format, false);
    same("tags and artwork removed");
    // Trailing container tags: the ID3 writer uses end chunks for AIFF/WAV.
    // APEv2/Lyrics3/ID3v1 are external trailers, also harmless to other streams.
    for trailer in [
        fixture::ape(),
        {
            let mut b = b"LYRICSBEGIN".to_vec();
            b.extend(vec![b'x'; 4096]);
            let n = b.len();
            b.extend(format!("{n:06}LYRICS200").as_bytes());
            b
        },
        {
            let mut b = vec![0; 128];
            b[..3].copy_from_slice(b"TAG");
            b
        },
    ] {
        std::fs::write(&p, &original).unwrap();
        let mut b = original.clone();
        b.extend(trailer);
        std::fs::write(&p, b).unwrap();
        same("trailing tag");
    }
    // Produce a valid, genuinely different signal, not a corrupt packet
    // which merely forces the fingerprint's whole-file fallback.
    static ENCODING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one_encoder = ENCODING.lock().unwrap();
    std::fs::write(&p, &original).unwrap();
    let changed = d
        .path()
        .join(format!("changed.{}", fixture::extension(format)));
    let codec = match format {
        "mp3-xing" | "mp3-cbr" => "libmp3lame",
        "flac" => "flac",
        "m4a" => "aac",
        "aiff" => "pcm_s16be",
        _ => "pcm_s16le",
    };
    let encoded = std::process::Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-threads",
            "1",
            "-filter_threads",
            "1",
            "-i",
        ])
        .arg(&p)
        .args(["-af", "volume=0.5", "-c:a", codec, "-threads", "1"])
        .arg(&changed)
        .output()
        .unwrap();
    assert!(
        encoded.status.success(),
        "{}",
        String::from_utf8_lossy(&encoded.stderr)
    );
    let after = fingerprint_file(&changed).unwrap();
    assert!(
        after.as_str().starts_with("a2:"),
        "valid changed {format}: {after}"
    );
    assert_ne!(after, expected, "{format}: changed audio");
}
#[test]
fn mp3_xing_retags() {
    exercise("mp3-xing")
}
#[test]
fn mp3_cbr_retags() {
    exercise("mp3-cbr")
}
#[test]
fn flac_retags() {
    exercise("flac")
}
#[test]
fn aiff_retags() {
    exercise("aiff")
}
#[test]
fn wav_retags() {
    exercise("wav")
}
#[test]
fn m4a_retags() {
    exercise("m4a")
}
