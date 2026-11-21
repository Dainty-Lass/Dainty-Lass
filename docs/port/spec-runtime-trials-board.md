# The Trials boards in Ramsgate, refreshed on arrival

Client only. Dainty Lass's own behaviour, from a static reading of the 1.14.7 `Dauntless-Win64-Shipping.exe`
(2026-11-20); no upstream source.

## Why

The two Trials boards standing in Ramsgate (solo and group) show the backend's `POST /trials/leaderboards/all`
(B33), which the client asks for once, at login. Returning to Ramsgate from a hunt doesn't ask again, so a run
finished since only shows after the next login.

## What the client has

`UPlayerArenaComponent`, a component of the local player controller, has two native, Blueprint-callable functions:

| Function | Parameters | What it does |
| --- | --- | --- |
| `InvalidateLeaderboardCache` | none | drops the boards it holds |
| `QueryTopLeaderboardsForDiegetic` | `ELeaderboardType LeaderboardTypeForQuery` (one byte: `None` 0, `Solo` 1, `Group` 2) | asks the Phoenix leaderboards for that board; the answer reaches each board through the delegates it bound when it set up |

Each board in Ramsgate holds the component and its own `ELeaderboardType`, binds the component's delegates, and
calls the query with its type.

## What the runtime does

On the client's `ProcessEvent`, after `Function Engine.PlayerController.ClientRestart`:

1. Nothing when the flag file `DISABLE_TRIALS_BOARD_REFRESH` is present.
2. Nothing unless `GWorld`'s full name contains `ramsgate_01_persistent` (any case), the hub map the Director
   launches. Once per world: a second `ClientRestart` in the same world does nothing.
3. Finds the `PlayerArenaComponent` instance whose outer is the controller the call was made on (one object
   array scan). None: logs `[TrialsBoard] Ramsgate: the controller has no PlayerArenaComponent; boards left as they
   are` and stops.
4. Calls `InvalidateLeaderboardCache`, then `QueryTopLeaderboardsForDiegetic` with `Solo`, then with `Group`,
   through `ProcessEvent`.
5. Logs `[TrialsBoard] Ramsgate: cache dropped=<bool>, solo asked=<bool>, group asked=<bool>`, each bool whether the
   call was made.

The backend sees a `POST /trials/leaderboards/all` on each arrival.

## Not verified

Only the tests ran. That the boards redraw from the new answer, rather than only the next time they set up, is the
in-game check: finish a Trial, return to Ramsgate, and look for the line above and the run on the board.
