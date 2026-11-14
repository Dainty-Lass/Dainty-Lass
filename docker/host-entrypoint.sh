#!/bin/sh
# dainty-host-docker: dainty-host in the server image, with the server's volumes and namespace. Commands behave as on
# a Linux host, except those whose job compose or the image already does.
set -eu

config=/opt/dainty-lass/.workspace/config

# True when an argument after the flag is the flag itself or --flag=value.
has() {
    flag=$1
    shift
    for arg in "$@"; do
        case "$arg" in
            "$flag" | "$flag"=*) return 0 ;;
        esac
    done
    return 1
}

docker_setup() {
    # The services are in the image, so nothing is installed (D-6: no runtime sources here)
    has --skip-install "$@" || set -- "$@" --skip-install
    if [ -d /game/Archon ] && ! has --game "$@"; then
        set -- "$@" --game /game
    fi
    dainty-host setup "$@"
    if [ -n "${DAINTY_REMOTE_ENROLL_URI:-}" ] && [ ! -e "$config/remote-managed.json" ]; then
        dainty-host remote enroll "$DAINTY_REMOTE_ENROLL_URI"
    fi
}

command=${1:-}
case "$command" in
    start)
        echo "In Docker the services are compose's: docker compose up -d (docker compose logs -f shows them)." >&2
        exit 2
        ;;
    update)
        echo "In Docker a new version is a new image: git pull && docker compose build && docker compose up -d" >&2
        echo "Then run docker compose run --rm host setup if the release notes say so." >&2
        exit 2
        ;;
    install-game)
        shift
        has --dir "$@" || set -- "$@" --dir /game
        exec dainty-host install-game "$@"
        ;;
    apply)
        if [ -f /config/settings.toml ]; then
            mkdir -p "$config"
            cp /config/settings.toml "$config/settings.toml"
            echo "settings.toml taken from DAINTY_CONFIG_DIR"
        elif [ ! -f "$config/settings.toml" ]; then
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
        echo "In Docker, ignore the systemd and ufw steps above: docker compose up -d starts the server"
        echo "(COMPOSE_PROFILES=public in .env for a public server). Invite a player with:"
        echo "  docker compose run --rm host invite --for \"Alex\""
        exit 0
        ;;
    *)
        exec dainty-host "$@"
        ;;
esac
