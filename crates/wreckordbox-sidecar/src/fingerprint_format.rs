//! Frozen a2 decoding-parameter encoding. Bump the fingerprint version if
//! this encoding or the demuxer's interpretation changes. Tags, artwork,
//! integrity/checksum hints and allocation hints are not decoding identity.
use symphonia::core::{codecs::CodecParameters, sample::SampleFormat};

pub(crate) fn hash(params: &CodecParameters) -> blake3::Hasher {
    let mut h = blake3::Hasher::new();
    h.update(b"wreckordbox audio fingerprint 2\0");
    // Symphonia exposes its numeric codec identifier through Display (hex).
    // Length-prefixed so no field can run into another.
    bytes(&mut h, Some(params.codec.to_string().as_bytes()));
    number(&mut h, params.sample_rate.map(u64::from));
    number(&mut h, params.sample_format.map(sample_format));
    number(&mut h, params.bits_per_sample.map(u64::from));
    number(&mut h, params.bits_per_coded_sample.map(u64::from));
    number(&mut h, params.channels.map(|c| u64::from(c.bits())));
    number(
        &mut h,
        params
            .channel_layout
            .map(|c| u64::from(c.into_channels().bits())),
    );
    number(&mut h, params.time_base.map(|t| u64::from(t.numer)));
    number(&mut h, params.time_base.map(|t| u64::from(t.denom)));
    // Never hash demuxer duration estimates (CBR MP3 depends on tag size).
    number(&mut h, Some(params.start_ts));
    number(&mut h, params.delay.map(u64::from));
    number(&mut h, params.padding.map(u64::from));
    number(&mut h, params.frames_per_block);
    bytes(&mut h, params.extra_data.as_deref());
    h
}
fn number(h: &mut blake3::Hasher, value: Option<u64>) {
    h.update(&[u8::from(value.is_some())]);
    if let Some(value) = value {
        h.update(&value.to_le_bytes());
    }
}
fn bytes(h: &mut blake3::Hasher, value: Option<&[u8]>) {
    number(h, value.map(|v| v.len() as u64));
    if let Some(value) = value {
        h.update(value);
    }
}
fn sample_format(format: SampleFormat) -> u64 {
    match format {
        SampleFormat::U8 => 1,
        SampleFormat::U16 => 2,
        SampleFormat::U24 => 3,
        SampleFormat::U32 => 4,
        SampleFormat::S8 => 5,
        SampleFormat::S16 => 6,
        SampleFormat::S24 => 7,
        SampleFormat::S32 => 8,
        SampleFormat::F32 => 9,
        SampleFormat::F64 => 10,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphonia::core::{
        audio::{Channels, Layout},
        codecs::{CODEC_TYPE_PCM_F32LE, CODEC_TYPE_PCM_S32LE},
        units::TimeBase,
    };

    #[test]
    fn decoding_changes_split_identity_but_allocation_hints_do_not() {
        let base = CodecParameters::new();
        let original = hash(&base).finalize();
        let variants: Vec<CodecParameters> = vec![
            {
                let mut p = base.clone();
                p.codec = CODEC_TYPE_PCM_S32LE;
                p
            },
            {
                let mut p = base.clone();
                p.codec = CODEC_TYPE_PCM_F32LE;
                p
            },
            {
                let mut p = base.clone();
                p.sample_format = Some(SampleFormat::S32);
                p
            },
            {
                let mut p = base.clone();
                p.sample_format = Some(SampleFormat::F32);
                p
            },
            {
                let mut p = base.clone();
                p.bits_per_sample = Some(24);
                p
            },
            {
                let mut p = base.clone();
                p.bits_per_coded_sample = Some(32);
                p
            },
            {
                let mut p = base.clone();
                p.channels = Some(Channels::FRONT_LEFT);
                p
            },
            {
                let mut p = base.clone();
                p.channels = Some(Channels::FRONT_RIGHT);
                p
            },
            {
                let mut p = base.clone();
                p.channel_layout = Some(Layout::Stereo);
                p
            },
            {
                let mut p = base.clone();
                p.time_base = Some(TimeBase::new(1, 44100));
                p
            },
            {
                let mut p = base.clone();
                p.start_ts = 1;
                p
            },
            {
                let mut p = base.clone();
                p.delay = Some(1);
                p
            },
            {
                let mut p = base.clone();
                p.padding = Some(1);
                p
            },
            {
                let mut p = base.clone();
                p.frames_per_block = Some(128);
                p
            },
            {
                let mut p = base.clone();
                p.extra_data = Some(vec![1, 2, 3].into_boxed_slice());
                p
            },
        ];
        let mut seen = std::collections::HashSet::new();
        for p in variants {
            let identity = hash(&p).finalize();
            assert_ne!(identity, original);
            assert!(seen.insert(identity));
        }
        let mut allocation = base;
        allocation.max_frames_per_packet = Some(8192);
        allocation.n_frames = Some(123_456);
        assert_eq!(hash(&allocation).finalize(), original);
    }
}
