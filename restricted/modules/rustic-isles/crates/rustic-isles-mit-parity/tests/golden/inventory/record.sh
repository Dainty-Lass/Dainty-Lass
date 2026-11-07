#!/bin/sh
# Runs record.cjs against a copy of the reference backend, inside a node:22 container (see record.cjs).
set -e
golden=/repo/crates/rustic-isles-mit-parity/tests/golden/inventory
cp -r /repo/reference/1.14.7/ParadoxBackend /app
cd /app
npm ci --ignore-scripts --no-audit --no-fund >/dev/null 2>&1
mkdir game-data-golden
for f in game-data/*.example.json; do cp "$f" "game-data-golden/$(basename "$f" .example.json).json"; done
cp $golden/game-data/*.json game-data-golden/
cp $golden/scenarios.json $golden/record.cjs .
node --import tsx record.cjs /out/expected.json 2>/dev/null || node --import tsx record.cjs /out/expected.json
