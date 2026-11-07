#!/usr/bin/env bash
# Runs the Trivy image pinned in .github/trivy/Dockerfile on the working directory, which must be the repository
# root (trivy.yaml's paths are relative to it). The database is cached under RUNNER_TEMP; TRIVY_DOCKER_SOCKET=1
# lends it the Docker socket, to scan an image the job built.
set -Eeuo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
image=$(sed -n 's/^FROM //p' "${root}/.github/trivy/Dockerfile")
cache=${TRIVY_HOST_CACHE:-${RUNNER_TEMP:-${TMPDIR:-/tmp}}/trivy-cache}
mkdir -p "$cache"

args=(--rm --user "$(id -u):$(id -g)" --env HOME=/tmp --env TRIVY_CACHE_DIR=/cache
  --volume "${cache}:/cache" --volume "${PWD}:/work" --workdir /work)
if [ "${TRIVY_DOCKER_SOCKET:-}" = 1 ]; then
  args+=(--volume /var/run/docker.sock:/var/run/docker.sock --group-add "$(stat -c %g /var/run/docker.sock)")
fi

exec docker run "${args[@]}" "$image" --config .github/trivy/trivy.yaml "$@"
