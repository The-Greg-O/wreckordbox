# Documentation

Start with the [project README](../README.md) and the glossary in
[`CONTEXT.md`](../CONTEXT.md). Commands in these guides run from the repository
root unless stated otherwise.

## Agents and architecture decisions

- [Driving wreckordbox from an agent](agents/guide.md): every `wreckordbox` CLI command, the `wreckordbox-mcp` server, recipes, and which decisions belong solely to the DJ.
- [Set planning guide](agents/set-planning.md): Recipes, transition rules, and building a Set plan.
- [Architecture decision records](adr/0001-library-stays-in-rekordbox-format.md): `docs/adr/0001` through `0008` — rekordbox-format Library + Sidecar, background Library service, protected player-format core, provenance/Ledger/Unwind, offline service, file-based intent & Projection, four-view Omarchy shell, and playlist-owned Mutants.
- [Window design](design/README.md): principles, requirements, tokens, and screen mock-ups for the four-view shell.

## Development

| Guide | Use it when |
| --- | --- |
| [Getting started](development/getting-started.md) | Setting up `mise`, prerequisites, mock mode, and safe desktop builds. |
| [Architecture](development/architecture.md) | Finding the owning crate (`wreckordbox*` or `rbl-*`) or tracing an edit. |
| [Conventions](development/conventions.md) | Following Rust, TypeScript, ADR 0003, token, and privacy rules. |
| [Product decisions](development/product-decisions.md) | Understanding wreckordbox's scope and how it differs from upstream `rbxport`. |
| [Rekordbox parity evidence](development/rekordbox-parity-decisions.md) | Establishing player-format compatibility evidence (`[OBS]`, `[ASSUME]`, `[UNKNOWN]`). |
| [Testing](development/testing.md) | Choosing test seams and running checks safely (`RBXPORT_TEST=1`). |
| [Integration-test binaries](development/integration-tests.md) | Running consolidated `it` modules and process-isolated test targets. |
| [Test tiers](development/test-tiers.md) | Understanding CI lane selection (`scripts/ci/validation-plan.mjs`). |
| [Ledger write paths](development/ledger-write-paths.md) | Auditing how every Library write records in the Sidecar's Ledger. |
| [Ledger failure paths](development/ledger-failure-paths.md) | Understanding Ledger spill recovery, replay, and discard rules. |
| [Key-writer evidence carry](development/key-evidence.md) | Understanding how verified audio key-tag rewrites carry Measurements. |
| [Analysis stages](development/analysis-stages.md) | Registering compiled analysis extensions in `rbl-analysis::stages`. |
| [Companion waveform](development/companion-waveform.md) | Reading three-band waveform overviews via automation. |
| [Debugging](development/debugging.md) | Inspecting logs, service state, environment variables, and cleanup. |
| [Releases and packaging](development/releases.md) | Local release builds, version sync, and Linux AppImage notes. |
| [Contributing](../CONTRIBUTING.md) | Reporting a bug or preparing a branch against `main`. |

## Using the app and CLI

- [Loading a Stick from the command line](user/stick-load.md): `stick list`, `stick dry-run`, `stick load`, Stick Protection, and deck settings.
- [Projection: Energy, Prep colour, and MyTags](user/projection.md): `projection-policy.toml`, Large-run backups, barriers, and `tag-schema.toml`.
- [Review: Facts waiting for you](user/review.md): inspecting, accepting, rejecting, and reopening held-back Facts.
- [Audio fingerprints and Measurements](user/fingerprints.md): how Stems and Intensity follow audio across moves and tag edits.
- [Backups](user/backups.md): Library + Sidecar archives, optional Stem audio, and `wreckordbox backup restore`.
- [Theme file](user/theme.md): customising colours, waveform themes, and fonts over Omarchy (`~/.config/wreckordbox/theme.toml`).
- [USB export preferences](user/usb-export.md): Sync Manager, Round-trip imports, compatibility conversion, and Stick Protection.
- [Track analysis settings](user/analysis-settings.md): BPM/grid modes, key preservation, and audio key-tag writing.
- [Waveform scrubbing](user/waveform-scrubbing.md): drag behaviour and its low-pass filter.
- [AppleScript](user/applescript.md): macOS scripting dictionary and commands.
- [Nix](user/nix.md): `nix develop` shell and source builds.
- [FAQ](user/faq.md): scope, rekordbox coexistence, and why DJ controllers are not mapped.

## Technical references

- [Standing Facts](reference/standing-facts.md): how `facts standing` ranks the DJ, source rules, and confidence.
- [Library location](reference/library-location.md): how `master.db` is discovered and switched.
- [USB export format](reference/usb-export-db.md): `export.pdb`, `exportLibrary.db`, and ANLZ record layouts.
- [USB export pipeline](reference/usb-export-pipeline.md): staging, verification, and recovery during USB sync.
- [Library backups](reference/backups.md): archive layout, parallel zipping, and restore journals.
- [Keyboard commands](reference/shortcut-commands.md): shared command registry and key bindings.
- [Auto Relocate search folders](reference/auto-relocate-search-folders.md): research notes comparing rekordbox Auto Relocate.
- [Library Undo coverage](reference/library-undo.md): which window actions Edit › Undo reverses.
- [LINK behavior and hardware coverage](reference/link-testing.md): PRO DJ LINK dialects and protocol checks.
- [Waveform calibration](reference/waveform-analysis.md): `PWV6` three-band overview measurement.
- [Crate guide](../crates/README.md), [Analysis crate](../crates/rbl-analysis/README.md), [Playback crate](../crates/rbl-deck/README.md), and [Separation engine](../separation-engine/README.md).
- [Licensing](../LICENSING.md): GPL-2.0-or-later terms and third-party notices.

## Inherited brand assets

`assets/brand/` contains upstream `rbxport`'s historical artwork and icons;
`wreckordbox` does not use upstream's logo as its own identity.
