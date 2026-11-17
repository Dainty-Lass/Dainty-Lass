<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime staff kick (RD27)

A staff kick, ban or time-out must take the player out of the game server they are on, not only out of chat and the
backend. Before this page the backend refused the player's own calls and their game server's writes for them
(`feat/dashboard-grants` 4960fb2d), but the player stayed in Ramsgate or the hunt until they left on their own. This
page gives the Director a way to tell its game servers, and the MIT runtime a way to send the player's client back to
the menu. It is new behaviour: nothing here is ported, and nothing was read from `restricted/`, Dauntless Revived or
Mystic Paradox. What crossed is engine facts (UE 4.26 names and the offsets already in `offsets.rs`).

Conventions are R0's: **S** server, times in `GetTickCount64` milliseconds unless a line says Unix milliseconds.
**(call)** marks a decision this page makes, **(ambiguity)** something only the game can settle; both are collected
in section 8.

---

## 1. Scope

| What | Where |
| --- | --- |
| The backend asks the Director to kick a player | `rustic_isles::Shared::kick_player` → `DedicatedServers::kick` |
| The Director writes a kick file for each server it runs | `rustic-isles-director` `Director::kick` |
| The server reads the file on the SP-3 tick and sends the player's client to the menu | `rustic-isles-runtime` `server_life::kick`, pure half `rustic-isles-runtime-core::kick` |

Not here: player-hosted hunts (R10's hosts run on players' machines, out of the Director's reach; the backend already
refuses their writes for a kicked player), and a server-side forced close of a client that ignores the call (8.3).

## 2. The channel (call 1)

Three channels were weighed:

1. **The server polls the backend** from the SP-3 tick. It would need an HTTP client in the game process, the game
   server key in a second place, and a request per server per second for the life of every server.
2. **R10's control pipe.** It exists only for player hosts, which the Director does not run; a Director server has no
   pipe.
3. **A file beside the game's executable**, written by the Director and read by the server. The runtime already reads
   files there (the `.flag` files, RT-14) and writes its log there; the Director already knows the executable's path
   (`GAMESERVER_BINARY_PATH`) and every server's port.

The file is taken: it adds no network code, no secret and no open handle to the game process, and the read is one
`GetFileAttributes`-cost check a second, only on a listening server. Anyone who can write beside the executable can
already replace the executable.

## 3. The file

- Name: `rustic_isles_kick_port<port>.txt`, `<port>` the server's listen port (R1 `config::port()`), beside the game's
  executable (`path_next_to_exe`).
- Content: one request a line, `<unix ms> <user id>\n`, appended. The user id is the game's own (`accounts.user_id`,
  the id the backend and the Director use): 1 to 128 printable ASCII characters, no space.
- The Director appends one line to the file of **every** server it lists (it does not know which one the player is
  on: the hub keeps no roster). Each server acts only on the players it has.
- The server reads the whole file and **removes it before acting**, so a request is carried out once and a file it
  cannot remove is not repeated forever (`remove_file` failing is not retried).
- A line is dropped when malformed, when its time is more than 120 s from the server's clock either way (a file left
  from an earlier server on the same port), or past the 64th id of the file. A repeated id counts once.

## 4. The server's step

Run from R8's SP-3 after handler, after the connection count, while R9 listens (the same guard as R8's steps):

1. The first time: read `DISABLE_STAFF_KICK.flag` once and log the config line (section 6).
2. At most once every 1000 ms (tick clock): if the file exists, read it, remove it, parse it (3), and add the ids to
   the waiting list with the current tick time.
3. With nothing waiting, stop.
4. For each client connection of the net driver (R9 `client_connections`): its owning actor
   (`UNetConnection.OwningActor`, R8's offset) when it is a live `PlayerController`; its player state
   (`APlayerController.PlayerState`, R7's offset); the player id read from the player state's net id (5).
5. Each waiting id that a connected player has is taken out of the list and that controller is sent to the menu (5).
   An id that waited more than 30 s with no such player is dropped and logged: the player is on another server, or
   left.

## 5. Reading the player and sending them to the menu

**The player id (ambiguity 1).** `APlayerState.UniqueId` is an `FUniqueNetIdRepl`; R7's `PS_NET_ID` is the pointer to
the net id object inside it. For the game's MCP sign-in the object is an `FUniqueNetIdString`, whose `UniqueNetIdStr`
(`FString`) follows the vtable. UE 4.26 puts it at +0x08; a build whose `FUniqueNetId` derives from `TSharedFromThis`
puts it at +0x18. The read tries +0x08, then +0x18, and takes the first that reads as a plausible id (section 3's
rule). A wrong offset reads garbage that matches no requested id, so it can't kick the wrong player; it can only kick
nobody, which the `connected ids` line (6) shows at once.

**The call (call 3).** On the controller, through R17's `invoke` (reflection by name, the hooked `ProcessEvent`), with
one `FText` parameter: first `ClientReturnToMainMenuWithTextReason`, then, when the class has no such function,
`ClientWasKicked`. Both are reliable client RPCs of `APlayerController`: called on the server, the engine sends them to
that client, which leaves for the main menu and so closes its connection. No new address in the executable is needed;
a game without either function logs that no call answered.

**The text (call 4).** `You were removed from the server by staff.`, made once by R17's `text_for`
(`Conv_StringToText`). English: the runtime has no catalog; the launcher's own messages are translated.

## 6. Log lines

```text
[MitRuntime] [Kick] on: watching rustic_isles_kick_port<port>.txt next to the executable
[MitRuntime] [Kick] off (DISABLE_STAFF_KICK.flag)
[MitRuntime] [Kick] request read: <n> id(s), <m> player(s) connected
[MitRuntime] [Kick] connected ids: [<id or ?>, ...]        (once per server: checks the net id read)
[MitRuntime] [Kick] <id> sent to the menu (<function>)
[MitRuntime] [Kick] <id> could not be kicked: no client call answered
[MitRuntime] [Kick] <id> not on this server; request dropped
```

The Director logs `[Kick] <id>: asked <n> game server(s) to send them to the menu`, and a warning naming the file when
one can't be written. No line carries a key, a token or a reason the staff gave.

## 7. Flag

`DISABLE_STAFF_KICK.flag` beside the executable, read once at the first tick: no file is read and nothing is sent.
The backend's refusals (dashboard-grants) still apply.

## 8. Calls and ambiguities

1. **(call 1)** The file channel (section 2).
2. **(call 2)** Every server gets every request; a server that doesn't have the player drops it after 30 s, so a
   player who joins a server within that time (a hunt starting) is still kicked there.
3. **(call 3)** A client RPC rather than closing the connection on the server: it is a reflected call the runtime
   already makes safely, the client leaves through its own menu path (no half-torn connection on the server), and
   the game thread is never held.
4. **(call 4)** The reason text is English.
5. **(ambiguity 1)** The net id string's offset (section 5). Settled by the play-test's `connected ids` line.
6. **(ambiguity 2)** Whether 1.14.7's client honours `ClientReturnToMainMenuWithTextReason` (or `ClientWasKicked`) by
   going to the menu: the play-test settles it.
7. **(ambiguity 3)** A modified client that ignores the call stays connected. Its writes are refused by the backend
   (dashboard-grants), and the AFK timeout ends an idle one; a server-side close (`UNetConnection::Close`, a native
   address) is left for a later slice if the play-test shows a need.

## 9. Tests

In CI (`rustic-isles-runtime-core::kick`): the file name and line format; parsing (stale, future, malformed, spaced,
over-long, duplicate, over 64); the waiting list (taken when connected, dropped after 30 s, an id added twice keeps its
first time); the net id read at +0x08 and +0x18 and nothing for a null object; the log lines. The backend's side
(`Shared::kick_player`, the Director slot) runs in the module tests.

## 10. Play-test (maintainer)

1. Start the server, join Ramsgate. The Ramsgate log has `[Kick] on: watching rustic_isles_kick_port8777.txt`.
2. From the dashboard, kick the player. Within a second or two: the Director logs `asked <n> game server(s)`; the
   Ramsgate log has `request read`, `connected ids: [<your user id>]` (if the id is `?` or different, ambiguity 1:
   report the line) and `<id> sent to the menu (ClientReturnToMainMenuWithTextReason)` (or `ClientWasKicked`).
3. The client goes back to the main menu, ideally showing the reason. Note what it shows.
4. The other servers' logs (training, any hunt) have `<id> not on this server; request dropped` 30 s later.
5. Repeat in a hunt with a second player: only the kicked one leaves; the hunt goes on for the other.
6. Ban and time-out: the same, and the launcher refuses to start the game until it ends.
7. Put `DISABLE_STAFF_KICK.flag` beside the executable, restart, kick: the log says `off`, the player stays (the
   backend still refuses their writes).
