//! Deterministic synthetic fixtures: generated PCM and existing generated compressed fixtures.
#![allow(
    dead_code,
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
use std::path::Path;
pub const FORMATS: [&str; 6] = ["mp3-xing", "mp3-cbr", "flac", "aiff", "wav", "m4a"];
pub fn extension(format: &str) -> &str {
    if format.starts_with("mp3") {
        "mp3"
    } else {
        format
    }
}
pub fn bytes(format: &str) -> Vec<u8> {
    match format {
        "mp3-xing" | "mp3-cbr" => {
            let mut b = include_bytes!("../../../rbl-deck/tests/fixtures/seek-noise.mp3").to_vec();
            let start = 10
                + (0..4)
                    .map(|i| usize::from(b[6 + i]) << (7 * (3 - i)))
                    .sum::<usize>();
            assert_eq!(&b[start..start + 4], &[0xff, 0xfb, 0x50, 0x00]);
            assert_eq!(&b[start + 36..start + 40], b"Info");
            if format == "mp3-cbr" {
                b.drain(start..start + 208);
            } else {
                b[start + 36..start + 40].copy_from_slice(b"Xing");
            }
            b
        }
        "flac" => include_bytes!("../../../rbl-audio/tests/fixtures/stereo-96k.flac").to_vec(),
        "m4a" => include_bytes!("../../../rbl-audio/tests/fixtures/tone-faststart.m4a").to_vec(),
        "wav" => wav(4410),
        "aiff" => {
            let mut comm = 1u16.to_be_bytes().to_vec();
            comm.extend(4410u32.to_be_bytes());
            comm.extend(16u16.to_be_bytes());
            comm.extend([0x40, 0x0e, 0xac, 0x44, 0, 0, 0, 0, 0, 0]);
            let mut sound = vec![0; 8];
            for i in 0..4410i16 {
                sound.extend(i.wrapping_mul(7).to_be_bytes());
            }
            let mut body = b"AIFF".to_vec();
            chunk(&mut body, b"COMM", &comm, true);
            chunk(&mut body, b"SSND", &sound, true);
            let mut out = b"FORM".to_vec();
            out.extend((body.len() as u32).to_be_bytes());
            out.extend(body);
            out
        }
        _ => panic!("unknown format"),
    }
}
pub fn wav(frames: u32) -> Vec<u8> {
    let mut b = b"RIFF".to_vec();
    b.extend((36 + frames * 2).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(44100u32.to_le_bytes());
    b.extend(88200u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend((frames * 2).to_le_bytes());
    for i in 0..frames {
        b.extend((i as i16).wrapping_mul(7).to_le_bytes());
    }
    b
}
pub fn write(root: &Path, format: &str) -> std::path::PathBuf {
    let p = root.join(format!("{format}.{}", extension(format)));
    std::fs::write(&p, bytes(format)).unwrap();
    p
}
pub fn chunk(b: &mut Vec<u8>, name: &[u8; 4], data: &[u8], big: bool) {
    b.extend(name);
    let len = data.len() as u32;
    b.extend(if big {
        len.to_be_bytes()
    } else {
        len.to_le_bytes()
    });
    b.extend(data);
    if data.len() % 2 != 0 {
        b.push(0);
    }
}
pub fn ape() -> Vec<u8> {
    // One valid 4 KiB APEv2 tag, including its 32-byte footer.
    let value = 4096 - 32 - 8 - 8;
    let mut b = (value as u32).to_le_bytes().to_vec();
    b.extend(0u32.to_le_bytes());
    b.extend(b"Comment\0");
    b.extend(vec![b'x'; value]);
    b.extend(b"APETAGEX");
    b.extend(2000u32.to_le_bytes());
    b.extend(4096u32.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    b.extend(0u32.to_le_bytes());
    b.extend([0; 8]);
    assert_eq!(b.len(), 4096);
    b
}
