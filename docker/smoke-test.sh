#!/usr/bin/env bash
# Dauntless Reforged Revival: the Docker image's smoke test (CI's docker job, and by hand from the repository root:
# bash docker/smoke-test.sh). A public server in throwaway volumes, without a game: the server comes up healthy from
# a server.toml, the gateway serves the certificate the server made, the allowlist's table is in the shared
# namespace, the invite link with its code comes from the file the server wrote, the host kit's commands run in the
# image, and the server exits cleanly on stop. Builds the image unless SMOKE_NO_BUILD=1.
set -euo pipefail

export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-reforged-smoke}"
export COMPOSE_FILE=compose.yaml
export REFORGED_GATEWAY_PORT=443
export REFORGED_GATEWAY_PUBLISH="127.0.0.1:${SMOKE_PORT:-18443}"
export REFORGED_GAME_PUBLISH_ADDRESS=127.0.0.1
smoke_dir="$(mktemp -d)"
export REFORGED_GAME_DIR="$smoke_dir/game" REFORGED_IMPORT_DIR="$smoke_dir/import"
mkdir -p "$REFORGED_GAME_DIR" "$REFORGED_IMPORT_DIR"

compose() { docker compose "$@"; }
step() { printf '\n== %s\n' "$*"; }
cleanup() {
	status=$?
	if [ "$status" -ne 0 ]; then
		compose ps -a || true
		compose logs --no-color --tail 80 || true
	fi
	compose down --volumes --remove-orphans >/dev/null 2>&1 || true
	rm -rf "$smoke_dir"
	exit "$status"
}
trap cleanup EXIT

if [ "${SMOKE_NO_BUILD:-0}" != 1 ]; then
	step build
	compose build
fi

step "the server's settings: a public server on 443, one edition, the allowlist for real"
compose run --rm -T --entrypoint sh host -c 'mkdir -p .workspace/config && cat >.workspace/config/server.toml' <<'TOML'
# docker/smoke-test.sh's server
server_name = "Smoke Test"
server_mode = "public"
bind_host = "0.0.0.0"
public_host = "smoke.example.org"
game_address = "203.0.113.7"
gateway_port = 443
firewall_mode = "nftables"
admin_enabled = true

[editions.refined]
enabled = false
TOML

step up
compose up -d --wait --wait-timeout 300
compose ps

step "the gateway serves the server's certificate"
expected="$(compose run --rm -T --entrypoint sh host -c 'cat /opt/reforged/.workspace/certs/gateway.crt' \
	| openssl x509 -noout -fingerprint -sha256)"
served="$(echo | openssl s_client -connect "$REFORGED_GATEWAY_PUBLISH" -servername smoke.example.org 2>/dev/null \
	| openssl x509 -noout -fingerprint -sha256)"
echo "served:   $served"
echo "expected: $expected"
[ "$served" = "$expected" ]

step "the allowlist's table closes the game ports"
compose exec -T server nft list table inet reforged_revival | tee "$smoke_dir/nft.txt"
grep -q 'udp dport 8770-8777 drop' "$smoke_dir/nft.txt"

step "the invite link, with its code"
compose run --rm host invite | tee "$smoke_dir/invite.txt"
grep -q 'reforged-revival://join?.*mode=public' "$smoke_dir/invite.txt"
grep -q 'host=smoke.example.org' "$smoke_dir/invite.txt"
grep -q 'code=open' "$smoke_dir/invite.txt"

step "the host kit's commands run in the image; setup is answered, not run"
compose run --rm host gamedata --help >/dev/null
if compose run --rm host setup --mode public --host smoke.example.org; then
	echo "setup should be refused in Docker (docker/host-entrypoint.sh)" >&2
	exit 1
fi

step "clean stop"
compose stop
[ -z "$(compose ps --status running --quiet)" ]
# 0 only when the server took the SIGTERM and shut its components down; a kill after the grace period is 137 or 143
exit_code="$(docker inspect -f '{{.State.ExitCode}}' "$(compose ps -aq server)")"
echo "server exit code: $exit_code"
[ "$exit_code" = 0 ]
echo "smoke test passed"
