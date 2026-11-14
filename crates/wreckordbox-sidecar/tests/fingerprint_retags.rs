#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
mod fingerprint_support;
use fingerprint_support as fixture;
use wreckordbox_sidecar::fingerprint_file;
#[test]
fn cbr_without_xing_and_a_four_kib_trailing_ape_tag_keeps_its_identity() {
    let d = tempfile::tempdir().unwrap();
    let p = fixture::write(d.path(), "mp3-cbr");
    let before = fingerprint_file(&p).unwrap();
    let mut b = std::fs::read(&p).unwrap();
    b.extend(fixture::ape());
    std::fs::write(&p, b).unwrap();
    let after = fingerprint_file(&p).unwrap();
    assert_eq!(after.legacy(), before.legacy());
    assert_eq!(after.as_str(), before.as_str());
}
#[test]
fn six_formats_match_frozen_a2_and_independently_built_foundation_a1() {
    let golden: serde_json::Value =
        serde_json::from_str(include_str!("fingerprint_support/golden.json")).unwrap();
    let d = tempfile::tempdir().unwrap();
    for format in fixture::FORMATS {
        let p = fixture::write(d.path(), format);
        let fresh = fingerprint_file(&p).unwrap();
        assert_eq!(
            fresh.as_str(),
            golden[format]["a2"].as_str().unwrap(),
            "{format}"
        );
        let old = wreckordbox_sidecar::Fingerprint::from_stored(
            golden[format]["a1"].as_str().unwrap().into(),
        );
        assert!(
            fresh.matches_stored(&old),
            "foundation {format}: {fresh} vs {old}"
        );
        let mut trailed = std::fs::read(&p).unwrap();
        trailed.extend(fixture::ape());
        std::fs::write(&p, trailed).unwrap();
        let fresh_trailed = fingerprint_file(&p).unwrap();
        let old_trailed = wreckordbox_sidecar::Fingerprint::from_stored(
            golden[format]["ape_legacy"].as_str().unwrap().into(),
        );
        assert!(
            fresh_trailed.matches_stored(&old_trailed),
            "foundation trailer {format}: {fresh_trailed} vs {old_trailed}"
        );
        assert_eq!(
            fresh_trailed.as_str(),
            fresh.as_str(),
            "new audio identity ignores trailer"
        );
    }
}
