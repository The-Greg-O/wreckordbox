# FAQ

[Documentation](../README.md)

## What is wreckordbox for?

`wreckordbox` prepares a rekordbox-compatible DJ Library on your computer — from
its desktop window, the `wreckordbox` CLI, or an AI assistant connected through
`wreckordbox-mcp` — and loads playable USB Sticks (and PRO DJ LINK exports) for
Pioneer / AlphaTheta players. It is not live DJ performance software; see the
[README](../../README.md) for its scope.

## Can I register or map a DJ controller?

No. Like upstream `rbxport`, `wreckordbox` does not register DJ controllers, map
their buttons, pads, or knobs, and has no performance mode (including models
such as the Pioneer DDJ-FLX4, DDJ-FLX8, and DDJ-FLX10). Use rekordbox or your
controller's own software for controller performance mode.

## Can I manage my Library in wreckordbox and also open it in rekordbox?

Yes, one at a time. `wreckordbox` edits the Library in rekordbox's own format
(`master.db` and `share` folder named by `masterDbDirectory` in rekordbox's
settings) and keeps all wreckordbox-only data (Facts, Tag schema, Runs, Ledger,
Stems, Mutants) in a separate `wreckordbox-sidecar/` folder beside it
([ADR 0001](../adr/0001-library-stays-in-rekordbox-format.md)), so rekordbox can
open the same Library without conversion.

Always use them one after the other: `wreckordbox` refuses to write while
rekordbox is running. Quit rekordbox before editing in `wreckordbox`, and quit
`wreckordbox` (and let any background `wreckordbox` service stop, or run
`wreckordbox stop`) before opening rekordbox. See
[Library location](../reference/library-location.md).
