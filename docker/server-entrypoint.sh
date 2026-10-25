#!/bin/sh
# Dauntless Reforged Revival: the server container (compose.yaml's server service) runs reforged-server-minimal, the
# whole server as one process, on the workspace volume. Its settings are .workspace/config/server.toml (or .yaml,
# .yml, .json; docs/hosting-docker.md), made empty on the first start so the server boots with its defaults: a
# private server on 127.0.0.1:8443. Not host.toml, which is the host kit's (reforged-server-console setup) and
# holds other keys; the server would read it as its own and get the defaults.
set -eu

# With --config the workspace is the file's workspace_dir, .workspace by default: relative to here
cd /opt/reforged
config=.workspace/config
mkdir -p "$config"

file=""
for ext in toml yaml yml json; do
	if [ -f "$config/server.$ext" ]; then
		file="$config/server.$ext"
		break
	fi
done
if [ -z "$file" ]; then
	file="$config/server.toml"
	echo "# The server's settings (docs/hosting-docker.md). Empty, it runs with the defaults: a private server." >"$file"
fi

exec reforged-server-minimal --config "$file" "$@"
