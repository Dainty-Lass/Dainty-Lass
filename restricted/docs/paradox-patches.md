# Changes to Mystic Paradox

rustic-isles (`modules/rustic-isles`, in-tree since RD27 and a submodule from DS6: the Node services under
`reference/1.14.7`, the runtime and the CatalogExporter under `native/`; `paradox/` and `native/` here before DS6) is
Mystic Paradox at commit `21b9ff5` (`pranav158/Mystic-Paradox`, upstream `main` on 2026-10-23) with the changes
below. Each is marked in the code with `[DaintyLass]` or, for the ones taken
over from the Dauntless Reforged Revival line and kept as received, `[Dauntless Reforged Revival]`
([NOTICE.md](../NOTICE.md)). A patch written against Mystic's tree is applied as
[paradox-runtime-patches.md](paradox-runtime-patches.md) describes.

## Infrastructure (this project's)

| What | Where |
| --- | --- |
| The runtime reads the backend address from `RUSTIC_ISLES_BACKEND_URL` or `DAINTY_BACKEND_URL`, so one build serves every server; without a generated deployment header it fails closed on a reserved name; the XMPP `bUseSSL` switch follows the address's scheme, so a `ws://` relay is not opened with TLS | `modules/rustic-isles/native/runtime/dllmain.cpp` |
| The launcher API (`/dainty/api`): registration with invite codes, account keys as the game's `AUTH_MODE=APIKEY` login, the live status, and the host's invite, account-status and kick routes | `modules/rustic-isles/reference/1.14.7/ParadoxBackend/src/routes/reforgedApi.ts`, `src/launcherApiRules.ts`; `routes/eos.ts` (the client-credentials token, banned accounts refused); `config/listenerPlan.ts` (`DAINTY_HTTP_ONLY`, `QOS_PORT=0`) |
| The Director listens on loopback (`BIND_HOST`) and lists its running game servers at `GET /api/instances` for the status | `modules/rustic-isles/reference/1.14.7/ParadoxDirector/src/server.ts`, `app.ts`, `controllers/gameservers.ts` |
| The admin API's kick (MIGRATION_PLAN.md RD25) | `routes/reforgedApi.ts` |
| `store-captures.json`: the five live-store captures the store generators read, by commit and SHA-256, fetched on the host's machine | `modules/rustic-isles/reference/1.14.7/ParadoxBackend/store-captures.json` |

## Gameplay fixes and settings, taken over (0004 to 0039)

Numbered as they were received. Those written against Mystic's `main` as it was then and later merged there
by Mystic (0006, 0009, 0019, 0032, 0034) are not listed: `21b9ff5` has them.

| # | What | Where |
| --- | --- | --- |
| 0004 | A hub stays up when its phantom player's data cannot load | runtime |
| 0005 | The XP and objectives a game server reports are stored (the objectives through Mystic's own mastery report) | backend |
| 0007 | XP and reward multipliers for private test servers (`DAINTY_XP_MULTIPLIER`, `DAINTY_REWARD_MULTIPLIER`; `dainty-host multipliers`) | backend |
| 0008 | The progression config is queried once at server start, so a hunt's loading screen does not wait a minute | runtime |
| 0010 | Claiming a completed bounty deletes it (`POST /bounty/delete/:userId`) | backend |
| 0011 | A hunt client is not forced ready after 3 s | runtime |
| 0012 | A restarted hunt waits for the player's data before spawning them | runtime |
| 0013, 0014 | Logs for a hunt's missing weapon, on the client and the server (diagnostics that led to 0015 and 0016; droppable) | runtime |
| 0015 | A dormant weapon is sent to its player in a hunt | runtime |
| 0016 | An always-relevant weapon is not culled at the world origin | runtime |
| 0017 | New and existing characters can quick chat and quick emoji | backend |
| 0018 | The login queue's maintenance status | backend |
| 0020 | A frame cap for the game servers (`DAINTY_SERVER_MAX_FPS`, `DAINTY_SERVER_IDLE_FPS`) | runtime |
| 0021 | The Training Grounds start on demand; quieter game-server logs | Director |
| 0022 | A new hunt only when there is memory for it (`GAMESERVER_MEMORY_GUARD`, `GAMESERVER_MIN_FREE_MB`, `GAMESERVER_HUNT_MEMORY_MB`) | Director |
| 0023 | Supplies refilled at every game login | backend |
| 0024 | Supplies without a stack cap | runtime |
| 0025 | Ramsgate chat, quick chat and emojis reach the City room | backend |
| 0026 | The client logs its inventory and loadout events (diagnostics that led to 0027; droppable) | runtime |
| 0027 | The arsenal looks its items up again when the inventory arrives late | runtime |
| 0028 | Several bounty tokens stay active | backend |
| 0029 | Cell fusion at the Middleman takes 30 minutes | runtime |
| 0030 | Platinum offers are free (`PLATINUM_STORE_FREE`) | backend |
| 0031 | Game servers run with the C locale, so broken Behemoth parts show as broken | runtime |
| 0033 | The on-demand Training Grounds stop again once empty (`training_idle_minutes`) | Director |
| 0036 | Weapons are served at item level 0 | backend |
| 0037 | The loadout slot repair re-applies cells, parts, dye and transmog | runtime |
| 0038 | Game servers answer a 1920x1080 viewport size, for aiming | runtime |
| 0039 | An RPC on an unbound controller channel is dropped (`DISABLE_RPC_UNBOUND_CHANNEL_GUARD.flag` turns the guard off) | runtime |
| 0040 | The runtime reads `DAINTY_BACKEND_URL`, `DAINTY_SERVER_MAX_FPS` and `DAINTY_SERVER_IDLE_FPS` (the 2026-11-06 rename from `REFORGED_`) | runtime |
| 0041 | The seam to the MIT runtime: with `rustic-isles-runtime.dll` loaded first, every hook goes through its broker and each hook or feature block it owns is left to it; without it nothing changes (RT-4, the root's `docs/port/spec-runtime-r0.md` section 7) | runtime |
| 0044 | The F2 console is left to the MIT runtime when it owns `CON-1`; without it nothing changes (the root's `docs/port/spec-runtime-r17.md` section 6) | runtime |
| 0045 | The game server keeps forcing its server flags right after each engine tick when the MIT runtime owns HL-1, so this runtime's own post-tick work still sees them (the root's `docs/port/spec-runtime-r5.md` section 9.2) | runtime |
| 0047 | With CL-1 (`FCommandLine::Get`) left to the MIT runtime, a server takes the engine's `MULTIHOME` for the player host's listen check and the watchdog's port probe from the line the MIT runtime serves (the root's `docs/port/spec-runtime-r4.md` section 8) | runtime |

0035 (an A/B flag for the controller-channel guard, diagnostics only) was removed on 2026-10-26. 0013, 0014
and 0026 are logging only and can go the same way once the runtime is rebuilt and checked with a client; they
are interleaved with later fixes in `dllmain.cpp`, so removing them is a hands-on change, not a reverse-apply.

Candidates to become `host.toml` settings rather than patches: 0007, 0023, 0024, 0029 and 0030 are server
policy, not fixes.

## Updating Mystic Paradox

Advance the fork's branch, replay the changes above onto the new base one by one, keep, reduce or drop each,
and record the new commit here and in [NOTICE.md](../NOTICE.md). `ParadoxLauncher`, `ParadoxTransport` and
Mystic's workflows stay out; the realms transport goes to `rustic-rifts`
([dainty-lass-plan.md](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/docs/dainty-lass-plan.md), DD14).
