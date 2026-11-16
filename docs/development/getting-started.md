# Getting started

[Documentation](../README.md) · Next: [Architecture](architecture.md)

Start in browser mode if you only want to explore or work on the React
interface: it runs against an in-memory mock backend without Rust or an
installed rekordbox Library.

## Prerequisites

Tool versions are pinned in [`mise.toml`](../../mise.toml) and
[`rust-toolchain.toml`](../../rust-toolchain.toml):

- **Node** `24`
- **pnpm** `10.17.1` (also pinned in `package.json`)
- **Rust** `1.99.0` (workspace `rust-version` MSRV is `1.89`)

With [mise](https://mise.jdx.dev/) installed, trust and install the pinned
toolchain from the repository root:

```sh
mise trust
mise install
```

Then prefix commands with `mise exec -- <command>` (or activate mise in your
shell). Without mise, install Node 24, enable Corepack (`corepack enable`), and
let `rustup` use `rust-toolchain.toml`.

Building the Rust workspace and desktop window also requires a C/C++ build
toolchain (SQLCipher, vendored OpenSSL, Rubber Band, libvorbis/libogg) and
platform libraries for Tauri 2:

| Platform | Status & system prerequisites |
| --- | --- |
| Linux (`aarch64` / `x86_64`) | Primary platform (developed on Arch Linux / Omarchy, `aarch64`). Needs WebKitGTK 4.1, GTK 3, AppIndicator, librsvg, ALSA, OpenSSL, Perl, `pkg-config`, and `patchelf`. |
| macOS (`arm64`) | Build in progress. Needs Xcode Command Line Tools (`xcode-select --install`) and the system WebKit framework. Note: Linux-specific crates such as `wreckordbox-stick-protection` target Linux mount semantics first. |
| Windows | Inherited upstream code remains in the tree, not actively targeted by this fork. Needs MSVC build tools, Windows SDK, WebView2, and Strawberry Perl (`OPENSSL_SRC_PERL`). |

On Arch Linux / Omarchy:

```sh
sudo pacman -S --needed base-devel perl pkgconf webkit2gtk-4.1 gtk3 libayatana-appindicator librsvg alsa-lib openssl patchelf
```

On Debian / Ubuntu:

```sh
sudo apt-get install build-essential perl pkg-config libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libgtk-3-dev libasound2-dev libssl-dev
```

## Install and run the browser UI (mock backend)

```sh
mise exec -- pnpm install --frozen-lockfile
mise exec -- pnpm dev:mock
```

(`pnpm dev:web` also selects the mock backend outside Tauri.) Open the local URL
printed by Vite (`http://localhost:1420`). Mock mode verifies frontend behavior
only and never opens a rekordbox Library.

## Build the Rust binaries

The workspace produces three main binaries (internal Cargo package names keep
upstream's `rbxport` / `rbl-*` naming for now; see
[Contributing](../../CONTRIBUTING.md#naming-in-the-codebase)):

```sh
mise exec -- cargo build --release -p rbxport -p wreckordbox -p wreckordbox-mcp
```

| Binary | Package | Role |
| --- | --- | --- |
| `target/release/wreckordbox-desktop` | `rbxport` (`src-tauri/`) | The Tauri 2 desktop window (hosts the Library service while open). |
| `target/release/wreckordbox` | `wreckordbox` (`crates/wreckordbox/`) | The headless CLI client (and background service runner via `serve`). |
| `target/release/wreckordbox-mcp` | `wreckordbox-mcp` (`crates/wreckordbox-mcp/`) | Stdio MCP server exposing safe Library/Sidecar tools to an AI assistant. See the [agent guide](../agents/guide.md). |

## Run the desktop app safely

`pnpm dev` builds and opens the Tauri desktop window, which looks for a
rekordbox Library in rekordbox's standard settings path. During development,
always refuse writes to any installed Library:

```sh
RBXPORT_TEST=1 mise exec -- pnpm dev
```

(`RB_LITE_TEST=1` is still accepted as upstream's legacy alias; `RBXPORT_*`
environment variables keep upstream's prefix for now.)

Neither variable supplies a test Library — they refuse writes to an installed
one. Writable tests build a disposable Library in a temporary folder with
`rbl_db::fixture::build` or the CLI test sandbox. Never test edits, analysis,
imports, restores, or Stick loads against real user data or a mounted USB Stick.

Note: `pnpm dev` runs `node scripts/sync-version.mjs` first, which reads the
newest merged `v*` tag in git history. If you cloned with `--depth 1`, run
`git fetch --tags` first, and review any incidental version-file diffs before
committing; see [Releases](releases.md).

## First verification

```sh
mise exec -- pnpm typecheck
mise exec -- pnpm lint
mise exec -- pnpm test
mise exec -- pnpm build
mise exec -- pnpm budget
```

For browser interaction tests against the mock bundle:

```sh
mise exec -- pnpm exec playwright install chromium
mise exec -- pnpm e2e
```

For Rust changes, start with the touched crate:

```sh
RBXPORT_TEST=1 mise exec -- cargo test -p <crate>
mise exec -- cargo clippy -p <crate> --all-targets -- -D warnings
mise exec -- cargo fmt -p <crate> --check
```

See [Testing](testing.md) and [Integration-test binaries](integration-tests.md)
for full check commands and process-isolated suites.

## First change

Read [Architecture](architecture.md) and [`CONTEXT.md`](../../CONTEXT.md) to
locate the owning layer and terminology, then follow
[Contributing](../../CONTRIBUTING.md).

## Forks and licensing

`wreckordbox` is a public fork of [`github.com/chrisle/rbxport`](https://github.com/chrisle/rbxport)
and is licensed as a whole under GPL-2.0-or-later. Any publicly distributed
binary or modified derivative must remain under an applicable GNU GPL version
and include the required copyright notices, licence text, and corresponding
source. Private use and private modifications do not require publication. See
[`LICENSE`](../../LICENSE) and [`LICENSING.md`](../../LICENSING.md) before
distributing source or binaries.
