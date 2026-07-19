#!/usr/bin/env bash
# Regenerate the user-guide screenshots by driving the app's walkthrough
# mode (DUALCUT_WALKTHROUGH) under Xvfb and grabbing a frame per
# "WALKTHROUGH-SHOT <name>" marker. Output: docs/guide/<name>.png
#
# Usage: scripts/walkthrough.sh <path-to-preview-binary>
set -euo pipefail
BIN=${1:?usage: walkthrough.sh <preview-binary>}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT="$ROOT/docs/guide"
mkdir -p "$OUT"

# Reuse an existing X display via WALKTHROUGH_DISPLAY, else start Xvfb.
DISPLAY_NUM=${WALKTHROUGH_DISPLAY:-:87}
if ! DISPLAY=$DISPLAY_NUM xset q > /dev/null 2>&1; then
  nohup Xvfb "$DISPLAY_NUM" -screen 0 1280x800x24 > /dev/null 2>&1 &
  XVFB_PID=$!
  trap 'kill $XVFB_PID 2>/dev/null || true' EXIT
  sleep 2
fi

# Work on a disposable copy of the demo project so commits are harmless.
WORK=$(mktemp -d)
cp -r "$ROOT/engine/examples/." "$WORK/"

capture() { # capture <name>
  sleep 0.4
  DISPLAY=$DISPLAY_NUM xwd -root -silent > /tmp/wt-frame.xwd || true
  [ -s /tmp/wt-frame.xwd ] || { echo "empty frame for $1" >&2; return 0; }
  python3 - "$OUT/$1.png" /tmp/wt-frame.xwd << 'EOF'
import struct, sys
d = open(sys.argv[2], "rb").read()
hdr = struct.unpack(">25I", d[:100])
hsize, w, h, bpl = hdr[0], hdr[4], hdr[5], hdr[12]
off = hsize + hdr[19] * 12
from PIL import Image
Image.frombytes("RGB", (w, h), d[off:off + bpl * h], "raw", "BGRX", bpl).save(sys.argv[1])
EOF
  echo "captured $1"
}

run_pass() { # run_pass [project-file]
  GDK_BACKEND=x11 WAYLAND_DISPLAY= DISPLAY=$DISPLAY_NUM \
    DBUS_SESSION_BUS_ADDRESS="unix:path=/nonexistent" \
    DUALCUT_API_PORT=0 DUALCUT_WALKTHROUGH=1 "$BIN" "$@" 2>/dev/null |
    while read -r line; do
      case "$line" in
        "WALKTHROUGH-SHOT "*) capture "${line#WALKTHROUGH-SHOT }" ;;
      esac
    done
}

run_pass                          # untitled: new-project
run_pass "$WORK/demo-project.json" # full tour
rm -rf "$WORK"
ls -la "$OUT"
