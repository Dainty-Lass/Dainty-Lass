#!/bin/sh
# Dainty Lass: the network profile's one-shot (compose.yaml network-init, DN12, docs/security/network.md). Runs as root in
# the server image with the cluster CA volume and the four nodes' workspace volumes, and leaves each node
#  - its public gateway certificate (the fingerprint the launcher pins when it measures a region),
#  - its node certificate and key and the CA's certificate (node-cert.pem, node-key.pem, cluster-ca-cert.pem),
#  - a host.toml naming it and listing the other three, for `dainty-host-docker apply` to run setup from.
# The CA file (its key) stays in /ca, which no other service mounts. Run again, it keeps what it made: the CA, every
# certificate and so every fingerprint; only host.toml is written each time.
#   NETWORK_NODES="oregon=oregon-1@172.28.77.11 new-york=new-york-1@172.28.77.12 ..."   region=node-id@address
set -eu

ca=/ca/cluster-ca.pem
port=443
nodes="${NETWORK_NODES:?NETWORK_NODES is the list region=node-id@address}"
fingerprints="$(mktemp -d)"

if [ ! -f "$ca" ]; then
	dainty-host network ca-init --out "$ca" >/dev/null
	echo "cluster CA made in the network-ca volume"
fi

for entry in $nodes; do
	region="${entry%%=*}"
	rest="${entry#*=}"
	id="${rest%@*}"
	address="${rest#*@}"
	tls="/nodes/$region/tls"
	mkdir -p "$tls" "/nodes/$region/config"
	if [ ! -f "$tls/gateway-cert.pem" ]; then
		dainty-host gateway make-cert --host "$region.cluster.test" --host "$address" --out "$tls" >/dev/null
	fi
	if [ ! -f "$tls/node-cert.pem" ]; then
		dainty-host network issue-node-cert --ca "$ca" --node "$id" --region "$region" \
			--host "$address" --host "$region.cluster.test" --out "$tls" >/dev/null
	fi
	dainty-host gateway make-cert --fingerprint "$tls/gateway-cert.pem" | sed -n 's/^Fingerprint: *//p' >"$fingerprints/$region"
done

for entry in $nodes; do
	region="${entry%%=*}"
	rest="${entry#*=}"
	id="${rest%@*}"
	address="${rest#*@}"
	{
		cat <<TOML
version = 1
mode = "public"
host = "$region.cluster.test"
# Documentation range: the harness plays no game, and a public server wants a public-looking address
game_address = "203.0.113.${address##*.}"
name = "Network node $id"
registration = "OPEN"
max_hunt_servers = 3

[switches]
rate_limit = "off"

# A node of the harness plays no game, so setup has no Wine prefix to make
[linux]
wine = "/nonexistent/wine"
xvfb = true

[network]
node = "$id"
region = "$region"
TOML
		for other in $nodes; do
			other_region="${other%%=*}"
			[ "$other_region" = "$region" ] && continue
			other_rest="${other#*=}"
			cat <<TOML

[[network.nodes]]
id = "${other_rest%@*}"
region = "$other_region"
host = "${other_rest#*@}"
port = $port
fingerprint = "$(cat "$fingerprints/$other_region")"
TOML
		done
	} >"/nodes/$region/config/host.toml"
	chown -R 1000:1000 "/nodes/$region"
	chmod 600 "/nodes/$region/tls/node-key.pem" "/nodes/$region/tls/gateway-key.pem"
	echo "$id ($region, $address): certificates and host.toml written"
done

chmod 700 /ca
