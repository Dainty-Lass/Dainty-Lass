<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Behaviour specification: the warm spare hunt server (prototype)

A hunt server spends most of its start in engine init: with batched hooks, 5.0-5.9 s from the Director's request
to the ready marker, of which about 4.5 s comes before the first map load (pak mount, engine init). A warm spare is
one hunt server started before anyone asks and held just before that map load, so a hunt request pays only the map
load and the listen. This is new behaviour, built from the engine facts below and the runtime's own SRV-1.
Nothing here is ported.

Off unless `settings.toml` has `warm_spare = true` under `[performance]` (`dainty-host setup --warm-spare`, undone
by `--no-warm-spare`). `setup` writes it to the Director's env file as `GAMESERVER_WARM_SPARE` (`1` or `0`), and
a file without the key keeps today's behaviour.

## 1. Engine facts (1.14.7, measured 2026-11-21)

- SRV-1's getter (`UGameMapsSettings::GetGameDefaultMap`) is called about 20 times per boot. Calls 1-13 come at
  once early in engine init, 14-19 about 0.2 s before `Starting Game.`, and call 20 at the same millisecond as
  `Starting Game.`: `StartGameInstance`, which then loads the URL it was given.
- The map load (`Starting Game.` to `Engine is initialized`) takes 0.3-0.5 s, and the first tick and the listen
  about 0.8 s more.
- The runtime's watchdog and the empty-server reaper start only after the listen, so a game thread held before the
  first map is neither reaped nor flagged. A 258 s hold ended normally, and the engine's hang detector didn't fire.

## 2. The runtime (`rustic_isles_runtime_core::spare`, `listen::spare`)

- **Spare.** A full-launch server with `RUSTIC_ISLES_SPARE=1`.
- **Hold.** On SRV-1 call `RUSTIC_ISLES_SPARE_HOLD_CALL` (default 14), the game thread logs
  `[Spare] holding before the first map (SRV-1 call 14); waiting for rustic_isles_spare_port<port>.txt` and looks
  for that file next to the executable every 25 ms.
- **The file.** `key=value` lines: `map` (required, with any `?` options the Director adds), `behemoth`, `hunt`,
  `players` and `week`. Blank lines and `#` comments are skipped, and the last of a key wins. A line that isn't a
  known key, a value with a control character, or a `week` outside 1 to 999 refuses the file, which is logged; the
  spare keeps waiting. Like the kick and sim files, the file is removed before it is acted on. At most 16 KiB is
  read.
- **The hunt.** `config::launch()` answers the spare's own launch with the hunt's map, Behemoth, hunt id and roster
  (its key, port and address stay), and `config::trials_week()` answers the file's `week`. Until then the getter
  answers the placeholder from the command line, and the hunt's URL is built once, after the hunt arrives.
- **Lines.** `[Spare] hunt arrived after <ms> ms held: map=<map>` names the map only, never the roster or hunt ids
  (as `[MapUrl]`).

## 3. The Director (`gameservers::spare`)

- **One spare at a time.** It starts after Ramsgate is ready, after each handover, and after any hunt server exits,
  whenever a hunt slot is free: the spare takes one of `MAX_CONCURRENT_HUNT_SERVERS`.
- **Starting it.** A spare goes through the memory guard and takes a port from the pool. Its arguments are the
  placeholder map (`adventure_moss_triforce`), `NO_BEHEMOTH`, `NO_MM_HUNTID` and `NO_EXPECTED_PLAYERS`, and its log
  file is `port<port>_hunt.log`, with a `warm spare` launch header.
- **Handing it a hunt.** A hunt request takes the spare when it is alive and has not reported ready. The Director
  writes the command file whole and renames it into place, appends a `Warm spare handed a hunt` header with the
  hunt's line to the spare's log, lists the server, and waits for the ready marker under the usual timeout.
- **Fallbacks.** With no usable spare, the request starts a server as before. A spare that reported ready before
  its hunt (the runtime didn't hold) is stopped, and spares stay off until the Director restarts. A spare that
  exits while waiting gives back its port and reservation.

## 4. Measured (this PC, 2026-11-21)

| Hunt start | Before (8.0-8.2 s) | Batched hooks | Warm spare |
| --- | --- | --- | --- |
| Request to ready, no clients | 8.0-8.2 s | 5.0-5.9 s | 1.5 s (`adventure_moss_triforce`) |
| Join to ready, 4 clients loading | 12.1-14.7 s | 11.5 s | 2.5 s (trial, `arena_ramsgate_00`) |

- A held spare costs 0.01 cores and 934 MB of commit (956 MB working set).
- The spare-served trial ran with 3 real clients and pinned its trials row (`Scheduled_Arena_Hunt_073`). Its
  `PlayerHuntID is empty` engine errors also appear on trials started without a spare.

## 5. Open before this leaves prototype

1. Call 14 is an index into one build's sequence of getter calls. A changed game build could move it, and the
   fallback in section 3 then turns spares off. A hold keyed to `StartGameInstance` itself would not depend on the
   count.
2. The placeholder's early getter answers may make the engine preload the placeholder map's packages. Compare a
   spare-served hunt's memory with a cold one on several maps.
3. A spare log file is named `port<port>_hunt.log` whatever its Behemoth, unlike a cold hunt's `port<port>_<label>.log`.
4. Hands-on: a full hunt, a Gauntlet and an escalation through a spare, with players fighting.
