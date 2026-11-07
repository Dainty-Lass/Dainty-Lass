#!/bin/sh
# Records scenarios-1.14.7.json from the reference Node services (see capture.ts). Run in a node:22 container with the
# repository mounted at /repo:
#   docker run --rm -v "$PWD:/repo" node:22 sh /repo/crates/rustic-isles-mit-parity/tests/golden/progression/record.sh
set -e
cp -r /repo/reference/1.14.7/ParadoxBackend /work
cd /work
npm ci --ignore-scripts --no-audit --no-fund
cp /repo/crates/rustic-isles-mit-parity/tests/golden/progression/capture.ts src/goldenCapture.ts
GOLDEN_FIXTURE_DIR=/repo/crates/rustic-isles-mit-parity/tests/golden/progression node --import tsx src/goldenCapture.ts
