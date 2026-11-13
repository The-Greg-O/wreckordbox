# wreckordbox-native-stems

[Crate index](../README.md) · [Service](../wreckordbox-service/README.md) · [Sidecar](../wreckordbox-sidecar/README.md)

Native stems: a track's Stems written to a Stick in the form a modded XDJ-AZ plays on its pads. The layout is the published AZ-MODS stems format: a `STEMS` folder at the Stick's root, one folder per track, four Ogg Vorbis files in it. Players without the mod ignore the folder, so the Stick stays Rekordbox-readable.

This crate is written from that format's specification and holds no AZ-MODS code (AZ-MODS is GPL-3.0-only; this project is GPL-2.0-or-later). It sits beside the export crates and only reads through them (ADR 0003).

## Start here

[`src/lib.rs`](src/lib.rs) is the whole interface: `write(&Request) -> Result<Outcome, NativeStemsError>`. [`src/naming.rs`](src/naming.rs) is the layout's naming rules, [`src/stick.rs`](src/stick.rs) the Spare stick marker and the lookup in the Stick's exported library, [`src/encode.rs`](src/encode.rs) the Ogg Vorbis encoding, [`src/identity.rs`](src/identity.rs) `source.json` and its fingerprint. [`src/work.rs`](src/work.rs) is the stems folder as it is worked in: finding it, publishing a set, recovery. [`tests/deck/mod.rs`](tests/deck/mod.rs) is an independent reader of the layout that stands in for the deck in every test.

## Contracts and safety

- A Stick is written to only when it carries the Spare stick marker: an empty file named `.rbxport-disposable` at its root, the same marker upstream's real-stick tests ask for. Without it `write` answers `NotSpareStick` before it reads or writes anything else there. Never put the marker on a Gig stick.
- The folder's name comes from the path the Stick's exported library holds for the track (`rbl_export::device_library`, Device Library Plus first, then the older database), never from a directory listing: the deck compares bytes, and a listing may spell the name in another Unicode normalization. The path is taken as the library spells it, less its leading `/`: a backslash is a character of a name, not a separator.
- The track is found by the Library's track identifier and, when both sides name one, the Library's own identifier, so a Stick exported from another Library does not match.
- Naming follows the layout: the key is the file name up to its last `.`; `_` is appended when the key ends in `.` or a space, or its part before the first `.` is a Windows device name; a second song with the same file name goes in `<key>-<CRC-32 of its path on the Stick, 8 lower-case hex digits>`.
- A key no folder can carry (empty or only spaces, starting with `.`, holding `\ / : * ? " < > |` or a control character) and a file name longer than the 255 bytes the deck keeps are answered as `Outcome::Skipped`, with the reason. That is a report, not an error.
- The four files are `vocals.ogg`, `other.ogg`, `bass.ogg` and `drums.ogg`: Ogg Vorbis, 44100 Hz, stereo, libvorbis quality 5, each exactly as long in sample frames as its source and starting at the same frame. Stems of unequal length, not as long as their parent, or at another sample rate are refused; nothing here resamples.
- `source.json` records which file the set was made from (path on the Stick, size, modification time, the layout's FNV-1a fingerprint), so the deck takes the set as proven. `versions.txt` is not written.
- A set is written into a dot-named staging folder and appears with one rename. A set made again swaps the whole folder, so the deck never sees half a set and its waveform cache (`azstem.env`) for the old audio goes too. The working names are short and this writer's own (`.wbx-<8 hex>.part`, `.old` and a `.dest` note naming the folder a set-aside set belongs in), so a name at the 255-byte limit can still be worked on.
- An interrupted write is put right by the next one: an unfinished staging folder is removed; a set that was set aside and never replaced is renamed back; one whose replacement arrived is removed. A set-aside folder with no note, and any other tool's working folder, is left alone. A complete set is never the thing deleted.
- Nothing follows a symbolic link. A `STEMS` folder or a track's folder that is a link, a library path with `..` in it, and a track file that is a link or is reached through one leading off the Stick are refused as `OffStick` with nothing written.
- A second song of the same file name goes in the layout's hashed folder only when that folder is free or provably this track's; one that holds another set (or a set with no record) is refused as `FolderTaken`, not overwritten. A folder name over 255 bytes, with the `_` or the hash the layout adds, is answered as skipped.
- The crate does not know what audio the Stems were made from. Its caller must check that the export on the Stick is that audio (`exported` gives the file); the service does, and refuses a stale export.
- Nothing under `PIONEER` or `Contents` is written.

## The encoder

`vorbis_rs` (BSD-3-Clause), which builds libvorbis (the aoTuV tuning) and libogg from source vendored in its `-sys` crates (BSD-3-Clause), with a C compiler and nothing installed on the machine. The layout asks for Vorbis "at any libvorbis quality" and its own writers use libvorbis, so the reference encoder is the closest match. See [`LICENSING.md`](../../LICENSING.md).

The tests decode with `lewton`, a Vorbis decoder written in Rust, so the reader shares no codec code with the writer.

## Run focused checks

```sh
cargo test -p wreckordbox-native-stems
/home/go/code/wreckordbox-notes/bin/heavy rust mise exec -- cargo test -p wreckordbox --test it stems_write::
```
