<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# The in-game relay and game tickets

The game used to need the launcher for as long as it ran: the launcher's relay on `127.0.0.1:61100` carried every
request to the server over TLS pinned to the invite, and the game logged in with the account key on its command
line. Now the runtime the game loads runs that relay itself, and the game logs in with a one-time ticket, so the
launcher can close once the game is up and the key never enters the game.

## How a launch goes

1. PLAY runs its checks as before, then looks for `dainty-lass:in-game-relay:v1` in the MIT runtime's bytes (the
   line the runtime logs when its relay starts). A runtime without it predates the in-game relay.
2. **The runtime carries the relay:** the launcher picks the loopback port the game will use (61100, or the first of
   41100 to 41109 that binds, as its own relay would) and runs no relay. It adds `DAINTY_SERVER_HOST`,
   `DAINTY_SERVER_PORT` and, for a public server, `DAINTY_SERVER_FP` (the invite's certificate SHA-256) to the game's
   environment. **It does not:** the launcher starts its own relay as before and removes those variables.
3. The launcher posts the key to `POST /dainty/api/GameTicket` and gets `{"ticket":"DLT_<48 hex>",
   "expiresInSeconds":300}`. The ticket goes on the command line as `-AUTH_PASSWORD=`, and into the launcher's
   redaction list. A server or gateway without the route (404, 405, a 403 that is no account refusal) gets the key
   there, as before.
4. In the game, the runtime's Init (client role only) reads the three variables with the backend address
   (`rustic_isles_runtime_core::config::relay`): the backend must be `http` on `127.0.0.1` or `localhost`, and its
   port is where the relay listens. A refused value is logged by its reason, never echoed, and no relay starts. The
   relay runs on its own thread (`dainty-relay`) on a current-thread Tokio runtime, and logs to the game's log with
   a `relay` prefix.
5. The game logs in at `/account/api/oauth/token` with the ticket as its `exchange_code`. `APIKEY` mode redeems a
   `DLT_` value once, within five minutes, checks the account is still eligible, and answers the game token as it
   does for a key. A second use, an expired ticket or one revoked by a ban is 400 or 403.

## What changed where

| Part | Change |
| --- | --- |
| `crates/dainty-relay` (new, MIT) | The relay, its local rules, the owner check over any table source, pinned TLS and the pooled connector, moved from `dainty-launcher-kit` unchanged in behaviour. The kit re-exports them at their old paths and keeps netstat as its table source. |
| `crates/rustic-isles-runtime` | `relay.rs` starts the relay; `ffi/tcp_table.rs` reads the TCP table through the IP helper (`GetExtendedTcpTable`) in netstat's lines, so the owner check is the launcher's, against the game's own pid. Under Wine there is none, as the launcher has none off Windows (RD21). |
| `crates/rustic-isles-runtime-core` | `config::relay`, the pure half: the variables, their checks and the capability marker. |
| `crates/rustic-isles-auth` | `game_tickets`: minted, kept by SHA-256 in memory (4096 at most), redeemed once; the `APIKEY` login takes them. |
| `crates/rustic-isles-launcher-api` | `POST /dainty/api/GameTicket`, refused as GetUserInfo refuses; a ban drops the account's tickets. |
| `crates/dainty-gateway` | `gameticket` (POST) is a public launcher API route. |
| `crates/dainty-launcher-kit` | PLAY: the marker check, the port choice, the variables and the ticket, with the old path kept for older runtimes and servers. |

## Limits

- A ticket is single-use. A game that logs in a second time in one session (a return to the title screen that signs
  in again, if 1.14.7 does that) is refused; Paradox's `LAUNCHER` mode, which the game ran against, has the same
  one-time code. Restarting the game from the launcher mints a new one.
- The in-game relay binds the port the launcher chose. If Windows lends that port out in the moment between the
  launcher's check and the game's start, the relay moves to an alternate port the game was not told about, and the
  game cannot reach the server; PLAY again picks a free one.
- The owner check is per connection, as in the launcher; in the game it can't be fooled by another process, and code
  injected into the game was already past it (RD9).

## Hands-on checks for the maintainer

Nothing here is played against the real client in CI.

1. Windows, public server with this branch's runtime and server: PLAY, reach the menu signed in. The launcher log
   says `relay in the game`; the game's log has `dainty-lass:in-game-relay:v1: relay on 127.0.0.1:61100`. Close the
   launcher: the game stays connected through a hunt, Ramsgate and chat.
2. The same with a private (Tailscale) server.
3. The game's command line (Task Manager, Details, Command line) shows `-AUTH_PASSWORD=DLT_...`, never `DLK_`;
   running the game again by hand with that line fails at login.
4. A server from before this branch: PLAY still works with the key and the launcher's relay (`relay in the
   launcher`).
5. Proton: PLAY through the inject helper; the relay starts in the game with no owner check, and the game reaches the
   menu.
6. Return to the title screen and sign in again, to see whether the game logs in a second time with its ticket.
