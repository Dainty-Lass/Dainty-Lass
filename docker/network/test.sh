#!/usr/bin/env bash
# DaintyLass: the network harness's test (DN12, docs/hosting-docker.md). From the repository root:
#   bash docker/network/test.sh
# Brings up compose.yaml's `network` profile (four nodes, the canary's regions, tc/netem between them) in throwaway
# volumes and checks that
#  1. every node reaches its three peers over the node link (mutual TLS) with a round trip near the injected one:
#     `reforged-host network status` run inside each node
#  2. the launcher's region ping, the real `region_ping` of rust/launcher (a cargo test, ignored by default), sees the
#     regions in the order the injected latencies give, from a player in NETEM_PLAYER_REGION, with each node as home
# Then it tears everything down. Environment:
#   NETWORK_NO_BUILD=1       the images are built already
#   NETWORK_OPTIONAL=1       exit 0 with a note, instead of failing, when Docker or NET_ADMIN is unavailable
#   NETWORK_REGION_PING=     auto (default: run it when the nodes' addresses and cargo are reachable, else say so and
#                            skip), require (CI) or skip. The addresses are the private network's: a Linux host
#                            reaches them, Docker Desktop does not
#   NETWORK_TOLERANCE_MS=    how far above the injected round trip a measurement may be (25)
#   REFORGED_NETWORK_*       compose.yaml's settings; the table this expects is read back from `docker compose config`
set -euo pipefail

export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-reforged-network}"
export COMPOSE_PROFILES=network
export COMPOSE_FILE=compose.yaml
tolerance="${NETWORK_TOLERANCE_MS:-25}"
region_ping="${NETWORK_REGION_PING:-auto}"
regions=(oregon new-york uk germany)

compose() { docker compose "$@"; }
step() { printf '\n== %s\n' "$*"; }
fail() { echo "network test failed: $*" >&2; exit 1; }
unavailable() {
	if [ "${NETWORK_OPTIONAL:-0}" = 1 ]; then
		echo "network harness skipped: $*"
		exit 0
	fi
	fail "$*"
}

{ command -v docker >/dev/null 2>&1 && docker compose version >/dev/null 2>&1 && docker info >/dev/null 2>&1; } \
	|| unavailable "Docker with Compose is not available"

started=0
cleanup() {
	status=$?
	if [ "$status" -ne 0 ] && [ "$started" = 1 ]; then
		compose ps -a || true
		compose logs --no-color --tail 40 || true
	fi
	compose down --volumes --remove-orphans >/dev/null 2>&1 || true
	exit "$status"
}
trap cleanup EXIT

if [ "${NETWORK_NO_BUILD:-0}" != 1 ]; then
	step build
	compose build net-oregon configure-oregon netem-oregon
fi

step "NET_ADMIN and netem"
netem_image="$(compose config --images | grep -- '-netem:' | head -n 1)"
docker run --rm --cap-add NET_ADMIN --entrypoint sh "$netem_image" -c 'tc qdisc add dev lo root netem delay 1ms' >/dev/null 2>&1 \
	|| unavailable "this Docker cannot add a netem qdisc in a container (CAP_NET_ADMIN, or the sch_netem kernel module, is missing)"

# What the nodes were told, read back from compose: the pairs' round trips in ms, and where the player is
config="$(compose config --format json)"
table="$(printf '%s\n' "$config" | sed -n 's/^ *"NETEM_RTT": *"\(.*\)",*$/\1/p' | head -n 1)"
player="$(printf '%s\n' "$config" | sed -n 's/^ *"NETEM_PLAYER_REGION": *"\(.*\)",*$/\1/p' | head -n 1)"
[ -n "$table" ] && [ -n "$player" ] || fail "compose config has no NETEM_RTT / NETEM_PLAYER_REGION"
echo "round trips: $table"
echo "player in:   $player"

# The round trip injected between two regions
injected() {
	local a="$1" b="$2" pair
	if [ "$a" = "$b" ]; then echo 0; return; fi
	for pair in $table; do
		case "$pair" in
		"$a:$b="* | "$b:$a="*) echo "${pair#*=}"; return ;;
		esac
	done
	fail "no round trip for $a and $b in $table"
}

step up
compose up -d --wait --wait-timeout 300 network
started=1
compose ps

# Never faster than injected (less a timer step); over by the machine's own time
within() { awk -v v="$1" -v want="$2" -v tol="$3" 'BEGIN { exit !(v >= want - 2 && v <= want + tol) }'; }

step "every node reaches its peers, with the injected round trips"
for region in "${regions[@]}"; do
	out="$(compose exec -T "gateway-$region" reforged-host network status --count 5 \
		--env-file /opt/reforged/.workspace/config/gateway.env)" || { echo "$out"; fail "$region: a peer is not up"; }
	echo "$out"
	peers=0
	while read -r _ node peer state _ rtt unit _; do
		[ "$unit" = ms ] || fail "$region -> $peer ($node): no round trip ($state)"
		want="$(injected "$region" "$peer")"
		[ "$state" = up ] || fail "$region -> $peer is $state"
		within "$rtt" "$want" "$tolerance" || fail "$region -> $peer: $rtt ms, injected $want ms (+$tolerance)"
		peers=$((peers + 1))
	done < <(printf '%s\n' "$out" | grep '^Peer:')
	[ "$peers" = 3 ] || fail "$region sees $peers peers, not 3"
done

# The launcher's region ping: a player in $player (egress to the Docker host is delayed by the round trip), each node home
step "the launcher's region ping follows the injected latencies"
run_region_ping=1
if [ "$region_ping" = skip ]; then
	run_region_ping=0
	echo "skipped (NETWORK_REGION_PING=skip)"
else
	node_address="$(printf '%s\n' "$config" | sed -n 's/.*"NETEM_NODES": *"oregon=\([0-9.]*\) .*/\1/p' | head -n 1)"
	why=""
	if ! command -v cargo >/dev/null 2>&1; then
		why="cargo is not installed"
	elif ! timeout 3 bash -c "exec 3<>/dev/tcp/$node_address/443" 2>/dev/null; then
		why="the nodes' addresses ($node_address ...) are not reachable from this host (Docker Desktop keeps them in its VM)"
	fi
	if [ -n "$why" ]; then
		[ "$region_ping" = require ] && fail "region ping: $why"
		run_region_ping=0
		echo "skipped: $why"
	fi
fi

if [ "$run_region_ping" = 1 ]; then
	for home in "${regions[@]}"; do
		published="$(compose port "net-$home" 443)"
		pin="$(compose exec -T "gateway-$home" reforged-host gateway make-cert --fingerprint \
			/opt/reforged/.workspace/tls/gateway-cert.pem | sed -n 's/^Fingerprint: *//p')"
		# The other regions, nearest to the player first, each with the round trip injected to it
		expect="$(for r in "${regions[@]}"; do
			[ "$r" = "$home" ] || printf '%s %s\n' "$(injected "$player" "$r")" "$r"
		done | sort -n | awk '{ printf "%s%s=%s", sep, $2, $1; sep = "," }')"
		echo "home $home ($published): expecting $expect"
		(
			cd rust
			RR_HARNESS_HOME="${published/0.0.0.0/127.0.0.1}" RR_HARNESS_PIN="$pin" RR_HARNESS_EXPECT="$expect" \
				RR_HARNESS_TOLERANCE_MS="$tolerance" \
				cargo test -p launcher --lib --locked -- --ignored --nocapture the_region_order_follows
		) || fail "region ping with $home as the home"
	done
fi

echo "network harness passed"
