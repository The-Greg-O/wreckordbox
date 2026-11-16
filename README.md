# wreckordbox

Wreckordbox helps DJs organise a rekordbox-compatible music library, prepare
tracks, and export USB Sticks for Pioneer / AlphaTheta players on Linux.
Listen, check beat grids and cues, and edit tracks in the desktop window, or
ask an assistant to prepare your Library through the CLI or MCP server.
It is a personal project maintained by one DJ and a hard fork of
[rbxport](https://github.com/chrisle/rbxport) by Chris Le.

Issues are welcome; please discuss a change in an issue before opening a pull
request. See [Contributing](CONTRIBUTING.md).

The vocabulary used across the app, CLI, and documentation is defined in
[`CONTEXT.md`](CONTEXT.md).

## How it works

- **The Library lives on the computer; a Stick is only a playable export.**
  The **Library** stays in rekordbox's own SQLCipher `master.db` and `USBANLZ`
  format ([ADR 0001](docs/adr/0001-library-stays-in-rekordbox-format.md)). No
  rekordbox file gains private tables or columns, rekordbox can still open the
  Library when wreckordbox is closed, and every **Stick** written by
  wreckordbox stays **Rekordbox-readable** on the deck and on other machines.
- **One offline Library service, three clients.**
  A background service (`wreckordbox-service`,
  [ADR 0002](docs/adr/0002-library-service-owns-the-library.md)) is the single
  writer to the Library and never accesses the network
  ([ADR 0005](docs/adr/0005-the-library-service-stays-offline.md)). The desktop
  window (`wreckordbox-desktop`), the headless `wreckordbox` CLI, and the
  `wreckordbox-mcp` stdio Model Context Protocol server are clients of that
  same service (hosted inside the window while open, or spawned on demand when
  closed). Edits made by an outside agent (such as Claude Code or Codex) appear
  live in the window; web lookups stay in the agent rather than inside the
  Library process. See [Driving wreckordbox from an agent](docs/agents/guide.md).
- **The Sidecar: Facts, Tag schema, Projection, Ledger, and Unwind.**
  Beside `master.db` sits the **Sidecar** (`wreckordbox-sidecar`), holding what
  rekordbox has no place for
  ([ADR 0004](docs/adr/0004-facts-carry-provenance-and-every-change-is-in-the-ledger.md),
  [ADR 0006](docs/adr/0006-intent-lives-in-files-and-projects-to-native-fields.md)):
  - **Facts** keyed to each track — **Judgements** (genre, mood, Situation,
    **Energy**) that follow the music across file moves and replacements, and
    **Measurements** (continuous **Intensity**, separated **Stems**) tied to a
    versioned audio stream fingerprint — each recorded with its source (`dj`,
    `agent:<name>`, `tool:<name>`, `web:<url>`) and confidence. A value set by
    the DJ's own hand outranks every other source.
  - Versioned plain-text **Tag schema** (`tag-schema.toml`) and **Recipes** for
    ordered **Set plans**. **Projection** (`wreckordbox project`) turns
    standing Facts into native rekordbox fields that a player's **Track Filter**
    understands (four MyTag categories, Energy in the star rating, and **Prep
    colour**), holding uncertain or policy-gated changes back for the DJ.
  - **Mutants**
    ([ADR 0008](docs/adr/0008-a-mutant-belongs-to-a-playlist-and-is-rendered-when-a-stick-is-loaded.md)):
    a playlist's own edited version of a track (muted Stems, shifted key,
    changed tempo, moved cues), stored as edits in the Sidecar and rendered to
    ordinary audio when a Stick is loaded; plus **Native stems** written in the
    published AZ-MODS Ogg Vorbis layout on a **Spare stick**.
  - A durable **Ledger** recording every change to the Library and Sidecar,
    grouped by **Run**, with before and after values, automatic **Large-run
    backups** before bulk runs, and out-of-order **Unwind**
    (`wreckordbox runs unwind`) that flags values changed since instead of
    overwriting them.
- **Intake, Review, Stick load, and DJ-only safeguards.**
  - **Intake** queues newly added tracks that have no Prep colour yet so title
    and artist cleanup, Tag schema chips, Energy, beat grids, and cues can be
    completed before marking a track Clean.
  - **Review** (`wreckordbox review`) presents Facts held back by
    `projection-policy.toml` for the DJ to accept, reject, or reopen.
  - **Stick load** (`wreckordbox stick dry-run` / `stick load`) enforces
    backend **Stick Protection** and requires a fresh ten-minute dry-run plan
    before syncing playlists and the five deck display-settings files onto an
    unlocked Stick, then reads the Stick back to verify what was written.
  - Decisions that belong solely to the DJ — turning **Library Protection**
    off, unlocking a Stick, accepting/rejecting waiting Facts, clearing a
    Projection barrier, discarding Ledger spills, and restoring a backup — are
    CLI-only and deliberately excluded from `wreckordbox-mcp`.

## Project status

`wreckordbox` is a **personal project** built around one working DJ's setup and
shared publicly under GPL-2.0-or-later in case parts of it are useful to others.
There are no pre-built binary releases, installers, or update feeds; build from
source. Bug reports and small fixes are welcome, though feature direction
follows the owner's workflow and [`docs/adr/`](docs/adr/); see
[Contributing](CONTRIBUTING.md).

- **Platforms.** Linux-first, developed and run on Arch Linux / Omarchy on
  `aarch64` (Apple Silicon under Asahi Linux). A native macOS (`arm64`) build
  is in progress (some crates, such as `wreckordbox-stick-protection`, target
  Linux mount semantics first). Inherited Windows and Intel macOS code from
  upstream remains in the tree and is not actively tested in this fork.
- **Desktop window.** A new four-view Omarchy-themed shell (`Library`,
  `Intake`, `Review`, `Stick`;
  [ADR 0007](docs/adr/0007-the-window-is-four-views-in-the-omarchy-look.md),
  [Window design](docs/design/README.md)) lives alongside upstream's classic
  layout behind the `view.viewShell` preference. Its behaviour is wired up, but
  its look, spacing, and density are still being refined screen by screen, so
  the classic layout remains the default for now.
- **What has been verified on real hardware.** On 2026-11-15, wreckordbox had
  its first hands-on run against a real Library and a physical Pioneer deck:
  - Rebuilt a ~330-track, ~40-playlist Library from a Stick
    (`wreckordbox library rebuild-from-stick`), carrying tracks, playlists and
    folders, star ratings, Prep colours, MyTags, hot and memory cues, beat
    grids, and the five deck settings files (`MYSETTING.DAT`, `MYSETTING2.DAT`,
    `DJMMYSETTING.DAT`, `DEVSETTING.DAT`, `djprofile.nxs`).
  - Opened that Library in `wreckordbox-desktop`, and used an AI assistant
    connected via `wreckordbox-mcp` to query tracks and build a set playlist.
  - Loaded a Stick with `wreckordbox` and **played from it on the physical
    deck** with waveform, beat grid, hot and memory cues, key, BPM, colour, and
    MyTag Track Filter all working.
- **What has not yet been verified on hardware.** Native stems on a modded
  XDJ-AZ, rendered Mutants on a deck, Set plans (`set-plan build`), lyrics,
  end-to-end Intake/Review agent workflows, and Round-trip imports after a gig
  have only been exercised against automated fixtures so far. Known rough edges
  from the first hardware test include Stick rebuilds not yet carrying cover
  artwork or play history (and missing tracks whose Stick filename was
  truncated and retagged), MyTag display ordering, and view-shell polish.

Like upstream `rbxport`, `wreckordbox` is **not** live DJ performance software:
it does not map DJ controllers or provide a live mixer/performance mode.

## Forked from rbxport

`wreckordbox` is forked from **[rbxport](https://github.com/chrisle/rbxport)**
by **Chris Le** (licensed under GPL-2.0-or-later) and has diverged by more than
1,000 commits.

- **What comes from `rbxport`:** the SQLCipher `master.db` layer (`rbl-db`),
  in-memory columnar index (`rbl-index`), ANLZ parser and writer (`rbl-anlz`),
  offline tempo/grid/key/waveform analyser (`rbl-analysis`), two-deck audio
  engine (`rbl-deck`), USB export writers (`rbl-export`, `rbl-pdb`,
  `rbl-onelibrary`), backup archive core (`rbl-backup`), PRO DJ LINK stack
  (`rbl-link`, `rbl-prolink`, `rbl-dbserver`, `rbl-nfs`), and the classic
  export-mode interface.
- **What differs in `wreckordbox`:** upstream `rbxport` intentionally keeps a
  small, focused feature set ("fewer features") across macOS, Windows, and
  Linux. `wreckordbox` restructures the application around a background Library
  service, the `wreckordbox` CLI and `wreckordbox-mcp` server for AI agents,
  the Sidecar (Facts, Tag schema, Projection, Review, durable Ledger, Unwind,
  Intensity, Stems, Native stems, Mutants, Set plans), backend Stick Protection
  and dry-run-gated Stick loads, and a Linux-first Omarchy-themed window —
  while keeping the low-level player-format crates (`rbl-pdb`,
  `rbl-onelibrary`, `rbl-export`, `rbl-anlz`, `rbl-db`) close to upstream so
  hardware-compatibility fixes carry over cleanly
  ([ADR 0003](docs/adr/0003-hard-fork-with-a-protected-core.md)).
- **Internal names:** Cargo crate names (`rbl-*` and the desktop shell's
  package `rbxport` / `rbxport_lib`), `package.json`'s `"name"`, and several
  inherited environment variables (`RBXPORT_TEST`, `RBXPORT_OPTIONS`,
  `RBXPORT_LOG_DIR`) keep upstream's names for now pending a separate rename
  migration. The built binaries (`wreckordbox-desktop`, `wreckordbox`,
  `wreckordbox-mcp`), application identifier (`com.wreckordbox.app`), and user
  config/data directories (`wreckordbox/`) already use wreckordbox's own names.

## Build from source

### 1. Toolchain and system prerequisites

Tool versions are pinned in [`mise.toml`](mise.toml) and
[`rust-toolchain.toml`](rust-toolchain.toml): **Node 24**, **pnpm 10.17.1**,
and **Rust 1.99.0** (workspace MSRV `1.89`).

With [mise](https://mise.jdx.dev/):

```sh
mise trust
mise install
```

(Without `mise`, install Node 24, run `corepack enable`, and let `rustup` read
`rust-toolchain.toml`.)

Install C/C++ build tools and Tauri 2 system libraries for your platform:

- **Arch Linux / Omarchy:**
  ```sh
  sudo pacman -S --needed base-devel perl pkgconf webkit2gtk-4.1 gtk3 libayatana-appindicator librsvg alsa-lib openssl patchelf
  ```
- **Debian / Ubuntu:**
  ```sh
  sudo apt-get install build-essential perl pkg-config libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libgtk-3-dev libasound2-dev libssl-dev
  ```
- **macOS (`arm64`, in progress):** Xcode Command Line Tools (`xcode-select --install`).

### 2. Explore the UI in browser mock mode (no Rust build)

```sh
mise exec -- pnpm install --frozen-lockfile
mise exec -- pnpm dev:mock
```

This serves the React interface against an in-memory mock backend and never
opens an installed rekordbox Library.

### 3. Build the desktop window, CLI, and MCP server

```sh
mise exec -- pnpm install --frozen-lockfile
mise exec -- pnpm build
mise exec -- cargo build --release -p rbxport -p wreckordbox -p wreckordbox-mcp
```

This produces `target/release/wreckordbox-desktop`,
`target/release/wreckordbox`, and `target/release/wreckordbox-mcp` (use
`mise exec -- pnpm tauri build --no-bundle` to embed `dist/` into a standalone
`wreckordbox-desktop` binary).

To launch the desktop app in development mode with writes to any installed
rekordbox Library refused:

```sh
RBXPORT_TEST=1 mise exec -- pnpm dev
```

For full setup, fixture-sandbox rules, and troubleshooting, read
[Getting started](docs/development/getting-started.md).

## Common commands

| Command | Purpose |
| --- | --- |
| `mise exec -- pnpm dev:mock` | Browser UI against the mock backend (no Rust or Library needed). |
| `RBXPORT_TEST=1 mise exec -- pnpm dev` | Tauri desktop app in dev mode with installed-Library writes refused. |
| `mise exec -- pnpm typecheck && mise exec -- pnpm lint` | TypeScript and ESLint checks. |
| `mise exec -- pnpm test` | Vitest unit and component tests. |
| `mise exec -- pnpm build && mise exec -- pnpm budget` | Production frontend build and start-up bundle budget check. |
| `mise exec -- pnpm e2e` | Playwright browser specs against `dist-mock/`. |
| `RBXPORT_TEST=1 mise exec -- cargo test -p <crate>` | Focused Rust crate tests (uses disposable fixtures; refuses real-Library writes). |

## Documentation

- [`CONTEXT.md`](CONTEXT.md): glossary of project terms.
- [Agent guide (`wreckordbox` & `wreckordbox-mcp`)](docs/agents/guide.md) and [Set planning](docs/agents/set-planning.md).
- [Architecture decision records (`docs/adr/`)](docs/adr/0001-library-stays-in-rekordbox-format.md).
- [Window design (`docs/design/`)](docs/design/README.md).
- [Getting started](docs/development/getting-started.md), [Architecture](docs/development/architecture.md), [Conventions](docs/development/conventions.md), [Testing](docs/development/testing.md), and [Debugging](docs/development/debugging.md).
- [Full documentation index](docs/README.md), [Rust crate guide](crates/README.md), and [Contributing](CONTRIBUTING.md).

## Licence and disclaimer

`wreckordbox` is free software licensed under **GPL-2.0-or-later**
(Copyright (c) 2026 Chris Le; Copyright (c) 2026 Greg Overton). See
[`LICENSE`](LICENSE) and [`LICENSING.md`](LICENSING.md) for bundled components
and the [fork distribution checklist](LICENSING.md#fork-distribution-checklist),
including which obligations are met and which remain unverified.

`wreckordbox` is provided "as is", without warranty of any kind, to the extent
permitted by applicable law. Use it at your own risk and keep independent
backups of your music files, rekordbox Library, Sidecar, and USB Sticks. Unless
required by applicable law or agreed to in writing, the authors and
contributors are not liable for any damages arising from using or being unable
to use `wreckordbox`, including lost or corrupted data, equipment issues, or
financial losses. See [`LICENSE`](LICENSE) for the full warranty and liability
terms.

`wreckordbox` is an independent project and is not affiliated with, endorsed
by, or sponsored by AlphaTheta Corporation, Pioneer DJ, Chris Le, or any other
third party. References to rekordbox, CDJ, XDJ, PRO DJ LINK, and other products
describe compatibility only; all trademarks belong to their respective owners.

### Upstream acknowledgements

`rbxport` by [Chris Le](https://github.com/chrisle/rbxport), and upstream's
acknowledgements: evanpurkhiser, Maddix, Morgan Page, nichi, profbx, Sean Tyas,
shiz, syl, trancejesus, xorbxbx, and AlphaTheta.
