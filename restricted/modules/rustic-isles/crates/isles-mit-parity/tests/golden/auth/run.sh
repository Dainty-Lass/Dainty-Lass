#!/bin/sh
# Regenerates golden.json from Node's own backend (RU5 auth): the real app.ts over SQLite, under Node 22. Needs Docker
# and network for `npm ci` (kept in the named volume isles-auth-golden-work between runs). Run from anywhere.
# Re-record only when the Node side changed on purpose, and say why in the commit.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../../.." && pwd)
python "$here/make_scenario.py" > "$here/scenario.json"
export MSYS_NO_PATHCONV=1
docker run --rm \
    -v isles-auth-golden-work:/work \
    -v "$root/reference/1.14.7/ParadoxBackend:/src:ro" \
    -v "$here:/golden:ro" \
    -v "$here/out:/out" \
    node:22 sh -c '
        set -eu
        cd /work
        if [ ! -d node_modules ]; then
            cp /src/package.json /src/package-lock.json /src/tsconfig.json .
            npm ci --ignore-scripts --no-audit --no-fund >/dev/null 2>&1
        fi
        rm -rf src && cp -r /src/src src
        # The routers load the game data exports at import; the examples stand in (never a game export)
        rm -rf game-data && cp -r /src/game-data game-data
        for f in game-data/*.example.json; do [ -e "$f" ] && cp "$f" "${f%.example.json}.json"; done
        cp /golden/record.cjs .
        node --require tsx/cjs record.cjs /golden/scenario.json /out/golden.json
    '
mv "$here/out/golden.json" "$here/golden.json"
rmdir "$here/out" 2>/dev/null || true
