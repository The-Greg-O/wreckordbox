# Development conventions

[Documentation](../README.md) · Next: [Testing](testing.md)

Follow the neighboring code's layout, naming, and glossary terms from
[`CONTEXT.md`](../../CONTEXT.md). Enforced lint and compiler rules live in
`Cargo.toml`, `eslint.config.js`, and `tsconfig.json`; keep diffs focused rather
than reformatting unrelated files.

## Rust

- Keep Tauri commands and window integration in `src-tauri/`. Crates under `crates/` (`rbl-*` and `wreckordbox*`) remain independent of Tauri.
- Respect [ADR 0003](../adr/0003-hard-fork-with-a-protected-core.md): do not reshape `rbl-pdb`, `rbl-onelibrary`, `rbl-export`, `rbl-anlz`, or `rbl-db`. Put fork additions in modules or `wreckordbox-*` crates beside them so player-compatibility fixes stay clean.
- Do not rename existing `rbl-*` / `rbxport` crate names or `RBXPORT_*` environment variables in unrelated changes; that migration is tracked separately.
- Return contextual errors. Workspace linting forbids `unwrap`, `expect`, and `panic!` outside tests unless a module explicitly opts out; never add an exemption to paper over a runtime failure.
- No panic may cross the IPC or service socket boundary.
- Avoid holding locks across `await`. Workspace Clippy denies `perf` lints, enables `pedantic`, and rejects unnecessary ownership, cloning, large enum variants, debug macros, and stdout printing (`-D warnings`).
- Route every Library mutation through `rbl-library` (`LibraryCore`) / `wreckordbox-service` so it honours Library Protection, records in the Sidecar's Ledger, and notifies open views. Writing to the Sidecar alone is not a Library edit.
- Preserve durable staging, fsync, and recovery behavior when touching files. A write to a staging path is not proof that publication succeeded.
- Run `cargo fmt -p <crate> --check` on crates you touch.

## CLI and MCP surface

- Every new `wreckordbox` CLI subcommand must have a matching heading in [`docs/agents/guide.md`](../agents/guide.md) (`crates/wreckordbox/tests/it/agent_guide.rs` checks `--help` against the guide).
- Decisions that belong solely to the DJ (turning Library Protection off, unlocking a Stick, accepting/rejecting/reopening Review items, acknowledging a Projection barrier, discarding Ledger spills, restoring a backup) remain CLI-only and must never be exposed as MCP tools on `wreckordbox-mcp`.
- Keep the Library service completely offline ([ADR 0005](../adr/0005-the-library-service-stays-offline.md)).

## TypeScript and React

- Use strict types (`tsconfig.json` enables `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, and unused-symbol checks).
- Put all backend calls in `src/ipc/`. Views call the typed `Backend` interface and never import Tauri's `invoke` directly.
- Extend `src/ipc/backend-mock.ts` alongside real IPC additions so `pnpm dev:mock` and Playwright tests stay truthful.
- Colour view-shell components only through role tokens (`--role-*` in `src/styles/tokens.css`), never hard-coded greys, so every skin and Omarchy theme renders cleanly. Meaningful musical colours (waveforms, cues, phrases, Camelot lights, Energy, Prep colours) use their dedicated tokens.
- Sorting, filtering, and searching happen in `rbl-index` / `wreckordbox-service`; render windowed rows rather than materialising the collection in React.
- Use events and observers rather than `setInterval` polling.
- Register keyboard actions through `useCommands` (`src/lib/commands.ts`); see [Keyboard commands](../reference/shortcut-commands.md).

## User-facing strings and generated files

Use `useTranslation()` from `src/i18n` for translated strings in shared views.
When adding keys to `src/i18n/ui.json`, run `node scripts/fill-missing-locales.mjs`
if needed so `public/locales/*.json` keep complete English fallbacks, and commit
generated files with their source change.

## Performance

`perf-budgets.json` defines enforced start-up bundle and frame gates.
`pnpm budget` checks the budget schema and start-up bundle size (345 KB gzipped
limit across initial chunks). `pnpm perf:gate` runs browser frame checks;
`pnpm perf:measure` measures the production preview. Never raise a budget gate
to make a regression pass.

## Formats, evidence, and privacy

Keep `[OBS]`, `[ASSUME]`, and `[UNKNOWN]` tags accurate when working near player
formats or protocols. Distinguish static diffs, loopback tests (`rbl-fakecdj`),
and physical-player verification.

**Nothing private belongs in this public repository:** never commit real audio,
model weights, screenshots, logs, personal filesystem paths, machine hostnames,
or real track, playlist, or tag names.

## Tests and comments

Test observable behavior at the project's seams: CLI commands against a service
on a disposable fixture Library, hosted shell integration modules, Vitest unit
tests, and mock-backed Playwright browser specs. Comments should explain
constraints, format evidence, or non-obvious invariants rather than restating
code.
