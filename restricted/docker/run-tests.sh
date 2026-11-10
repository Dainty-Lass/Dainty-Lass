#!/usr/bin/env bash
# Dainty Lass: the full test run on the built images (CI's docker job, and by hand from the
# repository root: bash docker/run-tests.sh). Builds the front, server and test images, then
#  1. every server test suite inside the test image, which is the server image plus the Rust toolchain
#     (docker/test-in-image.sh): the Rust server crates, with the allowlist's firewall test applying its nftables
#     rules for real in the container's own network namespace
#  2. the compose smoke test on the front and server images (docker/smoke-test.sh)
#  3. the network harness: four nodes with injected latency (docker/network/test.sh). It needs CAP_NET_ADMIN and the netem
#     kernel module, so it is skipped with a note, not failed, where Docker cannot give them
# Arguments pick the in-image suites (rust); SKIP_SMOKE=1 leaves out step 2, SKIP_NETWORK=1 step 3.
set -euo pipefail

image="${DAINTY_IMAGE:-dainty-lass}-test:${DAINTY_TAG:-local}"
export COMPOSE_FILE=compose.yaml

printf '\n== build\n'
docker compose build
docker compose --profile test build test

printf '\n== the test suites in the image\n'
docker run --rm --cap-add NET_ADMIN "$image" "$@"

if [ "${SKIP_SMOKE:-0}" != 1 ]; then
	printf '\n== the smoke test\n'
	env -u MSYS_NO_PATHCONV SMOKE_NO_BUILD=1 bash docker/smoke-test.sh
fi

if [ "${SKIP_NETWORK:-0}" != 1 ]; then
	printf '\n== the network harness\n'
	MSYS_NO_PATHCONV=1 NETWORK_OPTIONAL=1 bash docker/network/test.sh
fi

if [ "${SKIP_REMOTE:-0}" != 1 ]; then
	printf '\n== the remote management harness\n'
	MSYS_NO_PATHCONV=1 REMOTE_NO_BUILD=1 bash docker/remote/test.sh
fi
echo "full test run passed"
