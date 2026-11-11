#!/usr/bin/env bash
# dainty-healthcheck <service>: healthy once the service's port takes a TCP connection, the readiness `start` waits
# for. Each setting is read as the service reads it: the environment first, then its env file, then the default.
set -eu

name=${1:?usage: dainty-healthcheck gateway|allowlist|content|admin|backend|director}
case "$name" in
    gateway) bind_key=GATEWAY_BIND bind_default=0.0.0.0 port_key=GATEWAY_PORT port_default=443 ;;
    allowlist) bind_key=ALLOWLIST_BIND bind_default=127.0.0.1 port_key=ALLOWLIST_PORT port_default=41005 ;;
    content) bind_key=BIND_HOST bind_default=127.0.0.1 port_key=PORT port_default=41002 ;;
    admin) bind_key=BIND_HOST bind_default=127.0.0.1 port_key=PORT port_default=41003 ;;
    backend) bind_key=BIND_HOST bind_default=127.0.0.1 port_key=PORT port_default=41000 ;;
    director) bind_key=BIND_HOST bind_default=127.0.0.1 port_key=PORT port_default=41001 ;;
    *)
        echo "unknown service $name" >&2
        exit 2
        ;;
esac
env_file=/opt/dainty-lass/.workspace/config/$name.env

setting() {
    local key=$1 default=$2 value=
    if [[ -n ${!key:-} ]]; then
        printf '%s' "${!key}"
        return
    fi
    if [[ -r $env_file ]]; then
        value=$(grep -E "^$key=" "$env_file" | tail -n 1 | cut -d= -f2-) || value=
    fi
    if [[ $value == \"*\" && ${#value} -ge 2 ]]; then
        value=${value:1:${#value}-2}
    fi
    printf '%s' "${value:-$default}"
}

address=$(setting "$bind_key" "$bind_default")
port=$(setting "$port_key" "$port_default")
[[ $address == 0.0.0.0 ]] && address=127.0.0.1
: 3<>"/dev/tcp/$address/$port"
