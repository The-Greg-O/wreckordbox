# Licensing

wreckordbox (forked from [rbxport](https://github.com/chrisle/rbxport) by Chris
Le) is licensed as a whole under GPL-2.0-or-later. Both upstream's copyright
(`Copyright (c) 2026 Chris Le`) and the fork's (`Copyright (c) 2026 Greg Overton`)
apply; see [`LICENSE`](LICENSE). The complete GPL text is included in
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

## Separation engine

[`separation-engine/`](separation-engine/README.md) is an out-of-process Python
worker written by the fork's author and included under this repository's
GPL-2.0-or-later licence; it is run over standard input/output and is not
linked into any Rust binary. Its optional full backend installs third-party
Python packages (such as `audio-separator`, MIT) and downloads model weights at
run time rather than vendoring them in this repository.

## Fork distribution checklist

This is a source-distributed GPL-2.0-or-later fork, not a new licence grant for
upstream’s work. The status below describes files in this repository; it is not
a certification of future binary distributions.

| Obligation | Repository status |
| --- | --- |
| Preserve upstream copyright, licence and warranty notices; include the GPL text | Retained in `LICENSE`, the vendored `COPYING`, source headers and this document. Chris Le’s notice remains beside Greg Overton’s. |
| License the combined derivative work under compatible GPL terms | Met by the repository’s GPL-2.0-or-later declaration; vendored Rubber Band uses those same terms. |
| Identify modified files and dates of changes (GPLv2 section 2(a)) | Fork provenance is documented here and in the README, and changes are dated in Git history. A complete per-file notice audit has not been performed; this obligation is not claimed fully verified. |
| Supply complete corresponding source, including build/install scripts, when distributing binaries (GPLv2 section 3) | Source and build scripts are public here. No fork binary release channel is configured; any future distributor must provide the exact corresponding source for their binaries and preserve required notices. |
| Retain third-party notices | Existing vendored notices are retained; dependency-source notices are described above. Binary packaging must carry the applicable notices too. |

Contributors must have the right to submit their changes under GPL-2.0-or-later
and preserve existing notices. Do not import GPL-3.0-only AZ-MODS code.
