#!/usr/bin/env bash
# Dainty Lass: the Docker images' smoke test (CI's docker job, and by hand from the repository root:
# bash docker/smoke-test.sh). A public server set up in throwaway volumes on example game data, without a game:
# every service comes up healthy, the gateway serves the certificate setup made, the allowlist's table is in the
# shared namespace, the backend makes an invite, and everything stops. Builds the images unless SMOKE_NO_BUILD=1.
set -euo pipefail

export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-dainty-smoke}"
export COMPOSE_PROFILES=public
# From the environment when set, so the oracle runs this against the root's compose file (spec-docker D-8)
export COMPOSE_FILE=${COMPOSE_FILE:-compose.yaml}
export DAINTY_GATEWAY_PORT=443
export DAINTY_GATEWAY_PUBLISH="127.0.0.1:${SMOKE_PORT:-18443}"
export DAINTY_GAME_PUBLISH_ADDRESS=127.0.0.1
smoke_dir="$(mktemp -d)"
export DAINTY_GAME_DIR="$smoke_dir/game" DAINTY_IMPORT_DIR="$smoke_dir/import" DAINTY_CONFIG_DIR="$smoke_dir/config"
mkdir -p "$DAINTY_GAME_DIR" "$DAINTY_IMPORT_DIR" "$DAINTY_CONFIG_DIR"

compose() { docker compose "$@"; }
step() { printf '\n== %s\n' "$*"; }
cleanup() {
	status=$?
	if [ "$status" -ne 0 ]; then
		compose ps -a || true
		compose logs --no-color --tail 80 || true
	fi
	COMPOSE_PROFILES=public,toml,mongo compose down --volumes --remove-orphans >/dev/null 2>&1 || true
	rm -rf "$smoke_dir"
	exit "$status"
}
trap cleanup EXIT

if [ "${SMOKE_NO_BUILD:-0}" != 1 ]; then
	step build
	compose build
fi

step "fresh toml profile apply with no MongoDB succeeds"
cat <<'TOML' >"$DAINTY_CONFIG_DIR/settings.toml"
version = 1
mode = "public"
host = "smoke.example.org"
game_address = "203.0.113.7"
name = "Fresh Toml Apply"
registration = "INVITECODE"
max_hunt_servers = 1
TOML
COMPOSE_PROFILES=public,toml compose run --rm configure
compose run --rm --entrypoint sh host -c 'cat /opt/dainty-lass/.workspace/config/backend.env' | grep -q 'Fresh Toml Apply'
compose down --volumes --remove-orphans >/dev/null 2>&1

step "seed pre-cutover mongo-data volume"
COMPOSE_PROFILES=public,mongo compose up -d --wait mongo
COMPOSE_PROFILES=public,mongo compose exec -T mongo mongosh --quiet "mongodb://127.0.0.1:27017/reforged?replicaSet=rs0" --eval '
	db.accounts.insertOne({
		_id: ObjectId("507f1f77bcf86cd799439011"),
		userId: "user-docker-01",
		name: "SlayerDocker",
		displayName: "SlayerDocker",
		displayNameNormalized: "slayerdocker",
		email: "slayer.docker@example.invalid",
		passwordHash: "$2b$10$abcdefghijklmnopqrstuuABCDEFGHIJKLMNOPQRSTUVWXYZ01234",
		status: "active",
		approvalStatus: "approved",
		roles: ["player"],
		usernameSet: 1,
		createdAt: new Date("2026-01-15T12:00:00Z")
	});
	db.characters.insertOne({
		_id: ObjectId("507f1f77bcf86cd799439012"),
		characterId: "char-docker-01",
		userId: "user-docker-01",
		name: "SlayerDocker",
		createdDate: new Date("2026-01-15T12:00:00Z")
	});
	db.reforgedInvites.insertOne({
		invitecode: "INV-DOCKER-01",
		usesRemaining: 5,
		infiniteUses: false,
		note: "Pre-cutover Docker invite",
		createdAt: new Date("2026-01-15T12:00:00Z")
	});
'

step "backend refuses to start on unmigrated mongo-data"
compose run --rm host gamedata --examples
if compose run --rm backend >"$smoke_dir/unmigrated.txt" 2>&1; then
	cat "$smoke_dir/unmigrated.txt"
	echo "ERROR: backend started despite unmigrated mongo-data" >&2
	exit 1
fi
cat "$smoke_dir/unmigrated.txt"
grep -q "Unmigrated MongoDB data" "$smoke_dir/unmigrated.txt"

step "db migrate and db verify move mongo-data into the Rust store"
compose run --rm host setup --mode public --host smoke.example.org --game-address 203.0.113.7 --name "Smoke Test" | tee "$smoke_dir/setup.txt"
# setup no longer moves it (dainty-legacy's `import reforged` does, or these two by hand); verify writes the marker
compose run --rm host db migrate
compose run --rm host db verify
COMPOSE_PROFILES=public,mongo compose rm -sf mongo

step "the toml profile applies settings.toml alone"
compose run --rm --entrypoint sh host -c 'cat /opt/dainty-lass/.workspace/config/settings.toml' 	| sed 's/^name = .*/name = "Smoke Toml"/' >"$DAINTY_CONFIG_DIR/settings.toml"
COMPOSE_PROFILES=public,toml compose run --rm configure
compose run --rm --entrypoint sh host -c 'cat /opt/dainty-lass/.workspace/config/backend.env' | grep -q 'Smoke Toml'

step "up, configure first"
COMPOSE_PROFILES=public,toml compose up -d --wait --wait-timeout 300
compose ps

step "the gateway serves setup's certificate"
fp_file="$(compose run --rm --entrypoint sh host -c 'cat /opt/dainty-lass/.workspace/tls/gateway-cert.pem')"
served="$(echo | openssl s_client -connect "$DAINTY_GATEWAY_PUBLISH" -servername smoke.example.org 2>/dev/null | openssl x509 -noout -fingerprint -sha256)"
expected="$(printf '%s\n' "$fp_file" | openssl x509 -noout -fingerprint -sha256)"
echo "served:   $served"
echo "expected: $expected"
[ "$served" = "$expected" ]

step "the gateway proxies /dainty/api/ServerStatus and migrated accounts from the Rust backend"
printf '%s\n' "$fp_file" >"$smoke_dir/gateway-cert.pem"
status_json="$(curl -fsS --cacert "$smoke_dir/gateway-cert.pem" --resolve "smoke.example.org:${SMOKE_PORT:-18443}:127.0.0.1" "https://smoke.example.org:${SMOKE_PORT:-18443}/dainty/api/ServerStatus")"
echo "ServerStatus: $status_json"
printf '%s\n' "$status_json" | grep -q '"online":true'

user_taken_json="$(curl -sS --cacert "$smoke_dir/gateway-cert.pem" --resolve "smoke.example.org:${SMOKE_PORT:-18443}:127.0.0.1" -H "Content-Type: application/json" -d '{"Username":"SlayerDocker","InviteCode":"INV-DOCKER-01"}' "https://smoke.example.org:${SMOKE_PORT:-18443}/dainty/api/Register")"
echo "Register SlayerDocker (migrated account): $user_taken_json"
printf '%s\n' "$user_taken_json" | grep -q '"username_taken"'

user_new_json="$(curl -fsS --cacert "$smoke_dir/gateway-cert.pem" --resolve "smoke.example.org:${SMOKE_PORT:-18443}:127.0.0.1" -H "Content-Type: application/json" -d '{"Username":"DockerNewbie","InviteCode":"INV-DOCKER-01"}' "https://smoke.example.org:${SMOKE_PORT:-18443}/dainty/api/Register")"
echo "Register DockerNewbie with migrated invite: $user_new_json"
printf '%s\n' "$user_new_json" | grep -q '"UUK"'

step "migrated invite is present in the Rust backend"
compose run --rm host invite --list | tee "$smoke_dir/invites-list.txt"
grep -q 'INV-DOCKER-01' "$smoke_dir/invites-list.txt"

step "the allowlist's table closes the game ports"
compose exec -T allowlist nft list table inet dainty_lass | tee "$smoke_dir/nft.txt"
grep -q 'udp dport 8770-8777 drop' "$smoke_dir/nft.txt"

step invite
compose run --rm host invite --for "Smoke" | tee "$smoke_dir/invite.txt"
grep -q 'dainty://' "$smoke_dir/invite.txt"

step doctor
compose run --rm host doctor --no-hash || true

step "clean stop"
compose stop
[ -z "$(compose ps --status running --quiet)" ]
echo "smoke test passed"
