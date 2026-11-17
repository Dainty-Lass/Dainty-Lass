# Play-test pass, 2026-11

The one hands-on pass after R21 (`spec-runtime-r21.md` §8–9), on the MIT `rustic-isles-runtime.dll` alone. Tree:
`plan` 64565345 on the maintainer's Windows PC, game 1.14.7 at `D:\LooseLibrary\Dauntless\1.14.7`. The pre-R21 runtime
set (`Rustic-Isles.dll` 20261113-7992e094 and its `runtime.json`) is kept in `.workspace\runtime-kept-pre-r21` for the
rollback rehearsal.

Results: **pass**, **fail**, **skipped** (with who deferred it), or *pending*.

## Set-up

| Step | Result | Notes |
| --- | --- | --- |
| Tree at `plan`, `settings.toml` in place | pass | Main checkout moved from `feat/epic-signin-reason` (all in `plan`) to `plan` 64565345; `settings.toml` already there |
| Build the DLL, `publish-runtime` | pass | Published 20261114-231d20b1, `rustic-isles-runtime.dll` sha256 1c7ca158c8469ef7... |
| Keep the previous runtime set | pass | `.workspace\runtime-kept-pre-r21` (one DLL: the 20261113 set had no MIT DLL or `winmm.dll`) |
| Two accounts (Tempest + another omnicell; none; a hammer) | *pending* | |
| `hub_afk_seconds`, `hunt_afk_seconds` 120, `idle_fps` 5, `setup` | pass | `director.env` has `DAINTY_HUB_AFK_SECONDS=120`, `DAINTY_HUNT_AFK_SECONDS=120`, `DAINTY_SERVER_IDLE_FPS=5`; the old file is `settings.toml.pre-playtest` |
| R21 §8.1 the runtime set | pass | `runtime.json` lists only `rustic-isles-runtime.dll`; no `Rustic-Isles.dll` in `.workspace\runtime` (no `winmm.dll` built) |
| R21 §8.1 `doctor`, no retired-name warning | pass | First run warned about a leftover `Rustic-Isles.dll` in the game's `Win64` (same hash as the kept copy): removed, then 0 warnings. Old `rustic_isles_dll_port*.log` moved to `.workspace\logs\pre-playtest-2026-11-14`. `CALL_HUNTPASS_QUERY.flag` left in `Win64` from 2026-11-10 (an AGPL-era flag the MIT runtime doesn't read) |

## Client and login

First session 13:01-13:06: login, Ramsgate, one hunt (Snow Jen, port 8770), back. Checked from the logs:

| Step | Result | Notes |
| --- | --- | --- |
| R21 §8.2 client status line | pass | `[MitRuntime] rustic-isles-runtime 0.0.0 role=client port=0 owns=103 ids`; no `seam=`, `[MitSeam]`, `[UrlRedirect]`, `[ExitTrace]`, `[HUDTrace]`, `[ClientInventoryTrace` |
| R21 §8.2 launcher log, Process Explorer | pass | Launcher (`tauri dev`, `plan` + fix): `installed runtime 20261114-bd4df6bd`, then one `loading ...\Win64\rustic-isles-runtime.dll into the game, SHA-256 ec7f94c4...`. The game's modules: `rustic-isles-runtime.dll` from `Win64` (hash matches the published one), no `Rustic-Isles.dll`, only the system `WINMM.dll` |
| R0 step 2 client spine | pass | `install SP-1 ok`; quit from the menu exits 0 (launcher: `the game exited with code Some(0)` at 2026-11-14 21:09, 2026-11-17 01:08 and 01:38); the two code-3 exits were the console crash, fixed by CON-2 |
| R0 step 3 Director servers | pass | Ramsgate and the hunt: `role=server`, `install SP-2 ok`, `SP-3 ok`, `[Dispatch] SP-3 first tick`; the empty hunt reaped itself 60 s after the last client left (`[Reap] ... TerminateProcess(0)`) |
| R4 step 1 | pass (log) | `install CL-2/CL-1/CL-8/CL-9/CL-7/CL-5 ok`; `[CommandLine] engine line served: NoEAC=1 DisableEOS=1 ... Culture=off Multihome=none`; `[Login] IsEOSEnabled -> 0 (MCP login)` |
| R2 step 1 | pass (log) | `install HTTP-1 ok`, `[BackendAddress] source=env api=http://127.0.0.1:61100`, `[Http] redirect active host=...` |
| R17 press-start, branding, console | pass (log) | `[MitPressStart] welcome text set`, `[MitBranding] collapsed ...`, `[MitConsole] console on F2`; no `parameter size ... call skipped`. Visual checks *pending* |
| R15 step 1 | pass | One `[SupplyCap] removed the stack cap of 5 supply item(s)` line in each of the client, Ramsgate and the hunt |
| R16 step 3 | pass | After the hunt start: `[Travel] leftover-world release on`, `released the stuck Async keep flag on 19 object(s) ... default_input_bpc_C` |

| R17 step 5 `stat fps` | **fail** | F2 opens the console; `stat fps` kills the client at once (13:10:36). UE crash report: `LowLevelFatalError [File:...Core/Public/Misc/Exec.h] [Line: 23] Pure virtual not implemented (FExec::Exec)`, minidump `Archon/Saved/Crashes/UE4CC-Windows-DF9FAE66...`. Runtime log: `[MitException] kernelbase!RaiseException code=0x4000 ... address=+0x265CEA6`, then `TerminateProcess code=3` (R3's crash path worked: no 60 s wait). Stack (RVAs): `+0x265CEA6 <- +0x265F7CA <- +0x2613A6D` (the fatal error) `<- +0xC8C268` (the pure-virtual stub) `<- +0x261B672` (its caller) `<- +0x281F52D <- +0x42653BA`, from the window procedure. `t.MaxFPS 60` crashes the same way (second dump `UE4CC-Windows-B4096EFE...`, same stack), so every console command does. From the dumps: `+0x261B610` walks a static list of command handlers (`+0x260FDB0` returns it; inline `TArray`, data at `+0x40`, count at `+0x48`) and calls each one's vtable slot 1 (`Exec`) until one returns true; one entry's slot 1 is `+0xC8C240`, the Shipping `PURE_VIRTUAL(FExec::Exec)` stub (it loads `Exec.h`, line 0x17, then the fatal error). A Shipping game never walks that list without a console, so the trap is the game's own and the console only reaches it; the AGPL's console (R17 6.1) did nothing more. The pre-R21 run is folded into the rollback rehearsal (maintainer, 2026-11-14): today's kit and launcher remove `Rustic-Isles.dll` by name. **Fix** on `fix/console-exec-stub` f42bc50e (CON-2: the stub answers not handled); retest **pass** (14:04): `owns=104`, `install CON-2 ok`, `[MitConsole] a command handler without Exec answered not handled (vtable +0x53B7220)`; `stat fps` shows and the game stays up |

Watch: the client logs `[MitInput] activate: timeout after 8000-22531 ms, restoring input without the cinematic clear` 15 times after the first possession in Ramsgate (13:01:38-13:01:53). Input worked, but R14 expects `activate` lines without timeouts; see R14 step 2.

## Found before PLAY

| What | Result | Notes |
| --- | --- | --- |
| The MIT runtime's per-port log | **fail** | Ramsgate (Director-started, pid 67488) has `rustic-isles-runtime.dll` loaded and prints `RUSTIC_ISLES_GAMESERVER_READY`, but no `rustic_isles_dll_port8777.log` is written anywhere. `crates/rustic-isles-runtime/src/ffi/log.rs` `open()` uses `OpenOptions::new().access_mode(FILE_APPEND_DATA).create(true)`; std refuses that (`InvalidInput: creating or truncating a file requires write or append access`, reproduced with a 10-line program on this PC), so every line is dropped. A slice bug in R0's log, not an R21 regression: it blocks every log-line check in this pass. **Fixed** on `fix/runtime-log-open` a6323cd6 (`append(true)`, with a test); rerun on runtime 20261114-bd4df6bd: Ramsgate writes `rustic_isles_dll_port8777.log` |

## Servers

| Step | Result | Notes |
| --- | --- | --- |
| R21 §8.3 server log | pass | Ramsgate: `role=server`, install lines, Director saw ready after 6.2 s, `[NetConnEdge] tick=1 connCount -2 -> 0` at listen and `0 -> 1` at 13:01:28 |
| R5 step 1 | pass (part) | `install HL-12 ok`, HL-5/HL-10 dropped, HL-6..HL-9 off, HL-11 pending, `GetViewportSize answers 1920x1080`; no `NOT applied`/`NOT installed` |
| R9 step 1 | pass | `[MapUrl] map=... maxPlayers=4`, `ready marker written launchId=`, `listening on port 8777` |
| R8 step 1 | pass | `[FramePace] server frame cap: 30 fps, idle ... 5 fps`, `[ExpectedPlayers] roster sync`, `[AfkTimeout] config hub=120 s (env) hunt=120 s (env)`, `[Watchdog] started`, `released the engine's 32 MB out-of-memory reserve` |
| R8 step 2 | pass | `no client for 15 s: 5 fps`; `[WorkingSet] trimmed` 62 s later. CPU and `[Perf]` not checked |
| R8 step 3 | pass (log) | `client connected: 30 fps`; `[AfkTimeout] role=hub ... applied 120 s`. Being sent back after 120 s idle *pending* |
| R6 step 1 | pass (part) | `[WidgetGuard] server guards on (boot patch applied: yes)`, `install UI-12 ok`; no `bytes differ` or `group aborted`. 8 `UI-1 skipped HideHoldText` lines at each boot |
| R3 step 1 (servers) | pass (part) | `[MitCrashWait] ReportCrash wait ... 60000->5000 ms: patched`; client EXIT-1..EXIT-12 ok |
| R20 step 1 | pass (log) | `[PlayerCanRestart] forced true`, `[OwnerPawnRelevancy] ... ViewTarget ... -> pawn`, `[NoLocalPlayer] StartMatch at the first login: called` |
| R20 step 3 | pass (log) | Hunt: StartMatch line, one `[Bleedout] BleedoutDuration was 0; set to 30s` |

Watch: Ramsgate logged `[Progression] configuration not loaded yet` at 09:03:22 and `configuration loaded` only at 10:00:05, with nobody connected; the hunt loaded it when the client arrived. See R7 step 2.

## Loading, travel and roles

*pending*

## Two players

*pending*

## Trials and escalation

*pending*

## Realms

*pending*

## Last: Training Grounds stop, leftovers, rollback, exit

*pending*

## Outside the runtime

*pending*

## Dashboard (2026-11-16, with `feat/dashboard-grants` 57192a80 merged locally)

Dashboard in dev mode (`ui`, `bun run dev`, dev sign-in as owner) against this server. Grant, player details, online state and kick all fail:

| What | Result | Cause |
| --- | --- | --- |
| Backend admin calls | **fail** (intermittent) | Every 15 s the backend refuses one of two `GET /admin/v1/online-players` with `401 ADMIN_OPERATOR_REPLAY`. The dashboard's signer (`ui/packages/fleet/src/game.ts`, `String(Date.now())`) can sign two identical requests in one millisecond; the backend keys its replay set on the signature. `dainty-admin`'s `Upstream::next_timestamp` already guards against this; the dashboard doesn't |
| Player page details, Grant | **fail** | `player(username)` goes through the module interface for 1.14.7 (`module_player`): `userId` is the store's account key (`account:01M73Z...@local`), not the game's user id (`9c65cf9f-...`, `accounts.user_id`), and joined, last sign-in, access, approval and roles are left empty. The page then asks the backend `playersById` for the account key, gets nothing (`Unknown` badge), and Grant is shown only when that lookup found an account |
| Online state | **fail** | The module's `Lookup` answers `online` from backend presence plus 3 min of recent activity. The game's live WebSocket goes through the gateway to chat (`127.0.0.1:41006`), not to the backend's presence, so a player in Ramsgate reads offline |
| Kick | **fail** | The module's kick revokes unused exchange codes and calls `presence().close_connections`, which only forgets the user ("Only forgets the user here", `crates/rustic-isles/src/shared.rs:79`). Nothing reaches the game server or the chat socket, so the player stays in the hub and can still queue a hunt |
| Chat log and chat filter | **fail**, set-up | Nothing reached the dashboard's chat logs and a masked term (`potato`) masked in the tester but not in the game: `settings.toml` had no `dashboard_url`, so chat neither posts its log nor fetches the filter (as documented when it is left out). Set `dashboard_url = "http://localhost:5173"`, reran `setup`, restarted: chat logs `filter version 2 (1 terms)` and the dashboard serves `GET /api/ingest/chat-filter`. Retest: **pass** for the game and the overall log (`potato` masked in game; the line is in the chat log, `verdict=masked`). The player page's Chat tab stays empty: the line is stored under `sender_user_id = 9c65cf9f-...` (the game id) and the page searches by the admin API's `account:...@local`, the same id bug as above. History gets no entry for a masked line: `/api/ingest/chat` writes no audit event, and History lists staff actions only, so that is a missing feature, not a fault |
| Kick with `feat/dashboard-grants` cf391169 (local merge 77163668) | **fail** (half) | Kick at 01:03:35: chat closes the session (`chat: kick uid=9c65cf9f-...`), and the client's own calls are refused (`POST /party`, `GET /party/invites` -> 403), so no chat and no queueing. But the player stays in Ramsgate (nothing tells a game server to drop a player), and crafting still works: Ramsgate's server posts `POST /inventory ... gsKey=Y` on the player's behalf at 01:06:01 and 01:06:03, both 200. Calls a game server makes for a player don't check the kick |
| Kick with 4960fb2d + `feat/runtime-kick` ac35f239 (local merge 8702298b) | **pass** (removal), **fail** (rejoin) | 01:37:08 kick: the Director asks Ramsgate, the runtime logs `[Kick] connected ids: [9c65cf9f-...]` and `sent to the menu (ClientReturnToMainMenuWithTextReason)`, and the client is at the menu. But the kick keeps every token revoked until the launcher signs in again: `/profile/update`, `/migration/trigger`, `/entitlementsv2` and `/party` answer 403, and chat refuses `reason=throttled`. Maintainer: a kick must lift once the player is removed; only bans and time-outs persist. Sent to the dashboard-grants session |
| Kick with `feat/runtime-kick` 82c5a601 (has `feat/dashboard-grants` 785108d7; local merge fcc6fce4) | **pass** | The kick sends the player to the menu, lifts once they are removed (at most 20 s), and they rejoin from the menu without restarting the launcher |
| Grant route (`POST /admin/v1/players/:userId/grants`, signed as operator 0) | **pass** (API) | 02:26: 100 000 steel and gilded marks, 5 000 each of `CURRENCY_PJM_WEAPON`/`_ARMOR`/`_ALCHEMY`, 1 000 000 rams to SilentSpud: `status: done`, one character. The client shows them only after logging in again (the hub keeps the inventory it loaded); after relogging they show |
| Cell fusion's unlock | note | The Middleman's fusion is Hunter's Journey node `Middleman.Unlock`: parent `Slayer_05` (7 500 player XP), cost 25 `CURRENCY_PJM_WEAPON`. SilentSpud has `Slayer_05` complete and `Middleman.Unlock` locked (status 0). A direct SQL edit of `player_journey` is refused by the store's guard trigger, so journey nodes are unlocked in the game with the granted tokens |
| Lady Luck's Store, buying again | **fail** | Items already bought (`PR_FRANK` 02:34:16, `AC_HEAD_TRIALS_01` 02:34:25, `AC_HEAD_TRIALS_00` 02:34:29) still show in the shop and can be bought again. The repeat buys at 02:36:16 and 02:36:20 are `STORE` transactions that change nothing (no charge, no duplicate), so it's the listing that is wrong. The offers say `maxAllowed: 1`, but the SKU builder (`crates/rustic-isles-economy/src/bounties/store.rs:556`) sets `remaining` to `maxAllowed` for everyone, and the game reads `/product/skus/public?requiredTags=...`. Whether the per-player state belongs in that route or another is not yet known |
| Lady Luck's Store | **pass** | Bought with marks: `STORE` transactions at 02:34 (`PR_FRANK` for 300 steel marks; two `AC_HEAD_TRIALS_*` for gilded marks) |
| `Middleman.Unlock` in the game | **pass** | Unlocked from the Hunter's Journey: Ramsgate took 25 `CURRENCY_PJM_WEAPON` (5005 -> 4980, `INVENTORY_GAMESERVER` at 02:35:59) |

### Solo pass, group 1 (2026-11-17, one account)

| Step | Result | Notes |
| --- | --- | --- |
| R17 press-start popup | pass | Dainty Lass welcome text |
| R17 login screen | pass | Stock art, no "Update ..." version line |
| R17 credits | pass | Dainty Lass block above Phoenix Labs, survives scrolling and reopening; `[MitCredits] Dainty Lass block above Phoenix Labs: placed 4/4 at index 1` |
| R15 step 3 cell fusion | pass | 30 min in the client; server `[CellFusion] set 3/3 fusion recipe(s) of cell_exchange_table to 30 min`. The Middleman's dust time skip works |
| R20 step 1 / R5 step 3 vendors | pass (behaviour) | Two vendors, camera frames them. No `[VendorFix] absorbed UpdateCameraForAspect` line in Ramsgate's log all session: watch. The 17 `Divide by zero` lines in Ramsgate's engine log come from `city_jump_vent_00_splined_bp_C:ReceiveActorBeginOverlap` (jump vents), not vendors |
| R8 step 4 / R20 stamina | pass | Drains and refills |
| R8 step 3 AFK | pass | Sent to the menu after 2 min idle; the hub stays up |
| R21 step 4 stores | pass | See Lady Luck above |
| R15 step 2 stack above 30 | pass | The grant route refuses `QI_11A_CRIT_POTION` (422 `is not in the server's catalog`): it accepts only ids `inventory_storage_policy.json` names, and none of the five cap-lifted `QI_11A_*` tonics are there, though `catalog_1_14_7.jsonl` has them. A gap in `feat/dashboard-grants`, fixed in a6c24ae9 (in `feat/runtime-kick` f2d9cbe4; needs `dainty-host gamedata`): 40 `QI_11A_CRIT_POTION` (Granny's Heartbreaker) granted at 03:14 and stored, but the client lists it nowhere: this build's UI doesn't show that tonic. 40 `QI_STAMINA_POTION_00` (Stamina Tonic) granted instead: **pass**, the stack shows 40 and keeps it after a hunt |

### Solo pass, group 2 (2026-11-17 02:43-03:02 UTC)

Training Grounds twice, three hunts (Moss Triforce to the end, Snow Jen left with Go Now, Snow Jen quit from the menu). No crash dialog. From the logs:

| Step | Result | Notes |
| --- | --- | --- |
| R16 steps 2-3 travel | pass | No crash; `[Travel] leftover-world release on`, `released the stuck Async keep flag on 24 object(s) ... default_input_bpc_C`; no `survived the purge`, no `+0x04272D28` |
| R7 step 5 Training Grounds arrival | pass | `[Readiness] client: released the idle arrival trigger sinceClientRestartMs=3000` |
| R6 step 3 Training Grounds 3 min | pass | No restart; the hub stayed up |
| R7 step 4 hunt loading | pass | Seconds: ready 19:49:38, the player in at 19:49:44 |
| R6 step 4 / R5 step 3 hunt | pass | UI-3..UI-11 `install ... ok`; one `UI-1 skipped HideHoldText` per server; no server exit, no `[Panic]` |
| R20 step 3 solo hunt | pass | `[Bleedout] BleedoutDuration was 0; set to 30s` on every hunt; downed timer, death, respawn and stamina fine |
| R2 step 2 hunt grant | pass | `[Http] grant path=/progression/9c65.../season43/9 grant=gs-...`; XP and rewards in Ramsgate |
| R11 step 4 Go Now | pass | Back to Ramsgate |
| R3 step 2 quit from a hunt | pass | Client exits 0 (`the game exited with code Some(0)`, 03:01:02); no crash dialog |
| R8 step 5 / R21 step 5 empty hunts end | pass | Each hunt: `[Reap] disposable server empty 60 s after its last client left -> TerminateProcess(0)`; the Director records `code=0` (ports 8770, 8771, 8772) |
| Training Grounds idle stop (G-1 shape, 5 min) | pass, watch | `[TrainingOnDemand] the Training Grounds stopped (no player for 5 min)`, but the server exits `code=1` with no `[MitExit]` line: the Director kills it rather than asking it to quit. R21 §8.7 only asks that it exits |

### Solo pass, group 3: omnicell (R14), 2026-11-17 03:30-03:46 UTC

Account SilentSpud with `PR_FRANK` (bought at Lady Luck). Hunts on ports 8770 (Moss Triforce), 8771 (Moss Croissant), 8772.

| Step | Result | Notes |
| --- | --- | --- |
| R14 step 2 Ramsgate | pass (note) | Input fine. The omnicell is part of the combat HUD only, so Ramsgate shows none (the checklist's "HUD shows the omnicell" in Ramsgate doesn't apply). Ramsgate: `[MitPlayerRole] apply ... item=PR_FRANK pawnRole=0x... applied=1` once (20:33:07 local) |
| R14 step 4 hunt | **fail** | The omnicell shows in the hunt and fires (server `[MitPlayerRole] ability: ... activated=1`), but it disappears when the glider comes out or after one Behemoth is beaten, and stays unusable until the player leaves the hunt. Server side the role lives all hunt: `refresh: done ... pushes=3` at arrival, `purge ... reason=route-remove` only at the leave (`connCount 1 -> 0`). On the hunt servers `apply ... item=PR_FRANK pawnRole=null ... applied=0`. Client: `lifecycle replayed trigger=pump gameplayEquipped=0->0 roleActive=0->0 bpEquipCalled=0->0` at arrival; no `[MitHud]` line in the whole client log (R14 12.3's airship hide and restore never ran); later in Ramsgate `re-applied the role attempt=1..3 id=PR_FRANK roleAfter=null`. Not yet run on the pre-R21 pair |
| R14 step 4 retest with `fix/omnicell-mid-hunt` 2fe549d5 (runtime 20261117-27a14d39) | **fail** | First try ran the old DLL: the launcher played its installed runtime without fetching the server's new one (it started while the server was down for the publish); after a launcher restart it trusted and installed 20261117-27a14d39 and injected `dcb98ffb...`. Same symptom, now pinned down: the omnicell is tied to the **first combat area**; it vanishes on leaving that area after the kill and comes back on returning there and pressing V. Client: `role intact after unsheathe` with the same role at every draw; server: `ability: handle=58 ... activated=1` six times, same handle. So the role survives; the client's combat-HUD omnicell is bound to the first encounter. Sent back to the omnicell-drop session |
| R14 V-press count (port 8772, Moss Triforce, 21:21 local) | **fail**, diagnostic | Maintainer counted presses; the server's `ability:` lines counted arrivals. First combat area: 3 presses, 3 lines (`handle=14 ... activated=1` at 21:21:56, 21:22:17, 21:22:37). Second combat area: 6 presses, none worked, **0 lines**: the presses never left the client. So the client stops sending the ability once away from the first area |
| R14 retest with `fix/omnicell-mid-hunt` fadddb3c (runtime 20261117-40da8100, `b3f1a12e...`; local merge 1978a3a8) | **pass** (areas 1-2) | Hunt server: `[MitRelevance] route: role helper actor=frank_drone_bp_C_... route=global` at 21:29:03, one second after the join. First area 2 presses, second area 2 presses, 4 server lines (`handle=14 ... activated=1` at 21:29:35, 21:30:10, 21:30:58, 21:31:19); the omnicell stays on the combat HUD in the second area. No `rpc: dropped ... frank_drone` |
| R14 step 6 back to Ramsgate | pass (note) | Input fine; no omnicell HUD in Ramsgate by design |
| R14 step 7 Training Grounds twice | pass | No crash |
| R14 step 9 loadout slot 3 | pass | Still slot 3 after a hunt |

### Solo pass, group 4: Trials (R18), 2026-11-17 04:41-04:49 UTC

| Step | Result | Notes |
| --- | --- | --- |
| R18 step 3 launch-pinned row | pass (log) | Both arenas (`arena_ramsgate_00`, ports 8771 and 8772): `install ESC-1 ok`, `[TrialsSchedule] Initialize: launch-pinned row='Scheduled_Arena_Hunt_072'`, the row Ramsgate's board is on. Watch: `launch-pinned row missing 'Scheduled_Arena_Hunt_072' table=gen_arena_hunts_schedule_new`, the second, empty arena table (Ramsgate's `patchedRows=0/0`) |
| R14 step 8 airship HUD on a Trials launch | pass | `[MitHud] hide trigger=state-update`, `airship ended`, `restore ... reason=player-control sinceAirshipEndMs=19750` (the first `[MitHud]` lines of the pass: hunt launches never send `OnAirshipStateUpdate` through ProcessEvent) |
| Omnicell in the arena | pass | `route: role helper ... frank_drone`, `ability: handle=49 ... activated=1` |

Escalation: **pass**. Solo Umbral escalation (`escalation_island_umbral_00`, port 8770, 05:17 UTC): after each round `[EscalationBuffChoiceSpawn] queued ... trigger=EventEnding(7)`, `dispatching`, `dispatched`; relic pickups appear, one is chosen, the next round starts (R18 step 5, R21 §8.6).

23 war pikes granted (05:1x UTC, `status: done`, stored as instanced items like `WP_MS_HOST`): every non-transmog `WP_MS_*` with a display name, less the three owned and `WP_MS_UNEQUIPPED`; eight with `<MISSING STRING TABLE ENTRY>` names (MINT, GLITTER, FULGAR_ALPHA, HABANERO, MANTIS, FROSTWOLF, AMBER, FENROAR) were then granted too, at the maintainer's request (`status: done`). Weapon Smith tiers ("Tier 1" on the tooltip) are stored somewhere not yet traced; every owned weapon's `itemData` is null. For the escalation, SilentSpud got 50 000 each of `CURRENCY_PJM_WEAPON`/`_ARMOR`/`_ALCHEMY`/`_SEASON_13_PAST`/`_SEASON_13_FUTURE` and 5 000 000 rams (04:59 UTC) to buy the Hunter's Journey power nodes: gear has no per-item level in 1.14.7 (every instanced item's `itemData` is null), power comes from `WeaponPowerUpgrade*`, `WeaponCritUpgrade*`, `HealthUpgrade`, `StaminaUpgrade` and the like.

### Last phase (solo), 2026-11-17

| Step | Result | Notes |
| --- | --- | --- |
| R21 §8.7 Training Grounds stop (G-1) | pass, watch | `training_idle_minutes = 1`, `setup` (`TRAINING_IDLE_STOP_MINUTES=1`), restart. Training Grounds on request 05:29:52, ready 05:30:03; `connCount 0 -> 1` 05:30:07, `1 -> 0` 05:30:24; `[TrainingOnDemand] no player in the Training Grounds for 1 min - stopping them (pid=72428)` 05:32:11 (1 min 47 s after the leave: the Director's idle check runs on an interval); `Gameserver process exited port=8776 label=training_dojo code=1`. Watch: exit code 1 with no `[MitExit]` line, so the Director kills it rather than asking it to quit |
| R21 §8.9 leftovers in `.workspace\runtime` | pass | The kept `Rustic-Isles.dll` copied in: `doctor` warns `Rustic-Isles.dll in ...\.workspace\runtime is a runtime R21 retired -> run dainty-host publish-runtime`; `publish-runtime` removes it; `doctor` stops warning. Watch: every publish while a game server has the DLL loaded leaves `rustic-isles-runtime.dll.rr-old-<n>` behind (two by now), which `doctor` doesn't mention |
| R21 §8.9 leftovers in the game's `Win64` | pass | `doctor` warns `... is a runtime R21 retired -> delete it`. The launcher, seeing the set no longer matches, offers "The game runtime has an update"; accepting it (`installed runtime 20261117-40da8100`, 05:35:30) removes `Rustic-Isles.dll`, and PLAY starts the game with only `rustic-isles-runtime.dll` loaded (checked in the process's modules). The launcher log doesn't name the removed file |
| R21 §8.11 exit | pass (client), watch (server) | Quitting the client from the menu exits 0 every time (`the game exited with code Some(0)`). Hunts end themselves with code 0; a Director-stopped server (the Training Grounds idle stop) exits 1 with no `[MitExit]` line |

### Flags and the forced crash (solo), 2026-11-17 05:38-05:55 UTC

Each group's flag files went next to the game executable and were removed after its step; the R15 settings were put back.

| Step | Result | Notes |
| --- | --- | --- |
| R17 step 6 branding flags | pass | `DISABLE_PRESS_START_BRANDING`, `DISABLE_CREDITS_BRANDING`, `DISABLE_LOGIN_BRANDING`: stock press-start text, stock version lines, no Dainty Lass credits; no `[MitBranding]`/`[MitPressStart]`/`[MitCredits]` line |
| R6 step 7 client flag | pass | `DISABLE_HUD_LEGENDARY_GUARD`: `install UI-13 off (DISABLE_HUD_LEGENDARY_GUARD.flag)` |
| R17 step 7 `AUTO_PRESS_START` | pass (note) | The client passes the popup, presses Play and reaches Ramsgate alone. Two `[MitAutoLogin]` lines (`press start: OnPressToStartDismissed(UserIndex = 0) called`, `Play pressed (poll 4)`), not three: step 3's `StartTravelToCity` never logged, the Play button's own travel got there first |
| R5 step 5 server flags | pass (log) | One Ramsgate boot (22:42:51 local): `HL-12 off (DISABLE_SERVER_LOCALE_FIX.flag)` with the served line `Culture=off` (R4 step 6's flag half); `HL-4 off (KEEP_WEB_BROWSER.flag)` and a `UnrealCEFSubProcess.exe` running; `HL-3 off (SHOW_SERVER_WINDOW.flag)` but no server window (the server still runs `NullRHI=1`); `HL-11 off (DISABLE_SERVER_VIEWPORT_SIZE.flag)`; `ENABLE_SERVER_AUDIO`: no `sound posting suppressed` line |
| R11 step 6, R12 step 6, R3 step 4, R6 step 7 server flags | pass (log) | `[MitNet] spline guard off`; `install RG-7 flag DISABLE_RPC_UNBOUND_CHANNEL_GUARD` (not the spec's `rpc guard off` wording); `[MitCrashWait] ... native 60000 ms crash wait kept`; `UI-4..UI-11 off (DISABLE_CALLOUT_GUARDS.flag)`; `UI-12 off (DISABLE_INVENTORY_REFRESH_GUARD.flag)`. The hub ran with them; no 10-minute soak |
| R20 step 5 `DISABLE_OWNER_PAWN_RELEVANCY` | pass | No `[OwnerPawnRelevancy]` line in Ramsgate (boot 22:44:02 local, login 22:51:19); the login completes and the player moves |
| R3 step 3 forced client crash | pass (note) | `cdb -p <client> ~0s; r rip=0; .detach` at 22:52:11 local: `[MitException] VEH code=0xC0000005 ... rip=null` (22:52:11.646), then `[MitExit] kernelbase!TerminateProcess handle=self code=3` 0.45 s later; launcher `the game exited with code Some(3)`. No `[MitEngineError] fatal error` line: the client goes from the exception straight to the exit (EXIT-13's error writer is a server install) |
| R15 step 3 `cell_fusion_minutes = 5` | pass | Server and client `[CellFusion] set 3/3 fusion recipe(s) of cell_exchange_table to 5 min`; a new fusion shows 5 minutes in the client |
| R15 step 5 `keep_supply_stack_caps = true` | pass | Server (22:53:05) and client (22:53:45): `[SupplyCap] DAINTY_KEEP_SUPPLY_STACK_CAPS=true: the game's stack caps are kept`. Settings put back afterwards |

## Fix branches

| Branch | SHA | What |
| --- | --- | --- |
| `fix/runtime-log-open` | a6323cd6 | The MIT runtime's per-port log was never created (std refuses `create` with a custom `access_mode` alone). On `plan` 7eea2f16 |
| `feat/dashboard-grants` | 785108d7 | Staff grants, player ids and details, online state, kick/ban/time-out across tools, History for every action (dashboard-grants session). Merge after `feat/runtime-kick` conflicts in `crates/rustic-isles/src/shared.rs`; `feat/runtime-kick` already contains it |
| `feat/runtime-kick` | 82c5a601 | A kick sends the player from their game server to the menu (`docs/port/spec-runtime-kick.md`), then lifts |
| `fix/launcher-full-width` | cfe3d38a | Host and Preferences capped their scroll area (900/820 px), leaving an empty strip right of the content in wider windows |
| `fix/omnicell-mid-hunt` | fadddb3c | R14: game-thread gate and sheathe watch (2fe549d5); R13: the Frank drone and other role helpers route globally, so the omnicell works outside the first combat area |
| `fix/ladyluck-owned-items` | f375a7c4 | A bought one-time offer lists with `remaining` 0. Merged locally (1551ca5a); in-game: **pass**, `PR_FRANK` and the Trials heads show as owned |
| `fix/console-exec-stub` | f42bc50e | CON-2: every console command reached the game's Shipping `FExec::Exec` stub and killed the client. On `plan` 7eea2f16; independent of the log fix |

The pass ran on `plan` 7eea2f16 (RT-1, MinHook through the crate) plus `fix/runtime-log-open` (runtime 20261114-bd4df6bd), and from 13:37 also `fix/console-exec-stub` (a local merge, runtime 20261114-2fef918d).
