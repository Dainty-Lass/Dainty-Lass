#!/bin/sh
# DaintyLass: a legacy server's containers (compose.yaml's legacy profile, compose.legacy.yaml),
# `reforged-legacy metagame|deploy`. Does what `reforged-host start` does for a legacy server
# (rust/services/host/src/legacy/mod.rs, start), then runs the service with its env file.
#  - metagame: its database migrated and the game-server key registered, as DR's install did, then served
#  - deploy: DR's deploy server, which starts the 1.4.4 game servers through `deploy run-game`
#    (.workspace/bin/reforged-legacy-gameserver, Wine on a virtual display) in this container, so they stop with it
set -eu

workspace=/opt/reforged/.workspace
config="$workspace/config"
role="${1:?usage: reforged-legacy metagame|deploy}"
env_file="$config/$role.env"

if [ ! -f "$env_file" ]; then
	echo "$role | not set up yet: docker compose run --rm host setup --edition legacy ... (docs/hosting-docker.md)" >&2
	exit 78
fi
cd "$workspace"

case "$role" in
metagame)
	db="$(sed -n 's/^METAGAME_DB_FILENAME=//p' "$env_file" | tail -n 1 | tr -d '"')"
	metagame migrate --env-file "$env_file"
	metagame db gs-key "$db" "$config/gameserver.key"
	exec metagame serve --env-file "$env_file"
	;;
deploy)
	if ! command -v deploy >/dev/null 2>&1; then
		# DR's deploy server is not ported yet: say so once and stay up, rather than restart forever.
		echo "deploy | ! this image has no deploy bin (DR's deploy server): no game server can start." >&2
		exec sleep infinity
	fi
	exe="$(sed -n 's/^DEPLOY_DR_GAME_EXE=//p' "$env_file" | tr -d '"')"
	if [ -z "$exe" ] || [ ! -f "$exe" ]; then
		echo "deploy | ! no game executable at ${exe:-(none)}: mount your Dauntless 1.4.4 folder (REFORGED_GAME_DIR)" >&2
	fi
	exec deploy serve --env-file "$env_file"
	;;
*)
	echo "unknown role $role" >&2
	exit 2
	;;
esac
