#!/bin/sh
# dainty-director: what `dainty-host start` does before the Director on Linux, then the Director. The game servers are
# its children (Wine on a virtual display) in this container, so they stop with it.
set -eu

root=/opt/dainty-lass
workspace=$root/.workspace
env_file=$workspace/config/director.env

if [ ! -f "$env_file" ] || [ ! -x "$workspace/bin/dainty-gameserver" ]; then
    echo "director | not set up yet: docker compose run --rm host setup ... (docs/hosting-docker.md)" >&2
    # EX_CONFIG; restart: unless-stopped tries again
    exit 78
fi

# Made here so the first game server's readiness timeout doesn't include it; setup usually made it already.
prefix=$workspace/wine
if [ ! -f "$prefix/system.reg" ]; then
    echo "director | making the Wine prefix (once, a minute or two)"
    mkdir -p "$prefix" "$workspace/logs"
    WINEPREFIX=$prefix WINEDEBUG=-all WINEDLLOVERRIDES="winmm=n,b;mscoree=d;mshtml=d" \
        xvfb-run -a wine wineboot --init >"$workspace/logs/wine.log" 2>&1 || true
    WINEPREFIX=$prefix wineserver -w || true
    if [ ! -f "$prefix/system.reg" ]; then
        echo "director | warning: the Wine prefix was not made; see .workspace/logs/wine.log" >&2
    fi
fi

game_exe=$(grep -E '^DAINTY_GAME_EXE=' "$env_file" | tail -n 1 | cut -d= -f2-) || game_exe=
case "$game_exe" in
    \"*\")
        game_exe=${game_exe#\"}
        game_exe=${game_exe%\"}
        ;;
esac
if [ -n "$game_exe" ] && [ -f "$game_exe" ]; then
    # A runtime switch the game servers read beside the executable
    flag="$(dirname "$game_exe")/CALL_HUNTPASS_QUERY.flag"
    [ -e "$flag" ] || : >"$flag" 2>/dev/null || true
else
    echo "director | warning: no game executable at ${game_exe:-(none)}: the Director runs but no game server can start" >&2
    echo "director | mount the 1.14.7 folder (DAINTY_GAME_DIR) and run setup with --game /game" >&2
fi

exec dainty-host director serve --env-file "$env_file"
