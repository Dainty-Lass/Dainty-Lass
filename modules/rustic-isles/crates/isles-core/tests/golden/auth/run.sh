#!/bin/sh
# Regenerates golden.json from Node's own backend (RU5 auth): the real app.ts over SQLite, under Node 22. Needs Docker
# and network for `npm ci` (kept in the named volume isles-auth-golden-work between runs). Run from anywhere.
# Re-record only when the Node side changed on purpose, and say why in the commit.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../../.." && pwd)
python "$here/make_scenario.py" > "$here/scenario.json"
python "$here/make_scenario.py" 112 > "$here/scenario112.json"
export MSYS_NO_PATHCONV=1
docker run --rm \
    -v isles-auth-golden-work:/work \
    -v "$root/reference/1.14.7/ParadoxBackend:/src:ro" \
    -v "$root/reference/1.12.0/ParadoxBackend:/src112:ro" \
    -v isles-auth-golden-work112:/work112 \
    -v "$here:/golden:ro" \
    -v "$here/../../../testdata/auth:/testdata:ro" \
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
        node --require tsx/cjs record.cjs /golden/scenario.json /testdata /out/golden.json
        # 1.12.0 (MongoDB only): its login and matchmaking routers over a stand-in persistence layer
        cd /work112
        if [ ! -d node_modules ]; then
            cp /src112/package.json /src112/package-lock.json /src112/tsconfig.json .
            npm ci --ignore-scripts --no-audit --no-fund >/dev/null 2>&1
        fi
        rm -rf src && cp -r /src112/src src
        rm -rf game-data && cp -r /src112/game-data game-data 2>/dev/null || mkdir game-data
        for f in game-data/*.example.json; do [ -e "$f" ] && cp "$f" "${f%.example.json}.json"; done
        cp /golden/record.cjs .
        node --require tsx/cjs record.cjs /golden/scenario112.json /testdata /out/golden112.json 1.12.0
    '
mv "$here/out/golden.json" "$here/golden.json"
mv "$here/out/golden112.json" "$here/golden112.json"
rmdir "$here/out" 2>/dev/null || true
