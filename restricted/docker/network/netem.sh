#!/bin/sh
# DaintyLass: the network harness's latency (DN12, docs/hosting-docker.md). Runs in a node's network namespace
# (compose.yaml's netem-* services) and shapes its egress with tc/netem:
#  - to each other node: half the round trip the table says, so the two directions add up to the table's value
#  - to everything else (the Docker host, which plays a player): the whole round trip between the player's region
#    (NETEM_PLAYER_REGION) and this node, since nothing delays the way in
# ARP is never delayed. The table is compose.yaml's DAINTY_NETWORK_RTT_* (docs/hosting-docker.md).
#   NETEM_SELF=oregon  NETEM_NODES="oregon=172.28.77.11 new-york=..."  NETEM_RTT="oregon:new-york=70 ..."
#   NETEM_PLAYER_REGION=germany  NETEM_IFACE=eth0
set -eu

iface="${NETEM_IFACE:-eth0}"
self="${NETEM_SELF:?NETEM_SELF is the region of the node}"
nodes="${NETEM_NODES:?NETEM_NODES is the list region=address}"
table="${NETEM_RTT:?NETEM_RTT is the list region:region=milliseconds}"
player="${NETEM_PLAYER_REGION:-}"

number() {
	case "$1" in
	'' | *[!0-9.]* | *.*.* | .) echo "not a number of milliseconds: '$1'" >&2; exit 2 ;;
	esac
}

# The round trip between two regions: 0 for the same one, else the table's value in either order
rtt() {
	if [ "$1" = "$2" ]; then echo 0; return; fi
	for pair in $table; do
		case "$pair" in
		"$1:$2="* | "$2:$1="*) number "${pair#*=}"; echo "${pair#*=}"; return ;;
		esac
	done
	echo "no round trip for $1 and $2 in NETEM_RTT" >&2
	exit 2
}

half() { awk "BEGIN { printf \"%.1f\", $1 / 2 }"; }

peers=0
for entry in $nodes; do
	[ "${entry%%=*}" = "$self" ] || peers=$((peers + 1))
done
# Bands: 1 is untouched (ARP), 2..peers+1 are the peers, the last is everything else
bands=$((peers + 2))
[ "$bands" -le 16 ] || { echo "too many nodes for one prio qdisc" >&2; exit 2; }
priomap=""
i=0
while [ "$i" -lt 16 ]; do
	priomap="$priomap $((bands - 1))"
	i=$((i + 1))
done

tc qdisc del dev "$iface" root 2>/dev/null || true
# shellcheck disable=SC2086  # the priomap is a list of words
tc qdisc add dev "$iface" root handle 1: prio bands "$bands" priomap$priomap
tc filter add dev "$iface" protocol arp parent 1: prio 1 u32 match u32 0 0 flowid 1:1

band=1
for entry in $nodes; do
	region="${entry%%=*}"
	address="${entry#*=}"
	[ "$region" = "$self" ] && continue
	band=$((band + 1))
	delay="$(half "$(rtt "$self" "$region")")"
	tc qdisc add dev "$iface" parent "1:$band" handle "$((band * 10)):" netem delay "${delay}ms"
	tc filter add dev "$iface" protocol ip parent 1: prio 2 u32 match ip dst "$address/32" flowid "1:$band"
	echo "netem: $self -> $region ($address): ${delay} ms each way"
done

outside=0
[ -n "$player" ] && outside="$(rtt "$player" "$self")"
tc qdisc add dev "$iface" parent "1:$bands" handle "$((bands * 10)):" netem delay "${outside}ms"
echo "netem: $self -> everything else (a player in ${player:-no region}): ${outside} ms"
tc qdisc show dev "$iface"
exec sleep infinity
