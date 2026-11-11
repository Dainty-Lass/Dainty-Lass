#!/usr/bin/env bash
# The network profile's harness (DN12): four nodes with injected latency. Checks the peers and their round trips, the
# launcher's region ping, cross-node matchmaking (DN8), wrong-node refusal and redirect (DN6), regional Ramsgate
# (DN9) and chat federation (DN10b). Plays no game.
#   NETWORK_TOLERANCE_MS   how far above the injected round trip a measurement may be (25)
#   NETWORK_REGION_PING    auto (default), require (CI) or skip
#   NETWORK_NO_BUILD=1     use the images as built
#   NETWORK_OPTIONAL=1     skip, not fail, where Docker can't give CAP_NET_ADMIN or netem
set -euo pipefail
cd "$(dirname "$0")/../.."

export COMPOSE_PROJECT_NAME=${COMPOSE_PROJECT_NAME:-dainty-network}
export COMPOSE_PROFILES=network
# Taken from the environment when set, so the oracle can point these scripts at another file (D-8)
export COMPOSE_FILE=${COMPOSE_FILE:-compose.yaml}
# Git Bash would otherwise rewrite the /opt/... paths handed to docker
export MSYS_NO_PATHCONV=1
tolerance=${NETWORK_TOLERANCE_MS:-25}
region_ping=${NETWORK_REGION_PING:-auto}
regions=(oregon new-york uk germany)

work=$(mktemp -d)
# Windows' curl takes C:/... paths, which MSYS_NO_PATHCONV no longer makes for it
command -v cygpath >/dev/null 2>&1 && work=$(cygpath -m "$work")
started=0
cleanup() {
    status=$?
    if [[ $status -ne 0 && $started -eq 1 ]]; then
        echo
        echo "== network harness failed; the containers and their logs"
        docker compose ps -a || true
        docker compose logs --tail 80 || true
    fi
    if [[ $started -eq 1 ]]; then
        docker compose down --volumes --remove-orphans >/dev/null 2>&1 || true
    fi
    rm -rf "$work"
    exit "$status"
}
trap cleanup EXIT

step() { printf '\n== %s\n' "$*"; }
fail() {
    echo "network test failed: $*" >&2
    exit 1
}
unavailable() {
    if [[ ${NETWORK_OPTIONAL:-} == 1 ]]; then
        echo "network harness skipped: $*"
        exit 0
    fi
    fail "$*"
}
contains() {
    [[ $1 == *"$2"* ]] || {
        printf '%s\n' "$1" >&2
        fail "$3 does not contain $2"
    }
}
# The first string field of compact JSON (as JSON.stringify writes it)
json_str() { grep -o "\"$2\":\"[^\"]*\"" <<<"$1" | head -n 1 | cut -d'"' -f4 || true; }
header() { grep -i "^$2:" <<<"$1" | head -n 1 | cut -d: -f2- | sed 's/^ *//' || true; }

# req <region> <method> <path> [curl arguments...]: sets status, headers and body, through the published port
declare -A published
req() {
    local region=$1 method=$2 path=$3
    shift 3
    local out
    out=$(curl -ksS -X "$method" -D "$work/headers" -w '\n%{http_code}' "https://${published[$region]}$path" "$@") ||
        fail "$method $path on $region: curl failed"
    status=${out##*$'\n'}
    body=${out%$'\n'*}
    headers=$(tr -d '\r' <"$work/headers")
}
json_req() {
    local region=$1 method=$2 path=$3 data=$4
    shift 4
    req "$region" "$method" "$path" -H 'content-type: application/json' --data-raw "$data" "$@"
}
expect_status() {
    [[ $status == "$1" ]] || {
        printf '%s\n' "$body" >&2
        fail "$2 answered $status, not $1"
    }
}

# inner <region> <port> <method> <path> <body> [header...]: raw HTTP on 127.0.0.1 inside gateway-<region>
inner() {
    docker compose exec -T "gateway-$1" bash -c '
        port=$1 method=$2 path=$3 body=$4
        shift 4
        exec 3<>"/dev/tcp/127.0.0.1/$port"
        {
            printf "%s %s HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n" "$method" "$path"
            printf "Content-Type: application/json\r\nContent-Length: %s\r\n" "${#body}"
            for h in "$@"; do printf "%s\r\n" "$h"; done
            printf "\r\n%s" "$body"
        } >&3
        tr -d "\r" <&3' _ "${@:2}"
}
inner_status() { head -n 1 <<<"$1" | cut -d' ' -f2; }
chat() { inner "$1" 41006 "$2" "$3" "${4:-}"; }

step "Docker"
{ docker compose version && docker info; } >/dev/null 2>&1 || unavailable "Docker with Compose is unavailable"

if [[ ${NETWORK_NO_BUILD:-} != 1 ]]; then
    step "build"
    docker compose build net-oregon configure-oregon netem-oregon
fi

step "NET_ADMIN and netem"
netem_image=$(docker compose config --images | grep -- '-netem:' | head -n 1)
[[ -n $netem_image ]] || fail "no netem image in the compose file"
docker run --rm --cap-add NET_ADMIN --entrypoint sh "$netem_image" -c 'tc qdisc add dev lo root netem delay 1ms' ||
    unavailable "Docker can't give CAP_NET_ADMIN or has no sch_netem"

step "the injected round trips"
config=$(docker compose config --format json)
rtt_table=$(grep -o '"NETEM_RTT": *"[^"]*"' <<<"$config" | head -n 1 | cut -d'"' -f4)
player=$(grep -o '"NETEM_PLAYER_REGION": *"[^"]*"' <<<"$config" | head -n 1 | cut -d'"' -f4)
[[ -n $rtt_table ]] || fail "no NETEM_RTT in the compose config"
declare -A rtt
for pair in $rtt_table; do
    a=${pair%%:*} rest=${pair#*:}
    b=${rest%%=*} value=${rest#*=}
    rtt[$a:$b]=$value
    rtt[$b:$a]=$value
done
injected() { if [[ $1 == "$2" ]]; then echo 0; else echo "${rtt[$1:$2]:?no round trip for $1 and $2}"; fi; }
echo "$rtt_table (a player in ${player:-no region})"

step "up"
started=1
docker compose up -d --wait --wait-timeout 300 network
for region in "${regions[@]}"; do
    published[$region]=$(docker compose port "net-$region" 443 | sed 's/^0\.0\.0\.0:/127.0.0.1:/')
done

step "peers and latencies"
for region in "${regions[@]}"; do
    out=$(docker compose exec -T "gateway-$region" dainty-host network status --count 5 \
        --env-file /opt/dainty-lass/.workspace/config/gateway.env) || {
        printf '%s\n' "$out" >&2
        fail "network status on $region"
    }
    echo "$out"
    peers=0
    while read -r _ node peer state _ measured unit _; do
        peers=$((peers + 1))
        [[ $state == up && $unit == ms ]] || fail "$region sees $node ($peer) $state"
        want=$(injected "$region" "$peer")
        awk -v m="$measured" -v w="$want" -v t="$tolerance" 'BEGIN { exit !(m >= w - 2 && m <= w + t) }' ||
            fail "$region -> $peer: $measured ms, injected $want ms (+$tolerance)"
    done < <(grep '^Peer:' <<<"$out")
    [[ $peers -eq 3 ]] || fail "$region sees $peers peers, not 3"
done

step "the launcher's region ping"
oregon_address=$(docker compose exec -T gateway-oregon hostname -i | awk '{ print $1 }')
reachable() { timeout 3 bash -c ": <>/dev/tcp/$1/443" 2>/dev/null; }
skip_ping=
case "$region_ping" in
    skip) skip_ping="NETWORK_REGION_PING=skip" ;;
    auto | require)
        if ! command -v cargo >/dev/null 2>&1; then
            skip_ping="no cargo"
        elif ! reachable "$oregon_address"; then
            skip_ping="$oregon_address:443 is not reachable from this machine (Docker Desktop)"
        fi
        if [[ -n $skip_ping && $region_ping == require ]]; then
            fail "the region ping can't run: $skip_ping"
        fi
        ;;
    *) fail "NETWORK_REGION_PING is auto, require or skip, not $region_ping" ;;
esac
if [[ -n $skip_ping ]]; then
    echo "(skipped: $skip_ping)"
else
    for home in "${regions[@]}"; do
        pin=$(docker compose exec -T "gateway-$home" dainty-host gateway make-cert --fingerprint \
            /opt/dainty-lass/.workspace/tls/gateway-cert.pem | sed -n 's/.*Fingerprint:[[:space:]]*//p' | head -n 1)
        expect=$(for other in "${regions[@]}"; do
            [[ $other == "$home" ]] || echo "$(injected "${player:-$other}" "$other") $other"
        done | sort -n | awk '{ printf "%s%s=%s", (NR > 1 ? "," : ""), $2, $1 }')
        echo "home $home: $expect"
        RR_HARNESS_HOME=${published[$home]} RR_HARNESS_PIN=$pin RR_HARNESS_EXPECT=$expect \
            RR_HARNESS_TOLERANCE_MS=$tolerance \
            cargo test -p dainty-launcher-kit --lib --locked -- --ignored --nocapture the_region_order_follows ||
            fail "the region order from $home"
    done
fi

# register <region> <name>: sets the player's key, user id, account id and token in <region>_*
declare -A key user account token
register() {
    local region=$1 name=$2
    json_req "$region" POST /dainty/api/Register "{\"Username\":\"$name\"}"
    expect_status 200 "Register $name"
    key[$region]=$(json_str "$body" UUK)
    [[ -n ${key[$region]} ]] || fail "Register $name: no UUK in $body"
    req "$region" GET /dainty/api/GetUserInfo -H "x-dainty-user-api-key: ${key[$region]}"
    expect_status 200 "GetUserInfo for $name"
    user[$region]=$(json_str "$body" UserId)
    account[$region]=$(header "$headers" x-dainty-account-id)
    [[ -n ${user[$region]} && -n ${account[$region]} ]] || fail "GetUserInfo for $name: no UserId or x-dainty-account-id"
    req "$region" POST /account/api/oauth/token \
        --data-urlencode grant_type=exchange_code --data-urlencode "exchange_code=${key[$region]}"
    expect_status 200 "the token for $name"
    token[$region]=$(json_str "$body" access_token)
    [[ -n ${token[$region]} ]] || fail "the token for $name: no access_token"
    echo "$name on $region: ${user[$region]}"
}
bearer() { echo "Authorization: bearer ${token[$1]}"; }

# status_until <region> <needle> [auth header]: polls /candidate/status until it contains the needle
status_until() {
    local region=$1 needle=$2 auth=${3:-$(bearer "$1")}
    for _ in $(seq 40); do
        req "$region" GET /candidate/status -H "$auth"
        [[ $body == *"$needle"* ]] && return 0
        sleep 0.5
    done
    printf '%s\n' "$body" >&2
    fail "the candidate on $region never showed $needle"
}

step "players"
register oregon SlayerOregon
register germany SlayerGermany

step "DN8: cross-node matchmaking"
for region in oregon germany; do
    json_req "$region" POST /candidate/join '{"gameMode":"ISLAND","playerHuntId":"CR19_PlayerHunt_Example"}' \
        -H "$(bearer "$region")"
    expect_status 200 "the join on $region"
done
declare -A session host
for region in oregon germany; do
    status_until "$region" '"status":"IN_PROGRESS"'
    session[$region]=$(json_str "$body" gameSessionId)
    host[$region]=$(json_str "$body" host)
done
[[ -n ${session[oregon]} && ${session[oregon]} == "${session[germany]}" && ${host[oregon]} == "${host[germany]}" ]] ||
    fail "the two players are in different sessions: ${session[oregon]} (${host[oregon]}), ${session[germany]} (${host[germany]})"
host_node=${session[oregon]##*@}
host_region=${host_node%-[0-9]*}
[[ $host_region == oregon || $host_region == germany ]] || fail "the session ${session[oregon]} names no player's node"
remote=oregon
[[ $host_region == oregon ]] && remote=germany
remote_account=${account[$remote]}
echo "session ${session[oregon]} on $host_node; $remote's player is the remote one"

api_key=$(docker compose exec -T "gateway-$host_region" sh -c \
    'grep "^METAGAME_API_KEY=" /opt/dainty-lass/.workspace/config/director.env | tail -n 1 | cut -d= -f2- | tr -d "\""')
[[ -n $api_key ]] || fail "no METAGAME_API_KEY in $host_region's director.env"
game=x-rustic-isles-gameserver-apikey:\ $api_key
out=$(inner "$host_region" 41000 POST "/progression/$remote_account/ExperienceTrack_PlayerLevel/250" "" "$game")
[[ $(inner_status "$out") == 200 ]] || fail "progression on the host node: $out"
out=$(inner "$host_region" 41000 POST /inventory "{\"accountId\":\"$remote_account\",\"characterId\":\"char-$remote_account\",\"transactionId\":\"tx-ring-loot-1\",\"addStackedItems\":[{\"catalogId\":\"MonsterPart_Gnasher_Tail\",\"quantity\":15}]}" "$game")
contains "$out" '"forwarded_to_home":true' "the host node's inventory answer"
out=$(inner "$host_region" 41000 POST "/bounty/$remote_account" '{"bounties":[{"bounty_id":"BOUNTY_RING_TEST","progress":1}]}' "$game")
[[ $(inner_status "$out") == 200 ]] || fail "bounty on the host node: $out"
req "$remote" GET "/progression/${user[$remote]}" -H "$(bearer "$remote")"
contains "$body" 250 "the remote player's progression at home"
req "$remote" GET "/inventory/${user[$remote]}/char-$remote_account" -H "$(bearer "$remote")"
contains "$body" MonsterPart_Gnasher_Tail "the remote player's inventory at home"
req "$remote" GET "/bounty/${user[$remote]}" -H "$(bearer "$remote")"
contains "$body" BOUNTY_RING_TEST "the remote player's bounties at home"

step "DN6: wrong node"
# oregon_instance: a new instance for oregon's player; its candidate's gameSessionId is the instance's global id
instance=
oregon_instance() {
    local previous=$instance
    json_req oregon POST /candidate/join '{"gameMode":"ISLAND","playerHuntId":"Example_Hunt_Easy"}' \
        -H "$(bearer oregon)"
    expect_status 200 "oregon's join"
    for _ in $(seq 40); do
        status_until oregon '"status":"IN_PROGRESS"'
        instance=$(json_str "$body" gameSessionId)
        [[ -n $instance && $instance != "$previous" ]] && return 0
        sleep 0.5
    done
    fail "oregon's join made no new instance"
}
oregon_instance
json_req germany POST /candidate/join "{\"gameMode\":\"ISLAND\",\"instanceId\":\"$instance\"}" -H "$(bearer germany)"
expect_status 409 "germany's join of oregon's instance"
contains "$body" WRONG_NODE "the refusal"
docker compose exec -T gateway-germany sed -i '/^\[network\]$/a wrong_node_policy = "redirect"' \
    /opt/dainty-lass/.workspace/config/host.toml
docker compose restart gateway-germany
online=0
for _ in $(seq 60); do
    if curl -ksf "https://${published[germany]}/dainty/api/ServerStatus" 2>/dev/null | grep -q '"online":true'; then
        online=1
        break
    fi
    sleep 1
done
[[ $online -eq 1 ]] || fail "germany is not back online after the restart"
oregon_instance
json_req germany POST /candidate/join "{\"gameMode\":\"ISLAND\",\"instanceId\":\"$instance\"}" -H "$(bearer germany)"
contains "$body" '"hostNode":"oregon-1"' "the redirect"
ticket=$(header "$headers" x-dainty-session-ticket)
[[ -n $ticket ]] || fail "the redirect carries no x-dainty-session-ticket"
json_req oregon PUT /gamesession/epiceos '{}' -H "x-dainty-session-ticket: $ticket"
expect_status 200 "epiceos with the ticket"
contains "$body" '"sessiontoken"' "epiceos with the ticket"
json_req oregon POST /candidate/join "{\"gameMode\":\"ISLAND\",\"instanceId\":\"$instance\"}" \
    -H "x-dainty-session-ticket: $ticket"
expect_status 200 "the join on oregon with the ticket"
contains "$body" "\"instanceId\":\"$instance\"" "the join on oregon with the ticket"

step "DN9: regional Ramsgate"
# report <region> <who> <list>: the player's region report, nearest first
report() {
    local list=$3 json="" sep=""
    for entry in $list; do
        json="$json$sep{\"region\":\"${entry%=*}\",\"rttMs\":${entry#*=}}"
        sep=,
    done
    json_req "$1" POST /network/v1/regions "{\"regions\":[$json]}" -H "x-dainty-user-api-key: ${key[$2]}"
    expect_status 204 "the region report on $1"
}
city='{"gameMode":"CITY","playerHuntId":"ShatteredIsles_ReturnToRamsgate"}'
report oregon oregon "germany=12 uk=35 new-york=85 oregon=150"
json_req oregon POST /candidate/join "$city" -H "$(bearer oregon)"
expect_status 200 "the Ramsgate join"
status_until oregon '"host":"203.0.113.14"'
report oregon oregon "oregon=8 new-york=70 uk=140 germany=150"
json_req oregon POST /candidate/join "$city" -H "$(bearer oregon)"
expect_status 200 "the second Ramsgate join"
status_until oregon '"host":"203.0.113.11"'

step "DN10b: chat federation"
out=$(curl -ksi --http1.1 --max-time 3 -H 'Connection: Upgrade' -H 'Upgrade: websocket' \
    -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' -H 'Sec-WebSocket-Version: 13' \
    -H 'Sec-WebSocket-Protocol: xmpp' "https://${published[oregon]}//" | tr -d '\r' || true)
contains "$out" "101 Switching Protocols" "the WebSocket upgrade"
grep -qi '^sec-websocket-protocol: xmpp' <<<"$out" || fail "the upgrade doesn't echo the xmpp protocol"

# login <region> <handle> <player region> [token]
login() {
    local out
    out=$(chat "$1" POST /__chat/client/login \
        "{\"handle\":\"$2\",\"authcid\":\"${user[$3]}\",\"token\":\"${4:-${token[$3]}}\"}")
    contains "$out" '"ok":true' "the chat login of $2 on $1"
    printf '%s' "$out"
}
send() { chat "$1" POST /__chat/client/send "$2"; }
drain_for() { chat "$1" GET "/__chat/client/drain?handle=$2&expect=$3&timeout_ms=${4:-5000}"; }
drain() { chat "$1" GET "/__chat/client/drain?handle=$2&wait_ms=${3:-500}"; }
count() { grep -o "$2" <<<"$1" | wc -l | tr -d ' '; }

login oregon or oregon >/dev/null
login germany de germany >/dev/null
send oregon "{\"handle\":\"or\",\"whisper_to\":\"${user[germany]}\",\"body\":\"ring-whisper-one\"}" >/dev/null
out=$(drain_for germany de ring-whisper-one)
contains "$out" '"matched":true' "germany's player's frames"
out="$out$(drain germany de 1000)"
[[ $(count "$out" ring-whisper-one) -eq 1 ]] || fail "the whisper arrived $(count "$out" ring-whisper-one) times"
for region in oregon germany; do
    contains "$(chat "$region" GET "/__chat/history/direct?a=${user[oregon]}&b=${user[germany]}")" ring-whisper-one \
        "$region's direct history"
done

chat germany POST /__chat/client/close '{"handle":"de"}' >/dev/null
send oregon "{\"handle\":\"or\",\"whisper_to\":\"${user[germany]}\",\"body\":\"ring-whisper-offline\"}" >/dev/null
contains "$(chat germany GET "/__chat/history/waiting?account=${user[germany]}")" ring-whisper-offline \
    "germany's waiting queue"
# A uk session ticket for germany's player: a uk player's hunt, joined through germany's redirect
register uk SlayerUk
json_req uk POST /candidate/join '{"gameMode":"ISLAND","playerHuntId":"Example_Hunt_Easy"}' -H "$(bearer uk)"
expect_status 200 "uk's join"
status_until uk '"status":"IN_PROGRESS"'
uk_instance=$(json_str "$body" gameSessionId)
json_req germany POST /candidate/join "{\"gameMode\":\"ISLAND\",\"instanceId\":\"$uk_instance\"}" -H "$(bearer germany)"
contains "$body" '"hostNode":"uk-1"' "germany's redirect to the uk hunt"
uk_ticket=$(header "$headers" x-dainty-session-ticket)
[[ -n $uk_ticket ]] || fail "no uk session ticket for germany's player"
# The ticket itself is the chat token on a foreign node
out=$(login uk de-uk germany "$uk_ticket")
out="$out$(drain_for uk de-uk ring-whisper-offline)"
contains "$out" "urn:xmpp:delay" "the queued whisper"
out="$out$(drain uk de-uk 1000)"
[[ $(count "$out" ring-whisper-offline) -eq 1 ]] ||
    fail "the queued whisper arrived $(count "$out" ring-whisper-offline) times"
[[ $(chat germany GET "/__chat/history/waiting?account=${user[germany]}") != *ring-whisper-offline* ]] ||
    fail "germany's queue still holds the delivered whisper"

# DN8's hunt; a hunt room is Hunt-<ULID>.<host node>
hunt=${session[oregon]#*:}
room=Hunt-${hunt/@/.}
contains "$(send oregon "{\"handle\":\"or\",\"join_room\":\"$room\",\"name\":\"SlayerOregon\",\"wait_ms\":1000}")" 110 "oregon's room join"
contains "$(send uk "{\"handle\":\"de-uk\",\"join_room\":\"$room\",\"name\":\"SlayerGermany\",\"wait_ms\":1000}")" 110 "uk's room join"
drain uk de-uk >/dev/null
send oregon "{\"handle\":\"or\",\"say_room\":\"$room\",\"body\":\"ring-room-hello\"}" >/dev/null
contains "$(drain_for uk de-uk ring-room-hello)" '"matched":true' "the room message on uk"

contains "$(chat germany POST /__chat/mute "{\"account\":\"${user[germany]}\",\"reason\":\"harness\"}")" '"ok":true' \
    "the mute at germany"
drain oregon or >/dev/null
contains "$(send uk "{\"handle\":\"de-uk\",\"say_room\":\"$room\",\"body\":\"ring-muted\",\"wait_ms\":1000}")" \
    "<forbidden" "the muted player's message"
[[ $(drain oregon or 1500) != *ring-muted* ]] || fail "the muted message reached oregon"
contains "$(chat germany POST /__chat/unmute "{\"account\":\"${user[germany]}\"}")" '"ok":true' "the unmute"
send uk "{\"handle\":\"de-uk\",\"say_room\":\"$room\",\"body\":\"ring-unmuted\"}" >/dev/null
contains "$(drain_for oregon or ring-unmuted)" '"matched":true' "the message after the unmute"
chat oregon POST /__chat/client/close '{"handle":"or"}' >/dev/null
chat uk POST /__chat/client/close '{"handle":"de-uk"}' >/dev/null

echo
echo "network harness passed"
