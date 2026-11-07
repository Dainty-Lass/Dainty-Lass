#!/usr/bin/env bash
# Records node-<version>.golden.json from the Node reference in a node:22 container (RU5 characters-loadouts).
# From the repository root: crates/rustic-isles-mit-parity/tests/golden/characters/record.sh 1.14.7
# Needs Docker and network (npm ci). The reference tree is read-only; the container works on a copy.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../../../.." && pwd)"
for version in "$@"; do
    changelist=647472
    docker run --rm -e TARGET_CHANGELIST="$changelist" \
        -v "$root/reference/$version/ParadoxBackend:/ref:ro" -v "$here:/golden" \
        node:22 bash -c '
            set -e
            cp -r /ref /work && cd /work
            npm ci --no-audit --no-fund --ignore-scripts >/dev/null
            # No game data ships: the examples are enough for a route that reads none of it
            for f in game-data/*.example.json; do cp -n "$f" "${f%.example.json}.json"; done
            cp /golden/record.mts ./record.mts
            node --import tsx record.mts /golden/scenario.json "/golden/node-'"$version"'.golden.json"
        '
done
