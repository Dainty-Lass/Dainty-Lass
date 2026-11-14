#!/usr/bin/env bash
# The remote profile's harness (RM1-RM5): a controller and two managed installs. Checks enrollment, config sync, a
# remote ban, an offline controller, revocation, and that no secret crosses the wire.
#   REMOTE_NO_BUILD=1   use the images as built (and up --no-build)
#   REMOTE_KEEP_UP=1    leave the containers and volumes for a look afterwards
set -euo pipefail
cd "$(dirname "$0")/../.."

export COMPOSE_PROJECT_NAME=${COMPOSE_PROJECT_NAME:-dainty-remote}
export COMPOSE_PROFILES=remote
# Taken from the environment when set, so the oracle can point these scripts at another file (D-8)
export COMPOSE_FILE=${COMPOSE_FILE:-compose.yaml}
# Git Bash would otherwise rewrite the /opt/... paths handed to docker
export MSYS_NO_PATHCONV=1
ws=/opt/dainty-lass/.workspace

started=0
cleanup() {
    status=$?
    if [[ $status -ne 0 && $started -eq 1 ]]; then
        echo
        echo "== remote harness failed; the containers and their logs"
        docker compose ps -a || true
        docker compose logs --tail 80 || true
    fi
    if [[ ${REMOTE_KEEP_UP:-} != 1 ]]; then
        docker compose down --volumes --remove-orphans >/dev/null 2>&1 || true
    fi
    exit "$status"
}
trap cleanup EXIT

step() { printf '\n== %s\n' "$*"; }
fail() {
    echo "remote test failed: $*" >&2
    exit 1
}
contains() {
    [[ $1 == *"$2"* ]] || {
        printf '%s\n' "$1" >&2
        fail "$3 does not contain $2"
    }
}
# on <host> <command...>: a command in host-<host>, which must succeed
on() {
    local where=$1 out
    shift
    out=$(docker compose exec -T "host-$where" "$@" 2>&1) || {
        printf '%s\n' "$out" >&2
        fail "$* on $where failed"
    }
    printf '%s' "$out"
}
# fails_on <host> <command...>: a command that must fail
fails_on() {
    local where=$1
    shift
    if docker compose exec -T "host-$where" "$@" >/dev/null 2>&1; then
        fail "$* on $where succeeded"
    fi
}
cat_on() { on "$1" cat "$ws/$2"; }

no_build=()
if [[ ${REMOTE_NO_BUILD:-} != 1 ]]; then
    step "build"
    docker compose build net-controller configure-controller gateway-controller host-controller
else
    no_build=(--no-build)
fi

step "up"
docker compose down --volumes --remove-orphans >/dev/null 2>&1 || true
started=1
docker compose up -d --wait --wait-timeout 300 "${no_build[@]}" remote

step "enrollment"
alpha_uri=$(on controller dainty-host remote invite --node alpha --region us-east --label "Install Alpha" |
    grep -o 'dainty-lass-remote://enroll?[^[:space:]]*' | head -n 1)
bravo_uri=$(on controller dainty-host remote invite --node bravo --region eu-west --label "Install Bravo" |
    grep -o 'dainty-lass-remote://enroll?[^[:space:]]*' | head -n 1)
[[ -n $alpha_uri && -n $bravo_uri ]] || fail "remote invite printed no dainty-lass-remote://enroll? URI"
contains "$(on alpha dainty-host remote enroll "$alpha_uri" --json)" '"node": "alpha"' "alpha's enrollment"
fails_on alpha dainty-host remote enroll "$alpha_uri" --json
contains "$(on bravo dainty-host remote enroll "$bravo_uri" --json)" '"node": "bravo"' "bravo's enrollment"
for install in alpha bravo; do
    contains "$(on "$install" dainty-host remote sync --json)" '"applied_revision": 1' "$install's first sync"
done
listed=$(on controller dainty-host remote list --json)
contains "$listed" alpha "remote list"
contains "$listed" bravo "remote list"

step "config sync"
declare -A secrets_before
for install in alpha bravo; do
    secrets_before[$install]=$(on "$install" sha256sum "$ws/config/secrets.env" | cut -d' ' -f1)
done
on controller sed -i -e 's/^name = .*/name = "Fleet Friday Hunts"/' -e 's/^max_hunt_servers = .*/max_hunt_servers = 4/' \
    "$ws/config/settings.toml" >/dev/null
toml=$(cat_on controller config/settings.toml)
contains "$toml" 'name = "Fleet Friday Hunts"' "the controller's settings.toml"
contains "$toml" 'max_hunt_servers = 4' "the controller's settings.toml"
contains "$(on controller dainty-host remote set-config --json)" '"revision": 2' "set-config"
declare -A address=([alpha]=203.0.113.21 [bravo]=203.0.113.22)
for install in alpha bravo; do
    contains "$(on "$install" dainty-host remote sync --json)" '"applied_revision": 2' "$install's second sync"
    toml=$(cat_on "$install" config/settings.toml)
    contains "$toml" 'name = "Fleet Friday Hunts"' "$install's settings.toml"
    contains "$toml" 'max_hunt_servers = 4' "$install's settings.toml"
    contains "$toml" "game_address = \"${address[$install]}\"" "$install's settings.toml"
    after=$(on "$install" sha256sum "$ws/config/secrets.env" | cut -d' ' -f1)
    [[ $after == "${secrets_before[$install]}" ]] || fail "$install's secrets.env changed in the sync"
done
[[ ${secrets_before[alpha]} != "${secrets_before[bravo]}" ]] || fail "alpha and bravo share a secrets.env"

step "remote ban"
on alpha dainty-host invite --for Griefer99 --json >/dev/null
contains "$(on controller dainty-host remote exec --node alpha --action ban --account Griefer99 \
    --reason "fleet rule violation" --role admin --actor ctrl-admin --json)" '"status": "queued"' "the queued ban"
on alpha dainty-host remote sync --json >/dev/null
moderation=$(cat_on alpha config/remote-moderation.json)
contains "$moderation" '"target": "Griefer99"' "alpha's remote-moderation.json"
contains "$moderation" '"status": "banned"' "alpha's remote-moderation.json"
contains "$moderation" '"actor": "remote:controller:ctrl-admin"' "alpha's remote-moderation.json"
contains "$(on controller dainty-host remote status --json)" \
    "banned applied for Griefer99 by remote:controller:ctrl-admin" "the controller's status"

step "controller offline"
docker compose stop gateway-controller
for install in alpha bravo; do
    on "$install" dainty-healthcheck backend >/dev/null
    on "$install" dainty-healthcheck gateway >/dev/null
done
fails_on alpha dainty-host remote sync --json
status=$(on alpha dainty-host remote status --json)
contains "$status" '"detached": false' "alpha's status"
contains "$status" '"applied_revision": 2' "alpha's status"
docker compose start gateway-controller
back=0
for _ in $(seq 30); do
    if docker compose exec -T gateway-controller dainty-healthcheck gateway >/dev/null 2>&1; then
        back=1
        break
    fi
    sleep 1
done
[[ $back -eq 1 ]] || fail "the controller's gateway is not healthy 30 s after its start"

step "revocation"
contains "$(on controller dainty-host remote revoke bravo --json)" '"revoked": "bravo"' "the revocation"
fails_on bravo dainty-host remote sync --json
contains "$(on bravo dainty-host remote status --json)" '"detached": true' "bravo's status"
if docker compose exec -T host-controller dainty-host remote exec --node bravo --action doctor --role admin \
    --actor ctrl-admin --json >/dev/null 2>&1; then
    fail "remote exec on the revoked bravo succeeded"
fi
contains "$(on alpha dainty-host remote sync --json)" '"applied_revision": 3' "alpha's sync after the revocation"

step "wire hygiene"
declare -A wire
for install in controller alpha bravo; do
    wire[$install]=$(cat_on "$install" wire.jsonl)
    [[ -n ${wire[$install]} ]] || fail "$install's wire.jsonl is empty"
done
for prefix in enroll_send: enroll_recv: managed_send: managed_recv:; do
    grep -q "^$prefix" <<<"${wire[alpha]}" || fail "alpha's wire.jsonl has no $prefix line"
done
for prefix in controller_recv: controller_send:; do
    grep -q "^$prefix" <<<"${wire[controller]}" || fail "the controller's wire.jsonl has no $prefix line"
done
secret_values=()
for install in controller alpha bravo; do
    while IFS= read -r line; do
        value=${line#*=}
        value=${value#\"}
        value=${value%\"}
        [[ ${#value} -ge 8 ]] && secret_values+=("$value")
    done < <(cat_on "$install" config/secrets.env | grep '=')
done
[[ ${#secret_values[@]} -gt 0 ]] || fail "no secrets.env values to look for"
# shellcheck disable=SC2016 # $argon2 and $scrypt are literal hash markers
markers=(DAINTY_ADMIN_KEY DAINTY_ADMIN_API_KEY DAINTY_SECRET GATEWAY_SECRET ALLOWLIST_SECRET
    RUSTIC_ISLES_MANAGER_OPERATOR_SECRET DAINTY_DB_PASSWORD DLK_ "PRIVATE KEY" "SQLite format 3" '$argon2' '$scrypt')
for install in controller alpha bravo; do
    for needle in "${markers[@]}" "${secret_values[@]}"; do
        [[ ${wire[$install]} != *"$needle"* ]] || fail "$install's wire.jsonl carries a secret (${needle:0:4}...)"
    done
done

echo
echo "remote management harness passed"
