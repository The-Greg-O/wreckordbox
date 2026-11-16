# Nix

The repository retains a Nix flake inherited from upstream `rbxport` and
adjusted to build from source only (upstream's pre-built AppImage wrapper has
been removed, as `wreckordbox` publishes no binary releases).

> **Status note:** `wreckordbox` is primarily developed using `mise.toml` on
> Arch Linux / Omarchy (`aarch64`). When `Cargo.lock` or `pnpm-lock.yaml`
> changes, the fixed-output hashes in `packaging/nix/pin.json` must be refreshed
> with `bash packaging/nix/update-source-hashes.sh` before `nix build` will
> succeed. Note also that `packaging/nix/source.nix` currently builds only the
> desktop package (`-p rbxport`, binary `wreckordbox-desktop`); build the CLI
> (`wreckordbox`) and MCP server (`wreckordbox-mcp`) with Cargo directly or in
> `nix develop`.

## Development shell

Inside a checkout, `nix develop` opens a shell with Rust, Node 24, `pnpm`, and
the Linux Tauri system libraries:

```sh
nix develop
pnpm install --frozen-lockfile
RBXPORT_TEST=1 pnpm dev
```

## Build or run from source

From a local checkout (after refreshing `packaging/nix/pin.json` if lockfiles
have moved):

```sh
nix run .#source
```

Or directly from GitHub:

```sh
nix run github:The-Greg-O/wreckordbox
```
