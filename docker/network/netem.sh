#!/bin/sh
# dainty-netem: delays this node's egress by half the round trip to each other node, so the two directions add up to
# NETEM_RTT's value, and by the whole round trip to everything else (the Docker host, playing a player), since
# nothing delays the way in (DN12).
set -eu

self=${NETEM_SELF:?NETEM_SELF must name this node region}
nodes=${NETEM_NODES:?NETEM_NODES must list region=address entries}
rtts=${NETEM_RTT:?NETEM_RTT must list regionA:regionB=milliseconds entries}
player=${NETEM_PLAYER_REGION:-}
iface=${NETEM_IFACE:-eth0}

rtt() {
    [ "$1" = "$2" ] && {
        echo 0
        return
    }
    for pair in $rtts; do
        case "${pair%%=*}" in
            "$1:$2" | "$2:$1")
                value=${pair#*=}
                case "$value" in
                    "" | . | *[!0-9.]* | *.*.*)
                        echo "not a number of milliseconds: '$value'" >&2
                        exit 2
                        ;;
                esac
                echo "$value"
                return
                ;;
        esac
    done
    echo "no round trip for $1 and $2 in NETEM_RTT" >&2
    exit 2
}

peers=0
for entry in $nodes; do
    [ "${entry%%=*}" = "$self" ] || peers=$((peers + 1))
done
bands=$((peers + 2))
if [ "$bands" -gt 16 ]; then
    echo "too many nodes for one prio qdisc" >&2
    exit 2
fi
last=$((bands - 1))
# Everything unmatched goes to the last band
priomap=
i=0
while [ "$i" -lt 16 ]; do
    priomap="$priomap $last"
    i=$((i + 1))
done

tc qdisc del dev "$iface" root 2>/dev/null || true
# shellcheck disable=SC2086 # the priomap is 16 separate words
tc qdisc add dev "$iface" root handle 1: prio bands "$bands" priomap $priomap
# Band 1: ARP, never delayed
tc filter add dev "$iface" protocol arp parent 1: prio 1 u32 match u32 0 0 flowid 1:1

band=2
for entry in $nodes; do
    region=${entry%%=*}
    address=${entry#*=}
    [ "$region" = "$self" ] && continue
    trip=$(rtt "$self" "$region")
    half=$(awk -v t="$trip" 'BEGIN { printf "%.1f", t / 2 }')
    tc qdisc add dev "$iface" parent "1:$band" handle "$((band * 10)):" netem delay "${half}ms"
    tc filter add dev "$iface" protocol ip parent 1: prio 2 u32 match ip dst "$address/32" flowid "1:$band"
    echo "netem: $self -> $region ($address): $half ms each way"
    band=$((band + 1))
done

if [ -n "$player" ]; then
    whole=$(rtt "$player" "$self")
else
    whole=0
fi
tc qdisc add dev "$iface" parent "1:$band" handle "$((band * 10)):" netem delay "${whole}ms"
echo "netem: $self -> everything else (a player in ${player:-no region}): $whole ms"

tc qdisc show dev "$iface"
exec sleep infinity
