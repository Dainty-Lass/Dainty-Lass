#!/usr/bin/env bash
# DaintyLass: end-to-end Docker test for remote management (RM1-RM5, docs/remote-management.md).
# From the repository root:
#   bash docker/remote/test.sh
# Brings up compose.yaml's `remote` profile (one controller and two managed installs, alpha and bravo) in throwaway
# volumes and proves that:
#  1. both installs enroll over pinned TLS using single-use tokens (`dainty-host remote invite` + `enroll`), and
#     token replay is refused
#  2. changing a shared setting on the controller (`dainty-host remote set-config`) is picked up by both managed
#     installs on sync while keeping each install's own host, game_address and secrets.env untouched
#  3. running a remote ban (`dainty-host remote exec --node alpha --action ban ...`) applies on the target install
#     and records the remote operator actor (`controller:ctrl-admin`)
#  4. stopping the controller leaves both managed installs running and answering `/reforged/api/ServerStatus`
#  5. revoking one install (`dainty-host remote revoke bravo`) causes the controller to refuse bravo's sync and
#     commands while alpha continues syncing
#  6. inspecting the captured wire logs (`wire.jsonl`) from the controller and both managed installs proves that the
#     expected JSON protocol messages crossed the wire and zero secrets (`secrets.env` values, `RRK_`, `PRIVATE KEY`,
#     `SQLite format 3`, password hashes) ever crossed
# Environment:
#   REMOTE_NO_BUILD=1       the images are built already
#   REMOTE_KEEP_UP=1        leave the `remote` profile running on exit (for interactive inspection / UI screenshots)
set -euo pipefail

export MSYS_NO_PATHCONV=1
export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-reforged-remote}"
export COMPOSE_PROFILES=remote
export COMPOSE_FILE=compose.yaml

compose() { docker compose "$@"; }
step() { printf '\n== %s\n' "$*"; }
fail() { echo "remote test failed: $*" >&2; exit 1; }

started=0
cleanup() {
	status=$?
	if [ "$status" -ne 0 ] && [ "$started" = 1 ]; then
		compose ps -a || true
		compose logs --no-color --tail 40 || true
	fi
	if [ "${REMOTE_KEEP_UP:-0}" != 1 ]; then
		compose down --volumes --remove-orphans >/dev/null 2>&1 || true
	fi
	exit "$status"
}
trap cleanup EXIT

if [ "${REMOTE_NO_BUILD:-0}" != 1 ]; then
	step build
	compose build net-controller configure-controller gateway-controller host-controller
fi

step "tear down any previous remote volumes"
compose down --volumes --remove-orphans >/dev/null 2>&1 || true

step "start controller + two managed installs (alpha, bravo)"
up_flags=(-d --wait --wait-timeout 300)
if [ "${REMOTE_NO_BUILD:-0}" = 1 ]; then
	up_flags+=(--no-build)
fi
compose up "${up_flags[@]}" remote
started=1
compose ps

step "1. enroll both managed installs (alpha and bravo) and verify single-use token replay is refused"
invite_alpha="$(compose exec -T host-controller dainty-host remote invite --node alpha --region us-east --label "Install Alpha" | tr -d '\r')"
invite_bravo="$(compose exec -T host-controller dainty-host remote invite --node bravo --region eu-west --label "Install Bravo" | tr -d '\r')"
case "$invite_alpha" in
	dainty-lass-remote://enroll\?*) echo "alpha invite minted" ;;
	*) fail "unexpected alpha invite URI: $invite_alpha" ;;
esac
case "$invite_bravo" in
	dainty-lass-remote://enroll\?*) echo "bravo invite minted" ;;
	*) fail "unexpected bravo invite URI: $invite_bravo" ;;
esac

enroll_alpha_out="$(compose exec -T host-alpha dainty-host remote enroll "$invite_alpha" --json)"
echo "$enroll_alpha_out"
printf '%s\n' "$enroll_alpha_out" | grep -q '"node": "alpha"' || fail "alpha enrollment did not report node=alpha"

if compose exec -T host-alpha dainty-host remote enroll "$invite_alpha" >/dev/null 2>&1; then
	fail "replaying a redeemed single-use enrollment token succeeded"
fi
echo "single-use token replay refused as expected"

enroll_bravo_out="$(compose exec -T host-bravo dainty-host remote enroll "$invite_bravo" --json)"
echo "$enroll_bravo_out"
printf '%s\n' "$enroll_bravo_out" | grep -q '"node": "bravo"' || fail "bravo enrollment did not report node=bravo"

# Initial sync from both installs
sync_alpha_1="$(compose exec -T host-alpha dainty-host remote sync --json)"
sync_bravo_1="$(compose exec -T host-bravo dainty-host remote sync --json)"
printf '%s\n' "$sync_alpha_1" | grep -q '"applied_revision": 1' || fail "alpha did not sync revision 1: $sync_alpha_1"
printf '%s\n' "$sync_bravo_1" | grep -q '"applied_revision": 1' || fail "bravo did not sync revision 1: $sync_bravo_1"

list_1="$(compose exec -T host-controller dainty-host remote list --json)"
echo "$list_1"
printf '%s\n' "$list_1" | grep -q '"node": "alpha"' || fail "controller list missing alpha"
printf '%s\n' "$list_1" | grep -q '"node": "bravo"' || fail "controller list missing bravo"

step "2. change a shared setting on the controller and verify both installs pick it up while preserving local host and secrets"
alpha_secrets_sha_before="$(compose exec -T host-alpha sha256sum /opt/reforged/.workspace/config/secrets.env | awk '{print $1}')"
bravo_secrets_sha_before="$(compose exec -T host-bravo sha256sum /opt/reforged/.workspace/config/secrets.env | awk '{print $1}')"

compose exec -T host-controller sh -ec '
	sed -i "s/^name = .*/name = \"Fleet Friday Hunts\"/" /opt/reforged/.workspace/config/host.toml
	sed -i "s/^max_hunt_servers = .*/max_hunt_servers = 4/" /opt/reforged/.workspace/config/host.toml
'
set_cfg_out="$(compose exec -T host-controller dainty-host remote set-config --json)"
echo "$set_cfg_out"
printf '%s\n' "$set_cfg_out" | grep -q '"revision": 2' || fail "controller revision did not advance to 2"

sync_alpha_2="$(compose exec -T host-alpha dainty-host remote sync --json)"
sync_bravo_2="$(compose exec -T host-bravo dainty-host remote sync --json)"
echo "$sync_alpha_2"
echo "$sync_bravo_2"
printf '%s\n' "$sync_alpha_2" | grep -q '"applied_revision": 2' || fail "alpha did not pick up revision 2"
printf '%s\n' "$sync_bravo_2" | grep -q '"applied_revision": 2' || fail "bravo did not pick up revision 2"

alpha_toml="$(compose exec -T host-alpha cat /opt/reforged/.workspace/config/host.toml)"
bravo_toml="$(compose exec -T host-bravo cat /opt/reforged/.workspace/config/host.toml)"
printf '%s\n' "$alpha_toml" | grep -q 'name = "Fleet Friday Hunts"' || fail "alpha host.toml missing updated name"
printf '%s\n' "$alpha_toml" | grep -q 'max_hunt_servers = 4' || fail "alpha host.toml missing updated max_hunt_servers"
printf '%s\n' "$alpha_toml" | grep -q '203.0.113.21' || fail "alpha host.toml lost its local game_address"
printf '%s\n' "$bravo_toml" | grep -q 'name = "Fleet Friday Hunts"' || fail "bravo host.toml missing updated name"
printf '%s\n' "$bravo_toml" | grep -q 'max_hunt_servers = 4' || fail "bravo host.toml missing updated max_hunt_servers"
printf '%s\n' "$bravo_toml" | grep -q '203.0.113.22' || fail "bravo host.toml lost its local game_address"

alpha_secrets_sha_after="$(compose exec -T host-alpha sha256sum /opt/reforged/.workspace/config/secrets.env | awk '{print $1}')"
bravo_secrets_sha_after="$(compose exec -T host-bravo sha256sum /opt/reforged/.workspace/config/secrets.env | awk '{print $1}')"
[ "$alpha_secrets_sha_before" = "$alpha_secrets_sha_after" ] || fail "alpha secrets.env changed during config sync"
[ "$bravo_secrets_sha_before" = "$bravo_secrets_sha_after" ] || fail "bravo secrets.env changed during config sync"
[ "$alpha_secrets_sha_after" != "$bravo_secrets_sha_after" ] || fail "alpha and bravo should have distinct local secrets.env files"

step "3. run a remote ban from the controller and verify it applies on alpha with the remote operator actor"
# Create a player invite on alpha so an RRK_ key also exists locally on alpha (and must never cross the wire)
player_invite_alpha="$(compose exec -T host-alpha dainty-host invite --for Griefer99 --json)"
echo "$player_invite_alpha"

ban_exec_out="$(compose exec -T host-controller dainty-host remote exec \
	--node alpha --action ban --account Griefer99 --reason "fleet rule violation" \
	--role admin --actor ctrl-admin --json)"
echo "$ban_exec_out"
printf '%s\n' "$ban_exec_out" | grep -q '"status": "queued"' || fail "remote ban was not queued"

compose exec -T host-alpha dainty-host remote sync --json
alpha_mod="$(compose exec -T host-alpha cat /opt/reforged/.workspace/config/remote-moderation.json)"
echo "$alpha_mod"
printf '%s\n' "$alpha_mod" | grep -q '"target": "Griefer99"' || fail "alpha moderation ledger missing Griefer99"
printf '%s\n' "$alpha_mod" | grep -q '"status": "banned"' || fail "alpha moderation ledger missing banned status"
printf '%s\n' "$alpha_mod" | grep -q '"actor": "remote:controller:ctrl-admin"' || fail "alpha moderation ledger missing remote actor remote:controller:ctrl-admin"

ctrl_status_after_ban="$(compose exec -T host-controller dainty-host remote status --json)"
echo "$ctrl_status_after_ban"
printf '%s\n' "$ctrl_status_after_ban" | grep -q 'banned applied for Griefer99 by remote:controller:ctrl-admin' \
	|| fail "controller did not receive completed ban outcome from alpha"

step "4. stop the controller and verify both managed installs keep running"
compose stop gateway-controller
compose exec -T backend-alpha reforged-healthcheck backend || fail "backend-alpha stopped when controller went offline"
compose exec -T gateway-alpha reforged-healthcheck gateway || fail "gateway-alpha stopped when controller went offline"
compose exec -T backend-bravo reforged-healthcheck backend || fail "backend-bravo stopped when controller went offline"
compose exec -T gateway-bravo reforged-healthcheck gateway || fail "gateway-bravo stopped when controller went offline"

# Syncing while the controller is stopped fails cleanly without wiping local config or detaching the install
if compose exec -T host-alpha dainty-host remote sync >/dev/null 2>&1; then
	fail "alpha sync unexpectedly succeeded while gateway-controller was stopped"
fi
alpha_status_offline="$(compose exec -T host-alpha dainty-host remote status --json)"
echo "$alpha_status_offline"
printf '%s\n' "$alpha_status_offline" | grep -q '"detached": false' || fail "alpha detached merely because controller was offline"
printf '%s\n' "$alpha_status_offline" | grep -q '"applied_revision": 2' || fail "alpha lost its applied_revision while controller was offline"

compose start gateway-controller
for _ in $(seq 1 30); do
	if compose exec -T gateway-controller reforged-healthcheck gateway >/dev/null 2>&1; then
		break
	fi
	sleep 1
done
compose exec -T gateway-controller reforged-healthcheck gateway || fail "gateway-controller did not recover after start"

step "5. revoke bravo and verify the controller refuses bravo while alpha continues syncing"
revoke_out="$(compose exec -T host-controller dainty-host remote revoke bravo --json)"
echo "$revoke_out"
printf '%s\n' "$revoke_out" | grep -q '"revoked": "bravo"' || fail "revoke did not report bravo"

if compose exec -T host-bravo dainty-host remote sync >/dev/null 2>&1; then
	fail "revoked install bravo was still able to sync with the controller"
fi
bravo_status_revoked="$(compose exec -T host-bravo dainty-host remote status --json)"
echo "$bravo_status_revoked"
printf '%s\n' "$bravo_status_revoked" | grep -q '"detached": true' || fail "bravo did not mark itself detached after revocation refusal"

if compose exec -T host-controller dainty-host remote exec --node bravo --action doctor --role admin --actor ctrl-admin >/dev/null 2>&1; then
	fail "controller allowed queueing a command for revoked install bravo"
fi

# Alpha can still sync after bravo is revoked (revoke increments controller revision from 2 to 3)
compose exec -T host-alpha dainty-host remote sync --json | grep -q '"applied_revision": 3' \
	|| fail "alpha could not sync after bravo was revoked"

step "6. inspect captured wire payloads to prove no secret, account key or database file crossed"
wire_controller="$(compose exec -T host-controller cat /opt/reforged/.workspace/wire.jsonl)"
wire_alpha="$(compose exec -T host-alpha cat /opt/reforged/.workspace/wire.jsonl)"
wire_bravo="$(compose exec -T host-bravo cat /opt/reforged/.workspace/wire.jsonl)"
all_wire="$(printf '%s\n%s\n%s\n' "$wire_controller" "$wire_alpha" "$wire_bravo")"

[ -n "$wire_controller" ] || fail "controller wire log is empty"
[ -n "$wire_alpha" ] || fail "alpha wire log is empty"
[ -n "$wire_bravo" ] || fail "bravo wire log is empty"

printf '%s\n' "$wire_alpha" | grep -q '^enroll_send:' || fail "alpha wire log missing enroll_send"
printf '%s\n' "$wire_alpha" | grep -q '^enroll_recv:' || fail "alpha wire log missing enroll_recv"
printf '%s\n' "$wire_alpha" | grep -q '^managed_send:' || fail "alpha wire log missing managed_send"
printf '%s\n' "$wire_alpha" | grep -q '^managed_recv:' || fail "alpha wire log missing managed_recv"
printf '%s\n' "$wire_controller" | grep -q '^controller_recv:' || fail "controller wire log missing controller_recv"
printf '%s\n' "$wire_controller" | grep -q '^controller_send:' || fail "controller wire log missing controller_send"

echo "wire log lines captured: controller=$(printf '%s\n' "$wire_controller" | wc -l), alpha=$(printf '%s\n' "$wire_alpha" | wc -l), bravo=$(printf '%s\n' "$wire_bravo" | wc -l)"

# Check forbidden markers across every captured wire byte
for marker in \
	"REFORGED_ADMIN_KEY" \
	"REFORGED_ADMIN_API_KEY" \
	"REFORGED_SECRET" \
	"GATEWAY_SECRET" \
	"ALLOWLIST_SECRET" \
	"RUSTIC_ISLES_MANAGER_OPERATOR_SECRET" \
	"REFORGED_DB_PASSWORD" \
	"RRK_" \
	"PRIVATE KEY" \
	"SQLite format 3" \
	"\$argon2" \
	"\$scrypt"
do
	if printf '%s\n' "$all_wire" | grep -Fq "$marker"; then
		fail "forbidden secret marker '$marker' appeared in wire log"
	fi
done

# Also extract the actual generated secret values from controller, alpha and bravo and verify none of those values crossed
for host_svc in host-controller host-alpha host-bravo; do
	secrets_text="$(compose exec -T "$host_svc" cat /opt/reforged/.workspace/config/secrets.env | tr -d '\r')"
	while IFS='=' read -r key val; do
		val="${val%\"}"
		val="${val#\"}"
		if [ -n "$val" ] && [ "${#val}" -ge 8 ]; then
			if printf '%s\n' "$all_wire" | grep -Fq "$val"; then
				fail "actual secret value for $key from $host_svc appeared in wire log"
			fi
		fi
	done < <(printf '%s\n' "$secrets_text" | grep -E '^[A-Z0-9_]+=')
done

echo "wire inspection passed: 0 secret markers and 0 secret values from secrets.env in $(printf '%s\n' "$all_wire" | wc -c) bytes of wire logs"
echo "remote management harness passed"
