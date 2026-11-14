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

*pending*

## Found before PLAY

| What | Result | Notes |
| --- | --- | --- |
| The MIT runtime's per-port log | **fail** | Ramsgate (Director-started, pid 67488) has `rustic-isles-runtime.dll` loaded and prints `RUSTIC_ISLES_GAMESERVER_READY`, but no `rustic_isles_dll_port8777.log` is written anywhere. `crates/rustic-isles-runtime/src/ffi/log.rs` `open()` uses `OpenOptions::new().access_mode(FILE_APPEND_DATA).create(true)`; std refuses that (`InvalidInput: creating or truncating a file requires write or append access`, reproduced with a 10-line program on this PC), so every line is dropped. A slice bug in R0's log, not an R21 regression: it blocks every log-line check in this pass |

## Servers

*pending*

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

## Fix branches

None yet.
