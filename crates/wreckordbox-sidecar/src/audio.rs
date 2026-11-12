//! What an audio file says about itself, read from its container without
//! decoding: enough to hold a Stem to its parent's length.

use std::fs::File;
use std::path::Path;

use serde::{Deserialize, Serialize};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::{io, Result, SidecarError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioInfo {
    /// Length in sample frames (one frame is one sample per channel).
    pub samples: u64,
    pub sample_rate: u32,
    pub channels: u16,
    /// `None` for a format that has no fixed sample width.
    pub bits_per_sample: Option<u32>,
}

/// Reads the length, rate and shape of the audio file at `path`.
pub fn audio_info(path: &Path) -> Result<AudioInfo> {
    let unreadable = |why: &str| SidecarError::Audio {
        path: path.to_path_buf(),
        why: why.to_owned(),
    };
    let file = File::open(path).map_err(io("could not open", path))?;
    let stream = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
    let probed = symphonia::default::get_probe()
        .format(
            &Hint::new(),
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| unreadable(&e.to_string()))?;
    let track = probed
        .format
        .default_track()
        .ok_or_else(|| unreadable("it has no audio stream"))?;
    let params = &track.codec_params;
    Ok(AudioInfo {
        samples: params
            .n_frames
            .ok_or_else(|| unreadable("it does not say how long it is"))?,
        sample_rate: params
            .sample_rate
            .ok_or_else(|| unreadable("it does not say its sample rate"))?,
        channels: params
            .channels
            .and_then(|channels| u16::try_from(channels.count()).ok())
            .ok_or_else(|| unreadable("it does not say how many channels it has"))?,
        bits_per_sample: params.bits_per_sample,
    })
}
