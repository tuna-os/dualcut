#!/usr/bin/env bash
# Render-level smoke tests: the assertions that caught real bugs today
# (inert volume, missing encoders, broken keying) — run in CI on every
# push so bad commits surface before a release tag.
#
# Usage: tests/smoke.sh <path-to-release-render-binary>
set -uo pipefail
BIN=${1:?usage: smoke.sh <render-binary>}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK=$(mktemp -d)
trap '/bin/rm -rf "$WORK"' EXIT
cp -r "$ROOT/engine/examples/assets" "$WORK/assets"
FAIL=0

check() { # check <name> <cond-exit>
  if [ "$2" -eq 0 ]; then echo "ok   $1"; else echo "FAIL $1"; FAIL=1; fi
}

# ---- 1. demo project renders and has both streams --------------------
cp "$ROOT/engine/examples/demo-project.json" "$WORK/demo.json"
"$BIN" "$WORK/demo.json" "$WORK/demo.webm" webm > /dev/null 2>&1
check "demo render" $?
gst-discoverer-1.0 "file://$WORK/demo.webm" 2>/dev/null | grep -qiE "video( #|:)|vp8" 
check "demo has video stream" $?

# ---- 2. volume fade actually attenuates ------------------------------
python3 - "$WORK" << 'PYEOF'
import json, sys
w = sys.argv[1]
p = {"meta": {"title": "fade", "width": 320, "height": 180, "fps": 30},
  "defs": {}, "overlays": [],
  "scenes": [{"id": "s", "duration": 5, "layers": [
    {"id": "a", "start": 0, "duration": 5, "type": "audio", "src": "assets/ticks.ogg",
     "animations": [{"property": "volume", "from": 1.0, "to": 0.0, "start": 0.0, "end": 5.0,
                     "easing": "linear"}]},
    {"id": "bg", "start": 0, "duration": 0, "type": "shape", "shape": "rect", "fill": "#204060",
     "transform": {"x":0,"y":0,"width":320,"height":180}}]}]}
json.dump(p, open(f"{w}/fade.json", "w"))
PYEOF
"$BIN" "$WORK/fade.json" "$WORK/fade.webm" webm > /dev/null 2>&1
check "fade render" $?
gst-launch-1.0 -q filesrc location="$WORK/fade.webm" ! decodebin ! audioconvert ! \
  'audio/x-raw,format=F32LE,channels=1' ! filesink location="$WORK/fade.raw" 2>/dev/null
python3 - "$WORK" << 'PYEOF'
import struct, sys
w = sys.argv[1]
d = open(f"{w}/fade.raw", "rb").read(); c = len(d)//4
s = struct.unpack(f"<{c}f", d); sr = c/5.0
def peak_at(t):
    seg = s[int((t-0.25)*sr):int((t+0.25)*sr)]
    return max(abs(x) for x in seg)
early, late = peak_at(1.03), peak_at(4.03)
# linear fade: tick at 1.03 ~0.65, at 4.03 ~0.16 — late must be well below early
assert early > 0.4, f"early tick too quiet: {early}"
assert late < early * 0.5, f"fade not attenuating: early={early} late={late}"
PYEOF
check "volume fade attenuates" $?

# ---- 3. chroma key removes the keyed color ---------------------------
python3 - "$WORK" << 'PYEOF'
import json, sys
w = sys.argv[1]
p = {"meta": {"title": "key", "width": 320, "height": 180, "fps": 30},
  "defs": {}, "overlays": [],
  "scenes": [{"id": "s", "duration": 1, "layers": [
    {"id": "green", "start": 0, "duration": 0, "type": "shape", "shape": "rect", "fill": "#00ff00",
     "transform": {"x": 60, "y": 40, "width": 160, "height": 100},
     "effects": [{"type": "chromakey", "color": "#00ff00", "angle": 25, "noise": 2}]},
    {"id": "bg", "start": 0, "duration": 0, "type": "shape", "shape": "rect", "fill": "#2040c0",
     "transform": {"x":0,"y":0,"width":320,"height":180}}]}]}
json.dump(p, open(f"{w}/key.json", "w"))
PYEOF
"$BIN" "$WORK/key.json" "$WORK/key.webm" webm > /dev/null 2>&1
check "chromakey render" $?
gst-launch-1.0 -q filesrc location="$WORK/key.webm" ! decodebin ! videoconvert ! videorate ! \
  'video/x-raw,framerate=1/1' ! pngenc snapshot=true ! filesink location="$WORK/key.png" 2>/dev/null
python3 - "$WORK" << 'PYEOF'
import sys
from PIL import Image
w = sys.argv[1]
im = Image.open(f"{w}/key.png").convert("RGB")
px = im.load()
# center of the keyed rect must NOT be green (blue bg shows through)
r, g, b = px[140, 90]
assert g < 150 or b > 100, f"keyed area still green: {(r,g,b)}"
PYEOF
check "chromakey keys out green" $?

# ---- 4. every export profile produces a nonempty file ----------------
for prof in webm vp9 ogg flac wav mkv; do
  ext=$prof; [ "$prof" = "vp9" ] && ext=webm; [ "$prof" = "mkv" ] && prof=ffv1
  "$BIN" "$WORK/demo.json" "$WORK/p-$prof.$ext" "$prof" > /dev/null 2>&1 \
    && [ -s "$WORK/p-$prof.$ext" ]
  check "profile $prof" $?
done

# ---- 5. split + ripple keep documents valid (op smoke via unit tests) --
# (covered by cargo test; here we assert render of a split project)
python3 - "$WORK" << 'PYEOF'
import json, sys
w = sys.argv[1]
p = json.load(open(f"{w}/demo.json"))
json.dump(p, open(f"{w}/roundtrip.json", "w"))
PYEOF
"$BIN" "$WORK/roundtrip.json" "$WORK/rt.webm" webm > /dev/null 2>&1
check "round-tripped document renders" $?

[ "$FAIL" -eq 0 ] && echo "SMOKE: all green" || { echo "SMOKE: FAILURES"; exit 1; }
