//! Correct audio bounds where the pinned demuxer includes container metadata.
//! Header/footer inspection reads no audio payload.
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
};
fn at<const N: usize>(file: &mut File, pos: u64) -> io::Result<[u8; N]> {
    file.seek(SeekFrom::Start(pos))?;
    let mut b = [0; N];
    file.read_exact(&mut b)?;
    Ok(b)
}
pub(crate) fn audio_bytes(file: &mut File) -> io::Result<Option<u64>> {
    let len = file.metadata()?.len();
    if len < 12 {
        return Ok(None);
    }
    let head = at::<12>(file, 0)?;
    if &head[..4] == b"FORM" && matches!(&head[8..], b"AIFF" | b"AIFC") {
        let mut pos = 12;
        while pos + 8 <= len {
            let h = at::<8>(file, pos)?;
            let n = u64::from(u32::from_be_bytes(h[4..].try_into().unwrap_or_default()));
            if &h[..4] == b"SSND" {
                return Ok(n.checked_sub(8));
            }
            pos += 8 + n + (n % 2);
        }
    }
    Ok(None)
}
pub(crate) fn trailer_start(file: &mut File, mut end: u64) -> io::Result<u64> {
    loop {
        if end >= 128 && &at::<3>(file, end - 128)? == b"TAG" {
            end -= 128;
            continue;
        }
        if end >= 32 {
            let h = at::<32>(file, end - 32)?;
            if &h[..8] == b"APETAGEX" {
                let size = u64::from(u32::from_le_bytes(h[12..16].try_into().unwrap_or_default()));
                let flags = u32::from_le_bytes(h[20..24].try_into().unwrap_or_default());
                let total = size + if flags & 0x8000_0000 != 0 { 32 } else { 0 };
                if size >= 32 && total <= end {
                    end -= total;
                    continue;
                }
            }
        }
        if end >= 15 {
            let h = at::<15>(file, end - 15)?;
            if &h[6..] == b"LYRICS200" {
                let size = std::str::from_utf8(&h[..6])
                    .ok()
                    .and_then(|s| s.parse::<u64>().ok());
                if let Some(size) = size.filter(|n| *n >= 11 && *n <= end - 15) {
                    let start = end - 15 - size;
                    if &at::<11>(file, start)? == b"LYRICSBEGIN" {
                        end = start;
                        continue;
                    }
                }
            }
        }
        return Ok(end);
    }
}

/// These demuxers can mistake raw trailing tags for frames/atoms. A bounded
/// view gives a2 the real stream; legacy must still inspect the original view.
pub(crate) fn stream_end(file: &mut File) -> io::Result<Option<u64>> {
    let len = file.metadata()?.len();
    if len < 12 {
        return Ok(None);
    }
    let head = at::<12>(file, 0)?;
    if &head[..4] == b"fLaC" || &head[4..8] == b"ftyp" {
        let end = trailer_start(file, len)?;
        return Ok((end < len).then_some(end));
    }
    Ok(None)
}

/// Read-only bounded file; no normalized copy is written to disk.
pub(crate) struct Bounded {
    pub file: File,
    pub end: u64,
}
impl Read for Bounded {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let remaining = self.end.saturating_sub(self.file.stream_position()?);
        let count = buf
            .len()
            .min(usize::try_from(remaining).unwrap_or(usize::MAX));
        self.file.read(&mut buf[..count])
    }
}
impl Seek for Bounded {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        match from {
            SeekFrom::End(delta) => {
                let pos = i128::from(self.end) + i128::from(delta);
                let pos = u64::try_from(pos)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "negative seek"))?;
                self.file.seek(SeekFrom::Start(pos))
            }
            other => self.file.seek(other),
        }
    }
}
impl symphonia::core::io::MediaSource for Bounded {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.end)
    }
}
