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
- The Tag schema (ADR 0006) is `tag-schema.toml` in the same folder, also the DJ's to edit and version: a genre tree in which a child implies its parent, any number of other dimensions, and the deck view that says which dimensions are the four MyTag categories and how they are listed. [`src/tag_schema.rs`](src/tag_schema.rs) describes the format and checks it. Each dimension is a kind of Fact with the same name whose values are its terms' stable identifiers; in the tree a Fact names the most specific terms only. A schema with a mistake is refused whole, with each mistake's line.
- `wreckordbox schema convert` writes a first Tag schema from the Library's MyTags without changing the Library. A parent's tag put by hand on a track that also carries one of its children is not recorded as a Fact, because the child implies it; whether the deck shows such parents is the schema's `tag_parents`, set from what most tracks have.
- A Fact row is written once. Afterwards it is only ever marked stale (a Measurement whose audio changed) or superseded (the same source recorded a newer Fact of the same kind for the track). Nothing is deleted, so a Ledger can be added over what is stored.
- A Decision (a question, its context and its fixed candidates) and each answer a Decider gives it are rows of their own tables, written once and never changed, deleted or replaced; triggers in the store refuse all three. A Decision with no answer is pending. Answering again, by the same Decider or another, adds an answer beside the earlier ones.
- An answer's choice must be one of its Decision's candidates. How sure the Decider is, is stored as `choice_probability` (the probability it gives its choice of being right) and, when it supplies them, the probability of every candidate. Neither is called a confidence. The probabilities given, with the choice counted once, never add up to more than 1.
- An answer that produces a Fact is written with it in one transaction (`Sidecar::answer_decision_with_fact`): the answer, the Fact, what the Fact supersedes and the Run made for them, all or none.
- The store's shape has a version (`meta.schema_version`). An older store is brought up to date when opened, in one transaction; a newer one is refused.
- A Stem set is one file each for vocals, other, bass and drums, and every one must be exactly as long in samples as its parent. The length is read from each file; a set that breaks the rule is refused and nothing is recorded.
- Stem audio is kept as the tool wrote it (16-bit PCM WAV from the separation engine). Conversion for a Stick happens when the Stick is written.
- The fingerprint hashes the encoded audio stream inside the file, not the file: a move, a rename or a retag changes nothing, and any change to the audio does.

## Stale Measurements and re-attachment

[`src/attachments.rs`](src/attachments.rs) keeps the latest observed audio identity and the history of Fact attachments. Schema version 4 adds this after Mutants (2) and Decisions (3). The original Fact rows, identifiers, sources, confidence and Runs stay intact; reads resolve their current track through the attachment history. Stale marks are permanent, including after re-attachment.

`Sidecar::observe_audio` takes identifying fields, the current fingerprint when available, and a callback over a captured Library snapshot. It re-attaches only from a removed track, only to a track without Facts, and only on one unambiguous fingerprint match. Several fingerprint matches are held for review. Without an exact match, title, artist, mix name and duration can suggest review, never automatic attachment. Title and mix qualifiers are preserved. An existing track's Facts are never borrowed by a duplicate still in the Library.

The service calls this path for ordinary Fact reads and recording, and for Stems reads, separation and Native stems export. `facts.of_track` checks audio on every read: `facts` contains standing Facts, `stale` reports outdated Measurements, `unverified` reports Measurements whose audio cannot be read, and `reattached`/`review` report attachment results. `all: true` includes the full Fact history in `facts`; it is not a source of standing Measurements. `check_audio` remains accepted for compatibility. The CLI shows these reports as text too.

The store's `facts` and `facts_of_track` methods remain history readers. New consumers must verify audio and exclude stale, superseded and unverified Measurements before using them. New producers should capture identity through the service's observation path; raw recording cannot discover identifying fields from the Library. Detection happens when a command observes the track, not through a background filesystem watcher. Each check reads the audio once outside the Library lock.

Migration seeds identities from legacy fingerprints unless a later stale mark makes that evidence unsafe. Old identifying fields cannot be reconstructed after removal; future observations capture them. Missing audio preserves the last known identity but cannot prove a new match. Review candidates are persisted and recalculated on observation; this ticket does not add a command to accept an uncertain attachment.

## Run focused checks

```sh
cargo test -p wreckordbox-sidecar
```
