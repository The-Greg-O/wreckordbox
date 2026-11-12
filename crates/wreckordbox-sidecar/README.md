# wreckordbox-sidecar

[Crate index](../README.md) · [Service](../wreckordbox-service/README.md)

The Sidecar (ADR 0001, ADR 0004): wreckordbox's own store beside the Library, for what rekordbox has no place for. It is a folder holding a small SQLite database (`sidecar.db`) and the audio of the Stems. Nothing in it is a rekordbox-format file and this crate depends on none of the crates that read or write one.

The Library service owns it and is its only writer.

## Start here

[`src/lib.rs`](src/lib.rs) describes the whole interface. [`src/store.rs`](src/store.rs) is the database: Runs, kinds and Facts. [`src/kinds.rs`](src/kinds.rs) is the declared kinds and the kinds file. [`src/stems.rs`](src/stems.rs) is the first kind of Fact. [`src/decisions.rs`](src/decisions.rs) is Decisions and the answers Deciders gave them. [`src/fingerprint.rs`](src/fingerprint.rs) explains the fingerprint.

## Contracts and safety

- A Fact attaches to the Library's track, with the fingerprint of the track's audio recorded alongside, and has a kind, a value (JSON), a source, a confidence from 0 to 1, the Run that made it and a time.
- Each kind is declared a Judgement or a Measurement, once. The first Fact of a kind writes the declaration into the store; recording the same name the other way is refused.
- The declared kinds are the built-in ones (`Kinds::built_in`: stems, genre, mood, situation, sound, vocals, version, energy, prep_colour, intensity) and those in `kinds.toml` in the Sidecar's folder, which the DJ edits and versions. Each has a nature and a value type (`text`, `texts`, `integer`, `number`, `boolean`, `object`), optionally narrowed by `choices` or `min`/`max`. The file cannot redeclare a built-in kind, and a file with a mistake is refused whole. `Sidecar::record` takes any kind; the Library service checks a Fact against `Kinds` before recording it.
- A Fact row is written once. Afterwards it is only ever marked stale (a Measurement whose audio changed) or superseded (the same source recorded a newer Fact of the same kind for the track). Nothing is deleted, so a Ledger can be added over what is stored.
- A Decision (a question, its context and its fixed candidates) and each answer a Decider gives it are rows of their own tables, written once and never changed, deleted or replaced; triggers in the store refuse all three. A Decision with no answer is pending. Answering again, by the same Decider or another, adds an answer beside the earlier ones.
- An answer's choice must be one of its Decision's candidates. How sure the Decider is, is stored as `choice_probability` (the probability it gives its choice of being right) and, when it supplies them, the probability of every candidate. Neither is called a confidence. The probabilities given, with the choice counted once, never add up to more than 1.
- An answer that produces a Fact is written with it in one transaction (`Sidecar::answer_decision_with_fact`): the answer, the Fact, what the Fact supersedes and the Run made for them, all or none.
- The store's shape has a version (`meta.schema_version`). An older store is brought up to date when opened, in one transaction; a newer one is refused.
- A Stem set is one file each for vocals, other, bass and drums, and every one must be exactly as long in samples as its parent. The length is read from each file; a set that breaks the rule is refused and nothing is recorded.
- Stem audio is kept as the tool wrote it (16-bit PCM WAV from the separation engine). Conversion for a Stick happens when the Stick is written.
- The fingerprint hashes the encoded audio stream inside the file, not the file: a move, a rename or a retag changes nothing, and any change to the audio does.

## Run focused checks

```sh
cargo test -p wreckordbox-sidecar
```
