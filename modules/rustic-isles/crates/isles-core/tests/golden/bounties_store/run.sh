#!/bin/sh
# Regenerates 1.14.7.json and 1.12.0.json from Node's own routers (RU5 bounties-store). Needs Docker and network for
# `npm ci`; run from anywhere. Re-record only when the Node side changed on purpose, and say why in the commit.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../../.." && pwd)
for version in 1.14.7 1.12.0; do
    docker run --rm \
        -v "$root/reference/$version/ParadoxBackend:/src:ro" \
        -v "$here:/golden:ro" \
        node:22 sh -c '
            set -eu
            mkdir /work && cp -r /src/package.json /src/package-lock.json /src/tsconfig.json /src/src /work/
            cd /work && npm ci --ignore-scripts --no-audit --no-fund >/dev/null 2>&1
            cp /golden/record.cjs . && node --require tsx/cjs record.cjs '"$version"'
        ' > "$here/$version.json.new"
    mv "$here/$version.json.new" "$here/$version.json"
done
