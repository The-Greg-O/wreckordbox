# wreckordbox-sidecar

[Crate index](../README.md) · [Service](../wreckordbox-service/README.md)

The Sidecar (ADR 0001, ADR 0004): wreckordbox's own store beside the Library, for what rekordbox has no place for. It is a folder holding a small SQLite database (`sidecar.db`) and the audio of the Stems. Nothing in it is a rekordbox-format file and this crate depends on none of the crates that read or write one.

The Library service owns it and is its only writer.

## Start here

[`src/lib.rs`](src/lib.rs) describes the whole interface. [`src/store.rs`](src/store.rs) is the database: Runs, kinds and Facts. [`src/stems.rs`](src/stems.rs) is the first kind of Fact. [`src/fingerprint.rs`](src/fingerprint.rs) explains the fingerprint.

## Contracts and safety

- A Fact attaches to the Library's track, with the fingerprint of the track's audio recorded alongside, and has a kind, a value (JSON), a source, a confidence from 0 to 1, the Run that made it and a time.
- Each kind is declared a Judgement or a Measurement, once. The first Fact of a kind writes the declaration into the store; recording the same name the other way is refused.
- A Fact row is written once. Afterwards it is only ever marked stale (a Measurement whose audio changed) or superseded (the same source recorded a newer Fact of the same kind for the track). Nothing is deleted, so a Ledger can be added over what is stored.
- A Stem set is one file each for vocals, other, bass and drums, and every one must be exactly as long in samples as its parent. The length is read from each file; a set that breaks the rule is refused and nothing is recorded.
- Stem audio is kept as the tool wrote it (16-bit PCM WAV from the separation engine). Conversion for a Stick happens when the Stick is written.
- The fingerprint hashes the encoded audio stream inside the file, not the file: a move, a rename or a retag changes nothing, and any change to the audio does.

## Run focused checks

```sh
cargo test -p wreckordbox-sidecar
```
