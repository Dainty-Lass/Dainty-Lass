#!/bin/sh
# Dauntless Reforged Revival: `docker compose run --rm host <command>` (compose.yaml's host service) runs the host
# kit's commands (reforged-server-console <command>) inside the server image, in the server's network, with its
# volumes. The server itself is compose's (docker compose up -d) and reads .workspace/config/server.toml, not the
# host kit's host.toml (docs/hosting-docker.md), so the commands that start, configure or ask a host-kit server are
# answered here instead of running:
#  - start, update: compose's
#  - setup: writes host.toml and the env files, which this server does not read
#  - multipliers, backup: read that host.toml, and backup dumps a MongoDB this server does not have
#  - invite: asks the host kit's backend for a code; this server writes its invite link at every start instead
set -eu

workspace=/opt/reforged/.workspace
command="${1:-}"
case "$command" in
start)
	echo "In Docker the server is compose's: docker compose up -d (docker compose logs -f shows it)." >&2
	exit 2
	;;
update)
	echo "In Docker a new version is a new image: git pull && docker compose build && docker compose up -d" >&2
	exit 2
	;;
setup)
	echo "In Docker the server's settings are .workspace/config/server.toml (docs/hosting-docker.md):" >&2
	echo "edit it, then docker compose up -d --force-recreate. setup's host.toml is not what this server reads." >&2
	exit 2
	;;
multipliers | backup)
	echo "$command needs the host kit's host.toml, which this server does not use (docs/hosting-docker.md)." >&2
	exit 2
	;;
invite)
	if [ ! -f "$workspace/invite.txt" ]; then
		echo "No invite yet: the server writes .workspace/invite.txt when it starts (docker compose up -d)." >&2
		exit 2
	fi
	cat "$workspace/invite.txt"
	exit 0
	;;
esac
exec reforged-server-console "$@"
