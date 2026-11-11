#!/usr/bin/env bash
# A public server in Docker without a game: setup through host.toml, the [RB15] MongoDB import, every service healthy,
# the gateway's certificate and proxying, the allowlist's rules, an invite and a clean stop. Plays no game.
#   SMOKE_NO_BUILD=1 uses the images as built; SMOKE_PORT moves the gateway's published port (18443).
set -euo pipefail
cd "$(dirname "$0")/.."

export COMPOSE_PROJECT_NAME=${COMPOSE_PROJECT_NAME:-dainty-smoke}
export COMPOSE_PROFILES=public
# Taken from the environment when set, so the oracle can point these scripts at another file (D-8)
export COMPOSE_FILE=${COMPOSE_FILE:-compose.yaml}
port=${SMOKE_PORT:-18443}
export DAINTY_GATEWAY_PORT=443
export DAINTY_GATEWAY_PUBLISH=127.0.0.1:$port
export DAINTY_GAME_PUBLISH_ADDRESS=127.0.0.1

work=$(mktemp -d)
# Docker Desktop on Windows takes C:/... paths, and Git Bash then mustn't rewrite /opt/... arguments
if command -v cygpath >/dev/null 2>&1; then
    work=$(cygpath -m "$work")
    export MSYS_NO_PATHCONV=1
fi
mkdir -p "$work/game" "$work/import" "$work/config"
export DAINTY_GAME_DIR=$work/game DAINTY_IMPORT_DIR=$work/import DAINTY_CONFIG_DIR=$work/config

started=0
cleanup() {
    status=$?
    if [[ $status -ne 0 && $started -eq 1 ]]; then
        echo
        echo "== smoke test failed; the containers and their logs"
        docker compose ps -a || true
        COMPOSE_PROFILES=public,toml,mongo docker compose logs --tail 80 || true
    fi
    COMPOSE_PROFILES=public,toml,mongo docker compose down --volumes --remove-orphans >/dev/null 2>&1 || true
    rm -rf "$work"
    exit "$status"
}
trap cleanup EXIT

step() { printf '\n== %s\n' "$*"; }
fail() {
    echo "smoke test failed: $*" >&2
    exit 1
}
# contains <text> <needle> <what>
contains() {
    [[ $1 == *"$2"* ]] || {
        printf '%s\n' "$1" >&2
        fail "$3 does not contain $2"
    }
}
workspace_file() {
    docker compose run --rm --no-deps -T --entrypoint sh host -c "cat /opt/dainty-lass/.workspace/$1"
}

if [[ ${SMOKE_NO_BUILD:-} != 1 ]]; then
    step "build"
    docker compose build
fi
started=1

step "a fresh toml apply with no MongoDB"
cat >"$work/config/host.toml" <<'EOF'
version = 1
mode = "public"
host = "smoke.example.org"
game_address = "203.0.113.7"
name = "Fresh Toml Apply"
registration = "INVITECODE"
max_hunt_servers = 1
EOF
COMPOSE_PROFILES=public,toml docker compose run --rm configure
contains "$(workspace_file config/backend.env)" "Fresh Toml Apply" "backend.env"
COMPOSE_PROFILES=public,toml docker compose down --volumes --remove-orphans
rm -f "$work/config/host.toml"

# [RB15] Steps up to "migrate and verify" go with the MongoDB handling (Q-6)
step "[RB15] seed a pre-cutover mongo-data volume"
COMPOSE_PROFILES=public,mongo docker compose up -d --wait mongo
COMPOSE_PROFILES=public,mongo docker compose exec -T mongo mongosh --quiet \
    'mongodb://127.0.0.1:27017/reforged?replicaSet=rs0' --eval '
const created = new Date("2026-01-15T12:00:00Z");
db.accounts.insertOne({
  _id: ObjectId("507f1f77bcf86cd799439011"),
  userId: "user-docker-01",
  name: "SlayerDocker",
  displayName: "SlayerDocker",
  displayNameNormalized: "slayerdocker",
  email: "slayer.docker@example.invalid",
  passwordHash: "$2b$10$abcdefghijklmnopqrstuuS5Hq3cJ0dYQ9mFQ3sXjrnD4bJm6Z1Zy",
  status: "active",
  approvalStatus: "approved",
  roles: ["player"],
  usernameSet: 1,
  createdAt: created,
});
db.characters.insertOne({
  _id: ObjectId("507f1f77bcf86cd799439012"),
  characterId: "char-docker-01",
  userId: "user-docker-01",
  name: "SlayerDocker",
  createdDate: created,
});
db.reforgedInvites.insertOne({
  invitecode: "INV-DOCKER-01",
  usesRemaining: 5,
  infiniteUses: false,
  note: "Docker smoke test",
  createdAt: created,
});
'

step "[RB15] the backend refuses unmigrated data"
COMPOSE_PROFILES=public,mongo docker compose run --rm host gamedata --examples
if output=$(COMPOSE_PROFILES=public,mongo docker compose run --rm -T backend 2>&1); then
    printf '%s\n' "$output" >&2
    fail "the backend started on unmigrated MongoDB data"
fi
contains "$output" "Unmigrated MongoDB data" "the backend's refusal"

step "[RB15] migrate and verify"
COMPOSE_PROFILES=public,mongo docker compose run --rm host setup --mode public --host smoke.example.org \
    --game-address 203.0.113.7 --name "Smoke Test"
COMPOSE_PROFILES=public,mongo docker compose run --rm host db migrate
COMPOSE_PROFILES=public,mongo docker compose run --rm host db verify
COMPOSE_PROFILES=public,mongo docker compose rm -sf mongo

step "the toml profile applies host.toml alone"
workspace_file config/host.toml | sed 's/^name = .*/name = "Smoke Toml"/' >"$work/config/host.toml"
grep -q '^name = "Smoke Toml"' "$work/config/host.toml" || fail "the workspace's host.toml has no name line"
COMPOSE_PROFILES=public,toml docker compose run --rm configure
contains "$(workspace_file config/backend.env)" "Smoke Toml" "backend.env"

step "up, configure first"
COMPOSE_PROFILES=public,toml docker compose up -d --wait --wait-timeout 300
docker compose ps

step "the gateway serves setup's certificate"
workspace_file tls/gateway-cert.pem >"$work/gateway-cert.pem"
expected=$(openssl x509 -in "$work/gateway-cert.pem" -noout -fingerprint -sha256)
served=$(openssl s_client -connect "127.0.0.1:$port" -servername smoke.example.org </dev/null 2>/dev/null |
    openssl x509 -noout -fingerprint -sha256)
[[ -n $expected && $served == "$expected" ]] || fail "the gateway serves $served, setup made $expected"

step "the gateway proxies the Rust backend"
api() {
    curl -sS --cacert "$work/gateway-cert.pem" --resolve "smoke.example.org:$port:127.0.0.1" "$@"
}
base=https://smoke.example.org:$port
contains "$(api --fail "$base/dainty/api/ServerStatus")" '"online":true' "ServerStatus"
# [RB15] The migrated account and invite
taken=$(api -H 'content-type: application/json' -d '{"Username":"SlayerDocker","InviteCode":"INV-DOCKER-01"}' \
    "$base/dainty/api/Register")
contains "$taken" '"username_taken"' "Register as the migrated account"
registered=$(api --fail -H 'content-type: application/json' \
    -d '{"Username":"DockerNewbie","InviteCode":"INV-DOCKER-01"}' "$base/dainty/api/Register")
contains "$registered" '"UUK"' "Register with the migrated invite"

step "[RB15] the migrated invite is listed"
contains "$(docker compose run --rm -T host invite --list)" "INV-DOCKER-01" "invite --list"

step "the allowlist closes the game ports"
contains "$(docker compose exec -T allowlist nft list table inet dainty_lass)" "udp dport 8770-8777 drop" \
    "the allowlist's table"

step "invite"
contains "$(docker compose run --rm -T host invite --for "Smoke")" "dainty://" "the invite"

step "doctor"
docker compose run --rm -T host doctor --no-hash || echo "(doctor's failures are expected here: no game, no sources)"

step "clean stop"
docker compose stop
running=$(docker compose ps --status running --quiet)
[[ -z $running ]] || fail "still running after stop: $running"

echo
echo "smoke test passed"
