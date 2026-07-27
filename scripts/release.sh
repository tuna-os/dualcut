#!/usr/bin/env bash
# Cut a dualcut release: bump versions everywhere, commit, tag, push.
# CI then builds dualcut.flatpak and attaches it to the GitHub Release.
#
# Usage: scripts/release.sh vX.Y.Z ["release note line"]
set -euo pipefail
V=${1:?usage: release.sh vX.Y.Z [note]}
NOTE=${2:-"See ROADMAP.md for details."}
VER=${V#v}
cd "$(dirname "$0")/.."

sed -i "s/^version = \".*\"/version = \"$VER\"/" engine/Cargo.toml
(cd engine && cargo update -q -p dualcut-engine 2>/dev/null || cargo check -q >/dev/null)

python3 - "$VER" "$NOTE" << 'PY'
import sys, datetime
ver, note = sys.argv[1], sys.argv[2]
path = 'engine/build-aux/org.tunaos.dualcut.metainfo.xml'
t = open(path).read()
if f'version="{ver}"' not in t:
    entry = (f'    <release version="{ver}" date="{datetime.date.today()}">\n'
             f'      <description>\n        <p>{note}</p>\n      </description>\n'
             f'    </release>\n')
    t = t.replace('  <releases>\n', '  <releases>\n' + entry)
    open(path, 'w').write(t)
PY

git add -A
git commit -m "release $V

$NOTE

Co-authored-by: KiKaraage <KiKaraage@users.noreply.github.com>
Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
git tag -a "$V" -m "dualcut $V — $NOTE"
git push
git push origin "$V"
echo "tagged $V — CI will attach dualcut.flatpak to the release"
