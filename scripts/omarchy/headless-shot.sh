#!/bin/bash
# Screenshot the debug build without touching the desktop: runs a second
# instance inside a headless `cage` compositor and captures it with `grim`.
# Works while the screen is off or locked. Needs `cage`, `wlr-randr`, `grim`,
# and for a dev build the Vite server from `pnpm dev` on localhost:1420.
#
#   scripts/omarchy/headless-shot.sh [out.png] [WIDTHxHEIGHT] [settle-seconds]
set -euo pipefail

out=${1:-/tmp/rbxport-headless.png}
size=${2:-1800x1130}
settle=${3:-5}
root=$(cd "$(dirname "$0")/../.." && pwd)
bin=${RBXPORT_BIN:-$root/target/debug/rbxport}
runtime=${XDG_RUNTIME_DIR:?}

before=$(ls "$runtime" | grep -E '^wayland-[0-9]+$' || true)
WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman \
  cage -- "$bin" >"${out%.png}.log" 2>&1 &
cage_pid=$!
trap 'kill "$cage_pid" 2>/dev/null || true' EXIT

socket=
for _ in $(seq 1 50); do
  socket=$(ls "$runtime" | grep -E '^wayland-[0-9]+$' | grep -vxF "$before" | head -1 || true)
  [[ -n $socket ]] && break
  sleep 0.2
done
[[ -n $socket ]] || { echo "cage did not start; see ${out%.png}.log" >&2; exit 1; }

export WAYLAND_DISPLAY=$socket
sleep 1
wlr-randr --output HEADLESS-1 --custom-mode "$size"
sleep "$settle"
grim "$out"
echo "$out"
