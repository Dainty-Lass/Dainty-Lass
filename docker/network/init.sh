#!/bin/sh
# dainty-network-init: the network profile's cluster CA, each node's certificates and settings.toml (DN12). Only
# settings.toml is rewritten on a second run, so the CA, the certificates and every fingerprint survive. Runs as root.
set -eu

: "${NETWORK_NODES:?NETWORK_NODES must list region=node-id@address entries}"
if [ -z "$(echo "$NETWORK_NODES" | tr -d ' ')" ]; then
    echo "NETWORK_NODES is empty: list region=node-id@address entries" >&2
    exit 1
fi

ca=/ca/cluster-ca.pem
if [ ! -f "$ca" ]; then
    dainty-host network ca-init --out "$ca" >/dev/null
    echo "cluster CA made in the network-ca volume"
fi

region_of() { echo "${1%%=*}"; }
id_of() {
    rest=${1#*=}
    echo "${rest%%@*}"
}
address_of() { echo "${1#*@}"; }

for entry in $NETWORK_NODES; do
    region=$(region_of "$entry")
    id=$(id_of "$entry")
    address=$(address_of "$entry")
    tls=/nodes/$region/tls
    mkdir -p "$tls" "/nodes/$region/config"
    if [ ! -f "$tls/gateway-cert.pem" ]; then
        dainty-host gateway make-cert --host "$region.cluster.test" --host "$address" --out "$tls"
    fi
    if [ ! -f "$tls/node-cert.pem" ]; then
        dainty-host network issue-node-cert --ca "$ca" --node "$id" --region "$region" --host "$address" \
            --host "$region.cluster.test" --out "$tls"
    fi
done

fingerprint() {
    dainty-host gateway make-cert --fingerprint "/nodes/$1/tls/gateway-cert.pem" |
        sed -n 's/.*Fingerprint:[[:space:]]*//p' | head -n 1
}

for entry in $NETWORK_NODES; do
    region=$(region_of "$entry")
    id=$(id_of "$entry")
    address=$(address_of "$entry")
    toml=/nodes/$region/config/settings.toml
    {
        echo "# Written by dainty-network-init on every run: the harness plays no game, hence the documentation"
        echo "# range game address and the missing Wine."
        echo "version = 1"
        echo "mode = \"public\""
        echo "host = \"$region.cluster.test\""
        echo "game_address = \"203.0.113.${address##*.}\""
        echo "name = \"Network node $id\""
        echo "registration = \"OPEN\""
        echo "max_hunt_servers = 3"
        echo
        echo "[switches]"
        echo "rate_limit = \"off\""
        echo
        echo "[linux]"
        echo "wine = \"/nonexistent/wine\""
        echo "xvfb = true"
        echo
        echo "[network]"
        echo "node = \"$id\""
        echo "region = \"$region\""
        for other in $NETWORK_NODES; do
            other_region=$(region_of "$other")
            [ "$other_region" = "$region" ] && continue
            other_print=$(fingerprint "$other_region")
            if [ -z "$other_print" ]; then
                echo "no fingerprint for $other_region's gateway certificate" >&2
                exit 1
            fi
            echo
            echo "[[network.nodes]]"
            echo "id = \"$(id_of "$other")\""
            echo "region = \"$other_region\""
            echo "host = \"$(address_of "$other")\""
            echo "port = 443"
            echo "fingerprint = \"$other_print\""
        done
    } >"$toml.tmp"
    mv "$toml.tmp" "$toml"
    chown -R 1000:1000 "/nodes/$region"
    chmod 600 "/nodes/$region/tls/node-key.pem" "/nodes/$region/tls/gateway-key.pem"
    echo "$id ($region, $address): certificates and settings.toml written"
done

# The CA's key never leaves /ca, which no other service mounts
chmod 700 /ca
