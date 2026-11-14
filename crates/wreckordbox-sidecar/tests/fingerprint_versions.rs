//! Same packets can decode differently: versioned decoding identity, legacy compatibility.
#![allow(clippy::pedantic, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::path::Path;
use wreckordbox_sidecar::{fingerprint_file, Fingerprint, Kind, NewFact, Sidecar, Source, SourceKind};
fn wav(path: &Path, format: u16, rate: u32, value: u32) -> Vec<u8> {
    let data: Vec<u8> = (0..128).flat_map(|_| value.to_le_bytes()).collect();
    let mut out = b"RIFF".to_vec();
    out.extend((36+data.len() as u32).to_le_bytes()); out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes()); out.extend(format.to_le_bytes()); out.extend(1u16.to_le_bytes());
    out.extend(rate.to_le_bytes()); out.extend((rate*4).to_le_bytes()); out.extend(4u16.to_le_bytes()); out.extend(32u16.to_le_bytes());
    out.extend(b"data"); out.extend((data.len() as u32).to_le_bytes()); out.extend(&data);
    std::fs::write(path,out).unwrap(); data
}
// Frozen a1 specification, independent of the implementation under test.
fn legacy(data: &[u8], rate: u32) -> Fingerprint {
    let mut h=blake3::Hasher::new(); h.update(b"wreckordbox audio fingerprint 1\0"); h.update(&rate.to_le_bytes()); h.update(&1u32.to_le_bytes()); h.update(data);
    Fingerprint::from_stored(format!("a1:{}",h.finalize().to_hex()))
}
#[test]
fn pcm32_and_float32_with_identical_packets_are_different_audio() {
    let d=tempfile::tempdir().unwrap(); let pcm=d.path().join("pcm.wav"); let float=d.path().join("float.wav");
    let a=wav(&pcm,1,44100,0.25f32.to_bits()); let b=wav(&float,3,44100,0.25f32.to_bits());
    assert_eq!(a,b); assert_ne!(fingerprint_file(&pcm).unwrap(),fingerprint_file(&float).unwrap());
}
#[test]
fn old_measurements_keep_the_old_comparison_and_original_provenance() {
    let d=tempfile::tempdir().unwrap(); let path=d.path().join("track.wav"); let data=wav(&path,1,44100,0.25f32.to_bits()); let old=legacy(&data,44100);
    let s=Sidecar::open(&d.path().join("sidecar")).unwrap(); let source=Source::new(SourceKind::Tool,"legacy"); let run=s.begin_run(&source,"old measurement").unwrap();
    let f=s.record(&NewFact{track_id:"track",fingerprint:Some(&old),kind:Kind::measurement("intensity"),value:serde_json::json!({"summary":0.5}),source:&source,confidence:1.0,run}).unwrap();
    s.finish_run(run,"done").unwrap();
    assert!(s.mark_stale("track",&fingerprint_file(&path).unwrap()).unwrap().is_empty());
    // The stored a1 fingerprint cannot distinguish this replacement; do not
    // reinterpret old evidence using v2 or silently upgrade it from a read.
    wav(&path,3,44100,0.25f32.to_bits());
    assert!(s.mark_stale("track",&fingerprint_file(&path).unwrap()).unwrap().is_empty());
    assert_eq!(s.fact(f.id).unwrap().unwrap().fingerprint,Some(old));
    wav(&path,3,44100,0.5f32.to_bits());
    assert_eq!(s.mark_stale("track",&fingerprint_file(&path).unwrap()).unwrap(),vec![f.id]);
}
