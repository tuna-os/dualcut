#!/usr/bin/env bash
# Run heavy cargo work on the himachal build box (gtkbuild distrobox).
# Usage: scripts/remote-build.sh [cargo args...]   (default: build all features + test)
# Syncs the tree first; target/ stays remote.
set -euo pipefail
HOST=${DUALCUT_BUILD_HOST:-himachal}
cd "$(dirname "$0")/.."
rsync -az --delete --exclude target --exclude out --exclude .flatpak-builder \
  ./ "$HOST:dev/dualcut/"
ARGS=${*:-"build --features preview,scripting,vector"}
ssh "$HOST" "distrobox enter gtkbuild-f44 -- bash -c 'cd ~/dev/dualcut/engine && CARGO_TARGET_DIR=target-f44 cargo $ARGS'"
