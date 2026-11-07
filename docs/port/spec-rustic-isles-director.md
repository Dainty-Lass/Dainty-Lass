# Clean-Room Behaviour Specification: `crates/rustic-isles-director` (RD27)

> **Scope and purpose (RD27):** this is the clean-room behaviour specification for an MIT crate,
> `rustic-isles-director`: the Director of Rustic Isles, the service on the module's `director` port (61001) that
> starts, watches and stops the game servers (Ramsgate, the Training Grounds, one process per hunt), leases the hunt
> UDP ports, guards memory, resolves hunts from the hunt tables, and imports those tables from a CatalogExporter
> capture. It is unit 4 of [`spec-rustic-isles-core.md`](spec-rustic-isles-core.md) §16 and plugs into that crate as
> a `PortService` (core §11.2) and as the module's dedicated-servers slot (core §5.6). Its source is
> dainty-restricted's `modules/rustic-isles/crates/isles-director`, which is AGPL-3.0-only with Mystic Paradox's
> section 7 terms; an implementer who never sees it writes the MIT crate from this document alone.
>
> Everything below is stated from observable contracts: the routes and their bytes, the game server's command line
> and environment, the log files, the timings, the settings, the table formats, the importer's output files and the
> tests. No implementation code is reproduced and no private helper is named. Node file names appear only as
> provenance. Wire names the game runtime, the backend or a host's tooling depends on
> (`RUSTIC_ISLES_GAMESERVER_READY`, `RUSTIC_ISLES_GAMESERVER_LAUNCH_ID`, `RUSTIC_ISLES_TRIALS_WEEK`,
> `rustic_isles_dll_port<port>.log`, `[NetConnEdge]`, `deployserver`, the DataTable field names) are interface facts
> and are kept.
>
> Written against dainty-restricted `e0e238b4434316bb210677d88bc59e3448688794` (worktree
> `splashloader-integration-90130d`; the files of this unit last changed in `4efc8b5`) and dainty-lass
> `f9cb7b4b26309c7cd2d9ca9ea03bc93a65417978` (branch `feat/isles-auth-clean-room`).

---

## 1. Provenance and licensing classification

### 1.1 Evidence used

| Source | What it says |
| --- | --- |
| `modules/rustic-isles/NOTICE.md` | The whole module, every crate and file, is AGPL-3.0-only with Mystic Paradox's section 7 terms |
| File headers (`//!`) | Which TypeScript each file ports (`ParadoxDirector/src/...`, `shared/hunt-manifest`, `scripts/import_hunt_tables.cjs`) or which Dainty Lass row it was written for |
| `git log` of the crate | Every commit is the maintainer's: `9c6f786` (RU6), `aaaffcb`, `28b6128` (Windows spawn in the job), `dcad4c1`, `3691abc` (RU9 in-process Director), `2824b6d` (RB11 importer), `9ad97c6` (rename), `04992f5`, `4efc8b5` (flat crates) |
| `crates/isles-1-14-7/src/lib.rs` | The 1.14.7 `DIRECTOR` values (§12.4) and how the module composes the service |
| `crates/dainty-host/src/lib.rs` (restricted) | The two callers outside the module: the hidden `director` subcommand and `reforged-host gamedata`'s hunt-table step |
| dainty-lass `crates/dainty-hostkit/src/hostconfig.rs` | What `setup` writes into `director.env` (§11) |
| `native/runtime` and `native/catalog-exporter/HuntExporter.cpp` (restricted) | Only to confirm wire facts: the runtime prints the new readiness marker and reads the new env names; the exporter writes handles as `{"table","rowName"}` |

### 1.2 Classification and what is rewritten

Category A is code written for Dainty Lass with no Paradox code in it; Category B ports Paradox TypeScript or a
Dainty Lass patch to Paradox (the "Director patch 00xx" files are patches to Paradox, so B). As for core (§15 item 11
there), **the implementer writes every file from this spec**; nothing moves as it is.

| Restricted file (`crates/isles-director/`) | Cat. | Source | Rewritten as |
| --- | --- | --- | --- |
| `src/lib.rs` | mixed | RU6 service wiring (A); `GAMESERVER_ARGS`, `windowsHide` (B) | §12.2-§12.4 |
| `src/api.rs` | mixed | `src/app.ts`, `routes/matchmaker.ts` (B); Express 404 model (A) | §3 |
| `src/matchmaker.rs` | B | `controllers/matchmaker.ts` | §4 |
| `src/gameservers.rs` | B | `controllers/gameservers.ts`, `watchdog.ts` | §5 |
| `src/process.rs` | mixed | the 2026-07-19 freeze fix and DS5 (B); the tree handling (A) | §5.4-§5.7 |
| `src/os.rs`, `src/win_spawn.rs` | A | RU6, `28b6128` | §5.7, §12.5 |
| `src/cmdline.rs` | A | RU6 (MSVC argument rules) | §5.3 |
| `src/port_pool.rs` | B | `controllers/portPool.ts` | §6 |
| `src/hub.rs` | B | `controllers/hubSupervisor.ts` | §7.1 |
| `src/occupancy.rs` | B | `controllers/hubOccupancy.ts` (patch 0033) | §7.3, §7.4 |
| `src/memory.rs` | B | `controllers/memoryAdmission.ts` (patch 0022) | §8 |
| `src/hunt.rs` | B | `shared/hunt-manifest`, the audits of `gameservers.ts` | §9 |
| `src/hunt_tables.rs` | B | `scripts/import_hunt_tables.cjs` (RB11) | §10 |
| `src/config.rs` | B | `server.ts`, `gameservers.ts` env reads | §11 |
| `src/log.rs`, `src/random.rs` | A | RU6 | §2.3; §5.2 (ids), §9.3 (picks) |
| `src/bin/isles-fake-gameserver.rs` | A | RU6 | §13 |
| `tests/*` | A (authored); `units.rs` and `hunt.rs` port Node tests' cases (B) | | §14, described only |

### 1.3 What stays in dainty-restricted

- The 1.14.7 version crate's composition (`isles-1-14-7`: `DIRECTOR`, the module spec listing the services) until
  unit 12 lands, and the hidden `reforged-host director` subcommand (restricted's `host`), which builds the module
  with the Director service alone.
- `reforged-host gamedata`'s call of the importer (restricted's `host`); it switches to this crate's function.
- The Node reference (`reference/1.14.7/ParadoxDirector`), its `*.example.json` tables, and the test that compares
  the importer's output byte for byte with the Node script (§14.1).
- The game runtime and the CatalogExporter (`native/`), which produce the readiness marker, the occupancy log and the
  hunt capture this crate reads.

### 1.4 dainty-lass's `crates/director`

`crates/director` is RR1's 12-line placeholder (package `director`, a `COMPONENT` constant and a `main` that prints
`director: not implemented yet` and fails), registered so the workspace listed every planned member when the
Director was to be a standalone `service-kit` binary. The Director is now a service of the isles module, so there is
nothing in it to reuse and nothing it should depend on. The new crate is `crates/rustic-isles-director`; whether the
placeholder is removed is the integration pass's call (a workspace-member change). The host kit's `director` names
(`director.env`, the `director` service row, `GeneratorKind::HuntTables`'s label naming
`isles_director::hunt_tables::import_hunt_tables`) are the host kit's and change only in its own row.

What it does reuse from dainty-lass: `service-kit` (`js`, `jsjson`, `json::Json`, `body::{parse_bodies, BodyConfig,
ExpressBody}`), `module-api` (`BoxFuture`, `ModuleError`, `PortRange`) and `rustic-isles` (core: `PortService`,
`Shared`, `Settings`, `GameDataRoot`, `Handlers`, `app`, `REQUEST_ID_HEADER`, `DedicatedServers` and its types).
`module-api`'s process tree (`tree.rs`) and the host kit's are crate-private and adopt a child after it is spawned;
the Director must create the game server inside its job (§5.7), so it carries its own platform layer.

---

## 2. Conventions

### 2.1 Text, JSON, numbers, times

- JSON answers as `JSON.stringify` writes them (`service_kit::json::Json`, `content-type: application/json;
  charset=utf-8`, insertion order: `serde_json` with `preserve_order`, as core §17 item 8).
- Tables are parsed as `JSON.parse` does (`service_kit::jsjson::parse`), and objects iterate in JavaScript's key
  order: keys that are array indexes (canonical decimal `0` to 2^32-2) first in ascending order, then the rest in
  insertion order. Every "for each row" below is in that order; it decides random picks under a fixed index, audit
  lists and the importer's output.
- **JS Number** of a setting is `service_kit::js::number` of its text: trimmed, empty is 0, unreadable is NaN, `0x`
  hex allowed. **JS String** of a JSON value is `service_kit::js::string` (absent `undefined`, `null` `null`, a number
  as JavaScript writes it, an array its items joined with `,`, an object `[object Object]`).
- A number written into a log line or error text is written as JavaScript writes it in a template (`5`, `0.1`,
  `NaN`).
- ISO times are `Date#toISOString`'s (`2026-11-05T12:00:00.000Z`). The Director's clock is the system clock, never
  core's replay scope (§16 item 15).

### 2.2 HTTP answers

Every answer passes core's pipeline (core §3.1, §3.8), so it carries `x-rustic-isles-request-id`, except the body
parser's error pages (§3.1). `HEAD` on a `GET` route answers the GET's status and headers with no body.

### 2.3 Logs

Lines go to `tracing` at the level stated. Texts are Node's, kept word for word so a host's greps still match; a line
"with the error" carries the error as a structured `error` field, not in the message. Never a setting's value, the
gameserver key or a game server's arguments (§15).

---

## 3. Routes (the `director` port, default 61001, bound 127.0.0.1)

The rows are core Appendix A's D1-D5. D1-D4 are this crate's handlers; D5 (`POST
/api/matchmaker/resolve-hunt-launch-manifest`, side `rifts`, when `rifts`) is answered by rustic-rifts through core
when rifts is loaded, and is unmounted otherwise. No gateway route reaches this port.

### 3.1 The Director's wrapping of core's dispatcher

The service's router is core's `app(shared, Service::Director, handlers)` with two layers around it, outermost first:

1. **Express 404 page.** Any 404 that comes out of the inner layers (core's empty 404 for an unmatched path, a
   method no row takes, or an unmounted D5) is replaced by Express's `finalhandler` page: status 404, headers
   `content-security-policy: default-src 'none'`, `x-content-type-options: nosniff`, `content-type: text/html;
   charset=utf-8`, and the request id header copied from the inner answer when it had one; body (for `HEAD`, none):

   ```text
   <!DOCTYPE html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n<title>Error</title>\n</head>\n<body>\n<pre>Cannot {METHOD} {path}</pre>\n</body>\n</html>\n
   ```

   (`\n` is a newline). `{path}` is the request path without the query, as received; the whole `Cannot ...` text is
   HTML-escaped (`&` `&amp;`, `<` `&lt;`, `>` `&gt;`, `"` `&quot;`, `'` `&#39;`). None of the handlers answers 404.
2. **Body parser** (`service_kit::body::parse_bodies`, limit 100 KiB = 102 400 bytes, Express's `express.json()`
   and `express.urlencoded()`): runs on every request before the pipeline. A JSON or form content type is parsed
   and stored as the request's `ExpressBody`; any other or no content type leaves it absent. Its failures (400
   malformed, 413 over the limit, 415 charset or encoding) are `service_kit::body`'s Express error pages and skip
   the pipeline, so they carry no request id (§16 item 2).

### 3.2 `GET /health/ready` (D1)

`200 {"ok":true,"service":"deployserver"}` once the persistent hubs' boot has finished (§5.11), else
`503 {"ok":false,"service":"deployserver"}`. "Finished" is set after the boot ran, whatever its outcome (§16 item 6),
and never when the Director cannot launch (§12.3).

### 3.3 `GET /api/hub-status` (D2)

`200 {"ramsgate":<hub>,"trainingDojo":<hub>}`, each `<hub>` being `null` unless that hub's supervisor has a current
server that is ready and whose process is alive at the moment of the answer (the "fencing": a cached ready is no
proof). A live hub is `{"ready":true,"host":<MY_IP>,"port":<n>,"processId":<pid>}`, with `host` left out when
`MY_IP` is unset and `processId` left out when the pid is unknown. Changes no state.

### 3.4 `GET /api/instances` (D3)

`200` and an array, in start order, of every listed game server that is ready and alive:

```json
{"id":"<kind>-<port>","kind":"city|dojo|hunt","map":"<map>","behemoth":<string or null>,"players":<n>,"maxPlayers":<n or null>,"playerIds":[...],"startedAt":"<ISO>"}
```

`kind` is `city` for Ramsgate, `dojo` for the Training Grounds, else `hunt`; `players` the length of its expected
roster (0 without one); `maxPlayers` 0 for a hub, `PUBLIC_HUNT_MAX_PLAYERS` for a hunt (`null` when that is not a
finite number; an integer is written without a fraction); `playerIds` the roster's ids as they were sent (any JSON
value); `startedAt` the ready time. No pids, addresses or launch ids. Before any start: `[]`.

### 3.5 `POST /api/matchmaker/handle-matchmaking-for-player` (D4)

The body is the parsed `ExpressBody` (absent when the content type was neither JSON nor a form). §4 decides; success
is `200` and the connection details `{"host":<MY_IP>,"port":<n>}` (`host` left out when `MY_IP` is unset; an empty
`MY_IP` is `""`). Any failure is logged at error as `handle-matchmaking-for-player failed: {error}` and answered
`500 {"error":"matchmaking_failed"}` (the backend treats any non-200 as a failed matchmaking; a failure is never
redirected to Ramsgate, which would make the client claim a hunt was found).

---

## 4. The matchmaking request

### 4.1 Reading the body

An absent body is the error `Cannot read properties of undefined (reading 'GameMode')`. Otherwise the fields are
`GameMode`, `GameArgs`, `HuntId`, `ExpectedPlayers` (each possibly absent), and the Director logs at info
`Handling matchmaking with GameMode: {String(GameMode)} HuntId: {String(HuntId)} and GameArgs: {String(GameArgs)}`.

**Non-blank** (for `HuntId` and `GameArgs`): absent or `null` is "none"; a string is itself when its JS-trimmed text
is not empty (the untrimmed text is used), else none; any other type is the error `HuntId.trim is not a function`
(the same text for `GameArgs`, as Node's throw reads; §16 item 3).

**Length** of `ExpectedPlayers`: an array's item count, a string's UTF-16 length, otherwise none.

### 4.2 By `GameMode` (only a string counts)

| `GameMode` | Behaviour |
| --- | --- |
| `CITY` | Ramsgate's connection details (§5.9) |
| `SHARED` | With a non-blank `HuntId`: equal to `ShatteredIsles_TrainingDojo` gives the Training Grounds for a request (§7.3). Otherwise, when `ExpectedPlayers` is present, not `null` and has a length above 0: public hunt reuse when it applies (§5.10), else a hunt started from the hunt id (§5.8); a failure there is also logged at error `SHARED hunt '{hunt}' failed to resolve/start: {error}`. Otherwise logged at error `SHARED hunt '{hunt}' had no ExpectedPlayers; cannot start a hunt server` and the error `SHARED hunt '{hunt}' had no ExpectedPlayers`. A blank `HuntId` falls to "unsupported" |
| `ISLAND` | With a non-blank `GameArgs`: a hunt from travel arguments (§5.8). Else, with a non-blank `HuntId` and `ExpectedPlayers` present and not `null`: a hunt from the hunt id. Else falls to "unsupported". A failure of either start is logged at error `ISLAND hunt (huntId='{String(HuntId)}') could not start: {error}` and returned |
| anything else | "unsupported" |

**Unsupported**: logged at error `Matchmaking failed: unsupported request GameMode='{String(GameMode)}'
HuntId='{String(HuntId)}'`, error `Unsupported matchmaking request GameMode='{..}' HuntId='{..}'`.

### 4.3 Travel arguments (`GameArgs`)

A travel URL `map?Key=Value?Key=Value...`. The **map** is the text before the first `?` (possibly empty). A key's
value is that of the first `?`-separated part after the map that is `Key=...` (the text after the first `=`, so
`a?K=v=w` gives `v=w`) or exactly `Key` (empty value); keys match case-sensitively. A hunt from travel arguments
starts with:

- map: the map text, unchanged;
- Behemoth: `MonsterClass`'s value, or none;
- matchmaker hunt id: `HuntID`'s value when its trimmed text is not empty, else, when `HuntId` is non-blank, the
  resolver's pick for it (§9.3; its error fails the request), else none;
- expected players (§4.4).

### 4.4 Expected players (travel-argument hunts)

None when `ExpectedPlayers` is absent, `null` or of length 0, or when `HuntId` is not non-blank (a non-string
`HuntId` is the `.trim` error). Otherwise `ExpectedPlayers` must be an array (else the error `PlayerIds.map is not a
function`) and each item, as sent, is paired with the `HuntId` text.

---

## 5. Game servers

### 5.1 The registry

The Director keeps an ordered list of game servers. Each has a launch id (a random lower-case UUID v4 from the
system generator), its UDP port, map, optional Behemoth, optional matchmaker hunt id, whether it is Ramsgate or the
Training Grounds (else a hunt), its start time, its process, a state (`starting`, `ready`, `failed`), its ready time
and an optional expected roster (ids as sent, each with the player hunt id it came for). A server is listed from just
after its process starts until it fails its readiness wait, exits, or is cleaned up.

### 5.2 Starting one

In order; an error at any step is returned and undoes what was taken:

1. The Director is stopping: `the Director is stopping`.
2. **Hunts only, the stagger**: the hunt launch queue gives this start the slot `max(queue tail, now)` and moves the
   tail to slot + stagger; the start waits until its slot. Stagger: `SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP` ×
   1000 ms when finite and at least 1, else 1 ms (`setTimeout` of a non-positive or NaN delay).
3. A new launch id.
4. **Port.** Ramsgate: the range's last port. Training Grounds: the one before it. A hunt, checked after the
   stagger so a burst cannot race past it: when the listed hunt count is at least `MAX_CONCURRENT_HUNT_SERVERS`
   (compared as numbers, so NaN never refuses; §16 item 7), the error (kind "no capacity") `Hunt server capacity
   reached ({count}/{cap})`; then the memory guard reserves (§8; refused: warn `[MemoryGuard] refusing a new hunt
   ({count} running): {reason}`, error kind "memory" with that reason); then a port lease for the launch id (§6;
   none free: `No free ports left!`).
5. **Label**: `ramsgate`, `training_dojo`, else the Behemoth path's last `/` segment cut at its first `.`
   (`/Game/Monsters/fulgar/fulgar_bp.fulgar_bp_C` is `fulgar_bp`), else `hunt`.
6. **Log file** (§5.5): its folder is created (failure `{folder}: {error}`) and the file opened for append (failure
   `{path}: {error}`); the launch header is written.
7. **Environment** (§5.4) and **command line** (§5.3). No `GAMESERVER_BINARY_PATH` or no `METAGAME_API_KEY`:
   `GAMESERVER_BINARY_PATH and METAGAME_API_KEY must be set to start a game server`.
8. **Spawn** (§5.7); failure `spawn {program} {os error}`. The pid is recorded (on a hunt's lease too), `pid={pid or
   unknown}` plus a blank line goes to the log file, and info `{label} spawned on port {port} (pid={pid or unknown})
   -> {log path}`. The server is appended to the list as `starting`.
9. **Readiness wait** (§5.6), at most the ready timeout: `GAMESERVER_READY_TIMEOUT_MS` when finite and in
   [1, 2 147 483 647], else 1 ms.
   - Ready: state `ready`, ready time now, info `{label} ready on {MY_IP or undefined}:{port} after {ms}ms`.
   - Not ready: state `failed`, removed from the list, the memory reservation given back. When the process is still
     alive it is stopped (§5.7) and its port comes back with its exit; when it is gone already, a hunt's lease is
     released now. Error (kind "startup"): `{label} port {port} exited before ready (code={code}, signal={signal})`,
     or `{label} port {port} did not report ready within {GAMESERVER_READY_TIMEOUT_MS as a number}ms`.

A failure before the spawn releases the hunt's lease and gives back its reservation. A successful hunt keeps its
reservation until it lapses (§8).

### 5.3 The command line

The program is `GAMESERVER_BINARY_PATH` (on Linux `setup` points it at the host kit's Wine/Proton wrapper, which
reads its own `REFORGED_*` variables from the environment). Arguments, in the order the runtime reads them:

| # | Argument | Hub value |
| --- | --- | --- |
| 1 | `METAGAME_API_KEY` | the same |
| 2 | port | the hub's port |
| 3 | map | `/Game/Maps/ramsgate/ramsgate_01_persistent`, `/Game/Maps/islands/dojo/training_dojo_persistent` |
| 4 | Behemoth, or `NO_BEHEMOTH` | `NO_BEHEMOTH` |
| 5 | matchmaker hunt id, or `NO_MM_HUNTID` | `NO_MM_HUNTID` |
| 6 | the roster as `{String(id)}:{playerHuntId}` joined with `,`, or `NO_EXPECTED_PLAYERS` without a roster (an empty roster is the empty string) | `NO_EXPECTED_PLAYERS` |
| 7 | `{MY_IP or undefined}:{port}` | the same |
| 8.. | the version's standard switches, then `-LogCmds="{value}"` (§11, §12.4) | the same |

Each argument reaches the process as exactly one argument: on Unix as one `argv` entry (no shell); on Windows as a
command line the MSVC runtime (and Unreal's parser after it) splits back into the same list: the program quoted
whole (`"{program}"`), each argument after one space, an argument that is empty or contains a space, tab, newline,
vertical tab or `"` written in quotes with each `"` escaped by `\` and the backslashes before a `"` (and before the
closing quote) doubled; other arguments as they are. An argument containing NUL refuses the spawn on every platform
(§16 item 12).

| Arguments | Windows command line |
| --- | --- |
| `C:\Game\x.exe`; `plain`, `-LogCmds="A B"`, (empty), `a\b\`, `x y\` | `"C:\Game\x.exe" plain "-LogCmds=\"A B\"" "" a\b\ "x y\\"` |

### 5.4 The environment

The child gets this environment and nothing else (cleared, then set):

1. The module's settings `vars()` (core §6.1: switches, the env file over them, the forced values over those).
2. The controller's process environment over them (as Node's `--env-file`, the process wins). Variables whose name
   or value is not valid Unicode are skipped (§16 item 13).
3. `RUSTIC_ISLES_GAMESERVER_LAUNCH_ID` and `RUSTIC_ISLES_TRIALS_WEEK` removed, then
   `RUSTIC_ISLES_GAMESERVER_LAUNCH_ID={launch id}` set, and `RUSTIC_ISLES_TRIALS_WEEK={www}` when the matchmaker hunt
   id is `Arena_MatchmakerHunt_{Easy|Hard|Elite}_{www}` with `www` three ASCII digits (an arena hunt keeps the week it
   was launched for; the hubs follow the runtime's own rotation).

On Windows the block is sorted by upper-cased name. No `MYSTICPARADOX_*` copies are added (core §17 item 2; the
runtime reads the new names).

### 5.5 The log file

`{GAMESERVER_LOG_DIR}/port{port}_{label}.log`, appended to. In order:

1. The launch header: `=== Gameserver launch {local time as M/D/YYYY, h:mm:ss AM} (local time) ===\n` then
   `port={port} map={map} behemoth={Behemoth or NO_BEHEMOTH} mmHunt={id or NO_MM_HUNTID} players={argument 6}\n\n`.
2. `pid={pid or unknown}\n\n` (the process's first output may come before it).
3. Everything the process writes on stdout and stderr, as raw bytes.
4. After its exit: `\n=== Gameserver process exited port={port} label={label} pid={pid or undefined} code={code or
   null} signal={signal or null} ===\n`; the same text (without the `===` frame) is logged at warn.

The file belongs to one writer that ends when the process's pipes and its exit are all done; a write failure stops
writing but never blocks or fails the readers (a server that dies at once closes its pipes before its exit is seen,
and its exit line must not crash anything: Node's DS5 "write after end").

### 5.6 Readiness and the pipes

Both pipes are read to their end, always, in chunks (the 2026-07-19 freeze: a paused stdout filled the pipe and the
game thread blocked holding the CRT's stdio lock). stdout is also scanned line by line (split at `\n`, lossy UTF-8)
until the marker is found:

- A JS-trimmed line `RUSTIC_ISLES_GAMESERVER_READY launchId={id} port={port}` with `{id}` non-empty of `0-9`, `a-f`,
  `-` and `{port}` non-empty ASCII digits is a marker. For this launch id and port: ready, and the scan stops. Any
  other well-formed marker: warn `Ignoring mismatched gameserver readiness marker on port {port}: {line}`.
- A scan buffer without a newline over 65 536 bytes keeps only its last 4 096.
- stdout ending without a marker means the process is going: the wait takes its exit and fails as "exited before
  ready". The process's exit during the wait fails it the same way; the timeout fails it as "did not report ready".

The pre-rename marker `MYSTICPARADOX_GAMESERVER_READY` is not accepted (core §17 item 2).

### 5.7 Processes and their trees

Each game server leads a process tree that ends with it:

- **Windows**: created hidden when `GAMESERVER_WINDOWS_HIDE` is not exactly `0` (unset is `1`; show state SW_HIDE),
  inside a new job object from its first instruction (the job attribute at creation, not assigned afterwards), the
  job with kill-on-close so the tree also ends when the Director's process dies. Only its three standard handles are
  inherited (stdin the null device, stdout and stderr pipes), never another handle of the controller. The pipes are
  read without blocking a thread (a blocked pool thread would hold the runtime's shutdown while the game lives), and
  the exit is polled every 50 ms; its code is reported as the signed 32-bit value. Asking a tree to stop ends the job.
- **Unix**: a new process group the server leads, stdin the null device, the two pipes; on Linux the child gets
  SIGTERM when the thread that started it goes (parent-death signal). Stop is SIGTERM to the group; kill is SIGKILL
  to the group; dropping the last handle to a tree kills the group. The exit reports the code or the signal name
  (`SIGHUP`, `SIGINT`, `SIGQUIT`, `SIGILL`, `SIGABRT`, `SIGBUS`, `SIGFPE`, `SIGKILL`, `SIGSEGV`, `SIGPIPE`,
  `SIGTERM`, any other `SIG{n}`).
- When the leader exits, whatever is left of its tree is killed at once (a stray Xvfb would hold the port).
- **Stopping one** (a timed-out start, an idle Training Grounds, the shutdown): an error `the process is gone` when it
  has exited; else the tree is asked to stop, and if the leader is still alive after the stop grace (10 s), warn
  `a game server (pid={pid}) ignored its stop; killing its process tree` and kill it.

### 5.8 Hunts

- **From travel arguments** (§4.3): a hunt start with the map, Behemoth, matchmaker hunt id and roster as derived;
  answers the connection details for its port.
- **From a hunt id** (SHARED, or ISLAND without travel arguments): the resolver's manifest for the hunt id and
  `ExpectedPlayers` (§9.3; its errors fail the request), then a hunt start with map `String(mapPath)`, Behemoth
  `String(behemothPath)` unless it is `NO_BEHEMOTH`, matchmaker hunt id `String(matchmakerHuntId)`, and the roster
  `ExpectedPlayers`' items each with the hunt id text.

Hunts are never restarted and never reaped by wall clock: the runtime ends a hunt nobody is connected to.

### 5.9 Connection details of a hub

- **Ramsgate** with `RAMSGATE_HUB` off: the error `the dedicated Ramsgate hub is switched off (RAMSGATE_HUB=off)`.
- A hub whose supervisor's current server is ready and alive: its details.
- Otherwise, with `HUB_PROMPT_RESTART` on: a current server whose process is gone is cleaned up (reason `getter:
  process gone`, §5.11); no current server asks the supervisor for one (reason `request`, not awaited). Then, in
  every case, the error (kind "startup") `Ramsgate is not ready` or `Training Dojo is not ready`.

### 5.10 Public hunt reuse

Only with `ENABLE_PUBLIC_HUNT_REUSE` exactly `1` and a player hunt id whose trimmed text is not empty. The first
listed server that is `ready` (its process is not checked; §16 item 8), a hunt, with a non-empty roster whose first
entry came for this player hunt id and fewer entries than `PUBLIC_HUNT_MAX_PLAYERS` (a number comparison; NaN never
reuses) takes the joiners: each item of an `ExpectedPlayers` array (a string: each of its characters as a
one-character string) not already in the roster (JSON equality) is appended with this hunt id. Info
`[PublicHuntReuse] joining existing server port={port} pid={pid or undefined} huntId={id}
roster={count}/{max}`; the answer is that server's details. The roster only grows. Keyed on the player hunt id, not
the matchmaker row (picked at random, which would split a party).

### 5.11 Exits, clean-up, the watchdog, start-up and shutdown

**An exit**: a hunt is removed from the list and its lease released (owner-checked). A hub's exit, with
`HUB_PROMPT_RESTART` on, cleans it up with reason `exit code={code} signal={signal}`; off, it is left to the
watchdog.

**Clean-up of a server** (idempotent: the exit, the getters and the watchdog may all report the same one): while
stopping, it is only removed from the list. Otherwise it is removed when listed, then:

- the Training Grounds being stopped for idleness (§7.3): the idle stop ends, the policy resets, the supervisor
  releases the hub, and when that succeeds the hub is no longer wanted and info `[TrainingOnDemand] the Training
  Grounds stopped (no player for {minutes} min); they start again on the next request`;
- a hub that is its supervisor's current server: warn `RAMSGATE HAS FALLEN ({reason})! Restarting!` or `Training Dojo
  down ({reason})! Restarting!`, then the supervisor's exit handling (§7.1);
- a hub that was listed but never current (it exited during its own readiness wait, whose failure drives the
  retry): info `[HubSupervisor] {label} pid={pid or undefined} exited during startup ({reason}); its start attempt
  handles the retry`;
- a hunt: its lease released (owner-checked).

**The watchdog**, every 60 s from the service's start (first pass after 60 s), only when the Director can launch:
info `Running Gameserver Watchdog!`; every listed server whose process is gone is warned `Cleaning up dead gameserver
on port {port}` and cleaned up (`watchdog: process gone`); orphaned leases are reclaimed (§6); Ramsgate's supervisor
ticks when `RAMSGATE_HUB` is on, the Training Grounds' when wanted; the idle-stop check (§7.3); the missing-hunt
summary (§9.6); processes that have exited and are no longer referenced by a server or a lease are forgotten. A
pass that fails (panics) is logged at error `Gameserver watchdog pass failed` and the next pass still runs.

**Start-up** (in the background once the listener is up; §12.3): the table audit (§9.7); the hunt ports, the
range's first port to its last but two, go into the pool (none when the range is shorter; §16 item 9); then the
hubs one after the other, Ramsgate first: each with reason `boot`, or info `[RamsgateHub] RAMSGATE_HUB=off - no
dedicated Ramsgate hub (realms run their own on the host's PC)` and `[TrainingHub] TRAINING_HUB=off - the dedicated
Training hub starts on its first request (fallback for hosted Training)` when switched off. A hub that failed is
reported, never fatal: warn `Persistent hub startup reported failures ({labels joined by "; "}) - continuing with
whichever hubs are up; retrying with backoff until up`. Then the hubs' boot is finished (§3.2).

**Shutdown** (the service's `stopping`): stopping is set (no restart, no new start), every supervisor's retry timer
cancelled, every listed server's tree asked to stop; the Director waits, polling every 50 ms, until none is alive or
the grace plus 1 s (11 s) passed, kills what is left, and empties the list. A **kill-all** (a test that ends in a
hurry) sets stopping, cancels the timers and kills every tree at once.

---

## 6. Port leases (hunts)

A pool of free ports (a FIFO queue) and held leases (port, owner = launch id, optional pid), in the order taken.

- **Add** a port: ignored when it is already free or held.
- **Take** for an owner: the oldest free port, so a just-freed port is reused last; none when empty.
- **Attach a pid**: only for the port's current owner.
- **Release** (port, owner): only the current owner frees it (back at the end of the queue); a late or duplicate
  release from an old owner changes nothing and reports false.
- **Reclaim** (the watchdog's fallback when an exit never arrives): every lease whose owner is not a listed server
  and whose pid is none or no longer alive is released and warned `[PortPool] reclaimed port {port} from launch
  {owner} (pid={pid or none} gone, no exit event)`. A live pid keeps its lease even unlisted: that process may still
  own the socket.

The hubs' ports are fixed (§5.2) and never leased.

---

## 7. The hubs

### 7.1 The supervisor (one per hub; labels `ramsgate`, `training_dojo`)

State: the current hub server (none), one start in flight (none), a failure count, the time the next attempt is due
(0), when the current hub became current, one retry timer. Times are a monotonic clock in ms (a wall-clock step back
must not push a down hub's attempt into the future). Backoff: 5, 15, 30, 60, 120 s, then 300 s for every later
failure (failure n uses entry min(n, 6)). Stable uptime: 120 s.

- **Ensure (reason)**: a current server that is alive is the answer. A dead current one is first marked gone (below)
  with reason `{reason}: process gone`. A start in flight: every caller shares it (single flight). Before the due
  time: the retry timer is armed (if not already) and the answer is none, so requests cannot skip the backoff.
  Otherwise the timer is cancelled and a start runs on its own task (it finishes whether or not anyone awaits it).
  Never fails: a failed start answers none.
- **A start**: info `[HubSupervisor] {label} starting ({reason}, attempt {failures+1})`. The start function failing
  is a failure `start failed` with its error; a server that is already dead when its start returns is a failure
  `exited right after reporting ready`. Success: it becomes current, the due time resets to 0, info
  `[HubSupervisor] {label} up ({reason}; failures so far {n})`.
- **A failure**: the count goes up, the due time is now + its backoff, the start is no longer in flight, the timer is
  armed, error `[HubSupervisor] {label} {what} (failure {n}); retrying in {seconds}s` (with the error when there is
  one).
- **Marking gone** (only the current server; anything else is a late exit and is ignored): no current server. When a
  start is in flight its outcome decides the backoff and nothing else changes. Up at least the stable uptime: the
  count resets, due now, warn `[HubSupervisor] {label} exited after {s}s up ({reason}) - restarting now`. Earlier: a
  failure is counted and the due time set, warn `[HubSupervisor] {label} exited {s}s after ready ({reason}; failure
  {n}) - restart in {delay}s` (`{s}` rounded to whole seconds).
- **Exit (server, reason)**: when the server was current and marked gone, ensure with reason `exit: {reason}` (so a
  stable exit restarts at once, an early one waits for its timer).
- **Release (server, reason)**: only the current server; it is forgotten with no failure, no backoff and no timer,
  info `[HubSupervisor] {label} released ({reason}) - started again on the next request`; reports whether it acted.
- **Tick** (the watchdog): a current server that is dead is an exit (`watchdog: process gone`); a live one with
  failures counted and up at least the stable uptime resets the count, info `[HubSupervisor] {label} stable for
  {s}s - failure count reset`. No current server and none in flight: ensure `watchdog`.
- **The retry timer**: armed only when there is no timer, no start in flight and no current server; it fires at the
  due time (at once if past), clears itself and ensures `retry`. Dispose cancels it.
- A log call that panics never breaks supervision.

### 7.2 The hubs' start function

Both hubs' cold starts go through one gate, never at once (a concurrent Ramsgate and Training start crashed
Ramsgate's async loading thread). Holding it: for any reason other than `boot`, wait for the hunt launch queue's
tail as it stands now, at most 60 s (capped: warn `[HubSupervisor] {label} hunt launch queue still busy after {ms}
ms - cold-starting anyway`; else, after 100 ms or more, info `[HubSupervisor] {label} waited {ms} ms for the hunt
launch queue before cold start`). For the Training Grounds the occupancy log is re-based (§7.4) and the idle policy
reset. Then a hub start (§5.2) with the hub's map and no Behemoth, matchmaker id or roster.

### 7.3 Training Grounds on demand and the idle stop (patch 0033)

- **At boot** when `TRAINING_HUB` is on: an ordinary persistent hub. Off: not wanted until the first request.
- **A request** (§4.2) when not at boot: the last-request time is now; while an idle stop is in progress the request
  waits, polling every 500 ms, at most 30 s; the hub becomes wanted (the first time: info `[TrainingHub] on-demand
  Training hub requested for the first time (TRAINING_HUB=off) - starting it`); unless a ready, alive hub is current,
  the request awaits `ensure("request")`; then the connection details (§5.9) as for a boot hub.
- **The idle stop** is active only when not at boot, `TRAINING_IDLE_STOP_MINUTES` gives a positive value (1-1440
  whole minutes; unset, empty or `0` is off; anything else warns `[TrainingOnDemand] TRAINING_IDLE_STOP_MINUTES='{raw
  trimmed}' is not a whole number from 0 to 1440; the Training Grounds keep running` and is off) and
  `GAMESERVER_RUNTIME_LOG_DIR` is set (else warn `[TrainingOnDemand] TRAINING_IDLE_STOP_MINUTES is set but
  GAMESERVER_RUNTIME_LOG_DIR is not: the Training Grounds keep running` and off).
- **Each watchdog pass**, with it active: an idle stop in progress whose process is still alive 120 s after it was
  sent is abandoned (warn `[TrainingOnDemand] the Training Grounds (pid={pid or undefined}) did not stop within 2 min;
  leaving them running`, policy reset); with one in progress nothing else happens. Otherwise, unless the hub is
  wanted and its current server ready and alive with a ready time, the policy resets. Else the occupancy count is
  polled, and when the policy says stop: the stop is recorded with its time, info `[TrainingOnDemand] no player in
  the Training Grounds for {minutes} min - stopping them (pid={pid or undefined})`, and the tree is asked to stop
  (§5.7; an error is warned `[TrainingOnDemand] could not stop the Training Grounds` with it, and the stop is not
  recorded). The exit then releases the hub (§5.11).
- **The policy** (idle time `idle_ms`, 0 never stops): an unknown count or a count above 0 clears the idle start and
  never stops. A count of 0 sets the idle start to now if it has none; stop when `now - max(idle start, ready time,
  last request) >= idle_ms` (a player on the way in has no connection yet). Reset clears the idle start.

### 7.4 The occupancy log

The runtime writes each change of its remote-client count to
`{GAMESERVER_RUNTIME_LOG_DIR}/rustic_isles_dll_port{port}.log` (the Training Grounds' port), lines like:

```text
[12:00:01.234] pid=292 [NetConnEdge] tick=1234 connCount 0 -> 1 connMax=16 NetDriver=0x1
```

Under Wine the pid in the line is not the one the Director knows, so position, not pid, decides what belongs to the
current process.

- **Re-base** (just before the hub's process starts): reading position = the file's size now (0 when missing), no
  partial line, count unknown.
- **Poll**: size below the position (truncated or replaced): position and partial line back to 0. No new bytes: the
  last count. Otherwise read the new bytes, or only the last 4 MiB of them when there are more (the partial line is
  then dropped); bytes are Latin-1 (one char per byte); split at `\n` after the kept partial line; the last piece
  becomes the partial line unless it is over 65 536 chars (then dropped); every complete line with an edge sets the
  count. A read error keeps the last count. The count is unknown until the first edge.
- **An edge**: the first `[NetConnEdge] tick=` in the line, one or more ASCII digits, ` connCount `, a signed integer
  (optional `-`, digits), ` -> `, a signed integer; the count is the second integer when it is 0 or more, and unknown
  when it is negative. Anything after is ignored; a line that does not match changes nothing.

The pre-rename file name `mysticparadox_dll_port{port}.log` is not followed (core §17 item 2).

---

## 8. The memory guard (patch 0022)

With `GAMESERVER_MEMORY_GUARD` on, a new hunt starts only when the memory available now, minus what hunts started in
the last `GAMESERVER_MEMORY_RESERVATION_SECONDS` still have to allocate, covers one more hunt and the floor: refused
like a full cap otherwise.

- **Options**: enabled when the trimmed value is `1`, `true`, `yes` or `on` (any case). Three numbers, each trimmed,
  empty or unset its default, else all ASCII digits within its range, else the default and the problem
  `{NAME}='{trimmed}' is not a whole number from {min} to {max}; using {default}` (warned `[MemoryGuard] {problem}`
  when the service is built, even with the guard off):

  | Setting | Default | Range |
  | --- | --- | --- |
  | `GAMESERVER_MIN_FREE_MB` | 1024 | 0-1 048 576 |
  | `GAMESERVER_HUNT_MEMORY_MB` | 1536 | 1-1 048 576 |
  | `GAMESERVER_MEMORY_RESERVATION_SECONDS` | 120 | 0-3600 |

- **Available**: on Linux `MemAvailable` from `/proc/meminfo` (`MemAvailable: <n> kB`, × 1024); otherwise, or when
  that cannot be read, the free physical memory (Windows' available physical bytes when above 0; Linux's available
  pages × page size when both are positive); otherwise unknown.
- **Status**: expired reservations (hold time reached) dropped; reserved MB = the sum of the held bytes / 1 MiB,
  rounded; available MB = bytes / 1 MiB, floored, read only when enabled; needed MB = hunt + floor.
- **Reserve**: disabled, or available unknown: admitted with no reservation (the guard never blocks a server it
  cannot measure). Known and `available - reserved < needed`: refused, the reason `Not enough free memory for
  another hunt server ({available or ?} MB available, {reserved} MB reserved for hunts still starting, {needed} MB
  needed)`. Else a reservation of the hunt size for the hold time.
- **Give back** (a start that failed): removes that reservation at once.

| Guard on, defaults | Result |
| --- | --- |
| 2560 MB available | admitted |
| 2559 MB available | refused |
| 5000 MB available, two hunts reserved | reserved 3072; a third refused: `... (5000 MB available, 3072 MB reserved for hunts still starting, 2560 MB needed)` |
| one given back | the next admitted; after 120 s reserved 0 |

---

## 9. Hunt tables and the resolver

Nothing exported from the game is committed (AGENTS.md). The tables are the game's DataTable exports, written by the
importer (§10) from a CatalogExporter capture of the host's own install, or vendored from FModel exports; restricted
keeps `*.example.json` copies for smoke tests.

### 9.1 Files and format

In the module's Director folder (core §8.1 `game_data.director_dir`), the version's list (core
`director_files(trials)`; 1.14.7 has the Trials tables, so all seven): `player_hunts_table.json`, then the matchmaker
tables in this order, which is also their name order for lookups: `matchmaker_hunts_table` (the main table),
`arena_easy_matchmaker_hunts`, `arena_hard_matchmaker_hunts`, `arena_elite_matchmaker_hunts`,
`arena_hard_matchmaker_hunts_new`, `arena_elite_matchmaker_hunts_new` (the `_new` Trials tables are reached only
through tag routing).

A file is a JSON array whose first element is an object with `Rows`, an object of row name to row (also `Name`,
`Type: "DataTable"`). Fields the Director reads:

| Table | Field | Shape |
| --- | --- | --- |
| player | `MatchmakerHuntIDs` | array of handles `{"DataTable":<ref or null>,"RowName":"<matchmaker row>"}`; `RowName: "None"` is the tag-routed sentinel |
| player | `MatchmakerHuntsByTag` | array of lists `{"MatchmakerTable":{"ObjectName":"DataTable'<table>'","ObjectPath":".."},"HuntTags":[<query>...]}`; the raw capture shape `{"matchmakerTable":"<table>","queries":[...]}` is read too |
| query | `QueryTokenStream` / `queryTokenStream` / `tokenStream`; `TagDictionary` / `tagDictionary` | a number array; an array of `{"TagName":".."}` or of strings |
| matchmaker | `HuntTags` | array of tag strings |
| matchmaker | `MapList` | array of `{"MapName","MapAssetName":"/Game/<package>.<object>","Biome","Weighting"}` |
| matchmaker | `SpecificBehemoth.BehemothAsset.AssetPathName` | the Behemoth class path, or `None` |
| matchmaker | `GameModeOverride`, `MaxPlayers` | audits only |

### 9.2 Loading

At service build, once (a changed file needs a restart). Each file is read as UTF-8 text (unreadable or missing:
`[hunt-manifest] Required hunt table is missing: {path}`), one leading BOM dropped, parsed (`{path}: {parser
error}`), and must have an object at `[0].Rows` (else `[hunt-manifest] {path} is not a DataTable export (no Rows)`).
Any failure, or no Director folder, leaves every table empty, so no hunt can start while the hubs still run (Node's
Director exited instead): error `Hunt tables not loaded, no hunt can start: {error}` or `No Director game data folder
is configured ([game_data] director_dir), so no hunt can start`.

Lookups read only a table's own rows, and a row whose value is `null` is absent, so a hunt id such as `constructor`
or `__proto__` finds nothing. A matchmaker row is looked up in every table in the order above; the first (main)
wins a name collision.

### 9.3 The resolver

Injected for tests: a clock (ms) and an integer source `[min, max)` (default: the system generator, unbiased). A pick
among n candidates is the source's value for `[0, n)`, clamped to n-1. Its log lines can be silenced.

**The matchmaker hunt id of a player hunt**:

1. No own row: the missing-hunt callback (§9.6) and the error `player_hunts_table has no usable row for HuntId
   '{id}' (missing row or empty MatchmakerHuntIDs)`.
2. Direct candidates: the `RowName` strings of `MatchmakerHuntIDs` that are not empty and name a matchmaker row.
3. None: tag-routed candidates (§9.4); when there are some, info `[HuntTagRouting] {id} resolved to {n} matchmaker
   row(s) via tag query`.
4. Still none: the error `player_hunts_table row '{id}' has no resolvable 1.12 MatchmakerHuntIDs` (sic; §16 item 4).
5. Tag-routed with arena week rows among them (`Arena_MatchmakerHunt_{Easy|Hard|Elite}_{www}`): the featured week
   (§9.5); those ending `_{week}` are picked from (info `[TrialsRotation] interval={minutes}m featured=_{week} ->
   {chosen}`); none: warn `[TrialsRotation] no arena row matched featured week _{week}; falling back to a numbered
   arena candidate` and a pick among the arena rows. Other candidates: a pick among all.

**Behemoth of a matchmaker row**: no row: `matchmaker_hunts_table has no row for MatchmakerHuntId '{id}'`. The value
at `SpecificBehemoth.BehemothAsset.AssetPathName` when it is not `null`, not the string `None` and its JS String
trimmed is not empty (that value, whatever its JSON type); else none (Hunting Grounds generate their encounters).

**Map of a matchmaker row**: `MapList` not a non-empty array: `matchmaker_hunts_table row '{id}' has no MapList`.
Usable entries: `MapAssetName` a string starting `/Game/` and containing `.`; none: `matchmaker_hunts_table row
'{id}' has no valid /Game/ map asset path`. A pick among them, cut before its last `.`.

**The manifest** (hunt id as sent, roster):

1. `ExpectedPlayers` must be an array of 1-4 items, each a non-empty string; else `ExpectedPlayers must contain
   between one and four account IDs` (checked first, so a bad party never reaches the missing-hunt registry).
2. The key is `String(hunt id)`; the matchmaker hunt id, Behemoth (or `"NO_BEHEMOTH"`), map.
3. The object, in this order: `huntId` (the value as sent; left out when absent), `matchmakerHuntId`, `mapPath`,
   `behemothPath`, `expectedPlayerString` (`{id}:{key}` joined with `,`).

`{"huntId":"Plain_Hunt","matchmakerHuntId":"MM_Plain","mapPath":"/Game/Maps/test/plain","behemothPath":"NO_BEHEMOTH","expectedPlayerString":"acc-1:Plain_Hunt"}`

### 9.4 Tag routing and the tag query

**Tag-routed candidates** of a player row: for each list in its `MatchmakerHuntsByTag` array: the table name is
`MatchmakerTable.ObjectName` (when not `null`) or `matchmakerTable` (when not `null`), a string else empty; when it
contains `'`, the text between the first and second `'`. A name that is not one of the six: warn `[HuntTagRouting]
{player hunt id} references matchmaker table '{name}' which is not registered; add it to HUNT_TABLE_FILES in
shared/hunt-manifest` and the list is skipped. The queries are `HuntTags` or `queries` (whichever is not `null`
first) when a non-empty array, else the list is skipped. Every row of that table with a non-empty `HuntTags` array
that every query matches (queries are ANDed) is a candidate, in table order; duplicates across lists stay.

**A tag query** against a row's tags (its string entries only), as Unreal's `FGameplayTagQuery` evaluator:

- The stream is the first of `QueryTokenStream`, `queryTokenStream`, `tokenStream` that is not `null`, when an
  array; else empty. The dictionary likewise from `TagDictionary`, `tagDictionary`: each string entry as it is, any
  other entry its `TagName` (when not `null`) else `""`.
- Fewer than 3 tokens: no match. Token 0 (the version) is ignored; token 1 (has-root) must be truthy as JavaScript
  reads it, else no match. Then one expression.
- An expression starts with its type token (a number; anything else is NaN). Types 1-3 (Any/All/NoTagsMatch): a
  count, then that many dictionary indexes; an index that is a non-negative whole number naming a non-empty string
  entry contributes that tag, anything else (including a token past the end) contributes nothing. Type 1: some tag
  matches; 2: every contributed tag matches (none contributed is true); 3 and any other type within [1, 3] such as
  2.5: no tag matches. Types 4-6 (Any/All/NoExprMatch): a count, then that many nested expressions, all evaluated; 4
  any true, 5 all true, any other type within [4, 6]: none true.
- A count is the token's number rounded up when positive, else 0 (absent is 0).
- An expression whose type token is past the end, or of a type outside [1, 3] and [4, 6], is an overflow: the whole
  query does not match. A count larger than what is left of the stream must not iterate past the stream (the result
  is the same as reading past the end; §15).
- A tag matches a row tag when equal, or when the row tag starts with it followed by `.` (whole segments:
  `Hunt.Arena.Easy.Week` satisfies `Hunt.Arena.Easy`; `Hunt.Arena.EasyX` does not).

| Stream (dictionary) | Row tags | Match |
| --- | --- | --- |
| `[0,1,1,1,0]` (`Hunt.Arena.Easy`) | `Hunt.Arena.Easy`; `Hunt.Arena.Easy.Week`; `Hunt.Arena.Hard`; `Hunt.Arena.EasyX` | yes; yes; no; no |
| `[0,1,2,2,0,1]` (`A`,`B`) | `A`; `A`,`B` | no; yes |
| `[0,1,3,1,0]` (`Hunt.Arena.Easy`) | `Hunt.Arena.Hard`; `Hunt.Arena.Easy` | yes; no |
| `[0,1,5,2,1,1,0,3,1,1]` (`A`,`B`): ALL(ANY(A), NONE(B)) | `A`; `A`,`B` | yes; no |
| `[0,0,1,1,0]`; `[0,1,9,1,0]`; `[0,1,1,1]`; `{}` | `A` | no |

### 9.5 The Trials rotation

`TRIALS_ROTATION_MINUTES`: unset is 10 080 (a week); otherwise its JS Number, which must be a whole number in
[1, 525 600], else warn `[TrialsRotation] invalid TRIALS_ROTATION_MINUTES='{raw}'; using 10080` and 10 080 (an empty
value is 0, so it warns). The featured week at time t (ms, the resolver's clock) is `(floor(t / (minutes × 60 000))
mod 181) + 1`, three digits zero-padded (the runtime uses the same formula): t = 0 is `001`, one week `002`, 181
weeks `001`; with 30 minutes, t = 30 min is `002`.

### 9.6 The missing-hunt registry

Each absent player row a resolution asks for is counted by id with its first and last time. The first sighting only
is logged at error: `[HuntTableGap] Client requested HuntId '{id}' which is absent from player_hunts_table. This hunt
CANNOT start. The vendored tables predate this content — re-export with CatalogExporter EXPORT_HUNTS=1 and import the
new rows.` Each watchdog pass with entries warns `[HuntTableGap] {n} hunt id(s) requested but missing from
player_hunts_table: {id} x{count}, ...`, most requested first (ties in first-seen order). The registry's JSON form, in
that order, is `[{"huntId","count","firstSeen":"<ISO>","lastSeen":"<ISO>"}...]` (not served by any route).

### 9.7 The start-up audits (change nothing)

**Table audit**, per player row:

- `MatchmakerHuntIDs` not a non-empty array: its tag-routed candidates (with their warnings); none: skipped; else it
  is a tag-routed row, and when k of its n candidates have no usable map (§9.3) the problem `{id} -> {k}/{n}
  tag-routed rows have no valid map`.
- Else a direct row; for each entry: `RowName` `None` counts a tag-routed reference; no such matchmaker row: `{id} ->
  missing MatchmakerHunt '{String(RowName)}'`; one with no usable map: `{id} -> {name} has no valid map`.

**Coverage audit**, per player row: candidates are the direct ones (excluding `None` and unknown rows), else the
tag-routed ones; none: unresolved. For each candidate row: referenced (a set), its game mode (`GameModeOverride` when
a non-empty string, else `(map/default)`), its `MaxPlayers` when a safe integer, its usable maps. Unresolved ids,
maps and modes sorted, max players sorted numerically.

**Family coverage**: ids whose last character is an ASCII capital letter are grouped by the rest (first-seen order
of letters), groups ordered by the text `{prefix},{letter},{letter}...` as JavaScript sorts strings, groups under 3
members skipped, letters sorted: `[HuntTableCoverage] {prefix}: {first}-{last} ({n})`, plus ` (NON-CONTIGUOUS)` when
`last - first + 1 > n`. (A family ending at R is the cue that newer content was never exported.)

**Lines** (info unless stated): `[HuntTableAudit] playerRows={} directRows={} tagRoutedRows={} tagRoutedRefs={}
matchmakerRows={main table's rows} (+{tables - 1} aux tables) problems={}`; `[HuntLaunchCoverage]
resolvable={total - unresolved}/{total} referencedMatchmakerRows={} uniqueMaps={} gameModes={} unresolved={}`;
warn `[HuntLaunchCoverage] unsupported player hunt rows: {ids joined by ", "}` when any; the family lines; the first
20 problems each warn `[HuntTableAudit] {problem}`, then warn `[HuntTableAudit] {n} additional problems omitted` when
more.

---

## 10. The hunt-table importer (`reforged-host gamedata`, RB11)

A host tool: it merges a CatalogExporter `EXPORT_HUNTS=1` capture into the Director's tables. Its terminal output
is English (AGENTS.md).

### 10.1 Options and inputs

Options: export folder, vendor (tables) folder, optional repository root (only to show the export folder relative to
it), apply (else a dry run), update existing rows, verbose, optional backup stamp. The host kit calls it with
`<exports>/hunts_1_14_7`, the module's Director folder, apply on, update off, verbose off.

The capture folder (game-derived, never committed):

| File | Content |
| --- | --- |
| `hunt_export_manifest.json` (optional) | `{"gameVersion":"1.14.7","changelist":647472}` |
| `player_hunts.jsonl` | one JSON object per line: `rowName`, `sourceTable`, `region`, `matchmakerHuntIDs`, `matchmakerHuntsByTag`, `matchmakingType`, `matchmakingGameType`, `huntTags`, `escalationModeSpecification`, `escalationPatrolInitialChallengeLevel`, `escalationPatrolMaxChallengeLevel`, `recommendedEscalationLevel`, `hasPortals`, `hasGlitterEvent`, `hasPhaelanxEvent`, the five reward fields (`huntSuccessReward`, `firstCompletionSuccessReward`, `targetedHuntSuccessReward`, `huntFailureReward`, `targetedHuntFailureReward`), `minWeaponSkillLevel`, `recommendedWeaponSkillLevel`, `visibleWhileLocked` |
| `matchmaker_hunts.jsonl` | `rowName`, `sourceTable`, `region`, `huntTags`, `huntThreatLevel`, `gameModeOverride`, `dangerPerSecOverride`, `specificBehemoth` (`behemothName`, `behemothAssetPath`, `powerOverride`, `weighting`), `additionalSpecificBehemoths`, `mapMetaData`, `mapList` (`mapName`, `mapAssetPath` or `mapAssetName`, `biome`, `weighting`), `modifiers`, `gatherableTier`, `maxPlayers`, `isGeneratedEncounter`, `gameModeSpecificData` |

An exported handle is `{"table":"<table>","rowName":"<row>"}`; a tag list `{"matchmakerTable":"<table>","queries":
[{"tokenStreamVersion","tagDictionary":[string...],"tokenStream":[...],"userDescription","autoDescription"}]}`.
JSONL: the text with a BOM dropped and trimmed; empty is no rows; each `\n`-separated line trimmed, blank lines
skipped, each parsed (`malformed JSON on line {n} of {path}: {error}`, n counting lines of the trimmed text from 1).

**Refusals, in order**: no export folder: `No export found at {dir}` + newline + `Run CatalogExporter with
EXPORT_HUNTS=1 and inject into the CLIENT first.`; a JSON file unreadable or invalid: `failed to read {path}:
{error}` / `invalid JSON in {path}: {error}`; `gameVersion` not `1.12.0` or `1.14.7`: `Export {dir} declares
gameVersion '{version or (missing)}'.` + newline + `Enum numbers can only be decoded for: 1.12.0, 1.14.7. Add the
version's EMatchmakingGameType table first.`; either JSONL missing: `Export is missing player_hunts.jsonl or
matchmaker_hunts.jsonl.`.

The provenance marker is `CatalogExporter EXPORT_HUNTS {version}`.

### 10.2 Enums

| Field | Prefix | Values (index = number) | Written |
| --- | --- | --- | --- |
| `MatchmakingGameType` | `EMatchmakingGameType` | 1.12.0: `None, LegacyHunt, City, HuntingGround, Escalation, Trial, TrainingGround, Mission, Event, FTUE`; 1.14.7: the same with `Gauntlet` before `FTUE` | always by name, `EMatchmakingGameType::{Name}` |
| `MatchmakingType` | `EMatchmakingType` | `Reserved, Open, City` | as given |
| `Biome` (map entries) | `EHuntBiome` | `Moss, Ice, Arid` | as given |
| `GatherableTier` | `EGatherableTier` | `Tier0` to `Tier4` | as given |

Decoding a value: a string is the part after its last `::`, which must be one of the values (for the game type, of
either version's list); a number must be a non-negative whole number indexing the list (the game type's for the
given version). A value that does not decode, or is absent, is the problem `{row}: {field}={JSON text or undefined}
is not a {Prefix} value in {version}` and is kept as it was (an absent one stays absent).

### 10.3 Converting exported rows

An exported handle that is an object whose `rowName` is a non-empty string other than `None` becomes
`{"DataTable":{"ObjectName":"DataTable'{table}'","ObjectPath":"{path}"},"RowName":"{row}"}`, `{table}` being its
`table` string or `undefined`; anything else is `{"DataTable":null,"RowName":"None"}`. `{path}` is the table's known
object path, else `Archon/Content/Gameplay/hunts/{table}.0`. Known paths come from the existing player and main
matchmaker files: in every row without a (non-null) `_importedFrom`, every nested object with `ObjectName`
`DataTable'{t}'` and a string `ObjectPath` (first seen wins).

A player row becomes, in this key order ("opt" = only when the exported field is present, copied as it is, `null`
included):

`HuntName` `{"CultureInvariantString":""}`; `HuntDescription` the same; `Region` handle(`region`);
`MatchmakerHuntIDs` handles; `MatchmakerHuntsByTag` lists (below); `MatchmakingType` enum; `MatchmakingGameType`
enum; `HuntTags` (`huntTags`, `[]` when absent or null); `EscalationModeSpecification` handle; opt
`EscalationPatrolInitialChallengeLevel`, `EscalationPatrolMaxChallengeLevel`, `RecommendedEscalationLevel`;
`bHasPortals`, `bHasGlitterEvent`, `bHasPhaelanxEvent` (opt, from `hasPortals`, ...); opt the five rewards
(`HuntSuccessReward`, ...); opt `MinWeaponSkillLevel`; `RecomendedWeaponSkillLevel` (sic, opt, from
`recommendedWeaponSkillLevel`); `IsVisibleWhileLocked` (opt, from `visibleWhileLocked`); `_importedFrom` the marker.

A tag list becomes `{"MatchmakerTable": <the table's reference {"ObjectName","ObjectPath"} when matchmakerTable is a
non-empty string, else null>, "HuntTags": [{"TokenStreamVersion": <or 0>, "TagDictionary": [{"TagName": t}...] (or
[]), "QueryTokenStream": <or []>, "UserDescription": <or "">, "AutoDescription": <or "">}...]}` ("or" = when absent
or null).

A matchmaker row is first checked: `mapList` must be a non-empty array (else rejected `{row}: no mapList`) with an
entry whose `mapAssetPath` (when a non-empty string, else `mapAssetName` when a string) starts `/Game/` (else rejected
`{row}: no /Game/ map path (got "{the first entry's mapAssetPath or mapAssetName, or (empty)}")`). A rejected row is
not merged. Otherwise, in this key order: `Region` handle; `HuntTags` (or `[]`); opt `HuntThreatLevel`;
`GameModeOverride` (or `""`); opt `DangerPerSecOverride`; `SpecificBehemoth` behemoth; `AdditionalBehemoths` `[]`;
`AdditionalSpecificBehemoths` behemoths; `MapMetaData` handle; `MapList` entries; `Modifiers` (or `[]`); opt-enum
`GatherableTier`; opt `MaxPlayers`; opt `bIsGeneratedEncounter` (from `isGeneratedEncounter`);
`GameModeSpecificData` handle; `_importedFrom`.

A behemoth: `{"BehemothName": <or "">, "BehemothAsset": {"AssetPathName": <behemothAssetPath when a non-empty
string, else "None">, "SubPathString": ""}}` then opt `PowerOverride`, `Weighting`. A map entry: `{"MapName": <or
"">, "MapAssetName": <mapAssetPath when non-empty, else mapAssetName when a string, else "">}` then enum `Biome`,
opt `Weighting`.

### 10.4 Merging

Tables: `player_hunts_table` from the player rows; the matchmaker rows grouped by `sourceTable` (a non-empty string,
else `matchmaker_hunts_table`), the main table first (even with no rows), then the others by name in byte order. A
table's file is `{vendor}/{table}.json`; missing, it starts as `[{"Name":"{table}","Type":"DataTable","Rows":{}}]`;
present, it must have an object at `[0].Rows` (else `{path} has no [0].Rows object`).

For each exported row (non-objects skipped; the name is `rowName` or `""`): converted; a name the file lacks is
**added**, placed after the nearest earlier row of the export's order that the file has, else before the nearest
later one, else at the end. A name whose file value is not an object is skipped. Otherwise the row is merged with the
existing one: unchanged rows are counted, changed ones **updated**, and written only with "update existing".

**Merging a row** (existing E, incoming I; the existing version is that of E's `_importedFrom` or `_updatedFrom`:
the digits and dots after the first `EXPORT_HUNTS `, else `1.12.0`). Fields `HuntName`, `HuntDescription`,
`AdditionalBehemoths` (not exported) and the provenance fields of I are skipped. For each other field of I:

- absent from E: added (placed by the reference key order of §10.3: after the nearest earlier field of that order
  that E has, else before the nearest later one, else before E's provenance field, else at the end); change `+{field}`;
- the same (below): nothing, except a by-name enum (the game type) whose JSON differs (a number for a name) is
  replaced, change `{field}(number->name)`;
- else replaced by the merged value; change `{field}`.

With any change, `_importedFrom` is set to the marker when E has one, else `_updatedFrom` (appended when new).

**The same** (a, b, field): an enum field: both decode (a with the existing version, b with the export's) to the
same name. Else equal JSON; strings of the FName fields (`RowName`, `HuntTags`, `TagName`, `Modifiers`,
`AssetPathName`, the five rewards) equal ignoring case; numbers within 1e-6 × max(1, |a|, |b|); arrays of equal
length and the same item by item (with the same field); objects: when either is a handle (has `RowName` and
`DataTable`), both are and their keys match (`{table}/{row}` lower-cased, the table from `DataTable.ObjectName`
`DataTable'{t}'` else `?`; an empty or `None` row is the key `None`); when either is an object reference (`ObjectName`
and `ObjectPath`), equal `ObjectName`; else every field of b except the unexported ones is in a and the same (with
that field). Otherwise not.

**The merged value** (a, b, field): an enum field: b when it is by name or a is not a string, else
`{Prefix}::{b decoded with the export's version, or undefined}`. Two handles whose keys are not `None` and whose
`DataTable.ObjectName` are equal and present: a with b's `RowName`; other handles: b. Two arrays: in b's order, each b
item takes the first not-yet-used a item that is the same (kept as a's), else the first not-yet-used a item of the
same identity merged with it, else itself; unused a items are dropped. Identity: an object with a string
`MapAssetName` is `map:` and the package (before the last `.`, when that is not the first character) lower-cased;
with an object `BehemothAsset`, `behemoth:` and `String(AssetPathName)` lower-cased; with an object
`MatchmakerTable` with a string `ObjectName`, `table:` and that name. Two plain objects (neither a handle, b not an
object reference): a's fields in a's order, each b field except the unexported ones added when missing (placed by the
reference order: for `MapList` entries `MapName, MapAssetName, Biome, Weighting`; for behemoths `BehemothName,
BehemothAsset, PowerOverride, Weighting`; otherwise b's own order) or merged when not the same. Anything else: b.

**Vendor-only rows** (in the file, not in this export) are kept; those whose `MatchmakingGameType` is a number are
decoded with their own version and, with "update existing", rewritten by name (**normalised**); one that does not
decode is the problem `{row}: vendored MatchmakingGameType={JSON} does not decode for {version}`.

A table changes when rows were added, or (with "update existing") updated or normalised.

### 10.5 The report and writing

Output lines, joined with `\n` (`{list}` is ` -> a, b, ...(+n)` showing at most the stated count, empty when none):

```text
Export: {export dir, relative to the root when under it} ({version}, CL {String(changelist) or ?}): {p} player rows, {m} matchmaker rows
Mode  : APPLY | DRY RUN (pass --apply to write)[ +update-existing]
(per table)
\n=== {table}[  (NEW vendor file)] ===
  vendored rows   : {n}
  export rows     : {n}
  added           : {n}{list of 12}
  changed         : {n}[ (not written: pass --update-existing)]{list of 8}
    fields        : {change x count, ...}            (when any; by count, most first, ties first seen)
      {row}: {changes}                               (verbose only, per updated row)
  unchanged       : {n}
  vendor-only     : {n} (kept; absent from this export){list of 12}
    game type     : {n} vendor-only row(s) with a numeric MatchmakingGameType -> name[ (not written: pass --update-existing)]{list of 12}   (when any)
  REJECTED        : {n}                              (when any; then up to 10 "      {problem}", "      ...and {n} more")
  rows after      : {n}
\n=== MatchmakingGameType ({version} enum) ===
  {name x count, ...}                                (player rows, first-seen order; an undecodable value is #{String(value)})
\n=== referential check ===
  all direct MatchmakerHuntIDs resolve | {n} dangling direct reference(s): (then up to 15 "      {row} -> {target}", "      ...and {n} more")
  every tag-routed MatchmakerTable has a vendored file | one "  tag-routed table without a vendored file: {row} -> {table}" per pair
\n=== enum problems ({n}) ===                        (when any; then up to 15 "  {problem}")
```

A dangling reference is a direct `RowName` (not empty or `None`) found in none of the merged matchmaker tables; an
unknown tag table is a `DataTable'{t}'` named by a tag list that is neither merged nor a file in the vendor folder.

- **Dry run**: the line `\nDry run complete. Nothing written. Re-run with --apply[ --update-existing] to commit.`
  and the report; nothing is written.
- **Apply**: any rejected matchmaker row refuses: `REFUSING TO WRITE: {n} matchmaker row(s) have no /Game/ map
  path.` + newline + `GetMapPathFromMatchmakerHuntId requires it, so these rows would resolve and then fail at
  launch.` + newline + `Re-export with the current HuntExporter (it reads the soft-pointer path via
  FName::GetRawString).`; any enum problem refuses: `REFUSING TO WRITE: {n} enum value(s) do not decode for
  {version}.`. Else an empty line; the vendor folder is created; the stamp is the given one or now's ISO time with
  `:` and `.` replaced by `-`; for each table that changes: an existing file is first copied to
  `{file}.{stamp}.bak` (line `Backed up: {bak name}`), then the file is written as `JSON.stringify(table, null, 2)`
  (two-space indent, JavaScript key order, no trailing newline), line `Wrote {file} ({rows} rows: +{added}[,
  ~{updated}, game-type names {normalised}])` (the bracket with "update existing"). New tables add `\nNEW vendor
  table(s): {names}`, `  DeployServer and the Metagame only read them once they are listed in HUNT_TABLE_FILES`,
  `  (shared/hunt-manifest/index.js).`; the last lines are `\nDone. Rebuild and restart DeployServer, then check the
  boot log:`, `  [HuntTableAudit]    problems=0`, `  [HuntLaunchCoverage] resolvable=N/N`.

The function returns the version, per-table summaries (name, path, new file, added, updated with their changes,
normalised, rejected, vendor-only, changes), the problems and the output text.

---

## 11. Settings

Read from the module's settings (core §6) when the service builds its Director, once per module start. `setup`'s
column is what dainty-lass's host kit writes into `director.env` today.

| Setting | Reading | Default | `setup` writes |
| --- | --- | --- | --- |
| `PORT_RANGE_BEGIN`, `PORT_RANGE_END` | `get`, a u16, required: else the start fails `the Director has no {NAME}` (core §6.4 forces them to the assigned game range) | — | the game range |
| `GAMESERVER_BINARY_PATH` | `get` (non-empty) | none: no launch | the game exe; on Linux the wrapper |
| `METAGAME_API_KEY` | `raw` (an empty value counts as set; §16 item 10) | none: no launch | the gameserver key (a secret) |
| `MY_IP` | `raw` | none: no `host`, address `undefined:{port}` | the game address |
| `SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP` | JS Number (unset NaN) | 1 ms stagger | `3` |
| `GAMESERVER_READY_TIMEOUT_MS` | JS Number (unset `30000`) | 30 s | `30000`, Linux `90000` |
| `MAX_CONCURRENT_HUNT_SERVERS` | JS Number (unset `3`) | 3 | the host's choice |
| `PUBLIC_HUNT_MAX_PLAYERS` | JS Number (unset `4`) | 4 | — |
| `ENABLE_PUBLIC_HUNT_REUSE` | raw equals `1` | off | — |
| `HUB_PROMPT_RESTART` | off when trimmed lower-case is `0`, `false`, `no`, `off` | on | — |
| `RAMSGATE_HUB` | unset or blank: on; trimmed lower-case `0`, `false`, `no`, `off`: off; anything else on | on | — |
| `TRAINING_ON_DEMAND` | trimmed lower-case `1`, `true`, `yes`, `on` | off | `1` or `0` |
| `TRAINING_HUB` | as `RAMSGATE_HUB`, its default "not `TRAINING_ON_DEMAND`" | on | — |
| `TRAINING_IDLE_STOP_MINUTES` | §7.3 | off | the idle minutes, or `0` |
| `GAMESERVER_RUNTIME_LOG_DIR` | `raw`, trimmed | none | the game exe's folder |
| `GAMESERVER_LOG_DIR` | `raw`; relative to the process's working folder (an empty value is that folder) | `{working folder}/../debug/gameservers` | the host's game-server log folder |
| `GAMESERVER_WINDOWS_HIDE` | raw not exactly `0` | hidden | — |
| `GAMESERVER_LOG_CMDS` | trimmed; used when 1-512 bytes of ASCII letters, digits, `_`, `,`, space; set but not so: the default and warn `GAMESERVER_LOG_CMDS must be "Category Verbosity" pairs separated by commas; using the default ({default})` at build | the version's (§12.4) | `LogPhoenixCharacter Warning, LogOnline Warning, LogLoad Off` unless verbose logs |
| `TRIALS_ROTATION_MINUTES` | §9.5 | 10 080 | — |
| `GAMESERVER_MEMORY_GUARD`, `GAMESERVER_MIN_FREE_MB`, `GAMESERVER_HUNT_MEMORY_MB`, `GAMESERVER_MEMORY_RESERVATION_SECONDS` | §8 | off, 1024, 1536, 120 | the first three |
| `DEPLOYSERVER_URL` | `raw`, trimmed empty or unset counts as unset (§12.3) | — | `127.0.0.1:61001` (backend.env) |

Any setting name beginning `MYSTPAX_` (the prefix before 2026-10-08) fails the build: `Rename these environment keys
to the RUSTIC_ISLES_ prefix: {names joined by ", "}` (only the settings, not the process environment; a stale key
would silently switch a runtime feature off in the game servers).

Read by no code here but handed to the game servers through the environment (§5.4): `REFORGED_GAME_EXE`,
`REFORGED_WINE`, `REFORGED_PROTON`, `REFORGED_WINEPREFIX`, `REFORGED_PROTON_DATA`, `REFORGED_XVFB` (the Linux
wrapper), `REFORGED_SERVER_MAX_FPS`, `REFORGED_SERVER_IDLE_FPS`, `RUSTIC_ISLES_RUNTIME_PROFILE`,
`REFORGED_BACKEND_URL` (the runtime). Written by `setup` and read by nothing (Node leftovers): `NODE_ENV`, `PORT`,
`BIND_HOST` (the controller's ports win), `SERVER_RUNTIME_AUTO_UPDATE`, `SERVER_RUNTIME_UPDATE_REQUIRED` (the signed
runtime updater is not ported), `LOG_LEVEL`.

Constants: stop grace 10 s, watchdog 60 s, hub queue wait cap 60 s, idle-stop abandon 2 min, on-demand wait 30 s,
backoff and stable uptime §7.1. The stop grace and the hub backoff list can be replaced in tests.

---

## 12. The public seam

### 12.1 Shape

One crate, `rustic-isles-director` (lib, plus the test bin of §13), MIT, in `crates/rustic-isles-director`.
Dependencies: `rustic-isles`, `module-api`, `service-kit`, and workspace `axum`, `tokio` (process, fs, io-util, sync,
time), `futures-util`, `chrono`, `serde_json` (`preserve_order` enabled in this manifest), `tracing`, `rand` (launch
ids and picks), `windows` (Windows: job objects, threading, pipes, file system, system information), `libc` (Unix).
Dev: `tempfile`, `tokio` (macros, net, rt-multi-thread).

**Seam changes for the integration pass**: the member and `rustic-isles-director` in `[workspace.dependencies]`;
`libc` in `[workspace.dependencies]` (today `module-api` and `hostkit` pin it themselves); nothing in `crates/dainty-protocol`,
`store` or the `clap` tree. Restricted's 1.14.7 crate and `host` switch their imports (§1.3); dainty-lass hostkit's
generator label, if it should name the new path, is hostkit's row.

### 12.2 Interface (names are the contract restricted's composer and host are written against)

| Item | Contract |
| --- | --- |
| `pub struct GameserverArgs { standard: &'static [&'static str], log_cmds: &'static str }`, `pub struct DirectorConfig { target_changelist: u32, args: GameserverArgs }` (`Copy`, `const`-constructible) | the version's values (§12.4); `launch_args(&Settings) -> Vec<String>` (the switches, `-LogCmds="..."` last), `log_cmds_invalid(&Settings) -> bool`, `hide_window(&Settings) -> bool` |
| `pub struct DirectorService` | `new(DirectorConfig)`, `with_launcher(DirectorConfig, Arc<dyn Launcher>)`, `config()`, `director() -> Option<Arc<Director>>`, `build(&Shared) -> Result<Arc<Director>, ModuleError>`; implements `rustic_isles::PortService` with port `director` (§12.3) |
| `pub struct InProcessDirector(pub Arc<Director>)` | implements `rustic_isles::DedicatedServers` (§12.3) |
| `pub fn register(&Arc<Director>, &mut Handlers)`, `pub fn router(Arc<Shared>, &Arc<Director>) -> Result<axum::Router, MountError>` | §3 |
| `pub struct Director` | `new(Config, hunt::Tables, MemoryAdmission, Arc<dyn Launcher>) -> Arc<Director>`; `startup`, `shutdown`, `kill_all`, `run_watchdog`, `persistent_hubs_ready`, `set_persistent_hubs_ready`, `persistent_hub_status() -> Value`, `instances_for_status() -> Value`, `gameservers()`, `missing_hunt_requests()`, `validate_hunt_tables()`, `is_stopping`; `config`, `resolver`, `ramsgate`, `training` readable (tests) |
| `pub struct Config` | `from_settings(&Settings, &DirectorConfig, PortRange, process_env: impl IntoIterator<Item = (String, String)>) -> Result<Config, String>`, `can_launch()`; public fields for tests (`stop_grace`, `hub_backoff_ms: Option<Vec<u64>>`); `Debug` shows only the port range, the binary path and `MY_IP`, never the key or the environment |
| `pub trait Launcher: Send + Sync { fn start(&self, &Command) -> io::Result<Started> }`, `pub struct OsLauncher` | §5.7; `Command { program, args, env, windows_hide }` has **no** `Debug` (the key is an argument); `Started { pid, stdout, stderr, wait, tree }`; `pub trait TreeControl { terminate, kill }`; `Exit { code: Option<i32>, signal: Option<String> }` |
| `pub mod cmdline` | `quote(arg, &mut String)`, `command_line(program, &[String]) -> String` (§5.3) |
| `pub mod port_pool` | `PortPool` (`new`, `add`, `take`, `attach_pid`, `release`, `reclaim(is_owner_live, is_pid_alive) -> Vec<Lease>`, `owner_of`, `pid_of`, `free_count`, `held_count`), `Lease { port, owner, pid }` |
| `pub mod hub` | `HubSupervisor<T>` (`new(Options<T>)`, `ensure`, `on_exit`, `release`, `tick`, `dispose`, `current`, `pending`, `failure_count`, `next_attempt_due_at`, `retry_armed`), `Options<T>` (label, start, is_alive, now, set_timer, log, backoff_ms, stable_uptime_ms), `Level`, `switch_on(Option<&str>, bool)`, `monotonic_ms()`, `DEFAULT_BACKOFF_MS`, `DEFAULT_STABLE_UPTIME_MS` |
| `pub mod occupancy` | `RuntimeLogOccupancy` (`new(path)`, `begin`, `poll -> Option<i64>`), `IdleStopPolicy` (`new(idle_ms)`, `reset`, `should_stop(now, count, ready_at, last_request_at)`) |
| `pub mod memory` | `MemoryAdmission` (`new(Options)`, `with(Options, available, now)`, `enabled`, `status`, `reserve`, `give_back`), `Options`, `Status`, `HuntMemoryUnavailable` (`Display` = the reason), `options_from_settings(&Settings) -> (Options, Vec<String>)`, `read_available_bytes(path, linux)` |
| `pub mod hunt` | `Tables { player_rows, matchmaker_sources }`, `load_tables(dir)`, `PLAYER_TABLE`, `MATCHMAKER_TABLES`, `HuntResolver` (`new(ResolverOptions)`, `matchmaker_hunt_id_for`, `behemoth_path_for`, `map_path_for`, `resolve_manifest`, `tag_routed_matchmaker_hunt_ids`, `find_matchmaker_row`, `set_quiet`), `ResolverOptions` (tables, log, trials rotation, now, random_int, on_missing_hunt), `HuntLog`, `evaluate_tag_query`, `current_trials_week`, `parse_trials_rotation_minutes`, `expected_player_args`, `validate_tables`, `audit_coverage`, `family_coverage`, `TableAudit`, `CoverageAudit`, `TRIALS_ROTATION_ROW_COUNT`, `DEFAULT_TRIALS_ROTATION_MINUTES` |
| `pub mod hunt_tables` | `import_hunt_tables(&ImportHuntTablesOptions) -> Result<ImportHuntTablesReport, String>`, `ImportHuntTablesOptions { export_dir, vendor_dir, repo_root, apply, update_existing, verbose, backup_stamp }`, `ImportHuntTablesReport { export_version, tables, conversion_problems, output }`, `TableMergeSummary`, `DEFAULT_EXPORT` (`hunts_1_14_7`), `SUPPORTED_VERSIONS` |
| `RAMSGATE_MAP`, `TRAINING_DOJO_MAP`, `TRAINING_DOJO_HUNT_ID` | the wire values of §5.3 and §4.2 |

### 12.3 The seam to core (`PortService`, §5.6 `DedicatedServers`)

- **`app(shared)`**: builds the Director from `shared.settings`, the version's `DirectorConfig`, the game range of
  `PORT_RANGE_BEGIN`/`END`, the process environment, the hunt tables of `shared.game_data` (§9.2), and a memory
  guard over its options; logs the memory problems and the `GAMESERVER_LOG_CMDS` warning. When the Director can
  launch (`GAMESERVER_BINARY_PATH` and `METAGAME_API_KEY` set), or `DEPLOYSERVER_URL` is unset or blank, it puts an
  `InProcessDirector` in `shared`'s dedicated-servers slot (core `set_dedicated_servers`), so matchmaking calls it in
  memory instead of over HTTP; otherwise the slot keeps core's HTTP client of `DEPLOYSERVER_URL`. Returns the router
  of §3. A build error fails the module's start.
- **`started()`**: cannot launch: warn `GAMESERVER_BINARY_PATH and METAGAME_API_KEY are not set in the env file: the
  Director starts no game server` and nothing else (no hubs, no watchdog, `/health/ready` stays 503). Else the boot
  (§5.11) on its own task, then the hubs' boot is finished; the watchdog task; info `Rustic Isles DeployServer
  started` and `Clear Skies, Slayer.`.
- **`stopping()`**: aborts both tasks and awaits the shutdown (§5.11); a second call does nothing.
- **`InProcessDirector`**: `configured()` is "can launch". `launch(request)`: the request's wire body (core §5.6
  `LaunchRequest::body()`) parsed as JSON (`null` when it does not parse) and decided as §4; success is `Online {
  host: <host or "">, port }`, any failure `Refused { reason: None }` (the module answers `director refused hunt
  launch`). `hub_status()`: always `Some(LiveHubStatus { ramsgate, training_dojo })` from §3.3, each `LiveHub {
  ready, host (or ""), port }` or none.

### 12.4 The 1.14.7 values (wire facts; unit 12's version crate carries them)

`target_changelist` 647472 (for the not-yet-ported runtime updater); standard switches `-EpicPortal`, `-server`,
`-nullrhi`, `-warp`, `-nosound` (a dedicated hub must not initialise audio: it played the launch audio on the host),
`-RepDriverEnable`; default `-LogCmds` `LogPhoenixCharacter VeryVerbose, LogOnline Verbose, LogLoad Off`.

### 12.5 The platform layer

The only `unsafe` code, in one module per platform with a scoped `#[allow(unsafe_code)]` and a `SAFETY` comment per
call, as `module-api`'s `tree.rs` does: Windows process creation in a job (§5.7), the non-blocking pipe reads and exit
poll, the free-memory query; Unix `kill(2)` on the group, the Linux parent-death signal, the free-pages query. The
fake game server's parent watch may use the same.

---

## 13. The fake game server (`rustic-isles-fake-gameserver`, a bin for the tests)

No game and no SDK. Takes the real arguments (§5.3); missing the port or map: usage text on stderr, exit 2. First it
prints one line (`launchId` is `RUSTIC_ISLES_GAMESERVER_LAUNCH_ID` or `""`, `trialsWeek` `RUSTIC_ISLES_TRIALS_WEEK`
or `null`, `inherited` `REFORGED_SERVER_MAX_FPS` or `null`):

```text
FAKE_ARGS {"args":[...],"launchId":"...","trialsWeek":...,"inherited":...}
```

Then by map:

| Map | Behaviour |
| --- | --- |
| `/Game/Maps/crash` | `LogWindows: fatal` on stderr, exit 3, before binding |
| `/Game/Maps/silent` | binds, never reports ready |
| `/Game/Maps/grandchild` | binds, starts a copy of itself (env `FAKE_GS_HOLDER=1`) that binds `127.0.0.1:0` and prints `FAKE_GRANDCHILD_UDP=<port>`, waits 500 ms, then reports ready |
| `/Game/Maps/chatty` | after ready writes 3 MiB (3072 lines of 1023 characters and a newline), then `CHATTY_DONE` |
| `/Game/Maps/flaky` | ready, then exit 5 after 300 ms |
| anything else | ready and runs |

Binding is UDP `127.0.0.1:{port}`; failure prints `Networking::Listen failed on {port}: {error}`, exit 4. Reporting
ready prints `LogNet: listening`, a marker for the launch id `00000000-0000-4000-8000-000000000000` (another
launch's, to be ignored), then this launch's marker, flushed. It runs until `{FAKE_GS_CONTROL}/stop-{port}` exists
(removed, exit 0, its grandchild dropped), a signal, or its tree ends; it never outlives its test: exit 9 when the
process `FAKE_GS_PARENT_PID` is gone or after `FAKE_GS_MAX_SECS` (default 900).

---

## 14. Test cases

### 14.1 What may be copied

Nothing. Restricted's `tests/*` and unit tests are authored code, described below only. The byte-for-byte comparison
of the importer with `import_hunt_tables.cjs` needs the Node script and stays in restricted, run against this crate
once restricted's host calls it; a difference there is a bug on this side. The "real tables" tests
(`*.gamedata.test.ts` ports: 168 player rows, 161 resolvable, max players `[4, 12]`) need game data and run only on a
machine that has it; write them to skip when the folder is absent.

### 14.2 Behaviour the authored tests fix

End-to-end tests run the real `OsLauncher` with the fake game server on a free band of UDP ports (bind-probed,
spread by the process id, below the dynamic range), with `MY_IP=100.101.102.103`, key `gameserver-key`, stagger 0,
ready timeout 15000, cap 2, `REFORGED_SERVER_MAX_FPS=30` and the fake's control variables, over tables with
`Plain_Hunt` → `MM_Plain` (`/Game/Maps/test/plain.plain`, no Behemoth) and `Boss_Hunt` → `MM_Boss`
(`/Game/Maps/test/boss.boss`, `/Game/Monsters/test/boss_bp.boss_bp_C`), the other five tables empty. Every test is
bounded (180 s; 60 s a step) and kills every tree however it ends. On Windows the process environment handed over is
at least `SystemRoot` and `PATH`.

| Area | Cases |
| --- | --- |
| Before boot | `/health/ready` 503 `{"ok":false,"service":"deployserver"}`, `application/json; charset=utf-8`; hub-status `{"ramsgate":null,"trainingDojo":null}`; instances `[]`; CITY 500 `{"error":"matchmaking_failed"}` |
| Boot | `GET /HEALTH/Ready/` 200 ok; Ramsgate on the last port, host `100.101.102.103`, ready, a pid; Training on the one before; Ramsgate's arguments 1-7 `gameserver-key`, port, Ramsgate's map, `NO_BEHEMOTH`, `NO_MM_HUNTID`, `NO_EXPECTED_PLAYERS`, `100.101.102.103:{port}`, then §12.4's switches; `inherited` `30`; a 36-character launch id |
| Matchmaking | CITY 200 `{"host":"100.101.102.103","port":<last>}` with a 36-character request id; SHARED `ShatteredIsles_TrainingDojo` the one before; ISLAND `/Game/Maps/test/x?MonsterClass=/Game/Monsters/a/fulgar_bp.fulgar_bp_C?HuntID=Arena_MatchmakerHunt_Hard_042` with `HuntId` `Plain_Hunt` and `["acc-1"]`: the first port, arguments 4-6 the Behemoth, `Arena_MatchmakerHunt_Hard_042`, `acc-1:Plain_Hunt`, trials week `042`, log file `port{first}_fulgar_bp.log`; SHARED `Boss_Hunt` `["acc-2","acc-3"]`: the next port, map `/Game/Maps/test/boss`, `MM_Boss`, `acc-2:Boss_Hunt,acc-3:Boss_Hunt`; a third hunt 500 and no log file for its port |
| Instances | ids `city-{last}`, `dojo-{last-1}`, `hunt-{first}`, `hunt-{first+1}`; the Boss hunt's players 2, maxPlayers 4, playerIds, Behemoth; the city's maxPlayers 0 |
| Ports | a stopped hunt leaves the list and the next hunt takes `first+2` (FIFO); its log starts `=== Gameserver launch `, holds the marker and `=== Gameserver process exited port={first} label=fulgar_bp pid=` |
| Refusals | `RAID` 500; a form body `GameMode=CITY` with no content type 500; shutdown empties the list and frees every port |
| HTTP | `/nope?x=1` 404 with `<pre>Cannot GET /nope</pre>`, `text/html; charset=utf-8` and a request id; `x-request-id: client-id-0001` kept; `x-rustic-isles-request-id: short` replaced; `POST /api/instances` 404 `Cannot POST /api/instances`; `HEAD /api/instances` 200, empty; `{oops` as JSON 400; a 101 KiB JSON body 413 |
| Failed starts | hubs off, timeout 1500: `/Game/Maps/crash` 500 and its log has `code=3`; `/Game/Maps/silent` 500; both ports come back; then `/Game/Maps/test/ok` 200; CITY 500 (`RAMSGATE_HUB=off`); Training on demand starts on its first request and answers the one-before-last port |
| Missing executable | two ISLAND requests 500, the list and instances empty, shutdown ends the retrying hubs |
| Dead hub | backoff `[100]`: Ramsgate stopped → current none, hub-status `null`, CITY 500, then a new current hub and CITY answers again |
| After ready | a `flaky` hunt answers 200, then leaves the list with `code=5`; only the steady one is listed; the next hunt starts |
| Pipes | a `chatty` hunt's log grows past 3 MiB and holds `CHATTY_DONE` |
| Trees | a `grandchild` hunt: both UDP ports held; shutdown frees both; a stop of the server alone frees the grandchild's port too |
| Reuse | `ENABLE_PUBLIC_HUNT_REUSE=1`: two SHARED `Plain_Hunt` requests get the same port; playerIds `["acc-1","acc-2"]` |
| Module | the 1.14.7 module over a recording host, every port 0 but the game range, the env file and `director_dir` set: both hubs on the range's last two ports, `/health/ready` 200, a hunt on the first port, three instances; unload frees every port |
| Seam | with `DEPLOYSERVER_URL=127.0.0.1:1` and the Director able to launch, after `app` the slot is configured and `hub_status` is `Some` with both hubs none, with no HTTP call |
| Settings | default `-LogCmds` §12.4; ` LogOnline Warning ` → `-LogCmds="LogOnline Warning"`; `x" -evil` → the default and invalid; hidden by default, `GAMESERVER_WINDOWS_HIDE=0` shown |
| Command line | §5.3's table |
| Travel args | `/Game/Maps/x/y?MonsterClass=/Game/Monsters/a/b_bp.b_bp_C?HuntID=MM_1?Flag`: `MonsterClass`, `HuntID` `MM_1`, `Flag` `""`, `Missing` none; `a?K=v=w` `v=w`; labels `fulgar_bp`, `hunt`; arena week of `Arena_MatchmakerHunt_Hard_042` `042`, of `..._Test_042` none |
| Port pool | ports 8780...: A takes 8780, releases; B takes it; A's release refused; C gets none; owner B. Release twice: second false. Add of a free or held port ignored. Release of a never-taken port changes nothing. FIFO. A timed-out start (A released, B, C, D taken) never gives a port two owners. Attach by a non-owner refused; reclaim keeps a live pid, keeps a listed owner, releases an unlisted dead one, and a late release after it is refused; a lease without a pid is reclaimable once unlisted |
| Memory | §8's table; options default off; `1`/512/2048 on with no problem, needed 2560; `lots` → the problem text with 1024; meminfo `MemAvailable: 8192000 kB` → 8 192 000 × 1024 bytes; a missing file falls back (none or positive) |
| Occupancy | re-based after two old edges: unknown; a non-edge line and `-2 -> 0`: 0; `0 -> 1`, `1 -> 2`: 2; `2 -> 1` and a split line: 1, then 0 when completed; no new bytes: stays; a missing file unknown; a 200-byte line and `0 -> 2`: 2; replaced by a shorter file with `-2 -> 0`: 0 |
| Idle policy | 5 minutes: §7.3 cases (0 at 0 no; 4 min no; 5 min yes; after reset and a request at 12 min, 16 no, 17 yes; a player in no; unknown no; empty again at 31 no, 36 yes); 0 never |
| Supervisor | a fake clock and timers: a failed boot arms 5000 and the retry starts it; eight failures' delays 5000, 15000, 30000, 60000, 120000, 300000, 300000, 300000; a failed restart after a stable crash is retried by its timer; concurrent ensures share one start; a stable exit restarts at once with no timer and count 0; a crash loop waits and a request reuses the armed timer; a non-current exit is ignored; the watchdog and the exit in one pass restart once; tick retries when due and never doubles; an exit during a start adds no failure; a hub dead when its start resolves is a failure; two supervisors sharing a gate never start at once; the start receives `boot` then `exit: exit code=0`; the default clock is monotonic; a released hub is not restarted, has no timer, count 0, starts at once on the next ensure, and an old hub's release is refused |
| Switches | `switch_on`: unset and empty take the default; `  ` with default off is off; `0`, `false`, `no`, `off`, ` OFF `, `False` off; `1`, `on`, `yes`, `true`, `anything` on |
| Tag queries | §9.4's table |
| Trials | §9.5's cases; `TRIALS_ROTATION_MINUTES` unset 10080, `30` 30, `0`, `600000`, `soon` → 10080 with three warnings, the first exactly `[TrialsRotation] invalid TRIALS_ROTATION_MINUTES='0'; using 10080` |
| Resolver | picks fixed to the first: §9.3's manifest; Boss `acc-1:Boss_Hunt,acc-2:Boss_Hunt`; `No_Such_Hunt`, `constructor`, `__proto__` are each the missing-row error and reported to the registry in that order; `Dangling_Hunt` the "1.12" error; a row whose only map is `Engine/Maps/none` the map error; parties `[]`, five ids, `[""]`, `["a",7]`, `null`, `"acc-1"` the party error; a Trials row tag-routed to an arena table of `Easy_001`, `Easy_002` (tag `Hunt.Arena.Easy.Week`) and `Hard_001`: at one week `Easy_002` with map `/Game/Maps/arena/a2`; at two weeks `Easy_001`, the warning naming `_003` and the info `Trials_Hunt resolved to 2 matchmaker row(s) via tag query`; the resolvable count of 7 such rows is 5 and logs nothing |
| Loading | seven tables, the player table with a BOM: one player row, six matchmaker sources; a missing third file → `[hunt-manifest] Required hunt table is missing: ` ending `arena_hard_matchmaker_hunts.json`; `{"Rows":{}}` → `... is not a DataTable export (no Rows)`; the file list equals core's `director_files(trials)` for both values |
| Importer | an export with manifest 1.14.7/647472, two player rows (numeric `matchmakingType` 1 and `matchmakingGameType` 3 and 9) and two matchmaker rows, into the `*.example.json`-shaped tables with stamp `2026-10-28T00-00-00-000Z`: no enum problem, every table written; authored here as well: a missing export folder, an unsupported version, a missing JSONL, a rejected map, a dry run that writes nothing, a backup name, `9` → `EMatchmakingGameType::Gauntlet`, an added row placed by export order, `--update-existing` writing a changed row with `_importedFrom` or `_updatedFrom` |

---

## 15. Security behaviour (for `docs/security/rustic-isles-director.md` and RX3)

**The trust boundary.** The Director binds 127.0.0.1 only and no gateway route reaches it; it has no authentication,
so anything that can open a loopback connection on the host can ask it to start game servers (bounded by the hunt
cap, the memory guard and the port pool) and read the hub status with pids. Its trusted inputs are the host's
settings and tables; its untrusted input is the matchmaking body, which the backend builds partly from what a player
sent (the travel URL's map, `MonsterClass` and `HuntID`, the `HuntId`) and partly from server-side ids.

**What reaches a command line or a file name** (the checklist's core):

| Input | Source | Reaches | Guarantee |
| --- | --- | --- | --- |
| program | `GAMESERVER_BINARY_PATH` (host) | the program | not a shell; quoted whole on Windows |
| key | `METAGAME_API_KEY` (secret) | argument 1 | never logged, never in a `Debug`; visible in the process list (below) |
| map | `GameArgs` before `?` (player, via the backend) or the tables | argument 3, the log header | one argument, never split; NUL refused |
| Behemoth | `MonsterClass` (player) or the tables | argument 4, the log header, the log file's label | one argument; the label has no `/` and stops at the first `.` |
| matchmaker hunt id | `HuntID` (player) or the resolver | argument 5, `RUSTIC_ISLES_TRIALS_WEEK` only through the fixed arena pattern | one argument |
| roster | `ExpectedPlayers` (backend) and `HuntId` (player) | argument 6 | one argument |
| address | `MY_IP` (host) | argument 7 | one argument |
| switches | the version's constants; `GAMESERVER_LOG_CMDS` only when it is `[A-Za-z0-9_, ]{1,512}` | arguments 8.. | a host typo or a quote cannot add a switch |
| environment | settings, the process, the launch id (UUID), the week (3 digits) | the child's environment | the child gets exactly this block |

- **Process trees**: every game server's tree ends with it, with the Director's stop, and with the Director's
  process (job kill-on-close on Windows; the group and the parent-death signal on Linux); only the three standard
  handles are inherited on Windows.
- **Resource bounds**: the hunt cap, the memory guard, the port pool; pipes always drained; the readiness scan, the
  occupancy partial line and per-poll read are capped (§5.6, §7.4); the body limit is 100 KiB.
- **Secrets never logged or in `Debug`**: `Config`'s `Debug` shows the ports, binary path and `MY_IP` only; `Command`
  has no `Debug`; settings' `Debug` is names only (core §6.1); no log line carries the arguments or the environment.
- **Known weak spots to list** (kept for parity; §16 proposes fixes):
  - Player-influenced text (map, `MonsterClass`, `HuntID`, `HuntId`) reaches the game server's command line.
    Quoting keeps each argument whole, but Unreal's parser looks for `-Switch` tokens and `Key=` values across the
    whole line, so a map such as `-ExecCmds=...` or a Behemoth containing `Key=` may be read as an engine switch.
    Whether the backend validates these before the Director sees them is the matchmaking unit's.
  - The gameserver key is an argument, so any local user can read it from the process list.
  - Every setting of the merged env files (`backend.env` with `admin.env` and `director.env`: signing keys,
    `API_KEY_HASH_SECRET`, SMTP and Discord secrets, the database URL) and the controller's whole environment reach
    every game server's environment (core §6.1's `vars()`).
  - Player-supplied hunt ids and travel arguments are logged as they are (control characters included) and the
    missing-hunt registry grows without bound, one entry per distinct unknown id.
  - The importer writes `{vendor}/{sourceTable}.json` for whatever `sourceTable` the capture names (a host's own
    file, but a `..` in it leaves the folder).
  - On Windows the log file name takes the label as is, so a `\` or `:` in a Behemoth path reaches the file name
    (a sub-folder that does not exist fails the start; `:` names an NTFS stream).

**Fuzz targets (`fuzz/fuzz_targets/`):**

- `rustic_isles_director_cmdline`: any list of argument strings: `command_line` never panics, and a reference MSVC
  splitter applied to its output gives back exactly the list (NUL-free inputs); a NUL is refused.
- `rustic_isles_director_matchmaking`: any bytes as a JSON body to §4's classifier over a recording launcher (no
  process): never panics, and either an error or a start whose arguments are exactly §5.3's for the derived values,
  each one argument, the label free of `/`.
- `rustic_isles_director_tag_query`: any JSON query and row tags: `evaluate_tag_query` never panics, never overflows
  the stack, and runs in time linear in the stream whatever the counts say.
- `rustic_isles_director_lines`: any bytes as process output and occupancy log text: the readiness scanner, the
  edge parser and the occupancy follower never panic, keep their buffers under the caps, and accept only a
  well-formed marker for the exact launch id and port.
- `rustic_isles_director_hunt_import`: any JSONL rows through the converters and the merge (in memory, no files):
  never panic; every converted row has its fixed key order.

---

## 16. Open questions and behaviour that looks wrong but is kept

Parity unless the coordinator decides otherwise; each departure here is a candidate decision row.

1. **No legacy names (core §17 item 2 applied).** Restricted also accepts the `MYSTICPARADOX_GAMESERVER_READY`
   marker, follows `mysticparadox_dll_port{port}.log`, and copies every `RUSTIC_ISLES_*` variable to a
   `MYSTICPARADOX_*` twin in the child's environment (and the launch id and week too) for runtimes from before the
   rename. The current runtime prints the new marker and reads only the new names, so the MIT crate drops all three.
   The `MYSTPAX_` refusal (§11) names an old prefix but accepts nothing: proposed kept.
2. **Body errors carry no request id**, and a `GET` with a malformed JSON body is a 400 (Node's GETs are before its
   parsers and its request context). Kept (restricted's order).
3. **`HuntId.trim is not a function`** is also the text for a non-string `GameArgs`. Kept.
4. **The "1.12" in `has no resolvable 1.12 MatchmakerHuntIDs`**, the importer's `1.12.0` support, and its closing
   lines naming `DeployServer`, `HUNT_TABLE_FILES` and `shared/hunt-manifest/index.js` (Node's) are kept as texts.
5. **The importer reads a handle's `table`**, which is what the CatalogExporter writes; restricted's own fixture uses
   `dataTable` and so produces `DataTable'undefined'` references. Kept; the MIT tests should use `table`.
6. **`/health/ready` is 200 after the boot even when a hub failed**, and stays 503 for ever when the Director
   cannot launch. Kept.
7. **Numbers as JavaScript compares them**: `MAX_CONCURRENT_HUNT_SERVERS` unreadable is NaN and never refuses; empty
   is 0 and refuses every hunt; `PUBLIC_HUNT_MAX_PLAYERS` NaN never reuses. Kept.
8. **Reuse checks `ready`, not that the process is alive** (the hub answers do check). Kept; a dead hunt leaves the
   list on its exit.
9. **A short range**: hunt ports are the range minus its last two; a range of one port puts the Training Grounds
   outside it. Kept (setup always gives eight).
10. **An empty `METAGAME_API_KEY` counts as set**, and an empty `MY_IP` answers `"host":""`. Kept.
11. **Number texts in two messages**: restricted writes `null` for a non-finite `GAMESERVER_READY_TIMEOUT_MS` and
    cap in its error texts where Node writes `NaN`; this spec says JavaScript's text (§2.1). Log/error text only.
12. **NUL in an argument**: restricted's Unix spawn refuses it, while its Windows command line is cut at it (the
    later arguments are lost). Proposed: refuse on every platform (§5.3).
13. **A non-Unicode process variable** makes restricted's start panic (it reads the environment as UTF-8). Proposed:
    skip such variables (§5.4).
14. **Tag query counts**: restricted iterates a huge count (`1e18`) to the end, which hangs; the result is the same
    as stopping at the stream's end, so the MIT crate stops there (§9.4). Nesting depth is bounded by the stream
    length; proposed: no extra cap unless the fuzz target shows a stack risk, then a depth cap that counts as an
    overflow.
15. **The Director ignores core's replay scope** (system clock, system random for launch ids and picks), so a
    shadow replay through it is not deterministic. Kept (game servers are not replayed).
16. **The weak spots of §15** (switch-shaped arguments, the key in the process list, every secret in the child's
    environment, raw client text in logs, the unbounded registry, `sourceTable` paths, Windows label characters).
    Proposed, each its own row: refuse a map that is not `/Game/...` and Behemoth or hunt ids outside a path-safe
    set; hand the key to the runtime another way (runtime patch); give the child an allowlist of variables; escape
    control characters in logged request text; cap the registry; refuse a `sourceTable` that is not a plain name;
    sanitize the label. None is done in this port.
17. **`HEAD` answers**: restricted empties the body after the handler, so `content-length` is 0 where Express sends
    the GET body's length. Not contracted; the tests check only the status and the empty body.
18. **`crates/director`** (§1.4): delete the placeholder, or keep the name for something else? The integration
    pass's call.

---

## Coordinator decisions (2026-11-05, before implementation)

These settle §16 and override anything above that disagrees.

- **Platform layer:** as `module-api/src/tree.rs` does it: one small module per platform under a scoped
  `#[allow(unsafe_code)]` with a one-line reason, `windows` (0.62.2) and `libc` (0.2.191) as target dependencies in
  the crate's own manifest at the versions module-api pins. No workspace change.
- **Item 1:** the `MYSTPAX_` start refusal is kept.
- **Item 2: the three robustness fixes are taken** (NUL in an argument refused everywhere, non-Unicode environment
  entries skipped, the tag-query count stops at the end of the stream). Each departs from restricted only where it
  crashed, hung or cut a command line.
- **Items 3-6:** parity; the security weak spots of item 3 go into `docs/security/rustic-isles-director.md` as
  known weak spots and are candidate decision rows for the maintainer.
- **Item 7:** the `crates/director` placeholder is left alone (integration pass).
