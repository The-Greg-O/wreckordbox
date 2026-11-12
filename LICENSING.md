# Licensing

rbxport is licensed as a whole under GPL-2.0-or-later. See
[`LICENSE`](LICENSE); the complete GPL text is included in
[`crates/rbl-deck/vendor/rubberband/COPYING`](crates/rbl-deck/vendor/rubberband/COPYING).

## Rubber Band

`crates/rbl-deck/vendor/rubberband` is the [Rubber Band
Library](https://breakfastquay.com/rubberband/) v4.0.0, vendored, and it is
GPL-2.0-or-later or a commercial licence from Particular Programs Ltd. This
project takes the GPL. Its own copyright and terms are in
`crates/rbl-deck/vendor/rubberband/COPYING`, unmodified.

Linking it makes the binary a combined work, so anything distributed from a
default build must comply with the GPL, including the corresponding-source
requirements.

It is here because a DJ deck needs a stretcher that holds a pitch. The WSOLA
backend written for this crate stretches tempo well and misses an interval by
25 to 50 cents, which is a key shift that puts a track in the wrong key.
Rubber Band R3 lands the same intervals inside 1.4 cents.

## Building without it

```sh
cargo build -p rbl-deck --no-default-features
```

The `rubberband` feature is on by default. With it off, `MASTER TEMPO` falls
back to the WSOLA backend and key shifting is not offered.

## libvorbis and libogg

`crates/wreckordbox-native-stems` encodes Native stems as Ogg Vorbis with the
`vorbis_rs` crate, which compiles libvorbis (with the aoTuV and Lancer
patches) and libogg from source carried in its `aotuv_lancer_vorbis_sys` and
`ogg_next_sys` crates. All of it is BSD-3-Clause (Xiph.Org Foundation and
the patch authors), which is compatible with GPL-2.0-or-later; the copyright
notices travel in those crates' sources.

The layout Native stems follow is published by the AZ-MODS project, which is
GPL-3.0-only. The writer is implemented from the layout's specification and
contains no AZ-MODS code.
