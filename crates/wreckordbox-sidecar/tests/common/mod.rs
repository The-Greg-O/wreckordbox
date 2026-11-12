#![allow(dead_code, clippy::pedantic, clippy::unwrap_used)]

use std::path::Path;

/// A 16-bit stereo 44.1 kHz WAV file of `frames` sample frames, every
/// sample `value`, optionally followed by a chunk of metadata.
pub fn write_audio(path: &Path, frames: u32, value: i16, comment: Option<&str>) {
    let data_len = frames * 4;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&0_u32.to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&44_100_u32.to_le_bytes());
    out.extend_from_slice(&(44_100_u32 * 4).to_le_bytes());
    out.extend_from_slice(&4_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for _ in 0..frames * 2 {
        out.extend_from_slice(&value.to_le_bytes());
    }
    if let Some(comment) = comment {
        let mut text = comment.as_bytes().to_vec();
        text.push(0);
        if text.len() % 2 == 1 {
            text.push(0);
        }
        let mut list = b"INFOICMT".to_vec();
        list.extend_from_slice(&u32::try_from(text.len()).unwrap().to_le_bytes());
        list.extend_from_slice(&text);
        out.extend_from_slice(b"LIST");
        out.extend_from_slice(&u32::try_from(list.len()).unwrap().to_le_bytes());
        out.extend_from_slice(&list);
    }
    let riff_len = u32::try_from(out.len() - 8).unwrap();
    out[4..8].copy_from_slice(&riff_len.to_le_bytes());
    std::fs::write(path, out).unwrap();
}
