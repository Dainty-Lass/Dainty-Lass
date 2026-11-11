#!/usr/bin/env bash
# The full Docker run (CI's Docker job), from the folder holding compose.yaml: the images, every suite in the test
# image, then the smoke, network and remote harnesses. Arguments pick the in-image suites.
#   SKIP_SMOKE=1, SKIP_NETWORK=1, SKIP_REMOTE=1 leave a harness out.
set -euo pipefail
cd "$(dirname "$0")/.."
# Taken from the environment when set, so the oracle can point these scripts at another file (D-8)
export COMPOSE_FILE=${COMPOSE_FILE:-compose.yaml}

step() { printf '\n== %s\n' "$*"; }

step "build the images"
docker compose build
docker compose --profile test build test

step "the suites in the test image"
docker run --rm --cap-add NET_ADMIN "${DAINTY_IMAGE:-dainty-lass}-test:${DAINTY_TAG:-local}" "$@"

if [[ ${SKIP_SMOKE:-} != 1 ]]; then
    step "smoke test"
    (unset MSYS_NO_PATHCONV && SMOKE_NO_BUILD=1 bash docker/smoke-test.sh)
fi
if [[ ${SKIP_NETWORK:-} != 1 ]]; then
    step "network harness"
    NETWORK_OPTIONAL=1 bash docker/network/test.sh
fi
if [[ ${SKIP_REMOTE:-} != 1 ]]; then
    step "remote management harness"
    REMOTE_NO_BUILD=1 bash docker/remote/test.sh
fi

echo
echo "full test run passed"
