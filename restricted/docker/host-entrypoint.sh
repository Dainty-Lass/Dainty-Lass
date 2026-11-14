#!/bin/sh
# Dainty Lass: `docker compose run --rm host <command>` (compose.yaml's host service) runs
# dainty-host inside the server image, in the server's network, with its volumes. Every command works as on a
# Linux host (docs/hosting-linux.md), with these differences (docs/hosting-docker.md):
#  - setup: the services are in the image already (--skip-install), the game folder is the one mounted at /game,
#    and if the mongo-data volume mounted at /opt/dainty-lass/.workspace/mongodb holds pre-cutover MongoDB data,
#    setup migrates and verifies it from the mongo profile's service at 127.0.0.1:27017 into store.
#    Each default is added only when the command line does not say otherwise.
#  - install-game: the versions go in the mounted game folder (--dir /game, unless given), so the Director's
#    containers, which mount it at the same path, see them.
#  - start and update are compose's: `docker compose up -d` runs the server, a new image updates it.
#  - apply (the `toml` profile's configure service): setup from settings.toml alone, before every `up`. A settings.toml
# in
#    /config (DAINTY_CONFIG_DIR) replaces the workspace's first, so that file is the server's whole configuration.
set -eu

workspace_toml=/opt/dainty-lass/.workspace/config/settings.toml

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
	if ! has --game "$@" && [ -d /game/Archon ]; then
		set -- "$@" --game /game
	fi
	dainty-host setup "$@"
	if [ -n "${DAINTY_REMOTE_ENROLL_URI:-}" ] && [ ! -f /opt/dainty-lass/.workspace/config/remote-managed.json ]; then
		dainty-host remote enroll "${DAINTY_REMOTE_ENROLL_URI}"
	fi
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
install-game)
	shift
	has --dir "$@" || set -- "$@" --dir /game
	exec dainty-host install-game "$@"
	;;
apply)
	if [ -f /config/settings.toml ]; then
		mkdir -p "$(dirname "$workspace_toml")"
		cp /config/settings.toml "$workspace_toml"
		echo "settings.toml taken from DAINTY_CONFIG_DIR"
	elif [ ! -f "$workspace_toml" ]; then
		echo "No settings.toml: put one in DAINTY_CONFIG_DIR (./config by default; docs/hosting-docker.md shows how)." >&2
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
	echo "  docker compose run --rm host invite --for \"Alex\""
	exit 0
	;;
esac
exec dainty-host "$@"
