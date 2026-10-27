#!/usr/bin/env bash
# Records node-<version>.golden.json from the Node reference in a node:22 container (RU5 characters-loadouts).
# From the repository root: crates/isles-core/tests/golden/characters/record.sh 1.14.7 [1.12.0]
# Needs Docker and network (npm ci). The reference tree is read-only; the container works on a copy.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../../../.." && pwd)"
for version in "$@"; do
    # The 1.12.0 tag's backend ran the same app on node:sqlite too; its changelist is the table's 392819
    changelist=647472
    [ "$version" = "1.12.0" ] && changelist=392819
    mongo_env=()
    if [ "$version" = "1.12.0" ]; then
        # The tag's backend is MongoDB only, and its transactions need a replica set
        docker rm -f isles-golden-mongo >/dev/null 2>&1 || true
        docker network create isles-golden >/dev/null 2>&1 || true
        docker run -d --rm --name isles-golden-mongo --network isles-golden mongo:7 --replSet rs0 --bind_ip_all >/dev/null
        until docker exec isles-golden-mongo mongosh --quiet --eval 'rs.initiate({_id:"rs0",members:[{_id:0,host:"isles-golden-mongo:27017"}]}).ok' >/dev/null 2>&1; do sleep 1; done
        until docker exec isles-golden-mongo mongosh --quiet --eval 'db.hello().isWritablePrimary' 2>/dev/null | grep -q true; do sleep 1; done
        mongo_env=(--network isles-golden -e "MONGODB_URI=mongodb://isles-golden-mongo:27017/?replicaSet=rs0")
    fi
    docker run --rm -e TARGET_CHANGELIST="$changelist" "${mongo_env[@]}" \
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
    docker rm -f isles-golden-mongo >/dev/null 2>&1 || true
done
