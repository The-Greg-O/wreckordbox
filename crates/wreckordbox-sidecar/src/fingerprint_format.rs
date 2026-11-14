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
    number(&mut h, params.channel_layout.map(|c| u64::from(c.into_channels().bits())));
    number(&mut h, params.time_base.map(|t| u64::from(t.numer)));
    number(&mut h, params.time_base.map(|t| u64::from(t.denom)));
    number(&mut h, params.n_frames);
    number(&mut h, Some(params.start_ts));
    number(&mut h, params.delay.map(u64::from));
    number(&mut h, params.padding.map(u64::from));
    number(&mut h, params.frames_per_block);
    bytes(&mut h, params.extra_data.as_deref());
    h
}
fn number(h: &mut blake3::Hasher, value: Option<u64>) {
    h.update(&[u8::from(value.is_some())]);
    if let Some(value) = value { h.update(&value.to_le_bytes()); }
}
fn bytes(h: &mut blake3::Hasher, value: Option<&[u8]>) {
    number(h, value.map(|v| v.len() as u64));
    if let Some(value) = value { h.update(value); }
}
fn sample_format(format: SampleFormat) -> u64 {
    match format {
        SampleFormat::U8 => 1, SampleFormat::U16 => 2, SampleFormat::U24 => 3, SampleFormat::U32 => 4,
        SampleFormat::S8 => 5, SampleFormat::S16 => 6, SampleFormat::S24 => 7, SampleFormat::S32 => 8,
        SampleFormat::F32 => 9, SampleFormat::F64 => 10,
    }
}
