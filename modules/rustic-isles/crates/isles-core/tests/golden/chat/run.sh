#!/bin/sh
# Regenerates chat.json from Node's own routers (RU5 chat). Needs Docker and network for `npm ci`; run from anywhere.
# Re-record only when the Node side changed on purpose, and say why in the commit.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../../.." && pwd)
docker run --rm \
    -v "$root/reference/1.14.7/ParadoxBackend:/src:ro" \
    -v "$here:/golden:ro" \
    node:22 sh -c '
        set -eu
        mkdir /work && cp -r /src/package.json /src/package-lock.json /src/tsconfig.json /src/src /work/
        cd /work && npm ci --ignore-scripts --no-audit --no-fund >/dev/null 2>&1
        cp /golden/record.cjs . && node --require tsx/cjs record.cjs
    ' > "$here/chat.json.new"
mv "$here/chat.json.new" "$here/chat.json"
