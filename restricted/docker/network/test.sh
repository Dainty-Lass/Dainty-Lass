#!/usr/bin/env bash
# Dainty Lass: the network harness's test (DN12, docs/hosting-docker.md). From the repository root:
#   bash docker/network/test.sh
# Brings up compose.yaml's `network` profile (four nodes, the canary's regions, tc/netem between them) in throwaway
# volumes and checks that
#  1. every node reaches its three peers over the node link (mutual TLS) with a round trip near the injected one:
#     `dainty-host network status` run inside each node
#  2. the launcher's region ping, the real `region_ping` of launcher (a cargo test, ignored by default), sees the
#     regions in the order the injected latencies give, from a player in NETEM_PLAYER_REGION, with each node as home
# Then it tears everything down. Environment:
#   NETWORK_NO_BUILD=1       the images are built already
#   NETWORK_OPTIONAL=1       exit 0 with a note, instead of failing, when Docker or NET_ADMIN is unavailable
#   NETWORK_REGION_PING=     auto (default: run it when the nodes' addresses and cargo are reachable, else say so and
#                            skip), require (CI) or skip. The addresses are the private network's: a Linux host
#                            reaches them, Docker Desktop does not
#   NETWORK_TOLERANCE_MS=    how far above the injected round trip a measurement may be (25)
#   DAINTY_NETWORK_*       compose.yaml's settings; the table this expects is read back from `docker compose config`
set -euo pipefail
# Git Bash on Windows would rewrite the /opt/... paths handed to docker into Windows ones
export MSYS_NO_PATHCONV=1

export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-dainty-network}"
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
	for svc in net-oregon configure-oregon netem-oregon; do
		compose build "$svc"
	done
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
	out="$(compose exec -T "gateway-$region" dainty-host network status --count 5 \
		--env-file /opt/dainty-lass/.workspace/config/gateway.env)" || { echo "$out"; fail "$region: a peer is not up"; }
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
		pin="$(compose exec -T "gateway-$home" dainty-host gateway make-cert --fingerprint \
			/opt/dainty-lass/.workspace/tls/gateway-cert.pem | sed -n 's/^Fingerprint: *//p')"
		# The other regions, nearest to the player first, each with the round trip injected to it
		expect="$(for r in "${regions[@]}"; do
			[ "$r" = "$home" ] || printf '%s %s\n' "$(injected "$player" "$r")" "$r"
		done | sort -n | awk '{ printf "%s%s=%s", sep, $2, $1; sep = "," }')"
		echo "home $home ($published): expecting $expect"
		(
			RR_HARNESS_HOME="${published/0.0.0.0/127.0.0.1}" RR_HARNESS_PIN="$pin" RR_HARNESS_EXPECT="$expect" \
				RR_HARNESS_TOLERANCE_MS="$tolerance" \
				cargo test -p dainty-launcher --lib --locked -- --ignored --nocapture the_region_order_follows
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
		/opt/dainty-lass/.workspace/config/director.env | tr -d '\r\n'
}

node_backend_post() {
	local region="$1" path="$2" gs_key="$3" body="${4:-}"
	printf '%s' "$body" | compose exec -T "gateway-$region" bash -c '
		path="$1"; gs_key="$2"; body="$(cat)"; len="${#body}"
		exec 3<>/dev/tcp/127.0.0.1/41000
		printf "POST %s HTTP/1.1\r\nHost: 127.0.0.1:41000\r\nx-rustic-isles-gameserver-apikey: %s\r\nContent-Type: application/json\r\nContent-Length: %d\r\nConnection: close\r\n\r\n%s" \
			"$path" "$gs_key" "$len" "$body" >&3
		cat <&3
	' _ "$path" "$gs_key"
}

node_chat_post() {
	local region="$1" path="$2" body="${3-}"
	[ -n "$body" ] || body='{}'
	compose exec -T "gateway-$region" bash -c '
		path="$1"; body="$2"; len="${#body}"
		exec 3<>/dev/tcp/127.0.0.1/41006
		printf "POST %s HTTP/1.1\r\nHost: 127.0.0.1:41006\r\nContent-Type: application/json\r\nContent-Length: %d\r\nConnection: close\r\n\r\n%s" \
			"$path" "$len" "$body" >&3
		cat <&3
	' _ "$path" "$body" | tr -d '\r' | tail -n 1
}

node_chat_get() {
	local region="$1" path="$2"
	compose exec -T "gateway-$region" bash -c '
		path="$1"
		exec 3<>/dev/tcp/127.0.0.1/41006
		printf "GET %s HTTP/1.1\r\nHost: 127.0.0.1:41006\r\nConnection: close\r\n\r\n" "$path" >&3
		cat <&3
	' _ "$path" | tr -d '\r' | tail -n 1
}

oregon_url="$(node_url oregon)"
germany_url="$(node_url germany)"

step "register and authenticate players on oregon-1 and germany-1"
reg_a="$(curl -sk -X POST "$oregon_url/dainty/api/Register" \
	-H 'Content-Type: application/json' \
	-d '{"Username":"SlayerOregon"}')"
echo "oregon register: $reg_a"
key_a="$(printf '%s' "$reg_a" | sed -n 's/.*"UUK":"\([^"]*\)".*/\1/p')"
[ -n "$key_a" ] || fail "failed to register Player A on oregon-1: $reg_a"

info_a="$(curl -sk -i "$oregon_url/dainty/api/GetUserInfo" \
	-H "x-dainty-user-api-key: $key_a")"
echo "oregon user info:"
echo "$info_a"
uid_a="$(printf '%s' "$info_a" | sed -n 's/.*"UserId":"\([^"]*\)".*/\1/p' | head -n 1)"
acct_a="$(printf '%s' "$info_a" | tr -d '\r' | sed -n 's/^[xX]-dainty-account-id: *//p' | head -n 1)"
[ -n "$uid_a" ] && [ -n "$acct_a" ] || fail "failed to read Player A user info on oregon-1"

tok_a_json="$(curl -sk -X POST "$oregon_url/account/api/oauth/token" \
	-H 'Content-Type: application/x-www-form-urlencoded' \
	-d "grant_type=exchange_code&exchange_code=$key_a")"
tok_a="$(printf '%s' "$tok_a_json" | sed -n 's/.*"access_token":"\([^"]*\)".*/\1/p')"
[ -n "$tok_a" ] || fail "failed to obtain access token for Player A: $tok_a_json"

reg_b="$(curl -sk -X POST "$germany_url/dainty/api/Register" \
	-H 'Content-Type: application/json' \
	-d '{"Username":"SlayerGermany"}')"
echo "germany register: $reg_b"
key_b="$(printf '%s' "$reg_b" | sed -n 's/.*"UUK":"\([^"]*\)".*/\1/p')"
[ -n "$key_b" ] || fail "failed to register Player B on germany-1: $reg_b"

info_b="$(curl -sk -i "$germany_url/dainty/api/GetUserInfo" \
	-H "x-dainty-user-api-key: $key_b")"
echo "germany user info:"
echo "$info_b"
uid_b="$(printf '%s' "$info_b" | sed -n 's/.*"UserId":"\([^"]*\)".*/\1/p' | head -n 1)"
acct_b="$(printf '%s' "$info_b" | tr -d '\r' | sed -n 's/^[xX]-dainty-account-id: *//p' | head -n 1)"
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
	sed "/^\[network\]$/a wrong_node_policy = \"redirect\"" /opt/dainty-lass/.workspace/config/settings.toml >/tmp/settings.toml
	cat /tmp/settings.toml >/opt/dainty-lass/.workspace/config/settings.toml
	rm -f /tmp/settings.toml
'
compose restart gateway-germany
for _ in $(seq 1 30); do
	if curl -sk "$germany_url/dainty/api/ServerStatus" | grep -q '"online":true'; then
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
session_ticket="$(printf '%s' "$redir_resp" | tr -d '\r' | sed -n 's/^[xX]-dainty-session-ticket: *//p' | head -n 1)"
[ -n "$session_ticket" ] || fail "missing x-dainty-session-ticket header on redirect"

admit_session="$(curl -sk -i -X PUT "$oregon_url/gamesession/epiceos" \
	-H "x-dainty-session-ticket: $session_ticket" \
	-H 'Content-Type: application/json' \
	-d '{}')"
echo "target-node game session with session ticket (no account key):"
echo "$admit_session"
printf '%s' "$admit_session" | head -n 1 | grep -q '200' || fail "target node rejected session ticket at /gamesession/epiceos"
printf '%s' "$admit_session" | grep -q '"sessiontoken"' || fail "target node did not return session token"

admit_join="$(curl -sk -i -X POST "$oregon_url/candidate/join" \
	-H "x-dainty-session-ticket: $session_ticket" \
	-H 'Content-Type: application/json' \
	-d "{\"gameMode\":\"ISLAND\",\"instanceId\":\"$oregon_inst2\"}")"
echo "target-node instance join with session ticket (no account key):"
echo "$admit_join"
printf '%s' "$admit_join" | head -n 1 | grep -q '200' || fail "target node rejected session ticket at /candidate/join"
printf '%s' "$admit_join" | grep -q "\"instanceId\":\"$oregon_inst2\"" || fail "target node did not join $oregon_inst2"

step "DN9: regional Ramsgate places player on the lowest-RTT healthy node"
regions_de_resp="$(curl -sk -i -X POST "$oregon_url/network/v1/regions" \
	-H "x-dainty-user-api-key: $key_a" \
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
	-H "x-dainty-user-api-key: $key_a" \
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

step "DN10b: chat federation across nodes (DMs, offline delivery on foreign login, room fanout, mutes)"
ws_upgrade="$(curl -sk -i --http1.1 --max-time 2 "$oregon_url//" \
	-H "Connection: Upgrade" \
	-H "Upgrade: websocket" \
	-H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" \
	-H "Sec-WebSocket-Version: 13" \
	-H "Sec-WebSocket-Protocol: xmpp" || true)"
echo "gateway WebSocket upgrade header:"
echo "$ws_upgrade" | head -n 6
printf '%s' "$ws_upgrade" | grep -q "101 Switching Protocols" || fail "gateway did not upgrade WebSocket to chat plugin: $ws_upgrade"
printf '%s' "$ws_upgrade" | grep -iq "sec-websocket-protocol: xmpp" || fail "chat plugin did not negotiate xmpp subprotocol: $ws_upgrade"

# 1. Player A logs into chat on oregon-1 and Player B logs into chat on germany-1; A whispers B
chat_login_a="$(node_chat_post oregon "/__chat/client/login" "{\"handle\":\"ada\",\"authcid\":\"$uid_a\",\"token\":\"$tok_a\"}")"
echo "chat login Player A on oregon-1: $chat_login_a"
printf '%s' "$chat_login_a" | grep -q '"ok":true' || fail "Player A chat login on oregon-1 failed: $chat_login_a"

chat_login_b="$(node_chat_post germany "/__chat/client/login" "{\"handle\":\"bo\",\"authcid\":\"$uid_b\",\"token\":\"$tok_b\"}")"
echo "chat login Player B on germany-1: $chat_login_b"
printf '%s' "$chat_login_b" | grep -q '"ok":true' || fail "Player B chat login on germany-1 failed: $chat_login_b"

dm_send="$(node_chat_post oregon "/__chat/client/send" "{\"handle\":\"ada\",\"whisper_to\":\"$uid_b\",\"body\":\"hello across the atlantic\"}")"
echo "oregon whisper send: $dm_send"
dm_recv="$(node_chat_get germany "/__chat/client/drain?handle=bo&expect=hello%20across%20the%20atlantic&timeout_ms=6000")"
echo "germany whisper recv: $dm_recv"
printf '%s' "$dm_recv" | grep -q '"matched":true' || fail "Player B on germany-1 did not receive whisper from oregon-1: $dm_recv"
printf '%s' "$dm_recv" | grep -q 'hello across the atlantic' || fail "missing whisper body on germany-1: $dm_recv"

dm_extra="$(node_chat_get germany "/__chat/client/drain?handle=bo&wait_ms=300")"
if printf '%s' "$dm_extra" | grep -q 'hello across the atlantic'; then
	fail "whisper arrived more than once on germany-1: $dm_extra"
fi

hist_or="$(node_chat_get oregon "/__chat/history/direct?a=$acct_a&b=$acct_b")"
hist_de="$(node_chat_get germany "/__chat/history/direct?a=$acct_a&b=$acct_b")"
echo "oregon direct history:  $hist_or"
echo "germany direct history: $hist_de"
printf '%s' "$hist_or" | grep -q 'hello across the atlantic' || fail "oregon-1 history missing direct message: $hist_or"
printf '%s' "$hist_de" | grep -q 'hello across the atlantic' || fail "germany-1 history missing direct message: $hist_de"

# 2. Player B goes offline; Player A on oregon-1 whispers offline Player B (homed on germany-1);
#    Player B joins a hunt on a third node (uk-1), logs into chat on uk-1 with the home-signed session
#    ticket, and receives the waiting DM once with <delay/>
node_chat_post germany "/__chat/client/close" '{"handle":"bo"}' >/dev/null
sleep 0.5

off_send="$(node_chat_post oregon "/__chat/client/send" "{\"handle\":\"ada\",\"whisper_to\":\"$uid_b\",\"body\":\"waiting while offline\"}")"
echo "oregon offline whisper send: $off_send"
sleep 0.5

waiting_de="$(node_chat_get germany "/__chat/history/waiting?account=$acct_b")"
echo "germany waiting queue before uk-1 login: $waiting_de"
printf '%s' "$waiting_de" | grep -q 'waiting while offline' || fail "offline whisper not queued at germany-1: $waiting_de"

uk_url="$(node_url uk)"
reg_c="$(curl -sk -X POST "$uk_url/dainty/api/Register" \
	-H 'Content-Type: application/json' \
	-d '{"Username":"SlayerUK"}')"
key_c="$(printf '%s' "$reg_c" | sed -n 's/.*"UUK":"\([^"]*\)".*/\1/p')"
[ -n "$key_c" ] || fail "failed to register Player C on uk-1: $reg_c"
tok_c_json="$(curl -sk -X POST "$uk_url/account/api/oauth/token" \
	-H 'Content-Type: application/x-www-form-urlencoded' \
	-d "grant_type=exchange_code&exchange_code=$key_c")"
tok_c="$(printf '%s' "$tok_c_json" | sed -n 's/.*"access_token":"\([^"]*\)".*/\1/p')"
[ -n "$tok_c" ] || fail "failed to obtain access token for Player C: $tok_c_json"

curl -sk -X POST "$uk_url/candidate/join" \
	-H "Authorization: Bearer $tok_c" \
	-H 'Content-Type: application/json' \
	-d '{"gameMode":"ISLAND","playerHuntId":"Example_Hunt_Easy"}' >/dev/null
uk_inst=""
for _ in $(seq 1 20); do
	cand_uk="$(curl -sk "$uk_url/candidate/status" -H "Authorization: Bearer $tok_c")"
	if printf '%s' "$cand_uk" | grep -q '"status":"IN_PROGRESS"'; then
		uk_inst="$(printf '%s' "$cand_uk" | sed -n 's/.*"gameSessionId":"\([^"]*\)".*/\1/p')"
		break
	fi
	sleep 0.5
done
[ -n "$uk_inst" ] || fail "failed to launch immediate instance on uk-1"
sleep 1

redir_uk="$(curl -sk -i -X POST "$germany_url/candidate/join" \
	-H "Authorization: Bearer $tok_b" \
	-H 'Content-Type: application/json' \
	-d "{\"gameMode\":\"ISLAND\",\"instanceId\":\"$uk_inst\"}")"
uk_ticket="$(printf '%s' "$redir_uk" | tr -d '\r' | sed -n 's/^[xX]-dainty-session-ticket: *//p' | head -n 1)"
[ -n "$uk_ticket" ] || fail "missing session ticket for uk-1 redirect: $redir_uk"

chat_login_b_uk="$(node_chat_post uk "/__chat/client/login" "{\"handle\":\"bo_uk\",\"authcid\":\"$uid_b\",\"token\":\"$uk_ticket\"}")"
echo "chat login Player B on uk-1 (foreign node): $chat_login_b_uk"
printf '%s' "$chat_login_b_uk" | grep -q '"ok":true' || fail "Player B cross-node chat login on uk-1 failed: $chat_login_b_uk"
if ! printf '%s' "$chat_login_b_uk" | grep -q 'waiting while offline'; then
	uk_delayed="$(node_chat_get uk "/__chat/client/drain?handle=bo_uk&expect=waiting%20while%20offline&timeout_ms=6000")"
	echo "uk-1 delayed drain: $uk_delayed"
	printf '%s' "$uk_delayed" | grep -q 'waiting while offline' || fail "Player B on uk-1 did not receive offline DM: $uk_delayed"
	printf '%s' "$uk_delayed" | grep -q 'urn:xmpp:delay' || fail "offline DM on uk-1 missing <delay/>: $uk_delayed"
else
	printf '%s' "$chat_login_b_uk" | grep -q 'urn:xmpp:delay' || fail "offline DM on uk-1 missing <delay/>: $chat_login_b_uk"
fi

waiting_de_after="$(node_chat_get germany "/__chat/history/waiting?account=$acct_b")"
echo "germany waiting queue after uk-1 login: $waiting_de_after"
if printf '%s' "$waiting_de_after" | grep -q 'waiting while offline'; then
	fail "germany-1 waiting queue was not cleared after delivery on uk-1: $waiting_de_after"
fi

# 3. Cross-node room fanout: Player A on oregon-1 and Player B on uk-1 join the same Hunt room and talk
hunt_room="Hunt-01J9Z3N5C8Q0V2X4Y6W8T0R2P4.oregon-1"
join_a="$(node_chat_post oregon "/__chat/client/send" "{\"handle\":\"ada\",\"join_room\":\"$hunt_room\",\"name\":\"SlayerOregon\",\"wait_ms\":400}")"
echo "Player A join $hunt_room on oregon-1: $join_a"
printf '%s' "$join_a" | grep -q '110' || fail "Player A failed to join $hunt_room: $join_a"

join_b="$(node_chat_post uk "/__chat/client/send" "{\"handle\":\"bo_uk\",\"join_room\":\"$hunt_room\",\"name\":\"SlayerGermany\",\"wait_ms\":600}")"
echo "Player B join $hunt_room on uk-1: $join_b"
if ! printf '%s' "$join_b" | grep -q '110'; then
	join_b_more="$(node_chat_get uk "/__chat/client/drain?handle=bo_uk&expect=110&timeout_ms=5000")"
	printf '%s' "$join_b_more" | grep -q '110' || fail "Player B failed to join $hunt_room on uk-1: $join_b / $join_b_more"
fi

room_say_a="$(node_chat_post oregon "/__chat/client/send" "{\"handle\":\"ada\",\"say_room\":\"$hunt_room\",\"body\":\"flare up from oregon\"}")"
room_heard_b="$(node_chat_get uk "/__chat/client/drain?handle=bo_uk&expect=flare%20up%20from%20oregon&timeout_ms=6000")"
echo "Player B on uk-1 heard room message: $room_heard_b"
printf '%s' "$room_heard_b" | grep -q '"matched":true' || fail "room message from oregon-1 did not fan out to uk-1: $room_heard_b"

# 4. Mute applied at Player B's home (germany-1) propagates to uk-1 and blocks Player B from speaking; unmute restores it
mute_resp="$(node_chat_post germany "/__chat/mute" "{\"account\":\"$acct_b\",\"reason\":\"federated mute test\",\"actor\":\"mod\"}")"
echo "mute Player B on germany-1: $mute_resp"
printf '%s' "$mute_resp" | grep -q '"ok":true' || fail "mute on germany-1 failed: $mute_resp"
sleep 0.5

muted_say="$(node_chat_post uk "/__chat/client/send" "{\"handle\":\"bo_uk\",\"say_room\":\"$hunt_room\",\"body\":\"should be blocked\",\"wait_ms\":400}")"
echo "muted Player B room attempt on uk-1: $muted_say"
if ! printf '%s' "$muted_say" | grep -q '<forbidden'; then
	muted_err="$(node_chat_get uk "/__chat/client/drain?handle=bo_uk&expect=forbidden&timeout_ms=5000")"
	printf '%s' "$muted_err" | grep -q '<forbidden' || fail "expected <forbidden for muted player on uk-1, got: $muted_say / $muted_err"
fi
ada_drain="$(node_chat_get oregon "/__chat/client/drain?handle=ada&wait_ms=400")"
if printf '%s' "$ada_drain" | grep -q 'should be blocked'; then
	fail "muted player's message reached oregon-1: $ada_drain"
fi

unmute_resp="$(node_chat_post germany "/__chat/unmute" "{\"account\":\"$acct_b\",\"actor\":\"mod\"}")"
echo "unmute Player B on germany-1: $unmute_resp"
printf '%s' "$unmute_resp" | grep -q '"ok":true' || fail "unmute on germany-1 failed: $unmute_resp"
sleep 0.5

unmuted_say="$(node_chat_post uk "/__chat/client/send" "{\"handle\":\"bo_uk\",\"say_room\":\"$hunt_room\",\"body\":\"unmuted from uk\"}")"
ada_heard_unmuted="$(node_chat_get oregon "/__chat/client/drain?handle=ada&expect=unmuted%20from%20uk&timeout_ms=6000")"
echo "Player A on oregon-1 heard unmuted Player B on uk-1: $ada_heard_unmuted"
printf '%s' "$ada_heard_unmuted" | grep -q '"matched":true' || fail "unmuted room message from uk-1 did not reach oregon-1: $ada_heard_unmuted"

node_chat_post oregon "/__chat/client/close" '{"handle":"ada"}' >/dev/null
node_chat_post uk "/__chat/client/close" '{"handle":"bo_uk"}' >/dev/null

echo "network harness passed"
