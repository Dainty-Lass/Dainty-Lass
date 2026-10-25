#!/bin/bash
# Dauntless Reforged Revival: compose.yaml's health check for the server container, `reforged-healthcheck`. Healthy
# once the gateway takes a connection, on the address and port the server's settings give (.workspace/config/server.*,
# as docker/server-entrypoint.sh picks it), or reforged-server-minimal's defaults without them: 127.0.0.1:8443.
set -u
config=/opt/reforged/.workspace/config
file=""
for ext in toml yaml yml json; do
	if [ -f "$config/server.$ext" ]; then
		file="$config/server.$ext"
		break
	fi
done

# The first `key = value`, `key: value` or `"key": value` line, unquoted; the two keys are top-level in every format
setting() {
	local value="" line
	if [ -n "$file" ]; then
		line="^[[:space:]]*\"?$1\"?[[:space:]]*[=:][[:space:]]*\"?([^\",[:space:]]+)\"?.*"
		value="$(sed -nE "s/$line/\1/p" "$file" | head -n 1)"
	fi
	printf '%s' "${value:-$2}"
}

host="$(setting bind_host 127.0.0.1)"
port="$(setting gateway_port 8443)"
[ "$host" = 0.0.0.0 ] && host=127.0.0.1
exec 3<>"/dev/tcp/$host/$port"
