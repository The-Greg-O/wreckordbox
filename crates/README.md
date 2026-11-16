# Rust crate guide

[Project documentation](../docs/README.md) · [Architecture](../docs/development/architecture.md)

Start with the crate that owns the behavior you want to change. None of the
crates under `crates/` depends on Tauri; desktop window coordination lives in
`src-tauri/` (package `rbxport`, binary `wreckordbox-desktop`), and frontend IPC
goes through `src/ipc/`.

Per [ADR 0003](../docs/adr/0003-hard-fork-with-a-protected-core.md), the
player-format crates (`rbl-pdb`, `rbl-onelibrary`, `rbl-export`, `rbl-anlz`,
`rbl-db`) stay close to upstream `rbxport` — add fork functionality beside them
rather than reshaping them. Existing `rbl-*` crate names keep upstream's prefix
until a separate rename migration runs.

## Library service, CLI, MCP, and Sidecar

| Crate | Responsibility |
| --- | --- |
| [wreckordbox-service](wreckordbox-service/README.md) | The background Library service, its Unix-socket client, and JSON-lines protocol. |
| [wreckordbox](wreckordbox/README.md) | The headless `wreckordbox` CLI client and service runner. |
| [wreckordbox-mcp](wreckordbox-mcp/src/lib.rs) | Stdio Model Context Protocol server wrapping safe service commands for AI assistants. |
| [wreckordbox-sidecar](wreckordbox-sidecar/README.md) | Sidecar store beside the Library: Facts, Tag schema, Decisions, Runs, Ledger, Unwind, Stems, and Mutants. |
| [wreckordbox-intensity](wreckordbox-intensity/README.md) | Continuous Intensity measurement over a track's length. |
| [wreckordbox-render](wreckordbox-render/README.md) | Offline rendering of Stems and Mutants with key shift and tempo stretch. |
| [wreckordbox-native-stems](wreckordbox-native-stems/README.md) | Writes separated Stems to a Spare stick in the published AZ-MODS Ogg Vorbis layout. |
| [wreckordbox-setplan](wreckordbox-setplan/src/lib.rs) | Builds ordered Set plans and transition notes from plain-text Recipes. |
| [wreckordbox-stick-protection](wreckordbox-stick-protection/src/lib.rs) | Backend-enforced Stick lock/unlock state and volume identity checks. |
| [wreckordbox-stick-outline](wreckordbox-stick-outline/src/lib.rs) | Reads and summarises tracks, playlists, and MyTags already on a Stick. |
| [wreckordbox-stick-write](wreckordbox-stick-write/src/lib.rs) | Shared Stick dry-run planning, plan confirmation, and load verification. |

## Library and media (`rbl-*`)

| Crate | Responsibility |
| --- | --- |
| [rbl-core](rbl-core/README.md) | Shared primitives, path helpers, and durable file publication. |
| [rbl-db](rbl-db/README.md) | SQLCipher `master.db` access, guarded writes, and test fixtures (protected core). |
| [rbl-index](rbl-index/README.md) | In-memory columnar browsing, sorting, filtering, and searching. |
| [rbl-library](rbl-library/README.md) | `LibraryCore`: loaded index, open views, undo history, Ledger hook, and single edit path. |
| [rbl-anlz](rbl-anlz/README.md) | Rekordbox ANLZ file parsing and authoring (protected core). |
| [rbl-audio](rbl-audio/README.md) | Audio decoding and export format conversion. |
| [rbl-analysis](rbl-analysis/README.md) | Offline tempo, beat grid, key, and waveform DSP. |
| [rbl-deck](rbl-deck/README.md) | Two-deck real-time playback, Rubber Band / WSOLA stretching, and scrubbing. |
| [rbl-backup](rbl-backup/README.md) | Backup archives (including Sidecar payloads) and recoverable restore journals. |
| [rbl-devices](rbl-devices/README.md) | Removable-volume discovery, inspection, and eject. |

## USB export (`rbl-*`)

| Crate | Responsibility |
| --- | --- |
| [rbl-export](rbl-export/README.md) | USB export and selective sync orchestration (protected core). |
| [rbl-pdb](rbl-pdb/README.md) | Legacy DeviceSQL `export.pdb` database (protected core). |
| [rbl-onelibrary](rbl-onelibrary/README.md) | SQLCipher `exportLibrary.db` / OneLibrary database (protected core). |

## PRO DJ LINK and diagnostics (`rbl-*`)

| Crate | Responsibility |
| --- | --- |
| [rbl-link](rbl-link/README.md) | Link Export orchestration and player state. |
| [rbl-prolink](rbl-prolink/README.md) | Announcement and status packet codecs. |
| [rbl-dbserver](rbl-dbserver/README.md) | RemoteDB browsing and session replies. |
| [rbl-nfs](rbl-nfs/README.md) | Read-only NFSv2 audio file delivery. |
| [rbl-fakecdj](rbl-fakecdj/README.md) | Loopback protocol test client (not firmware emulation). |
| [rbl-difftool](rbl-difftool/README.md) | Read-only `master.db` snapshot diff recorder. |

## First change

1. Read the owning crate's README (or `src/lib.rs`), public API, and a nearby test.
2. For database or service writes, always set `RBXPORT_TEST=1` and use `rbl_db::fixture::build` or `crates/wreckordbox/tests/common/`. Never touch an installed Library or a mounted USB Stick.
3. Run the crate's focused tests, Clippy (`-D warnings`), and `cargo fmt --check`; see [Testing](../docs/development/testing.md) and [Integration-test binaries](../docs/development/integration-tests.md).
