# rbl-library

[Crate guide](../README.md) · [Architecture](../../docs/development/architecture.md)

The Library core: the loaded index, its open views, the change generation,
and the single path every Library edit takes. It has no dependency on the
desktop framework, so the window's shell and the Library service
(ADR 0002) can both hold one.

## Reading path

1. `src/lib.rs`: `LibraryCore`, the `Host` a caller supplies, and `Touched`.
2. `tests/edit_path.rs`: the edit path at its public interface, against a
   throwaway Library.

## The edit path

`LibraryCore::edit` (or `write_then` with a refresh of the caller's own):

1. takes the edit gate, so edits are serial;
2. refuses while a Library restore is unfinished;
3. asks the `Host` to recover an interrupted analysis-file edit, unless one
   is being published right now;
4. opens a guarded `rbl_db::write::Writer`, which refuses while rekordbox is
   running and, under `RBXPORT_TEST`, for a detected real Library;
5. runs the edit, then re-reads only what it `Touched` on the same
   connection and publishes the new index;
6. returns a `Change`: what kind of data was touched and the generation.

Telling anyone (the window's `library:changed`, a service event) is the
caller's job, from the `Change` returned.

## Checks

```sh
RBXPORT_TEST=1 cargo test -p rbl-library
cargo clippy -p rbl-library --all-targets
```
