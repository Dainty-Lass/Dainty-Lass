<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: Rustic Isles game data (RX3)

The checklist for `crates/rustic-isles-gamedata`, the MIT game-data area and generators the Rustic
Isles module and host call (RD27, `docs/port/spec-rustic-isles-gamedata.md`).

How to use it: before a change to this crate merges, and before its cutover is signed (RD10), copy this list into the
change, check each item in the code (not from memory), link the code or test that shows it, and sign at the end with the
date and the commit reviewed. An item that does not hold is a bug to fix first, not a box to leave open.

## What it is and where the boundary is

The crate answers 31 backend rows (the login, Ramsgate and Gauntlet answers, the patch notes, the tuning answers and
the served progression config; `areas.rs`) inside the Rustic Isles backend (port 61000, loopback behind the
gateway), and holds the eight generators `dainty-host gamedata` runs on the host (`generators/`).

- **In (routes):** path params, echoed into JSON by the writer; request bodies of `POST /trials/leaderboards/all`
  and of any write on the Gauntlet host, through core's bounded reader (100 KiB); the replay-aware clock.
- **Stored:** a finished Gauntlet run (`PUT /process_run` from a game server) updates the players'
  `gauntlet_progress` rows; nothing else is written.
- **In (generators):** host-side files only: the host's own game exports, host-supplied responses, the store captures
  hostkit fetched at a pinned commit and checked by hash, and the version's Reward Cache plan and seed list.
- **Files read by a route:** `progression_config.json` under the configured backend folder, through core's cached
  read; no file is named by a request.
- **Files written:** fixed names under hostkit's folders, and backups beside their targets.

Code: `crates/rustic-isles-gamedata/src/` (`areas.rs`, `progression_config.rs`, `tuning.rs`, `gauntlet.rs`,
`patchnotes.rs`, `generators/`).

Fuzz targets in `fuzz`:

- `rustic_isles_gamedata_windows`: arbitrary JSON as `payload.paths`, any served id and recent-season text and any
  now in the JS date range: never panics; non-pass elements are unchanged; the path count grows by at most the
  recent-season count; every opened pass has the season window and every added one the future window.
- `rustic_isles_gamedata_formats`: arbitrary bytes as a catalog line, a nested `customData`, a store-image row name,
  a journey row and a Lady Luck source: the readers and the slug, decode, category and price rules never panic; a
  checked texture reference holds exactly its two quotes; `JSON.parse(stringify_pretty(v, n))` equals `v` for every
  parsed `v` and `n` in 0..=4.

## Checklist

### Routes

- [ ] Every "game" row runs `Auth::game_auth` before reading anything, and answers its refusal as made
      (`areas.rs`, `on`); the open rows are exactly the eleven spec §3.1 lists.
- [ ] Only `POST /trials/leaderboards/all` and the Gauntlet host's writes read a body, through
      `read_body_parts_limit` with `BODY_LIMIT`; a body error is core's error page (`trials_all`, `gauntlet_write`).
- [ ] `PUT /process_run` records only for a game server (`AuthData.is_gameserver` or a player-hosted `runtime`); a
      player's own token gets 403 before the body is used, so no one can post a run for another player
      (`gauntlet_write`).
- [ ] A Gauntlet write's body is logged only after `gauntlet::redacted` drops every field named like a credential
      (`session`, `token`, `key`, `secret`, `password`, `auth`): the client's join body carries `session_id`.
- [ ] The open level-access query answers a level number for any `account_id`: it tells anyone a player's best
      Gauntlet level and nothing else (kept open because the client sends it without a bearer).
- [ ] Params reach answers only as JSON strings (escaped by `service_kit::jsjson`) or as a log line's debug text.
- [ ] `GET /progression/config` reads the one fixed file name; a read error is logged and answered with the 500
      page, without the error text in the answer (`progression`).
- [ ] The cached document is never modified: the rewrite works on a clone (`progression_config::served`).
- [ ] A survey hook's status outside 100-999 is a 500 page, not a panic (`survey`).

### Bounded work

- [ ] The window rewrite is linear in the paths plus the recent-season list (`widen_huntpass_windows`).
- [ ] The generators are linear in their inputs except the Reward Cache sort (n log n over the plan) and the
      platinum pass over baked rows.
- [ ] The three regexes (the token pattern, the quest-rumour row, the character entitlement) and the texture path
      check use the `regex` crate, linear time.

### Generator outputs

- [ ] A texture reference built from a catalog icon in the Reward Cache and Lady Luck stores is checked
      (`/Game/` then letters, digits, `_`, `.`, `/`, `-` only), so an icon cannot put `'` into it
      (`io::checked_texture`).
- [ ] No generator joins a path from file content: every input and output path comes from the caller
      (`GeneratorPaths`, `run`), and backups go beside their target only (`io::back_up`).
- [ ] Reports and errors carry file paths and catalog ids, never a secret (the generators read none).
- [ ] An error writes nothing, except where spec §5 says a step had already written (none today).

### Known weak spots (kept for parity, spec §11, §12)

- [ ] The platinum store's catalog-icon fallback requires only the `/Game/` prefix, not the checked path: the client
      parses the reference (`platinum.rs`, `Inputs::images`).
- [ ] The storage-policy walk follows symbolic links inside the game data folder; a folder already entered is not
      entered again (`policy::walk`).
- [ ] The served progression config never refreshes while the module runs (core §14).
- [ ] With `RUSTIC_ISLES_REPLAY_HEADERS` on (off by default), the replay clock feeds the season window; core's
      window arithmetic saturates, so an extreme integer time gives a clamped window, not a panic
      (`season::active_season_window_ms`).

## Sign-off

Reviewed at commit `________` on `________` by `________`.
