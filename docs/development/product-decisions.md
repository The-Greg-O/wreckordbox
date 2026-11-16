# Product decisions

[`CONTEXT.md`](../../CONTEXT.md) · [ADRs](../adr/) · [Window design](../design/README.md)

## What wreckordbox is for

`wreckordbox` is a personal, Linux-first hard fork of
[rbxport](https://github.com/chrisle/rbxport) built for preparing a DJ's music
collection on a computer and carrying playable USB Sticks to Pioneer /
AlphaTheta decks:

- **The Library lives on the computer** in rekordbox's own format
  ([ADR 0001](../adr/0001-library-stays-in-rekordbox-format.md)); **a Stick is
  only a playable export for the deck**, kept Rekordbox-readable without custom
  database tables.
- **One offline Library service (`wreckordbox-service`) owns the Library**
  ([ADR 0002](../adr/0002-library-service-owns-the-library.md),
  [ADR 0005](../adr/0005-the-library-service-stays-offline.md)), with the
  desktop window (`wreckordbox-desktop`), the `wreckordbox` CLI, and the
  `wreckordbox-mcp` MCP server as equal clients. The goal is to let the DJ ask
  an outside AI assistant to do what would otherwise be done by hand, while the
  window is for looking, listening, beat-grid/cue editing, and hand work.
- **What rekordbox cannot hold lives in the Sidecar** (`wreckordbox-sidecar`):
  provenance-tracked Facts, Judgements and audio-fingerprinted Measurements
  (Intensity, Stems), plain-text Tag schemas and Set-plan Recipes that project
  into native rekordbox fields ([ADR 0006](../adr/0006-intent-lives-in-files-and-projects-to-native-fields.md)),
  playlist-owned Mutants rendered at Stick load time
  ([ADR 0008](../adr/0008-a-mutant-belongs-to-a-playlist-and-is-rendered-when-a-stick-is-loaded.md)),
  and a durable Ledger of Runs that can be unwound out of order
  ([ADR 0004](../adr/0004-facts-carry-provenance-and-every-change-is-in-the-ledger.md)).
- **The window is organised around the DJ's jobs** — Library, Intake, Review,
  and Stick — styled for Omarchy / tiling Linux desktops rather than cloning
  rekordbox pixel for pixel ([ADR 0007](../adr/0007-the-window-is-four-views-in-the-omarchy-look.md)).

## How this differs from upstream `rbxport`

Upstream `rbxport` deliberately keeps its feature set small ("fewer features")
so it stays a fast, minimal export-mode clone of rekordbox, while inviting forks
to pursue broader DJ workflows. `wreckordbox` takes up that invitation as a
personal hard fork while keeping the low-level player-format crates (`rbl-pdb`,
`rbl-onelibrary`, `rbl-export`, `rbl-anlz`, `rbl-db`) close to upstream so
hardware-compatibility fixes remain easy to carry across
([ADR 0003](../adr/0003-hard-fork-with-a-protected-core.md)).

Like upstream, `wreckordbox` is **not** live DJ performance software: it does
not map DJ controllers or replace a mixer/deck performance mode.
