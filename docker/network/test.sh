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
started=1
compose up -d --wait --wait-timeout 300 network
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

node_url() {
	local pub
	pub="$(compose port "net-$1" 443)"
	printf 'https://%s' "${pub/0.0.0.0/127.0.0.1}"
}

node_gs_key() {
	compose exec -T "gateway-$1" sed -n 's/^METAGAME_API_KEY="\{0,1\}\([^"]*\)"\{0,1\}$/\1/p' \
		/opt/reforged/.workspace/config/director.env | tr -d '\r\n'
}

node_backend_post() {
	local region="$1" path="$2" gs_key="$3" body="${4:-}"
	compose exec -T "gateway-$region" bash -c '
		path="$1"; gs_key="$2"; body="$3"; len="${#body}"
		exec 3<>/dev/tcp/127.0.0.1/61000
		printf "POST %s HTTP/1.1\r\nHost: 127.0.0.1:61000\r\nx-mysticparadox-gameserver-apikey: %s\r\nContent-Type: application/json\r\nContent-Length: %d\r\nConnection: close\r\n\r\n%s" \
			"$path" "$gs_key" "$len" "$body" >&3
		cat <&3
	' _ "$path" "$gs_key" "$body"
}

oregon_url="$(node_url oregon)"
germany_url="$(node_url germany)"

step "register and authenticate players on oregon-1 and germany-1"
reg_a="$(curl -sk -X POST "$oregon_url/reforged/api/Register" \
	-H 'Content-Type: application/json' \
	-d '{"Username":"SlayerOregon"}')"
echo "oregon register: $reg_a"
key_a="$(printf '%s' "$reg_a" | sed -n 's/.*"UUK":"\([^"]*\)".*/\1/p')"
[ -n "$key_a" ] || fail "failed to register Player A on oregon-1: $reg_a"

info_a="$(curl -sk -i "$oregon_url/reforged/api/GetUserInfo" \
	-H "x-reforged-user-api-key: $key_a")"
echo "oregon user info:"
echo "$info_a"
uid_a="$(printf '%s' "$info_a" | sed -n 's/.*"UserId":"\([^"]*\)".*/\1/p' | head -n 1)"
acct_a="$(printf '%s' "$info_a" | tr -d '\r' | sed -n 's/^[xX]-reforged-account-id: *//p' | head -n 1)"
[ -n "$uid_a" ] && [ -n "$acct_a" ] || fail "failed to read Player A user info on oregon-1"

tok_a_json="$(curl -sk -X POST "$oregon_url/account/api/oauth/token" \
	-H 'Content-Type: application/x-www-form-urlencoded' \
	-d "grant_type=exchange_code&exchange_code=$key_a")"
tok_a="$(printf '%s' "$tok_a_json" | sed -n 's/.*"access_token":"\([^"]*\)".*/\1/p')"
[ -n "$tok_a" ] || fail "failed to obtain access token for Player A: $tok_a_json"

reg_b="$(curl -sk -X POST "$germany_url/reforged/api/Register" \
	-H 'Content-Type: application/json' \
	-d '{"Username":"SlayerGermany"}')"
echo "germany register: $reg_b"
key_b="$(printf '%s' "$reg_b" | sed -n 's/.*"UUK":"\([^"]*\)".*/\1/p')"
[ -n "$key_b" ] || fail "failed to register Player B on germany-1: $reg_b"

info_b="$(curl -sk -i "$germany_url/reforged/api/GetUserInfo" \
	-H "x-reforged-user-api-key: $key_b")"
echo "germany user info:"
echo "$info_b"
uid_b="$(printf '%s' "$info_b" | sed -n 's/.*"UserId":"\([^"]*\)".*/\1/p' | head -n 1)"
acct_b="$(printf '%s' "$info_b" | tr -d '\r' | sed -n 's/^[xX]-reforged-account-id: *//p' | head -n 1)"
[ -n "$uid_b" ] && [ -n "$acct_b" ] || fail "failed to read Player B user info on germany-1"

tok_b_json="$(curl -sk -X POST "$germany_url/account/api/oauth/token" \
	-H 'Content-Type: application/x-www-form-urlencoded' \
	-d "grant_type=exchange_code&exchange_code=$key_b")"
tok_b="$(printf '%s' "$tok_b_json" | sed -n 's/.*"access_token":"\([^"]*\)".*/\1/p')"
[ -n "$tok_b" ] || fail "failed to obtain access token for Player B: $tok_b_json"

step "DN8: cross-node matchmaking ring places players on one hunt and forwards rewards to home node"
mm_a="$(curl -sk -X POST "$oregon_url/candidate/join" \
	-H "Authorization: Bearer $tok_a" \
	-H 'Content-Type: application/json' \
	-d '{"gameMode":"ISLAND","playerHuntId":"CR19_PlayerHunt_Example"}')"
echo "oregon matchmaking: $mm_a"
mm_b="$(curl -sk -X POST "$germany_url/candidate/join" \
	-H "Authorization: Bearer $tok_b" \
	-H 'Content-Type: application/json' \
	-d '{"gameMode":"ISLAND","playerHuntId":"CR19_PlayerHunt_Example"}')"
echo "germany matchmaking: $mm_b"

cand_a=""
cand_b=""
for _ in $(seq 1 40); do
	cand_a="$(curl -sk "$oregon_url/candidate/status" -H "Authorization: Bearer $tok_a")"
	cand_b="$(curl -sk "$germany_url/candidate/status" -H "Authorization: Bearer $tok_b")"
	if printf '%s' "$cand_a" | grep -q '"status":"IN_PROGRESS"' && printf '%s' "$cand_b" | grep -q '"status":"IN_PROGRESS"'; then
		break
	fi
	sleep 0.5
done
echo "oregon candidate:  $cand_a"
echo "germany candidate: $cand_b"
printf '%s' "$cand_a" | grep -q '"status":"IN_PROGRESS"' || fail "Player A candidate never became ready"
printf '%s' "$cand_b" | grep -q '"status":"IN_PROGRESS"' || fail "Player B candidate never became ready"

sess_a="$(printf '%s' "$cand_a" | sed -n 's/.*"gameSessionId":"\([^"]*\)".*/\1/p')"
sess_b="$(printf '%s' "$cand_b" | sed -n 's/.*"gameSessionId":"\([^"]*\)".*/\1/p')"
host_a="$(printf '%s' "$cand_a" | sed -n 's/.*"host":"\([^"]*\)".*/\1/p')"
host_b="$(printf '%s' "$cand_b" | sed -n 's/.*"host":"\([^"]*\)".*/\1/p')"
[ -n "$sess_a" ] && [ "$sess_a" = "$sess_b" ] || fail "ring gameSessionId mismatch: $sess_a vs $sess_b"
[ -n "$host_a" ] && [ "$host_a" = "$host_b" ] || fail "ring host mismatch: $host_a vs $host_b"
echo "matched into ring session $sess_a on $host_a"

case "$sess_a" in
	*@oregon-1)
		host_region=oregon
		remote_region=germany
		remote_url="$germany_url"
		remote_tok="$tok_b"
		remote_uid="$uid_b"
		remote_acct="$acct_b"
		;;
	*@germany-1)
		host_region=germany
		remote_region=oregon
		remote_url="$oregon_url"
		remote_tok="$tok_a"
		remote_uid="$uid_a"
		remote_acct="$acct_a"
		;;
	*@new-york-1)
		host_region=new-york
		remote_region=germany
		remote_url="$germany_url"
		remote_tok="$tok_b"
		remote_uid="$uid_b"
		remote_acct="$acct_b"
		;;
	*@uk-1)
		host_region=uk
		remote_region=oregon
		remote_url="$oregon_url"
		remote_tok="$tok_a"
		remote_uid="$uid_a"
		remote_acct="$acct_a"
		;;
	*) fail "unexpected ring session owner in $sess_a" ;;
esac

gs_key="$(node_gs_key "$host_region")"
[ -n "$gs_key" ] || fail "missing METAGAME_API_KEY on $host_region"

xp_resp="$(node_backend_post "$host_region" "/progression/$remote_acct/ExperienceTrack_PlayerLevel/250" "$gs_key" '{}')"
echo "forwarded XP response ($host_region -> $remote_region): $(printf '%s' "$xp_resp" | tail -n 1)"
printf '%s' "$xp_resp" | grep -q 'HTTP/1.1 200' || fail "XP forwarding failed: $xp_resp"

inv_body="{\"accountId\":\"$remote_acct\",\"characterId\":\"char-$remote_acct\",\"transactionId\":\"tx-ring-loot-1\",\"addStackedItems\":[{\"catalogId\":\"MonsterPart_Gnasher_Tail\",\"quantity\":15}]}"
inv_resp="$(node_backend_post "$host_region" "/inventory" "$gs_key" "$inv_body")"
echo "forwarded loot response ($host_region -> $remote_region): $(printf '%s' "$inv_resp" | tail -n 1)"
printf '%s' "$inv_resp" | grep -q '"forwarded_to_home":true' || fail "loot forwarding failed: $inv_resp"

bounty_body='{"bounties":[{"bounty_id":"BOUNTY_RING_TEST","progress":1}]}'
bounty_resp="$(node_backend_post "$host_region" "/bounty/$remote_acct" "$gs_key" "$bounty_body")"
echo "forwarded bounty response ($host_region -> $remote_region): $(printf '%s' "$bounty_resp" | tail -n 1)"
printf '%s' "$bounty_resp" | grep -q 'HTTP/1.1 200' || fail "bounty forwarding failed: $bounty_resp"

home_prog="$(curl -sk "$remote_url/progression/$remote_uid" -H "Authorization: Bearer $remote_tok")"
echo "home progression ($remote_region): $home_prog"
printf '%s' "$home_prog" | grep -q '250' || fail "forwarded XP not found on home node $remote_region: $home_prog"

home_inv="$(curl -sk "$remote_url/inventory/$remote_uid/char-$remote_acct" -H "Authorization: Bearer $remote_tok")"
echo "home inventory ($remote_region): $home_inv"
printf '%s' "$home_inv" | grep -q 'MonsterPart_Gnasher_Tail' || fail "forwarded loot not found on home node $remote_region: $home_inv"

home_bounty="$(curl -sk "$remote_url/bounty/$remote_uid" -H "Authorization: Bearer $remote_tok")"
echo "home bounty ($remote_region): $home_bounty"
printf '%s' "$home_bounty" | grep -q 'BOUNTY_RING_TEST' || fail "forwarded bounty not found on home node $remote_region: $home_bounty"

step "DN6: wrong-node join refuses by default and redirects with a session ticket when configured"
imm_a="$(curl -sk -X POST "$oregon_url/candidate/join" \
	-H "Authorization: Bearer $tok_a" \
	-H 'Content-Type: application/json' \
	-d '{"gameMode":"ISLAND","playerHuntId":"Example_Hunt_Easy"}')"
echo "oregon immediate hunt: $imm_a"
oregon_inst=""
for _ in $(seq 1 20); do
	cand_imm="$(curl -sk "$oregon_url/candidate/status" -H "Authorization: Bearer $tok_a")"
	if printf '%s' "$cand_imm" | grep -q '"status":"IN_PROGRESS"'; then
		oregon_inst="$(printf '%s' "$cand_imm" | sed -n 's/.*"gameSessionId":"\([^"]*\)".*/\1/p')"
		break
	fi
	sleep 0.5
done
[ -n "$oregon_inst" ] || fail "failed to launch immediate instance on oregon-1"
echo "oregon instance registered in ring: $oregon_inst"

sleep 1
refuse_resp="$(curl -sk -i -X POST "$germany_url/candidate/join" \
	-H "Authorization: Bearer $tok_b" \
	-H 'Content-Type: application/json' \
	-d "{\"gameMode\":\"ISLAND\",\"instanceId\":\"$oregon_inst\"}")"
echo "wrong-node refuse response:"
echo "$refuse_resp"
printf '%s' "$refuse_resp" | grep -q '409' || fail "expected 409 Conflict under default refuse policy"
printf '%s' "$refuse_resp" | grep -q 'WRONG_NODE' || fail "expected WRONG_NODE body under default refuse policy"

compose exec -T gateway-germany sh -c '
	sed "/^\[network\]$/a wrong_node_policy = \"redirect\"" /opt/reforged/.workspace/config/host.toml >/tmp/host.toml
	cat /tmp/host.toml >/opt/reforged/.workspace/config/host.toml
	rm -f /tmp/host.toml
'
compose restart gateway-germany
for _ in $(seq 1 30); do
	if curl -sk "$germany_url/reforged/api/ServerStatus" | grep -q '"online":true'; then
		break
	fi
	sleep 1
done

imm_a2="$(curl -sk -X POST "$oregon_url/candidate/join" \
	-H "Authorization: Bearer $tok_a" \
	-H 'Content-Type: application/json' \
	-d '{"gameMode":"ISLAND","playerHuntId":"Example_Hunt_Easy"}')"
oregon_inst2=""
for _ in $(seq 1 20); do
	cand_imm2="$(curl -sk "$oregon_url/candidate/status" -H "Authorization: Bearer $tok_a")"
	if printf '%s' "$cand_imm2" | grep -q '"status":"IN_PROGRESS"'; then
		oregon_inst2="$(printf '%s' "$cand_imm2" | sed -n 's/.*"gameSessionId":"\([^"]*\)".*/\1/p')"
		break
	fi
	sleep 0.5
done
[ -n "$oregon_inst2" ] || fail "failed to launch second immediate instance on oregon-1"
sleep 1

redir_resp="$(curl -sk -i -X POST "$germany_url/candidate/join" \
	-H "Authorization: Bearer $tok_b" \
	-H 'Content-Type: application/json' \
	-d "{\"gameMode\":\"ISLAND\",\"instanceId\":\"$oregon_inst2\"}")"
echo "wrong-node redirect response:"
echo "$redir_resp"
printf '%s' "$redir_resp" | grep -q '"hostNode":"oregon-1"' \
	|| fail "expected hostNode oregon-1 in redirect response"
session_ticket="$(printf '%s' "$redir_resp" | tr -d '\r' | sed -n 's/^[xX]-reforged-session-ticket: *//p' | head -n 1)"
[ -n "$session_ticket" ] || fail "missing x-reforged-session-ticket header on redirect"

admit_session="$(curl -sk -i -X PUT "$oregon_url/gamesession/epiceos" \
	-H "x-reforged-session-ticket: $session_ticket" \
	-H 'Content-Type: application/json' \
	-d '{}')"
echo "target-node game session with session ticket (no account key):"
echo "$admit_session"
printf '%s' "$admit_session" | head -n 1 | grep -q '200' || fail "target node rejected session ticket at /gamesession/epiceos"
printf '%s' "$admit_session" | grep -q '"sessiontoken"' || fail "target node did not return session token"

admit_join="$(curl -sk -i -X POST "$oregon_url/candidate/join" \
	-H "x-reforged-session-ticket: $session_ticket" \
	-H 'Content-Type: application/json' \
	-d "{\"gameMode\":\"ISLAND\",\"instanceId\":\"$oregon_inst2\"}")"
echo "target-node instance join with session ticket (no account key):"
echo "$admit_join"
printf '%s' "$admit_join" | head -n 1 | grep -q '200' || fail "target node rejected session ticket at /candidate/join"
printf '%s' "$admit_join" | grep -q "\"instanceId\":\"$oregon_inst2\"" || fail "target node did not join $oregon_inst2"

step "DN9: regional Ramsgate places player on the lowest-RTT healthy node"
regions_de_resp="$(curl -sk -i -X POST "$oregon_url/network/v1/regions" \
	-H "x-reforged-user-api-key: $key_a" \
	-H 'Content-Type: application/json' \
	-d '{"regions":[{"region":"germany","rttMs":12},{"region":"uk","rttMs":35},{"region":"new-york","rttMs":85},{"region":"oregon","rttMs":150}]}')"
echo "reported regions (germany nearest) response:"
echo "$regions_de_resp"
printf '%s' "$regions_de_resp" | head -n 1 | grep -q '204' || fail "expected 204 from POST /network/v1/regions, got: $regions_de_resp"

city_de="$(curl -sk -X POST "$oregon_url/candidate/join" \
	-H "Authorization: Bearer $tok_a" \
	-H 'Content-Type: application/json' \
	-d '{"gameMode":"CITY","playerHuntId":"ShatteredIsles_ReturnToRamsgate"}')"
echo "city matchmaking (germany nearest): $city_de"
cand_city_de=""
for _ in $(seq 1 20); do
	cand_city_de="$(curl -sk "$oregon_url/candidate/status" -H "Authorization: Bearer $tok_a")"
	if printf '%s' "$cand_city_de" | grep -q '"status":"IN_PROGRESS"'; then
		break
	fi
	sleep 0.2
done
echo "city candidate (germany nearest): $cand_city_de"
printf '%s' "$cand_city_de" | grep -q '"host":"203.0.113.14"' \
	|| fail "expected Germany Ramsgate hub (203.0.113.14), got: $cand_city_de"

regions_or_resp="$(curl -sk -i -X POST "$oregon_url/network/v1/regions" \
	-H "x-reforged-user-api-key: $key_a" \
	-H 'Content-Type: application/json' \
	-d '{"regions":[{"region":"oregon","rttMs":8},{"region":"new-york","rttMs":70},{"region":"uk","rttMs":140},{"region":"germany","rttMs":150}]}')"
echo "reported regions (oregon nearest) response:"
echo "$regions_or_resp"
printf '%s' "$regions_or_resp" | head -n 1 | grep -q '204' || fail "expected 204 from POST /network/v1/regions, got: $regions_or_resp"

city_or="$(curl -sk -X POST "$oregon_url/candidate/join" \
	-H "Authorization: Bearer $tok_a" \
	-H 'Content-Type: application/json' \
	-d '{"gameMode":"CITY","playerHuntId":"ShatteredIsles_ReturnToRamsgate"}')"
echo "city matchmaking (oregon nearest): $city_or"
cand_city_or=""
for _ in $(seq 1 20); do
	cand_city_or="$(curl -sk "$oregon_url/candidate/status" -H "Authorization: Bearer $tok_a")"
	if printf '%s' "$cand_city_or" | grep -q '"status":"IN_PROGRESS"'; then
		break
	fi
	sleep 0.2
done
echo "city candidate (oregon nearest): $cand_city_or"
printf '%s' "$cand_city_or" | grep -q '"host":"203.0.113.11"' \
	|| fail "expected Oregon Ramsgate hub (203.0.113.11), got: $cand_city_or"

echo "network harness passed"
