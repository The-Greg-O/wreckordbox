//! A carry observation cannot outlive or substitute its exact verified rewrite.
use super::*;
#[test]
fn only_the_observed_successful_publication_authorizes_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let one = dir.path().join("one.mp3");
    let two = dir.path().join("two.mp3");
    for path in [&one, &two] {
        write_mp3(path, Some(&busy_tag(Some("Cm"))), false);
    }
    let mut first = ready(&one, "Fm");
    let proof = first
        .inspect_verified(|old, new| Ok((read_key(old)?, read_key(new)?)))
        .unwrap();
    assert_eq!(proof.observed(), &(Some("Cm".into()), Some("Fm".into())));
    assert!(first.published_verification(&proof).is_none());
    let mut second = ready(&two, "Fm");
    second.publish().unwrap();
    assert!(second.published_verification(&proof).is_none());
    first.publish().unwrap();
    assert!(first.published_verification(&proof).is_some());
    first.rollback().unwrap();
    assert!(first.published_verification(&proof).is_none());
}
#[test]
fn modified_staging_cannot_produce_or_publish_an_observation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("one.mp3");
    write_mp3(&path, Some(&busy_tag(Some("Cm"))), false);
    let original = std::fs::read(&path).unwrap();
    let mut plan = ready(&path, "Fm");
    assert!(plan
        .inspect_verified(|_, staged| {
            let mut bytes = std::fs::read(staged)?;
            bytes.push(1);
            std::fs::write(staged, bytes)?;
            Ok(())
        })
        .is_err());
    assert!(plan.publish().is_err());
    assert_eq!(std::fs::read(path).unwrap(), original);
}
