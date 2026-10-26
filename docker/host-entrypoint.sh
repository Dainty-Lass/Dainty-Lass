#!/bin/sh
# Dauntless Reforged Revival: `docker compose run --rm host <command>` (compose.yaml's host service) runs
# reforged-host inside the server image, in the server's network, with its volumes. Every command works as on a
# Linux host (docs/hosting-linux.md), with these differences (docs/hosting-docker.md):
#  - setup: the services are in the image already (--skip-install), MongoDB is the mongo service
#    (--no-manage-mongo, the replica set at 127.0.0.1:27017), and the game folder is the one mounted at /game.
#    Each is added only when the command line does not say otherwise.
#  - start and update are compose's: `docker compose up -d` runs the server, a new image updates it.
#  - apply (the `toml` profile's configure service): setup from host.toml alone, before every `up`. A host.toml in
#    /config (REFORGED_CONFIG_DIR) replaces the workspace's first, so that file is the server's whole configuration.
set -eu

workspace_toml=/opt/reforged/.workspace/config/host.toml

has() {
	want="$1"
	shift
	for arg in "$@"; do
		case "$arg" in "$want" | "$want"=*) return 0 ;; esac
	done
	return 1
}

# setup with the container's defaults, each only where the command line does not say otherwise
docker_setup() {
	has --skip-install "$@" || set -- "$@" --skip-install
	if ! has --manage-mongo "$@" && ! has --no-manage-mongo "$@" && ! has --mongo-uri "$@"; then
		set -- "$@" --no-manage-mongo --mongo-uri "mongodb://127.0.0.1:27017/?replicaSet=rs0"
	fi
	if ! has --game "$@" && [ -d /game/Archon ]; then
		set -- "$@" --game /game
	fi
	reforged-host setup "$@"
}

command="${1:-}"
case "$command" in
start)
	echo "In Docker the services are compose's: docker compose up -d (docker compose logs -f shows them)." >&2
	exit 2
	;;
update)
	echo "In Docker a new version is a new image: git pull && docker compose build && docker compose up -d" >&2
	echo "(then docker compose run --rm host setup, which keeps your settings, if the release notes say so)." >&2
	exit 2
	;;
apply)
	if [ -f /config/host.toml ]; then
		mkdir -p "$(dirname "$workspace_toml")"
		cp /config/host.toml "$workspace_toml"
		echo "host.toml taken from REFORGED_CONFIG_DIR"
	elif [ ! -f "$workspace_toml" ]; then
		echo "No host.toml: put one in REFORGED_CONFIG_DIR (./config by default; docs/hosting-docker.md shows how)." >&2
		exit 2
	fi
	docker_setup
	exit 0
	;;
setup)
	shift
	docker_setup "$@"
	echo
	echo "In Docker (docs/hosting-docker.md): ignore the systemd and ufw steps above; compose runs the server."
	echo "  docker compose up -d                 starts it (COMPOSE_PROFILES=public in .env for a public server)"
	echo "  (a legacy server: REFORGED_EDITION, COMPOSE_FILE and COMPOSE_PROFILES as .env.example shows)"
	echo "  docker compose run --rm host invite --for \"Alex\""
	exit 0
	;;
esac
exec reforged-host "$@"
