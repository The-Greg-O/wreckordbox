# Releases and packaging

[Documentation](../README.md) · [Contributing](../../CONTRIBUTING.md)

`wreckordbox` is a personal fork and **publishes no public binary releases,
installers, auto-update feeds, or app-store packages**. Upstream `rbxport`'s
automated release, R2 CDN publication, Nix AppImage seeding, and Microsoft
Store Partner Center workflows have been removed from `.github/workflows/` so
this repository cannot publish to upstream's distribution channels, and the app
itself has no updater (`src-tauri/tests/it/upstream_pointers.rs`).

## Building for local installation

Build release binaries from a clean `main` checkout:

```sh
mise exec -- pnpm install --frozen-lockfile
mise exec -- pnpm build
mise exec -- cargo build --release -p rbxport -p wreckordbox -p wreckordbox-mcp
```

This produces:

- `target/release/wreckordbox-desktop` (desktop window; requires frontend `dist/` built into it via Tauri when bundled with `pnpm tauri build --no-bundle` or `pnpm tauri build`)
- `target/release/wreckordbox` (CLI and on-demand background Library service)
- `target/release/wreckordbox-mcp` (stdio MCP server for AI assistants)

For a standalone desktop binary that embeds the compiled frontend assets
(`tauri/custom-protocol`), build the shell with Tauri:

```sh
mise exec -- pnpm tauri build --no-bundle
```

## Versions and `sync-version.mjs`

Workspace version numbers live in `Cargo.toml` (`[workspace.package]`),
`package.json`, and `src-tauri/tauri.conf.json`.
`node scripts/sync-version.mjs --version X.Y.Z` updates those three files plus
workspace crate entries in `Cargo.lock` while leaving third-party dependency
versions untouched.

`pnpm dev` runs `node scripts/sync-version.mjs` with no arguments, which sets
the version from the newest `v*` tag merged into `HEAD`. Review any incidental
version-file changes before committing.

Production frontend builds (`pnpm build`) obfuscate application JavaScript
chunks (excluding `vendor`) and omit source maps, matching upstream's Vite
configuration; development and mock builds remain readable.

## Inherited Linux AppImage notes

The repository still retains two Linux AppImage workarounds from upstream that
remain tested in `scripts/` and `src-tauri/`:

### Linux AppImage GTK hook

An AppImage's `AppRun` hook exports `GTK_THEME=Adwaita:light` or
`Adwaita:dark`. `scripts/appimage-gtk-hook.mjs` installs and verifies the
pinned plugin in `packaging/appimage/linuxdeploy-plugin-gtk.sh`, which queries
the desktop color-scheme portal first. Keep `scripts/appimage-gtk-hook.test.mjs`
passing if touching `packaging/appimage/`.

### Linux AppImage blank window

Bundled `libwayland-client` inside an AppImage can conflict with rolling-release
host Mesa drivers (`EGL_BAD_PARAMETER`). `src-tauri/src/appimage_wayland.rs`
re-executes an AppImage launch once with the host's `libwayland-client` in
`LD_PRELOAD` and `DESKTOPINTEGRATION=1` unless `RBXPORT_WAYLAND_PRELOAD=1` is
set.
