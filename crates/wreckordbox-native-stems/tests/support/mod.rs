//! A temporary folder standing in for a Stick, written by the export
//! pipeline, and small synthetic Stems to put on it. Never a real Stick.
#![allow(
    dead_code,
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::path::{Path, PathBuf};

use rbl_export::device_library::{self, Format};
use rbl_export::{SourcePlaylist, SourceTrack};
use wreckordbox_native_stems::{Request, StemFiles, SPARE_STICK_MARKER};

/// Where the Stems' sound starts, in sample frames: silence before, a
/// tone after.
pub const ONSET: u64 = 9_000;

/// A 16-bit 44.1 kHz WAV of `frames` sample frames on `channels` channels:
/// silent up to [`ONSET`], then a tone of `hertz` with noise over it. The
/// noise is there so the encoded Stem has the size of real music: a bare
/// tone packs into a single Ogg page, which no real Stem does and which
/// decoders (the deck's included) do not trim to the stream's length.
pub fn write_tone(path: &Path, frames: u64, channels: u16, hertz: f64) {
    write_tone_at(path, frames, channels, hertz, 44_100);
}

pub fn write_tone_at(path: &Path, frames: u64, channels: u16, hertz: f64, rate: u32) {
    let data_len = u32::try_from(frames).unwrap() * u32::from(channels) * 2;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    let mut noise = 0x2545_f491_u32;
    for frame in 0..frames {
        let sample = if frame < ONSET {
            0
        } else {
            let t = (frame - ONSET) as f64 / f64::from(rate);
            noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let hiss = f64::from((noise >> 16) as i16) / 8.0;
            ((t * hertz * std::f64::consts::TAU).sin() * 16_000.0 + hiss) as i16
        };
        for _ in 0..channels {
            out.extend_from_slice(&sample.to_le_bytes());
        }
    }
    std::fs::write(path, out).unwrap();
}

/// A Stick in a temporary folder, and the files exported to it.
pub struct Rig {
    pub source: tempfile::TempDir,
    pub stick: tempfile::TempDir,
}

impl Rig {
    /// A Stick holding an export of one track per `(file name, track id)`,
    /// carrying the Spare stick marker.
    pub fn spare(tracks: &[(&str, u64)]) -> Self {
        let rig = Self::unmarked(tracks);
        std::fs::write(rig.stick.path().join(SPARE_STICK_MARKER), b"").unwrap();
        rig
    }

    /// The same Stick without the marker: a Gig stick, for all anyone knows.
    pub fn unmarked(tracks: &[(&str, u64)]) -> Self {
        let rig = Self {
            source: tempfile::tempdir().unwrap(),
            stick: tempfile::tempdir().unwrap(),
        };
        rig.export(tracks);
        rig
    }

    /// Exports `tracks` to the Stick, each from a file of that name in its
    /// own folder (so two may share a name), as the export pipeline does.
    pub fn export(&self, tracks: &[(&str, u64)]) {
        let tracks: Vec<SourceTrack> = tracks
            .iter()
            .map(|(name, id)| {
                let dir = self.source.path().join(format!("from-{id}"));
                std::fs::create_dir_all(&dir).unwrap();
                let path = dir.join(name);
                // Not audio anyone plays: bytes that differ per track.
                std::fs::write(&path, vec![*id as u8; 3_000 + *id as usize]).unwrap();
                SourceTrack {
                    id: *id,
                    source_path: path,
                    title: format!("Track {id}"),
                    artist: format!("Artist {id}"),
                    album: "Single".into(),
                    bpm_x100: 12_800,
                    duration_sec: 300,
                    ..SourceTrack::default()
                }
            })
            .collect();
        let playlist = SourcePlaylist {
            id: 10,
            name: "Set".into(),
            track_indices: (0..tracks.len()).collect(),
            ..Default::default()
        };
        rbl_export::export(self.stick.path(), &tracks, &[playlist]).unwrap();
    }

    /// Makes the Stick's library hold `path` (with its leading slash) for
    /// a track, as a Stick written by another tool might. The file itself
    /// is not moved.
    pub fn hold_in_library(&self, track_id: u64, path: &str) {
        let one = self.root().join("PIONEER/rekordbox/exportLibrary.db");
        let db = rbl_onelibrary::playlists::open_read_write(&one).unwrap();
        let name = path.rsplit('/').next().unwrap();
        let changed = db
            .execute(
                "UPDATE content SET path = ?1, fileName = ?2 WHERE masterContentId = ?3",
                (path, name, track_id as i64),
            )
            .unwrap();
        assert_eq!(changed, 1);
    }

    /// Makes the Stick's library say its tracks were exported from the
    /// Library `library_id`, as a Stick written by rekordbox does.
    pub fn exported_from_library(&self, library_id: u64) {
        let one = self.root().join("PIONEER/rekordbox/exportLibrary.db");
        let db = rbl_onelibrary::playlists::open_read_write(&one).unwrap();
        db.execute("UPDATE content SET masterDbId = ?1", [library_id as i64])
            .unwrap();
    }

    pub fn root(&self) -> &Path {
        self.stick.path()
    }

    /// The path the Stick's exported library holds for a track, without
    /// the leading slash.
    pub fn library_path(&self, track_id: u64) -> String {
        let library = device_library::read(self.root(), Format::OneLibrary).unwrap();
        let track = library
            .tracks
            .iter()
            .find(|t| t.master_content_id == track_id)
            .unwrap();
        track.path.trim_start_matches('/').to_owned()
    }

    /// The Library the Stick's library says the track was exported from.
    pub fn exported_from(&self, track_id: u64) -> u64 {
        let library = device_library::read(self.root(), Format::OneLibrary).unwrap();
        let track = library
            .tracks
            .iter()
            .find(|t| t.master_content_id == track_id)
            .unwrap();
        track.master_db_id
    }

    /// The track's file on the Stick, spelled as the library holds it.
    pub fn track_file(&self, track_id: u64) -> PathBuf {
        self.root().join(self.library_path(track_id))
    }

    /// Four Stems of `frames` sample frames each, in a folder of their own
    /// (standing in for the Sidecar's), one tone per role.
    pub fn stems(&self, name: &str, frames: u64) -> Stems {
        let dir = self.source.path().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let file = |role: &str, hertz: f64| {
            let path = dir.join(format!("{role}.wav"));
            write_tone(&path, frames, 2, hertz);
            path
        };
        Stems {
            vocals: file("vocals", 880.0),
            other: file("other", 660.0),
            bass: file("bass", 110.0),
            drums: file("drums", 220.0),
        }
    }

    pub fn request<'a>(&'a self, track_id: u64, stems: &'a Stems) -> Request<'a> {
        Request {
            stick: self.root(),
            library_id: 0,
            track_id,
            stems: stems.files(),
            frames: None,
        }
    }
}

/// Four Stem files, as a Sidecar would hand them over.
pub struct Stems {
    pub vocals: PathBuf,
    pub other: PathBuf,
    pub bass: PathBuf,
    pub drums: PathBuf,
}

impl Stems {
    pub fn files(&self) -> StemFiles<'_> {
        StemFiles {
            vocals: &self.vocals,
            other: &self.other,
            bass: &self.bass,
            drums: &self.drums,
        }
    }
}
