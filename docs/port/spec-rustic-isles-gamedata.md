# Clean-Room Behaviour Specification: `crates/rustic-isles-gamedata` (RD27)

> **Scope and purpose (RD27):** this is the clean-room behaviour specification for an MIT crate,
> `rustic-isles-gamedata`: unit 3 of [`spec-rustic-isles-core.md`](spec-rustic-isles-core.md) §16. It answers the 29
> routes of the `game-data` area group (the stubs and static answers the 1.14.7 client asks for at login, in Ramsgate
> and on the Gauntlet host, and the served progression config) and holds the eight backend game-data generators that
> `reforged-host gamedata` runs. Its source is dainty-restricted's `modules/rustic-isles/crates/isles-core`
> (`src/areas/game_data.rs`, `src/game_data/{progression_config,tuning,gauntlet,patchnotes,generators}.rs`,
> `src/game_data/src_tokens.txt`) and the generator mapping in restricted's `crates/host`, all AGPL-3.0-only with
> Mystic Paradox's section 7 terms; an implementer who never sees them writes the MIT crate from this document alone.
>
> Everything below is stated from observable contracts: the routes and their bytes, the input and output file
> formats, the transforms, the settings, the seam, the refusals and the tests. No implementation code is reproduced
> and no private helper is named. Node file names appear only as provenance. **No value exported from the game is in
> this document**: where a generator needs a table of game identifiers chosen by an author (the Reward Cache
> selection, the source token list), this spec gives its format and says who supplies it (§5.4, §6, §12).
>
> Written against dainty-restricted `e0e238b4434316bb210677d88bc59e3448688794` (worktree
> `splashloader-integration-90130d`; the unit's files last changed in `9ad97c6`) and dainty-lass
> `f9cb7b4b26309c7cd2d9ca9ea03bc93a65417978` (branch `feat/isles-auth-clean-room`).

---

## 1. Provenance and licensing classification

### 1.1 Evidence used

| Source | What it says |
| --- | --- |
| `modules/rustic-isles/NOTICE.md` | The whole module is a modified Mystic Paradox, AGPL-3.0-only plus `ADDITIONAL_TERMS.md` |
| File headers (`//!`) | `areas/game_data.rs` ports `routes/system.ts` and `routes/tuning.ts`; `tuning.rs` `routes/tuning.ts`; `gauntlet.rs` `gauntletPayloads.ts`; `progression_config.rs` `routes/system.ts`'s `WidenHuntPassWindows`; `patchnotes.rs` `routes/system.ts`; `generators.rs` eight `ParadoxBackend/scripts` generators (RB11) |
| `git log` | One author (the maintainer): `948f830`, `c0482b0` (RU5 game data), `8ad8443` (season window, rewrite, payloads), `2824b6d` (RB11 generators, `src_tokens.txt`), `04992f5` (1.12.0 removed), `9ad97c6` (rename). The patch-notes prose was written upstream (Paradox `48e3956`, "port the backend to Dauntless 1.14.7") |
| `docs/inventory/routes-1.14.7.md` (RU3a) | The 29 `game-data` rows, their gates (`HasParadoxBackendAuth` or none) and statuses |
| `docs/ru5-game-data.md`, `docs/GENERATING_GAME_DATA.md` | The route group, the scripts' disposition, the generator inputs and the host's steps |
| `tests/golden/game_data.json` (48 cases, `now` = `1793016000000`) | Read to confirm status, content type and body shape only; not copied (§10.1) |
| `tests/gamedata_generators.rs`, restricted `crates/host/tests/gamedata.rs` | The generators' byte-for-byte check against the Node scripts on synthetic fixtures; the host's step runner |
| dainty-lass `crates/hostkit/src/gamedata.rs`, `hooks.rs`, `paths.rs` (MIT) | The `reforged-host gamedata` orchestration this crate plugs into (§5.10) |

### 1.2 Per file: category, data class, where it goes

Every source file is Category B (a port of Paradox TypeScript) or written inside one, so nothing moves as it is.

| Restricted file | Code | Data it carries | Rewritten as |
| --- | --- | --- | --- |
| `areas/game_data.rs` | B (`system.ts`, `tuning.ts` handlers) | Log lines; the gate per route | §3, §8.3 `register` |
| `game_data/tuning.rs` | B | Constant wire answers (bounty, token and config identifiers the client resolves): **functional contract** | §3.4 |
| `game_data/gauntlet.rs` | B | The Gauntlet season id and answer shapes: functional contract | §3.5 |
| `game_data/patchnotes.rs` | B | The patch-notes structure (contract) and upstream prose (**expressive text**, §12 item 1) | §3.6 |
| `game_data/progression_config.rs` | B | Four window constants: functional contract | §3.7 |
| `game_data/generators.rs` | B (eight scripts) | Output shapes, rules, client tab tags and price rules (contract); the Reward Cache selection of 178 catalog ids with prices and overrides and two family orders (**authored data over game identifiers**, §5.4) | §4, §5 |
| `game_data/src_tokens.txt` | — | 2946 tokens mechanically extracted from `ParadoxBackend/src` (§6): **derived from AGPL source**, its effective part game identifiers | §6; not carried |
| restricted `crates/host/src/lib.rs` (`execute_rust_generator`) | A (RB11 glue) | Which export and capture paths each step reads | §5.10 |
| `game_data/{loader,season_window}.rs`, `game_data.rs` | — | Core's (spec-rustic-isles-core §8) | Not here |
| `game_data/golden.rs`, `tests/golden/*` | A | Recorded Node answers | Stay in restricted (§10.1) |
| `areas/inventory/progression_config.rs` | B | Inventory's reader of the same file | Unit 7 (economy), not here |

### 1.3 What stays in dainty-restricted

- The goldens (`tests/golden/game_data.json`, `progression_config.synthetic.json`, `capture.mjs`) and the Node
  byte-for-byte generator test, which runs restricted's reference scripts and is pointed at this crate (§10.3).
- The Reward Cache selection and the storage-policy seed list as **inputs** (§5.4, §6), until a decision says
  otherwise (§12 items 2, 3).
- The Director hunt-table importer (`isles-director::hunt_tables`, unit 4) and hostkit's progression-source builder
  (dainty-lass `hostkit::progression`, already MIT).
- The version tables and hooks (core §5.2) this crate reads; the version crate (unit 12) supplies them.

---

## 2. Conventions

As core §2 and auth §2.1-§2.2: JSON as `JSON.stringify` writes it (`service_kit::json::Json`, `content-type:
application/json; charset=utf-8`, insertion order: `serde_json` with `preserve_order` in this crate's manifest, core
§17 item 8); "empty" is a status with no body and no content type; times come from core's replay-aware clock
(`rustic_isles::replay::now_ms`); `Date.parse` is `service_kit::js::date_parse`; JS trim is `service_kit::js::trim`;
`String(x)` of a JSON value is `service_kit::js::string`; "truthy" is `service_kit::js::truthy`; JSON parsing as
`JSON.parse` is `service_kit::jsjson::parse`, which reads `1.0` as `1`.

- **`raw(NAME)`** is core §6.1's `Settings::raw`: the value as set, empty included, untrimmed.
- **The envelope** is `{"code":null,"message":"OK","payload":<payload>}`, in that order.
- **Text answer** is the status with `content-type: text/plain; charset=utf-8` and the body text.
- **Game auth** is `rustic_isles_auth::Auth::game_auth` (auth §4.1) on the call's request, through
  `Shared::auth()`: a refusal is answered as auth made it (empty 401 or 403, or a 500 page), before anything else.

### 2.1 The pretty JSON writer (`JSON.stringify(value, null, n)`)

Every generator writes files with one writer whose bytes match V8's:

- Objects list **integer-like keys first** (a key that is the canonical decimal text of an integer below
  4294967295, ascending numerically), then the others in insertion order. This applies at every depth, so a
  sorted map whose keys include `"10"` and `"9"` writes `"9"` then `"10"` first.
- Numbers as JavaScript prints them (`service_kit::jsjson::number`: `1.0` is `1`, `1e21` is `1e+21`, `0.1` stays).
- Strings: `"` → `\"`, `\` → `\\`, U+0008 `\b`, U+000C `\f`, U+000A `\n`, U+000D `\r`, U+0009 `\t`, any other
  control below U+0020 as `\u00xx` (lower-case hex); everything else, non-ASCII included, as itself.
- Empty array `[]`, empty object `{}` at any indent.
- Indent 0: compact (`,` and `:` with no spaces). Indent n > 0: each member on its own line indented
  `(depth + 1) × n` spaces, `": "` between key and value, `,` + newline between members, the closing bracket on its
  own line at `depth × n`. No trailing newline unless a generator adds one (§5 says which).

`stringify_pretty(value, n)` is public (§8.2); a value that round-trips through `JSON.parse` comes back equal.

---

## 3. Routes

### 3.1 The 29 rows

All are the backend's (`Service::Backend`), registered at exactly these path strings (core Appendix A rows B26-B34,
B36-B37, B40-B42, B45-B51, B62-B63, B82-B87). **Gate**: "game" is game auth first (§2); "open" is none. Core decides
matching, order, the Gauntlet-origin condition (core §4.2 `When::Gauntlet`) and the 404 fallthrough.

| # | Method | Path | Gate | Answer |
| --- | --- | --- | --- | --- |
| B26 | POST | `/heartbeat` | open | text `20000` |
| B27 | POST | `/event` | open | `{}` |
| B28 | GET | `/crash/CrashReporter/Ping` | open | text `OK` |
| B29 | POST | `/crash/CrashReporter/CheckReport` | open | `{}` |
| B30 | GET | `/survey/config` | game | the version hook (§3.2) |
| B31 | POST | `/account/migrate` | game | `{"migration_failed":false,"migration_finished":true}` |
| B32 | POST | `/profile/update` | game | empty 200 |
| B33 | POST | `/trials/leaderboards/all` | game, then body | §3.3 |
| B34 | POST | `/trials/leaderboards/solo/individual` | game | envelope of `{}` |
| B36 | POST | `/motd/` | game | empty 204 |
| B37 | GET | `/motd/trigger` | game | empty 404 (a 200 opened an empty popup in the client, 2026-10-15) |
| B40 | GET | `/playertreatments` | game | §3.2 |
| B41 | GET | `/playertreatments/` | game | the same handler (B40 always takes it; core §4.3) |
| B42 | GET | `/playertreatments/:userId` | game | the same |
| B45 | GET | `/eventstats/` | game | `{"stats":[]}` |
| B46 | GET | `/progression/config` | game | §3.7 |
| B47 | GET | `/config` | game | Gauntlet config (§3.5) |
| B48 | GET | `/^\/progression\/([^/]*)\/([^/]+)$/i` | game | Gauntlet progression (§3.5) |
| B49 | GET | `/^\/rewards\/personal\/([^/]*)\/([^/]+)$/i` | game | envelope of `{}` |
| B50 | GET | `/^\/rewards\/guild\/([^/]*)\/([^/]+)$/i` | game | envelope of `0` |
| B51 | GET | `/^\/leaderboard\/get_leaderboard\//i` | game | envelope of `[]` |
| B62 | GET | `/all/` | game | envelope of `{"messages":[]}` (the mailbox) |
| B63 | GET | `/patchnotes/:language/:gameversion` | open | §3.6 (the login screen asks before there is a token) |
| B82 | GET | `/game_tuning/seasonal_event_schedule` | open | §3.4 |
| B83 | GET | `/game_tuning/huntpass_xp_config` | open | §3.4 |
| B84 | GET | `/game_tuning/island_content_config` | open | §3.4 |
| B85 | GET | `/game_tuning/bounty_game_data` | open | §3.4 |
| B86 | GET | `/game_tuning/bounty_game_data_daily` | open | §3.4 |
| B87 | GET | `/game_tuning/bounty_game_data_weekly` | open | §3.4 |

Every JSON answer is 200 unless the row says otherwise. Request bodies are never read except by B33.

### 3.2 Answers that read the version or a setting

- **B30**: the hook `survey_config()` gives a status and an optional body (core §5.2 `StubReply`). The body is
  written as JSON with that status; no body is the empty status; a status that is not a valid HTTP status is 500.
  1.14.7's hook answers 200 with the envelope of `{"Triggers":[]}`.
- **B40-B42**: `{"treatments":["CohortTreatment.Dojo.B"]}` when `raw(COHORT_TREATMENT_LEGACY)` is unset or exactly
  `1`; any other value, empty included, `{"treatments":[]}`.

### 3.3 B33 `POST /trials/leaderboards/all`

Game auth, then the body through core's reader (`read_body_parts_limit` with the 100 KiB default, core §3.9). A body
error answers the core error page (400, 413, 415). The body is used only when it is a JSON object (or a form, which
core's reader gives as an object of strings); an array, scalar or absent body counts as `{}`. Each field is
`body.field ?? default` (absent or `null` take the default; anything else is echoed as given):
`difficulty` 1, `page` 0, `page_size` 100, `trial_id` `""`. The answer is the envelope of:

`{"difficulty":D,"guild":{},"page":P,"page_size":S,"trial_id":T,"world":{"group":{"difficulty":D,"entries":[]},"solo":{"all":{"difficulty":D,"entries":[]}}}}`

| Body | `payload` fields |
| --- | --- |
| none, `{}`, `[{"difficulty":9}]` | `difficulty` 1, `page` 0, `page_size` 100, `trial_id` `""` |
| `{"difficulty":3.0,"page":2,"page_size":"50"}` | `3`, `2`, `"50"` (types kept; `3.0` reads as `3`) |
| `{"difficulty":null,"page":{"a":[1,2e0]}}` | `1`; `page` `{"a":[1,2]}` |
| form `difficulty=2&trial_id=a%20b` | `"2"`, `"a b"` (form values are strings) |

Logged (info): `Trials leaderboard all (stubbed empty) trial_id=<String(T) as a debug string> difficulty=<String(D)>`.

### 3.4 `/game_tuning/*` (constant answers; key order is the wire order)

Every answer is the envelope of the payload shown.

- **B82** `{"ScheduledItems":[]}`
- **B83** `{"EventConfigs":[],"GlobalConfig":{"DifficultyBias":1,"GlobalMultiplier":1,"MaxXPAwarded":200}}`
- **B84** `{"DisallowedIslandContentAssets":[]}`
- **B85**, with `G` the item grant list below:
  `{"bounty_data":[{"bounty_id":"Bounty_Bronze_KillWithFriends","enabled":false},{"bounty_id":"Bounty_Silver_KillWithFriends","enabled":false},{"bounty_id":"Bounty_Gold_KillWithFriends","enabled":false}],"bounty_token_grant_hour":0,"bounty_token_id":"TOKEN_BOUNTY_DRAFT","bronze_count":9,"silver_count":3,"gold_count":1,"history_length":10,"item_grant_data":G,"max_slots":4,"new_season_reset_bounties":false,"num_draft_options":3,"num_spicy_options":1,"num_tokens_hp_start":4,"num_tokens_per_day":0,"premium_bounty_token_id":"TOKEN_BOUNTY_DRAFT_PREMIUM","token_rollover_warning_days":1000}`
- **B86** (an empty grant list on purpose: 1.14.7 measured negative with one, 2026-10-04):
  `{"automatic_claim":true,"automatic_draft":true,"bounty_data":[{"bounty_id":"Challenge_Daily_Bronze_GetHuntPassXP","enabled":false}],"bounty_token_grant_hour":0,"bounty_token_id":"TOKEN_DAILY_CHALLENGE_DRAFT","bronze_count":1,"delete_claimed_bounties":false,"gold_count":0,"history_length":10,"item_grant_data":[],"max_slots":1,"new_season_reset_bounties":true,"num_draft_options":3,"num_spicy_options":1,"num_tokens_hp_start":1,"num_tokens_per_day":0,"premium_bounty_token_id":"TOKEN_DAILY_CHALLENGE_DRAFT_PREMIUM","silver_count":0,"token_rollover_warning_days":1000}`
- **B87** (its key order differs from B86's; keep it):
  `{"automatic_claim":true,"automatic_draft":true,"bounty_data":[{"bounty_id":"26_11_7_Challenge_Season_BreakParts_Firesacs_Aether_Sally_Terra","enabled":false}],"item_grant_data":[],"bounty_token_id":"TOKEN_WEEKLY_CHALLENGE_DRAFT","bounty_token_grant_hour":0,"bronze_count":0,"silver_count":0,"gold_count":0,"history_length":8,"max_slots":4,"num_draft_options":3,"num_spicy_options":1,"num_tokens_hp_start":4,"num_tokens_per_day":0,"new_season_reset_bounties":true,"delete_claimed_bounties":false,"premium_bounty_token_id":"TOKEN_WEEKLY_CHALLENGE_DRAFT_PREMIUM","token_rollover_warning_days":1000}`

**The item grant list `G`** is built from the version's `bounty_item_grant_data` table (core §5.2), one object per
window in table order:
`{"on_claim_grant_item_amount":<amount>,"on_claim_grant_item_end_date":<end>,"on_claim_grant_item_event_id":"","on_claim_grant_item_id":<item id>,"on_claim_grant_item_start_date":<start>}`.
The windows must cover now, or the client's bounty component finds no season and the hub disconnects (1.14.7,
2026-10-03); keeping them current is the version crate's job.

### 3.5 The Gauntlet stubs (answer only on a Gauntlet origin; core §4.2)

The season id is `ESC_SEASON_1` (the client then asks progression and rewards for it).

- **B47** `/config`: when `raw(GAUNTLET_CONFIG_PROBE)` is unset or exactly `1`, the envelope of
  `{"gauntlet_id":"ESC_SEASON_1","start_at":S,"end_at":E}` with `S`, `E` the Hunt Pass season window containing now,
  as `IsoUtc` texts (core §8.4: `2026-07-30T17:00:00+00:00`); otherwise the envelope of `{}`. Logged (info) with the
  origin host.
- **B48**: params `0` (the season, may be empty) and `1`. When `0` lower-cased (Unicode) is `level_access`, the
  envelope of `0` (the level-access request is an integer, 0 for no access); otherwise the envelope of
  `{"personal_progression":0,"guild_progression":0}`.
- **B49** envelope of `{}`; **B50** envelope of `0`; **B51** envelope of `[]` (an array, not an object).

| Request (origin `gauntlet-prod.steelyard.ca`) | Payload |
| --- | --- |
| `GET /progression/ESC_SEASON_1/user-1` | `{"personal_progression":0,"guild_progression":0}` |
| `GET /progression//INVALID` | the same (empty season) |
| `GET /progression/level_access/user-1?account_id=x`, `GET /PROGRESSION/LEVEL_ACCESS/user-1` | `0` |
| `GET /config` at 2026-10-16T08:30:00Z | `{"gauntlet_id":"ESC_SEASON_1","start_at":"2026-07-30T17:00:00+00:00","end_at":"2026-10-22T17:00:00+00:00"}` |
| `GET /config` with `GAUNTLET_CONFIG_PROBE=0` | `{}` |
| `GET /config` from `mailbox-prod.steelyard.ca` | core's empty 404 (the row passes on) |

### 3.6 B63 Patch notes

The params are already percent-decoded (`/patchnotes/pt%2DBR/1.0` has language `pt-BR`); `gameversion` is only
logged. The answer depends on the version's `patchnotes_release_version` table value `R` (core §5.2):

- **`R` empty** (1.14.7: its runtime collapses the "Update <version>" line): the envelope of the welcome notes, in
  this key order: `date` `"2026-10-08T00:00:00.000+00:00"`, `description` (one welcoming sentence pair), `language`
  (the param), `notes`, `permalink` `"/patch-notes/rusticisles/"`, `release_version` `R` (so `""`), `title`
  `"Rustic Isles"`. `notes` is two objects, each `{"sections":[...],"title":..,"type":..}`:
  1. `title` `"Welcome to Rustic Isles"`, `type` `"new_to_dauntless"`, two sections: `{"description":..,"title":"The hunt continues","type":"featured"}` and `{"changes":[{"list":[three strings]}],"title":"Getting started","type":"default"}`;
  2. `title` `"Community"`, `type` `"quality_of_life"`, one section `{"changes":[{"list":[three strings]}],"title":"Help and feedback","type":"default"}`.

  Strings may use `**bold**`, which the client renders. The two descriptions and six list items are prose (§12
  item 1): what they must say is the project's welcome (a community project that keeps Dauntless alive and keeps the
  player's Slayer, gear and loadouts), getting started (press PLAY to reach Ramsgate, try weapons in the Training
  Grounds, form a party and hunt a first Behemoth) and feedback (expect bugs, tell the team what happened and when,
  thanks).
- **`R` not empty** (no shipped version; kept so the table means what core says): the envelope of
  `{"date":"2024-12-19T18:00:00.000+00:00","description":"Welcome to Rustic Isles!","language":<language>,"notes":[],"permalink":"/patch-notes/rusticisles/","release_version":R,"title":"Rustic Isles"}`.

### 3.7 B46 `GET /progression/config`: the served file

1. The backend file `progression_config.json` through core's **cached required read** (core §8.2: the first good
   read is kept for the life of the process). Any error: logged (error) `GET /progression/config: {error}` and the
   500 error page (`service_kit::body::error_page`). Node refuses to start without the file instead (§12 item 5).
2. The version hook `progression_config()` (core §5.2): `AsIs` answers the file unchanged; `RollingHuntpassWindows`
   (1.14.7) answers the file after the **window rewrite** below, at the replay-aware now. The cached document is
   never modified: the rewrite works on a copy, so each request rewrites from the file as read.

**The window rewrite** (`WidenHuntPassWindows`, 2026-10-03 and 2026-10-08: the client picks the active pass by date):

- Off when `raw(GAMESERVER_WIDEN_HUNTPASS_WINDOWS)` is exactly `0` (`off` is not off). Off, or `payload.paths` not an
  array (no `payload`, `payload` not an object, `paths` an object): the document unchanged, no log.
- **Served pass id** `A`: `raw(GAMESERVER_HUNTPASS_ID)` when set (an empty value stays empty), else the table's
  `active_huntpass_id`, JS-trimmed.
- **A pass path** is an element that is an object whose `progression_id` is a string starting, ASCII
  case-insensitively, with `eventpass_`, `d24_` or `season`. Non-objects and other paths are never touched.
- **Constants**: rolling start/end = the season window containing now (`IsoUtc`, core §8.4); ended end
  `2024-12-03T00:00:00+00:00` (the live service's last pass cutoff, `Date.parse` = 1733184000000); ended start
  `2019-01-01T00:00:00+00:00`; future window `2099-01-02T00:00:00+00:00` to `2099-12-31T00:00:00+00:00`.
- **Each pass path**, in order:
  - `progression_id` = `A`: `start_date` and `end_date` set to the rolling window ("opened"; every match is, not only
    the first);
  - otherwise, with `end` = `Date.parse(String(end_date))` (absent is `"undefined"`, so NaN): when `end` is NaN or
    after now, `end_date` is set to the ended end, then, when `Date.parse(String(start_date))` is NaN or at or after
    the ended end's instant, `start_date` is set to the ended start ("ended"). A pass that ended stays as it is.
  - "Set" replaces the value in place when the key exists, else appends the key; so a path with neither key gains
    `end_date` then `start_date` when ended, but `start_date` then `end_date` when opened.
- Logged (info) `Hunt pass windows: '{A}' {start}..{end} ({opened} path), {ended} other pass windows clamped to have
  ended`; with none opened, a warning `Hunt pass windows: the active pass '{A}' is not in the progression config`.
- **Recent seasons**: the list is `raw(GAMESERVER_RECENT_SEASONS)` when set (empty: no seasons), else the table's
  `recent_huntpass_seasons` joined with `,`; split on `,`, each JS-trimmed, empties dropped. The **template** is the
  first path (after the step above) whose `progression_id` is `A`, else the first whose `progression_id` is a string
  starting `season` ASCII case-insensitively; none: warning `Progression config: no season template found; recent
  seasons not added` and stop. The **existing** ids are the string `progression_id`s of the paths now. For each listed
  season not among the existing ids, a copy of the template with `progression_id` set to the season and its window
  set to the future window (set as above, in place) is appended to `paths`. The existing set is not updated, so a
  season listed twice is added twice (as Node). Added any: info `Progression config: added {n} recent season paths
  with future windows ({ids joined ", "})`.

Worked example (now `2026-10-13T18:00:00.000Z`, `A` = `season43`, recent `season43,season44,season45`), paths
`{"progression_id":id,"start_date":s,"end_date":e,"tracks":[1,2]}`:

| Path in | Out |
| --- | --- |
| `ExperienceTrack_Slayer` 2019 to 2099 | unchanged |
| `season19` `2020-08-23T00:00:00+00:00` to 2099 | end `2024-12-03T00:00:00+00:00`, start kept |
| `season10a` 2022-01-01 to 2022-03-01 | unchanged (ended) |
| `eventpass_x` 2030 to 2031 | start `2019-01-01T00:00:00+00:00`, end `2024-12-03T00:00:00+00:00` |
| `D24_old` `garbage` to `garbage` | start 2019, end 2024-12-03 (NaN clamps) |
| `season43` 2020 to 2021 | `2026-07-30T17:00:00+00:00` to `2026-10-22T17:00:00+00:00` |
| `null`, `5` | unchanged |
| (appended) | `{"progression_id":"season44","start_date":"2099-01-02T00:00:00+00:00","end_date":"2099-12-31T00:00:00+00:00","tracks":[1,2]}`, then `season45` |

With `GAMESERVER_HUNTPASS_ID= season99 `: nothing opens (warning), `season43` becomes an ordinary pass that ended
(unchanged), `season19` (the first `season` path) is the template. With `GAMESERVER_RECENT_SEASONS=" season60 ,,
season60,season43"`: `season60` is added twice. A config whose only path is `ExperienceTrack_Slayer`: unchanged.

---

## 4. Formats

### 4.1 Inputs (exported from the host's own install or supplied by the host; never committed)

Layout as hostkit's `Paths` (dainty-lass `crates/hostkit/src/paths.rs`): `raw_game_data` (the supplied sources),
`backend_game_data` (outputs), `exports` (`Items_Analysis`), `store_captures` (`DauntlessEndpointDocumentation`).

| Input | Where | Format and fields read |
| --- | --- | --- |
| Progression source | `raw/progression_config.source.json` | One object; `code`, `message`, `payload` (an object with `paths` an array of track objects). Supplied by the host or built by hostkit's progression builder from the export |
| Lady Luck source | `raw/ladyluck_store.source.json` | An array of entries: `id` (string), `displayName`, `displayDescription`, `displayPriority`, `prices` (array), `maxAllowed`, `tags`, `items` (array), `duplicateInstancedItems` |
| Catalog | `exports/catalog_1_14_7.jsonl` (CatalogExporter `EXPORT_CATALOG`) | JSON lines, one item per line: `itemId`, `displayName`, `description`, `tags` (strings), `isStackable` (bool), `maxQuantity` (number), `customData` (a **string** of JSON whose `ItemData` is itself a **string** of JSON with `RarityLevel` (number), `Icon`, `LargeIcon`, `CustomIcon` (asset paths)) |
| Journey nodes | `exports/slayers_path_1_14_7/player_journey_nodes.jsonl` (`EXPORT_SLAYERS_PATH`) | JSON lines: `nodeId`, `autoUnlockIfParentUnlocked` (bool), `childNodes` (strings, `None` for an empty slot), `currencyCosts` (`{currency, amount}`, currency `None` for empty), `rewards` (`{itemId}`, `None` for empty) |
| Store images | `exports/store_item_images_1_14_7.jsonl` (`EXPORT_STORE_RUNTIME`) | JSON lines: `rowName`, `standardImage`, `featureImage` (asset paths, or `None`) |
| Store captures | `store_captures/Store/Product/Skus/` | Recorded live store answers, each an array of SKUs (`id`, `displayName`, `displayDescription`, `displayPriority`, `prices` `[{currencyId, price}]`, `maxAllowed`, `tags`, `items` `[{catalogId, quantity}]`, `duplicateInstancedItems`, `entitlements` `[{name, duration}]`, `skuProgression`, `platformOfferId`, `loadoutSlots`): `ladyluckstore.json`, `awakening_webstore.json`, `dyes.json`, `hp_level_skip_rank.json`, `huntpass_store.json`. Fetched by hostkit at the commit and hashes `store-captures.json` pins |

**Reading JSON lines** (catalog, store images): lines split on `\n` (a trailing `\r` dropped), blank lines (after
trim) skipped, each parsed as `JSON.parse` (no BOM stripping: a BOM fails the first line); a parse failure is the
error `invalid JSON in {path}: {message}`. Catalog rows keep only those whose `itemId` is a non-empty string; a later
row with the same id replaces the earlier. Store-image rows keep those whose `rowName` is a string not starting
`NewRow`. A missing catalog is `Catalog dump not found: {path}`. Journey nodes are read per generator (§5.2, §5.3).

**The item data** of a catalog row: `customData` parsed, then its `ItemData` parsed; any step missing or failing gives
rarity 0 and empty icons. Rarity is `RarityLevel` when a finite number, truncated to an integer, else 0. **The
catalog image** of a row is the first non-empty of `CustomIcon`, `LargeIcon`, `Icon`.

**A texture reference** is `Engine.Texture2D'<path>'`; where §5 says "checked", the path must be `/Game/` followed by
one or more of ASCII letters, digits, `_`, `.`, `/`, `-`, else the error `Invalid cooked texture path: {path}`.

### 4.2 Outputs (the backend files of core §8.1)

| File | Top level | Writer | Trailing newline |
| --- | --- | --- | --- |
| `progression_config.json` | `{code, message, payload{..., paths}}` | indent 2 | no |
| `slayers_path.json` | the existing document, `payload.nodes` replaced | indent 2 | no |
| `slayers_path_definitions.json` | object: node id → definition (no wrapper) | **indent 1** | no |
| `reward_cache_store.json` | array of SKUs | indent 2 | yes |
| `ladyluck_store.json` | array of SKUs | indent 2 | no from §5.5, yes from §5.6 |
| `platinum_store.json` | array of SKUs | indent 2 | yes |
| `inventory_storage_policy.json` | `{schema, targetVersion, catalogSha256, generatedAt, referencedCatalogIds, sourceScope, entries}` | indent 2 | yes |

**A store SKU** (the shape bounties-store reads) is, in this key order: `id`, `displayName`, `displayDescription`,
`displayPriority`, `prices` (`[{"currencyId":..,"price":..}]`), `maxAllowed`, `images` (`{}` or
`{"standard":<texture reference>}`, absent in a Lady Luck SKU without one), `tags`, `items`
(`[{"catalogId":..,"quantity":..,"instanced":bool}]`), `duplicateInstancedItems` (catalog ids), then optional
`entitlements` (`[{"name":..,"duration":<integer>}]`) and `skuProgression`.

**Backups**: where §5 says "backed up", an existing target is first copied beside itself as
`<file name>.<stamp>.bak`, the stamp being the current UTC `toISOString()` with every `:` and `.` replaced by `-`
(`progression_config.json.2026-11-05T10-20-30-123Z.bak`); a copy failure is the error
`failed to back up {target} to {backup}: {message}`.

---

## 5. Generators

Each is a function of paths (and, for two, an input table) returning a **report** (lines joined with `\n`) or an
**error text** (`Result<String, String>`). An error writes nothing unless §5 says so. Reports and errors are host
terminal text, English; hostkit shows the last report line on success and the first error line on failure
(§5.10). Where restricted's Rust and the Node script disagree, the Node script in restricted's reference is the
oracle (restricted's byte-for-byte test checks it; §10.3); the deviations seen are listed in §12.

### 5.1 Progression config (`generate_progression_config.cjs`; `raw_dir`, `out_dir`, `apply`)

- No source: `ERROR: source not found at {path}` + newline + `Provide the progression payload from your own
  installation (see docs/GENERATING_GAME_DATA.md).` Unreadable or not JSON (one leading BOM dropped):
  `ERROR: source is not valid JSON: SyntaxError: {message}` (an unreadable file also says "not valid JSON").
  `payload.paths` not an array: `ERROR: source has no payload.paths[] array — refusing to write a shape the backend
  cannot read.`
- Output: `code` (the source's unless absent or `null`: then `null`), `message` (the source's when a string, else
  `"OK"`), `payload` (the source's payload with every key kept, `paths` in its place). Other top-level keys are
  dropped.
- Report: `source paths      : {n}`, `free_rewards tiers : {sum of each path's free_rewards array length}`,
  `mode              : APPLY` or `mode              : DRY RUN (pass --apply to write)`; dry run adds a blank line and
  `Dry run complete. Nothing written.`; apply creates `out_dir`, backs up, writes, and adds
  `Wrote progression_config.json ({n} paths)` after a blank line.

### 5.2 Slayer's Path graph (`generate_slayers_path.cjs`; export, target, `apply`)

- Export missing: `ERROR: export not found at {path} — run CatalogExporter with EXPORT_SLAYERS_PATH=1 first`; target
  missing: `ERROR: target not found at {path}` (hostkit seeds the example first). The export is read with one BOM
  dropped and the **whole text trimmed**, split on `\n`, blank lines skipped; a bad line is `ERROR: malformed JSON on
  line {n} of the export`, `n` counted from 1 in the trimmed text. The target must be JSON
  (`ERROR: invalid JSON in {path}: {message}`) with `payload.nodes` an object (`ERROR: existing slayers_path.json has
  no payload.nodes — refusing to guess at the shape`).
- Next nodes: for each row with a non-empty string `nodeId`, in export order,
  `"<id>":{"node_id":<id>,"node_status":0,"objectives":[]}` (a repeated id keeps its first position).
- Report: `existing baseline : {n} nodes`, `live 1.12 table   : {n} nodes`, `  added   : {n}`,
  `  dropped : {n}  (1.4.4 leftovers / renamed)`, then `critical unlock gates:` with one line per gate
  `  {gate padded to 30} before={present|MISSING padded to 8} after={present|MISSING}` for, in order, `Slayer_09`,
  `Slayer_10`, `Slayer_11`, `WeaponTypeGA_Spc_00`, `WeaponTypeEB_Spc_00`, `WeaponTypeCB_Spc_00`,
  `AirshipUpgrades_Islands_S`, `AirshipUpgrades_Islands_T`, `AirshipUpgrades_Islands_U`, `Escalation.Hard.Unlock`,
  `Lantern.Unlock` (node ids the client gates on). With dropped ids, two lines explaining they become orphans that
  read as locked, then up to 10 of them sorted and `  ...and {n} more`.
- Dry run: `Dry run. Nothing written. Re-run with --apply to commit.` Apply: the existing document with
  `payload.nodes` replaced in place, backed up, written; `Backed up: {backup name}`, `Wrote    : slayers_path.json
  ({n} nodes)` and two lines asking the host to restart and check that Trials, Grim Onslaught crafting and Islands
  S/T/U unlock.

### 5.3 Slayer's Path definitions (`generate_slayers_path_definitions.cjs`; export, target, source version, `apply`)

- Export missing: as §5.2. Read with one BOM dropped, **not trimmed**, split on `\n`, blank lines skipped, the same
  line error (numbered in the untrimmed text).
- For each row with a non-empty string `nodeId`: `childNodes` = its string items that are neither empty nor `None`;
  `currencyCosts` = its object items whose `currency` is a string other than `None`, each written
  `{"amount":..,"currency":..}` (`amount` only when present); `rewards` = the `itemId` strings of its items, neither
  empty nor `None`. A node with all three empty is left out. The definition is, in order,
  `"autoUnlockIfParentUnlocked":true` (only when the row's value is exactly `true`), then each non-empty list. Later
  rows with the same id replace earlier ones. The map is sorted by node id (byte order), then written by §2.1 (so
  integer-like ids would come first).
- Comparison with an existing target (missing: empty; unparseable or not an object: empty map; its text with CRLF
  turned into LF): report `source  : slayers_path_{source version} ({rows} nodes)`,
  `existing: {n} definitions; next: {n}`, `  added   : {n}` + `  {ids, ", "}` when any (next's order), likewise
  `  removed : ` (sorted) and `  changed : ` (ids whose compact JSON differs), and `  identical bytes: {true|false}`.
- Dry run: `Dry run. Nothing written. Re-run with --apply to write.` Apply: creates the parent folder, writes (no
  backup), `Wrote slayers_path_definitions.json. Restart the metagame to load it.`

### 5.4 Reward Cache store (`generate_reward_cache_store.ts`; catalog, output, **plan**)

The Reward Cache was a seasonal store the client no longer receives; the server rebuilds it from a curated
**plan** of catalog items. The plan is authored data over game identifiers (§1.2) and is an **input**, supplied by
the version crate (§8.2 `RewardCachePlan`; §12 item 2). Its parts:

| Part | Meaning |
| --- | --- |
| selections, in authored order | catalog id; category (`armour`, `weapon`, `style`, `event`, `limited`, `general`); price; featured (bool); may be rarity 3 (bool); optional display name and description overrides (an override entry is featured) |
| armour families | ordered catalog-id substrings ranking armour sets |
| banner families | ordered substrings ranking banner families |
| expected counts | per category, and the featured count; restricted's plan has 48, 34, 23, 55, 10, 8 (178) and 7 |

Category tags: armour `tab_season_armour`, weapon `tab_season_weapon`, style `tab_season_style`, event
`season_store_event`, limited `tab_season_limited`, general `season_store_general`.

**Order** (a stable sort of the selections): featured first; then category in the order above; within armour, the
index of the first armour family contained in the id (none: after all), then ids ending `_ULTRA` first, then slot by
the catalog row's tags (`Helm`, `Chest`, `Arms`, `Legs`, else last; exact tag strings); within weapon, ids starting
`WP_GA_` first and among those the higher rarity first; within event, ids starting `BNC_` first and among those the
banner family index, then the component (`BNC_STANDARD_` or `BNC_MESH_`, `BNC_FABRIC_`, `BNC_SIGIL_`,
`BNC_ANIMATION_`, other); finally the authored order.

**Each selection** in that order, position `i` from 0, must pass, else the error named (first failure stops):

| Rule | Error |
| --- | --- |
| id not already used | `Duplicate catalog selection: {id}` |
| price one of 700, 800, 1000, 2000, 2500, 3000 | `Unsupported price {price} for {id}` |
| id upper-cased contains none of `FOUNDER`, `TRANSLATOR`, `TRIALS`, `MASTERY` | `Exclusive/progression catalog id is not eligible: {id}` |
| id is not `WP_GA_SPOOKY_01` | `Quest/Rumour-only catalog item is not eligible: {id}` |
| in the catalog | `Catalog item not found: {id}` |
| name (override trimmed when non-empty, else the catalog `displayName` trimmed) non-empty | `Missing display name: {id}` |
| without an override, the name lower-cased does not start `~dnt~` or `<missing` and is not `tbd` or `empty slot` | `Unresolved display name for {id}: {name}` |
| no catalog tag (lower-cased) is `deprecated`, `hidden`, `initial`, `masteryreward`, `trialstore`, `wip` | `Forbidden catalog tag '{tag}' on {id}` |
| rarity ≥ 4, or a `glider` tag (lower-cased) with rarity ≥ 3, or "may be rarity 3" with rarity exactly 3 | `Selected item must be rarity 4+, a resolved rarity-3 glider, or an explicit rarity-3 premium selection: {id} has rarity {r}` |
| a catalog image | `Missing catalog image path: {id}` |
| the SKU id unique | `Duplicate generated SKU id: {sku}` |
| the image path checked (§4.1) | `Invalid cooked texture path: {path}` |

**The SKU**: `id` `season19_rewardcache_{category}_{slug}` (the slug: the catalog id lower-cased, each run of
characters that are not ASCII letters or digits written as one `_`, none leading or trailing); `displayName` the
name; `displayDescription` the description override trimmed when non-empty, else the catalog `description` trimmed
when non-empty, else `A premium Reward Cache cosmetic: {name}.`; `displayPriority` `i + 1`; `prices`
`[{"currencyId":"id_currency_seasonal_coin","price":<price>}]`; `maxAllowed` 1; `images`
`{"standard":<texture reference>}`; `tags` `season_store`, the category tag, `season_store_feature` (featured only),
`season19`, `reward_cache_reconstructed`, `reward_cache_rarity_{r}`; `items`
`[{"catalogId":<id>,"quantity":1,"instanced":<isStackable is not true>}]`; `duplicateInstancedItems` `[<id>]`.

After all: each category's count must equal the plan's (`{category} count {n} != expected {m}`) and the featured
count too (`Featured count {n} != expected {m}`). Then the parent folder is created and the file written. Report:
`Wrote {path}`, `SKUs: {n}; featured: {f}`, `Categories: armour={n}, weapon={n}, style={n}, event={n}, limited={n},
general={n}`, `Prices: {price}={count}, ...` (ascending price).

### 5.5 Lady Luck store from a supplied response (`generate_ladyluck_store.cjs`; `raw_dir`, `out_dir`, `apply`)

- No source: `ERROR: source not found at {path}` + newline + `Provide the store payload from your own installation
  (see docs/GENERATING_GAME_DATA.md).`; JSON errors as §5.1; not an array: `ERROR: source must be a JSON array of
  store entries.`
- Each entry, at index `i`: problems `entry {i}: missing string "id"` (id not a string), `entry {i} ({String(id)}):
  missing items[]`, `entry {i} ({String(id)}): missing prices[]`. Output entry, in order: `id` (as given),
  `displayName` (`?? ""`), `displayDescription` (`?? ""`), `displayPriority` (`?? 0`), `prices` (`?? []`),
  `maxAllowed` (`?? 0`), `tags` (`?? []`), `items` (`?? []`), `duplicateInstancedItems` (`?? []`); every other field
  (`images` included) is dropped.
- Report `store entries : {n}`, `mode          : APPLY|DRY RUN (pass --apply to write)`. Any problem: after a blank
  line `VALIDATION PROBLEMS ({n}):` and up to 15 problems indented two spaces (`  ...and {n} more`), then the error
  `ERROR: refusing to proceed until the source entries are well-formed.`, dry run or not. Dry run:
  `Dry run complete. Nothing written.` Apply: creates `out_dir`, backs up, writes,
  `Wrote ladyluck_store.json ({n} entries)`.

### 5.6 Lady Luck store from the live rotation (`enrich_ladyluck_store.ts`; catalog, live capture, store file)

Used when the host supplies no source (hostkit picks one of §5.5 and §5.6, §5.10).

- Errors: the catalog as §4.1; `Live Lady Luck capture not found: {path}`; `Lady Luck store data not found: {path}`
  (hostkit seeds the example); the live capture must be a non-empty array (`Live Lady Luck capture is not a SKU
  list`; one BOM dropped); the existing store must be an array (`Existing ladyluck_store.json is not a JSON array`).
  Existing SKUs are indexed by string `id` (a later one wins).
- Each live offer (objects only), in order: an offer naming a string `catalogId` absent from the catalog is skipped
  (`{id} ({ids, ", "} not in the catalogue)`). **Keep previous** when an existing SKU with the same id has `items`
  of the same length whose `catalogId` and `quantity` equal the live items' pairwise. The SKU: `id`; `displayName`,
  `displayDescription`, `displayPriority` as live (left out when absent); `prices` each `{currencyId, price}` from
  live (`null` when absent); `maxAllowed` live (`null` when absent); `images` (below); `tags` live (`[]` when not an
  array); `items` the previous SKU's as they are when kept, else each live item as
  `{"catalogId":..,"quantity":..,"instanced":<the catalog row's isStackable is not true>}`;
  `duplicateInstancedItems` the previous SKU's when kept, else live's (`[]` when absent).
- **Images**: when the first item's catalog row has a catalog image, `images` is the previous SKU's images (or `{}`)
  with `standard` set to its checked texture reference (server-authored art, independent of the client's store
  table naming); without one, the previous SKU's images when it had any, else no `images` key.
- Checks over the result: unique ids (`Duplicate Lady Luck SKU id: {id}`), exactly one price (`Expected exactly one
  price for {id}`), currency `id_currency_marks_steel` or `id_currency_marks_gilded` (`Unsupported Lady Luck currency
  for {id}`), a `ladyluckstore` tag (`Missing ladyluckstore tag on {id}`). Then the store file is overwritten (no
  backup) with a trailing newline. Report: `Wrote {path}`, `SKUs: {n} of {live} live; explicit catalog images: {n}`,
  `Steel offers: {n}; Gilded offers: {n}`, `Skipped live SKUs: {list "; " or none}`, `Retired SKUs: {existing ids
  not kept, sorted, ", " or none}`.

### 5.7 Platinum store (`generate_platinum_store.ts`; catalog, store images, capture folder, data folder)

The platinum store is rebuilt from captured offers ("captured" prices) and from the client's store table rows that
decode to a single catalog item ("reconstructed" prices).

**Inputs**: the catalog and store images (§4.1; `Store item images not found: {path}`); `reward_cache_store.json`
and `ladyluck_store.json` in the data folder (both required; their item `catalogId`s are **already sold**); captures
from the capture folder (a missing file is empty; a non-array is empty; bad JSON an error). **Quest-earned** ids: for
each store-image row name matching `^quest_rumour_(.+?)(?:_\d+)?$`, the capture upper-cased.

**Grantable**: in the catalog and no tag (lower-cased) among `deprecated`, `hidden`, `wip`, `initial`, `founder`,
`translator`, `mastery`, `partner`. **Instanced** here: the row's `isStackable` is exactly `false` (§12 item 9).
**Duplicate instanced items** of a list: the ids not starting `QI_`, `TOKEN_` or `CURRENCY_`.
**Platinum price**: the `price` of the first `prices` entry whose `currencyId` is `id_currency_platinum` (`null`
kept when the key is present). **Display priority**: `Number(displayPriority ?? 1000) || 1000` (a numeric string
counts, a fraction is kept, 0 and NaN give 1000). **Duration**: `Number(duration ?? 0) || 0`. A captured item's
`quantity` and an entitlement's `name` are copied as given and left out when absent (`JSON.stringify` drops
`undefined`).

**Category of a catalog id** (first rule that applies) and its tab tag:

| Id | Category | Tag |
| --- | --- | --- |
| `WP_<code>_...` with code (lower-cased) `ac`, `eb`, `dp`, `ih`, `ga`, `ms`, `cb` | `weapon_<code>` | `skin_weapon_<code>` |
| `AR_` | `armour` | `skin_armour` |
| `LT_` | `lantern` | `skin_lantern` |
| `DYE_` | `dye` | `dye_armour` |
| `EM_INTRO` | `arrival` | `social_arrival` |
| `EM_` | `emote` | `social_emote` |
| `EJ_` | `emoji` | `social_emojis` |
| `BNC_FABRIC` | `fabric` | `personality_fabric` |
| `BNC_STANDARD` | `standard` | `personality_standard` |
| `BNC_SIGIL`, `BNC_ANIMATION`, `FL_` | `flaresigil` | `personality_flaresigil` |
| `GD_` | `glider` | `hunting_gliders` |
| `QI_` | `tonics` | `supplies_supplies` |
| anything else (other `WP_` codes included) | none | — |

The tag table, in this order (it also orders the output), is: the seven weapon tags (`ac`, `eb`, `dp`, `ih`, `ga`,
`ms`, `cb`), `skin_armour`, `skin_lantern`, `dye_armour`, `personality_character` (category `slayer`),
`social_emote`, `social_emojis`, `social_arrival`, `personality_fabric`, `personality_standard`,
`personality_flaresigil`, `hunting_gliders`, `supplies_supplies`, `supplies_boost` (category `boosts`, which no id
maps to).

**Reconstructed price** by category and rarity: weapons 3 → 500, 4 → 800, 5 → 1200, else 800; armour 3 → 400, 4 →
600, 5 → 800, else 600; lantern 4 → 600, 5 → 800, else 600; dye, slayer, tonics, boosts 100; emote 3 → 300, 4 → 500,
5 → 700, else 500; emoji 200; arrival 1000; fabric 300; standard 400; flaresigil 250; glider 800; else 100.

**Images** for a row name and catalog ids: the baked row's `standardImage` is a string other than `None` → `{}` (the
client draws its own art); else the first listed id in the catalog with an item-data icon (custom, large, then small)
starting `/Game/` → `{"standard":"Engine.Texture2D'<path>'"}` (not checked, §11); else a baked `featureImage` other
than `None` → `{}`; else none.

Passes, in order; a SKU id or catalog id claimed by an earlier pass is not offered again. Each skip counts under its
reason (the reasons are report text; the counts are not reported today).

1. **Captured offers**: `awakening_webstore.json` then `dyes.json`. Per SKU object: claimed id → silently passed;
   then skip when its id is not a baked row (`tierA: sku id not baked into 1.12 StoreItemsTable`), `platformOfferId`
   truthy (`tierA: real-money platform offer`), `loadoutSlots` truthy (`tierA: loadoutSlots grant unimplemented`), no
   platinum price (`tierA: not platinum-priced`), no items, entitlements or non-null `skuProgression` (`tierA: no
   payload of any kind`); each item's id (string, else empty) not grantable (`tierA: item absent from 1.12 or
   excluded content`), already sold (`tierA: overlaps Lady Luck / Reward Cache`), claimed (`tierA: catalog id already
   sold by another platinum SKU`), quest-earned (`tierA: quest/Rumour-earned`). Tags from the items' categories (a
   sorted set). With no items but entitlements: when every entitlement `name` matches `^(ent_)?(ccfp|cchd|cchs|ccbd)_`
   or starts `character_facepaint_`, add `personality_character`; otherwise the SKU goes to the `huntpass_store`
   section. No tag and no section: `tierA: content type has no 1.12 store tab`. No images: `tierA: no baked or
   catalog artwork`. Then its ids are claimed. SKU: `id`; `displayName` (`?? id`); `displayDescription` (`?? ""`);
   the display priority; `prices` `[{"currencyId":"id_currency_platinum","price":<price>}]`; `maxAllowed` 1 with
   entitlements, else live (`?? null`); `images`; `tags`: `[huntpass_store]` for that section, else `webstore` (and
   `dyes` when an item id starts `DYE_`), then the category tags, then `platinum_store_captured`; `items` each
   `{catalogId, quantity, instanced}`; `duplicateInstancedItems`; `entitlements` (when any) each
   `{"name":..,"duration":..}`; `skuProgression` when truthy.
2. **Hunt Pass rank skips** (`hp_level_skip_rank.json`): not baked → `rank skip: sku id not baked`; needs a
   platinum price and a truthy `skuProgression` (`rank skip: missing price or progression`); claimed → silently
   passed. SKU: `displayName` `?? "Hunt Pass Level Skip"`, `maxAllowed` `?? null`, images with no ids (none → `{}`),
   `tags` `["hp_level_skip_rank","event","platinum_store_captured"]`, `items` `[]`, `duplicateInstancedItems` `[]`,
   `skuProgression`.
3. **Hunt Pass passes** (`huntpass_store.json`): not baked → `huntpass pass: sku id not baked`; no platinum price
   → `huntpass pass: no platinum price (already-owned capture)`; no entitlements → `huntpass pass: no entitlement
   payload`; claimed → passed. SKU: `displayName` `?? id`, `maxAllowed` `?? 1`, images as 2, `tags`
   `["huntpass_store","event_pass","platinum_store_captured"]`, empty `items` and duplicates, `entitlements` as 1.
4. **Reconstructed rows**: every baked row name, sorted by byte order, not claimed. It must start `sku_`, `single_`
   or `contract_` (`tierB: row family is a bundle/quest/pass, not a single decodable offer`); have a standard or
   feature image other than `None` (`tierB: baked row has no artwork`); **decode** to a catalog id: the first of
   those prefixes removed, the rest ASCII upper-cased, then that id if in the catalog, else the first of `AR_`,
   `WP_`, `DYE_`, `EM_`, `TITLE_`, `BNC_`, `GD_`, `LT_`, `AC_`, `QI_`, `EJ_`, `FL_` + it that is (`tierB: row name
   does not decode to a 1.12 catalog id`); not already sold, claimed, ungrantable or quest-earned (`tierB: ...` as
   tier A, `tierB: excluded catalog content`); a resolved name (`displayName` non-empty, not containing `MISSING
   STRING TABLE`, not starting `~DNT~`; `tierB: unresolved display name`); a category with a tag (`tierB: content
   type has no 1.12 store tab`); images (`tierB: no artwork resolved`). SKU: `id` the row name; `displayName` the
   catalog's (`?? id`); `displayDescription` the catalog `description` (`?? ""`); `displayPriority` 1000; the
   reconstructed price; `maxAllowed` `null` for tonics and boosts, else 1; `tags` `webstore` (and `dyes` for a dye),
   the category tag, `platinum_store_reconstructed`, `platinum_store_rarity_{r}`; one item with quantity 1.

**Output order**: sorted by `service_kit::collate::locale_compare` of the key `{section}|{tag}|{price}|{id}`, where
section is `2` for a `hp_level_skip_rank` tag, `3` for `huntpass_store`, else `1`; tag is the first of the tag table
the SKU carries, else `zzz`; price is the first price as an integer (0 when not one) zero-padded to 6. Written to
`platinum_store.json` in the data folder (created) with a trailing newline. Report:
`platinum_store.json: {n} SKUs ({c} captured price, {r} reconstructed price)`.

**Export file choice** (`resolve_platinum_export_file(exports, base)`): `{base}_1_12.jsonl` when it exists or when
`{base}_1_14_7.jsonl` does not, else the latter (§12 item 6).

### 5.8 Inventory storage policy (`generate_inventory_storage_policy.ts`; catalog, token source, game data folder)

Which catalog ids the server may store, and how: every catalog id the backend's code or game data names.

- Catalog missing: `Catalog dump not found: {path}`; read as JSON lines (§4.1); its SHA-256 is of the file's bytes.
- **Referenced ids**: the tokens matching `[A-Z][A-Z0-9_-]{2,}` (leftmost, non-overlapping) that are catalog ids,
  from (a) the token source (§6) and (b) every file under the game data folder, recursively, whose name ends
  (ASCII case-insensitively) `.ts` or `.json`, except `inventory_storage_policy.json` and names ending
  `.example.json` (so `raw/*.source.json` counts; backups end `.bak` and do not). Files are read in sorted path order;
  an unreadable one is an error `failed to read {path}: {message}`.
- **Entries**, by id in byte order: `{"storage":"stacked"|"instanced","maxQuantity":..,"tags":[..]}`: `stacked` when
  `isStackable` is exactly `true`; `maxQuantity` the row's when a finite number (as `JSON.parse` would give it), else
  0; `tags` the row's string tags.
- **Document**, in order: `"schema":1`, `"targetVersion":"1.14.7"`, `"catalogSha256":<hex>`,
  `"generatedAt":"2026-10-08"` (a fixed text, not today), `"referencedCatalogIds":<entry count>`,
  `"sourceScope":["src/**/*.ts","src/**/*.json","game-data/*.json"]` (fixed), `"entries":{...}`. Written to
  `inventory_storage_policy.json` in the folder (created), trailing newline. Report: `Wrote {path}`,
  `Referenced catalog ids: {n}`, `Catalog SHA-256: {hex}`.

### 5.9 One-step summary

| Step (hostkit `GeneratorKind`) | Runs | Inputs (§5.10 paths) |
| --- | --- | --- |
| `ProgressionConfig` | §5.1, apply | `raw`, `backend` |
| `SlayersPath` | §5.2, apply | `exports/slayers_path_1_14_7/player_journey_nodes.jsonl`, `backend/slayers_path.json` |
| `SlayersPathDefinitions` | §5.3, apply, source version `1_14_7` | the same export, `backend/slayers_path_definitions.json` |
| `RewardCache` | §5.4, then §5.8 | `exports/catalog_1_14_7.jsonl`, `backend/reward_cache_store.json`, the plan |
| `LadyLuckGenerate` | §5.5, apply | `raw`, `backend` |
| `LadyLuckEnrich` | §5.6, then §5.8 | the catalog, `store_captures/Store/Product/Skus/ladyluckstore.json`, `backend/ladyluck_store.json` |
| `PlatinumAndStoragePolicy` | §5.7, then §5.8 | catalog and `store_item_images` by §5.7's file choice, `store_captures/Store/Product/Skus`, `backend`; §5.8 always with `exports/catalog_1_14_7.jsonl` |
| `HuntTables` | not this crate (unit 4) | — |

A step that runs two generators reports both texts joined with `\n`; the first error stops it.

### 5.10 The `reforged-host gamedata` entry

The command and its orchestration are dainty-lass hostkit's (MIT, `crates/hostkit/src/gamedata.rs`,
`commands/gamedata.rs`): it builds the progression source from the export, fetches the pinned store captures,
decides each step's skip (missing export or source), seeds examples where a generator needs an existing file, picks
`LadyLuckGenerate` when `raw/ladyluck_store.source.json` exists and `LadyLuckEnrich` otherwise, and calls
`HostHooks::run_generator(paths, step)` for each step. This crate provides what that hook runs:
`generators::run(step, paths, inputs) -> Result<String, String>` (§8.2) for every backend step. The host's hook
(restricted's `crates/host` today) answers hostkit with code 0 and the report's last non-empty trimmed line (first
300 characters) on success, code 1 and the error's first non-empty trimmed line on failure; hostkit prints
`- {title}: ok` or `- {title}: failed: {line}` and runs the other steps.

---

## 6. `src_tokens.txt`

**What it is**: 2946 lines, each a distinct token matching §5.8's pattern, mechanically extracted from
`ParadoxBackend/src/**/*.ts` and `**/*.json` (RB11), then hand-edited by the rename (`9ad97c6`, 74 lines, which broke
its byte order). It lets §5.8 run without the Node source tree: with no source folder, the referenced ids come from
this list instead of a walk.

**What it contains**: mostly code identifiers (environment variable names such as `RUSTIC_ISLES_*` and `MYSTPAX_*`,
constants, regex classes like `A-Z0-9_`, hex fragments) that never match a catalog id, plus a few hundred **catalog
ids the backend's code names** (currencies, tokens, starter items). Only the latter have any effect.

**Classification**: not game data in the sense of an export, and not authored prose: **a derivative of AGPL source**
(its entire content is fragments of Paradox's TypeScript), whose functional part is a list of game identifiers.
**It is not carried into the MIT repository**, not even filtered: the filter needs the catalog (game data), and the
list's provenance is the AGPL tree either way.

**What replaces it**: the token source is an input (§8.2 `TokenSource`):

- `Walk(folder)`: §5.8's walk applied to a folder (any text files of the two extensions); or
- `Seeds(list)`: an explicit list of candidate ids, which the version crate supplies (restricted keeps today's list
  as its seed for parity, §12 item 3).

The storage policy then differs from today's only if the seeds differ from the list's effective part. The
`sourceScope` field stays the fixed text of §5.8.

---

## 7. Settings (read with `raw`, untrimmed unless said)

| Setting | Effect | Default |
| --- | --- | --- |
| `COHORT_TREATMENT_LEGACY` | exactly `1`: B40-B42 answer the Dojo treatment | `1` |
| `GAUNTLET_CONFIG_PROBE` | exactly `1`: B47 answers the season config | `1` |
| `GAMESERVER_WIDEN_HUNTPASS_WINDOWS` | exactly `0`: no window rewrite | rewrite |
| `GAMESERVER_HUNTPASS_ID` | the served pass, JS-trimmed (set but empty: empty) | the table's `active_huntpass_id` |
| `GAMESERVER_RECENT_SEASONS` | comma list (set but empty: none) | the table's `recent_huntpass_seasons` |

None is a `host.toml` field today: a host sets them by hand in the env file (§12 item 7). The generators read no
setting; their paths come from hostkit's `Paths`.

---

## 8. The public seam

### 8.1 Shape

One crate, `rustic-isles-gamedata` (lib), MIT, in dainty-lass `crates/rustic-isles-gamedata`. Dependencies:
`rustic-isles` (core: `Handlers`, `Call`, `Shared`, `Service`, `Method`, `Settings`, `VersionTables` and its
`ItemGrantWindow`, `ProgressionConfigRule`, `GameDataRoot`, `season`, `replay`, `read_body_parts_limit`, `BODY_LIMIT`),
`rustic-isles-auth` (through `Shared::auth()`), `service-kit` (`json`, `jsjson`, `js`, `body`, `collate`),
`module-api` (`BoxFuture`), and workspace `axum`, `http`, `serde_json` (`preserve_order` in its own manifest),
`tracing`, `regex`, `sha2`, `chrono`. The generators touch no core type and could sit behind a default feature
`generators` so hostkit-free builds of the module skip `regex`, `sha2` and `collate`.

**Seam changes for the integration pass**: `crates/rustic-isles-gamedata` in the workspace and
`[workspace.dependencies]`. hostkit's `GeneratorKind::label()` names restricted functions
(`isles_core::gamedata::...`); those texts are hostkit's and change only in a hostkit row (§12 item 8).

### 8.2 Interface (names are the contract restricted's composer and host are written against)

| Item | Contract |
| --- | --- |
| `pub fn register(handlers: &mut rustic_isles::Handlers)` | Registers the 29 handlers of §3.1 on `Service::Backend` at their exact path strings (core §5.7: a typo fails the build with `UnknownRoute`) |
| `pub fn routes() -> &'static [(rustic_isles::Method, &'static str)]` | The 29 (method, path) pairs in §3.1's order; a test checks them against `rustic_isles::routes::all()`'s `game-data` rows |
| `pub fn envelope(payload: Value) -> Value` | §2 |
| `pub mod tuning` | `seasonal_event_schedule()`, `huntpass_xp_config()`, `island_content_config()`, `item_grant_data(&[ItemGrantWindow])`, `bounty_game_data(&[ItemGrantWindow])`, `bounty_game_data_daily()`, `bounty_game_data_weekly()`: §3.4's values, enveloped except `item_grant_data` |
| `pub mod gauntlet` | `GAUNTLET_SEASON_ID`, `config_payload(now_ms)`, `progression_payload(first_segment)`, `guild_rewards_payload()`, `leaderboard_payload()`: §3.5's payloads (not enveloped) |
| `pub mod patchnotes` | `payload(language, release_version) -> Value`: §3.6 (enveloped) |
| `pub mod progression_config` | `served(rule, &file, &VersionTables, &Settings, now_ms) -> Value`, `widen_huntpass_windows(config, &tables, &settings, now_ms) -> Value`, `served_huntpass_id(&tables, &settings) -> String` (progression's Hunt Pass route uses it), `recent_seasons(&tables, &settings) -> Vec<String>`; the four window constants |
| `pub mod generators` | `stringify_pretty(&Value, indent) -> String` (§2.1); `generate_progression_config(raw_dir, out_dir, apply)`, `generate_slayers_path(export, target, apply)`, `generate_slayers_path_definitions(export, target, source_version, apply)`, `generate_reward_cache_store(catalog, output, &RewardCachePlan)`, `generate_ladyluck_store(raw_dir, out_dir, apply)`, `enrich_ladyluck_store(catalog, live, store)`, `generate_platinum_store(catalog, store_images, capture_dir, data_dir)`, `generate_inventory_storage_policy(catalog, &TokenSource, game_data_dir)`, `resolve_platinum_export_file(exports, base) -> PathBuf`; each `-> Result<String, String>` |
| `pub struct RewardCachePlan` | `selections: Vec<RewardCacheSelection>` (`catalog_id`, `category: RewardCacheCategory`, `price: i64`, `featured`, `allow_rarity3`, `display_name: Option`, `description: Option`), `armour_families: Vec<String>`, `banner_families: Vec<String>`, `expected: Vec<(RewardCacheCategory, usize)>`, `expected_featured: usize`; `RewardCacheCategory { Armour, Weapon, Style, Event, Limited, General }` with `as_str()` and `tag()` (§5.4). `RewardCachePlan::from_json(&Value)` reads the same from `{"selections":[{"catalogId","category","price","featured","allowRarity3","displayName","description"}],"armourFamilies":[..],"bannerFamilies":[..],"expected":{"armour":n,...},"expectedFeatured":n}` so a host can supply it as a file |
| `pub enum TokenSource { Walk(PathBuf), Seeds(Vec<String>) }` | §6 |
| `pub enum BackendStep { ProgressionConfig, SlayersPath, SlayersPathDefinitions, RewardCache, LadyLuckGenerate, LadyLuckEnrich, PlatinumAndStoragePolicy }` | §5.9; one-to-one with hostkit's `GeneratorKind` minus `HuntTables` (the host maps it) |
| `pub struct GeneratorPaths { raw_game_data, backend_game_data, exports, store_captures }` | from hostkit's `Paths` fields of the same names |
| `pub struct VersionInputs { reward_cache: RewardCachePlan, storage_tokens: TokenSource }` | what only the version knows (§5.4, §6) |
| `pub fn run(step: BackendStep, paths: &GeneratorPaths, inputs: &VersionInputs) -> Result<String, String>` | §5.9's mapping, applying every step (`apply` true) |

### 8.3 How it plugs into core

- **Handlers** (core §5.7): the composer calls `register` on its backend `Handlers`; nothing else is composed.
  Every handler reads only the call's shared state (`version.hooks`, `version.tables`, `settings`, `game_data`,
  `auth()`), params, origin host and request.
- **No slot, no start-up, no database.** The crate keeps no state of its own; the cached file read is core's.
- **Calls to other areas**: none. Progression (unit 7) calls `progression_config::served_huntpass_id`; anything
  else that needs the season window calls core's `season` module directly.
- **Clock**: every "now" is `rustic_isles::replay::now_ms()` (so the replay scope pins it, core §9).

---

## 9. What stays in dainty-restricted

- `areas/game_data.rs`, `game_data/{tuning,gauntlet,patchnotes,progression_config,generators}.rs` and
  `src_tokens.txt`, deleted once the composer registers this crate and the host maps `GeneratorKind` to `run`.
- The goldens (`game_data.json`, `capture.mjs`, `progression_config.synthetic.json`), the Node byte-for-byte test
  and the reference scripts, run against this crate (§10.3).
- The Reward Cache plan and the seed token list, as the version crate's inputs (§12 items 2, 3).
- The host hook that turns `run`'s result into hostkit's `RunResult` (§5.10), until the host moves.

---

## 10. Test cases

### 10.1 What may be copied

Nothing recorded and nothing exported: `tests/golden/game_data.json`, `progression_config.synthetic.json` and any
example or capture stay in restricted. The unit tests beside the restricted files, `tests/gamedata_generators.rs` and
restricted `crates/host/tests/gamedata.rs` are **authored test code**: described here only. Every fixture the MIT
tests use is synthetic (made-up ids such as `CURRENCY_RAMS`, `WP_SWORD_00`, `/Game/UI/Icons/Icon_0.Icon_0`), never a
row of a real catalog, capture or plan (AGENTS.md "Nothing from the game is committed").

### 10.2 Behaviour the authored tests fix

| Area | Cases |
| --- | --- |
| Routes | `routes()` equals the 29 `game-data` rows of core's table, in order; `register` builds with no `UnknownRoute`; with no token each "game" row answers auth's empty 401 and each "open" row its answer |
| Constant answers | §3.1's table byte for byte with content types: `POST /heartbeat` 200 `text/plain; charset=utf-8` `20000`; `GET /crash/CrashReporter/Ping` `OK`; `POST /profile/update` 200, no body, no content type; `POST /motd` (no slash) 204; `GET /motd/trigger?event_name=test` 404 empty; `GET /eventstats/` `{"stats":[]}`; `GET /all/` envelope of `{"messages":[]}`; §3.4 byte for byte (weekly's key order included) |
| Treatments | default, `COHORT_TREATMENT_LEGACY=1` → Dojo; `0`, empty → `[]`; `/playertreatments/some-user` same as default |
| Trials | §3.3's table; an invalid JSON body with a valid token → the 400 page; with no token → 401 (auth first) |
| Survey | 1.14.7's hook → 200 envelope of `{"Triggers":[]}`; a hook with no body → empty status; a hook with status 1000 → 500 |
| Gauntlet | §3.5's table; payload functions: `leaderboard_payload()` is an array, `progression_payload("LEVEL_ACCESS")` is `0`, `progression_payload("")` both levels 0, `guild_rewards_payload()` `0`, `config_payload` at 2026-10-16T08:30:00Z compact text `{"gauntlet_id":"ESC_SEASON_1","start_at":"2026-07-30T17:00:00+00:00","end_at":"2026-10-22T17:00:00+00:00"}` and the window contains now |
| Patch notes | release version `""`: `release_version` `""`, two notes with §3.6's titles and types, three list items each, language echoed (`pt%2DBR` → `pt-BR`); `"9.9"`: `notes` `[]`, the language `fr` echoed |
| Progression config | §3.7's worked example; the appended clone's compact text `{"progression_id":"season44","start_date":"2099-01-02T00:00:00+00:00","end_date":"2099-12-31T00:00:00+00:00","tracks":[1,2]}` after the file's own paths; served pass ` season19 ` → `season19`, set-but-empty → `""`; `GAMESERVER_WIDEN_HUNTPASS_WINDOWS=0` → unchanged, `off` → rewritten; `{}`, `{"payload":[]}`, `{"payload":{"paths":{}}}` unchanged; `Date.parse("2024-12-03T00:00:00+00:00")` = 1733184000000; `AsIs` returns the file; the route without a file → 500 page; a changed file is not reread (core's cache); a pinned replay time moves the window |
| Writer | `stringify_pretty` of `{"b":1,"10":2,"9":3,"a":[],"o":{}}` at 2 → keys `9`, `10`, `b`, `a`, `o`, `[]` and `{}` inline; `1.0` → `1`; `"\u0001\n\"é"` → `"\u0001\n\"é"`; indent 1 and 0 layouts; a parse of the output equals the input |
| Progression generator | the synthetic source (one `ExperienceTrack_PlayerLevel` path with one free reward tier) → `{code:null,message:"OK",payload:{paths:[...]}}`, no trailing newline, report `source paths      : 1`, `free_rewards tiers : 1`; dry run writes nothing; an existing target is backed up as `progression_config.json.<stamp>.bak` (stamp pattern `\d{4}-\d\d-\d\dT\d\d-\d\d-\d\d-\d{3}Z`); missing source, non-JSON, `payload.paths` absent → §5.1's errors |
| Slayer's Path | three synthetic rows (`Slayer_09` with a `None` child and one reward, `Slayer_10`, `Slayer_11` with only a `None` reward): graph has three nodes `{"node_id":..,"node_status":0,"objectives":[]}`; definitions has `Slayer_09` `{"autoUnlockIfParentUnlocked":true,"childNodes":["Slayer_10"],"currencyCosts":[{"amount":100,"currency":"CURRENCY_RAMS"}],"rewards":["WP_SWORD_00"]}` and `Slayer_10`, not `Slayer_11`, written at indent 1; a bad second line → `line 2`; leading blank lines shift the graph's count but not the definitions' |
| Reward Cache | a synthetic plan and catalog: order (featured first, category, armour family, `_ULTRA`, slot; axes by rarity; banners by family and component); each rule of §5.4's table triggers its error; `season19_rewardcache_armour_ar_x_helm_00_ultra`-style ids; trailing newline; expected-count mismatch fails without writing |
| Lady Luck | §5.5: `images` dropped, defaults filled, the problem lines and the refusal; §5.6: keep-previous by items, a catalog-missing offer skipped, images from the catalog, the four checks, retired ids |
| Platinum | per pass: each skip reason; rank skip and pass shapes; reconstructed price per category and rarity; decode with and without a type prefix; the sort key (section, tab order, zero-padded price); `_1_12` export preferred when both exist |
| Storage policy | `Seeds(["CURRENCY_RAMS","NOT_IN_CATALOG"])` plus a game data file naming `WP_SWORD_00` → two entries in byte order, `stacked` with `maxQuantity` 999999999 and `instanced` 1; `inventory_storage_policy.json` and `*.example.json` not scanned; `generatedAt` `2026-10-08`; `catalogSha256` the file's SHA-256 |
| `run` | each `BackendStep` reads §5.9's paths; `RewardCache` and the two Lady Luck/platinum steps append the policy report; a failing first generator returns its error and skips the second |

### 10.3 Checks that run in dainty-restricted against this crate

The game-data golden (48 cases recorded from the Node handlers at `now` = 1793016000000) and the generators'
byte-for-byte comparison with the reference scripts on restricted's synthetic fixture (which passes the seed list
and today's Reward Cache plan), once the composer and host call this crate. A failure there is a bug here, except for
the patch-notes prose if §12 item 1 is decided as own copy.

---

## 11. Security behaviour (RX3)

The routes take untrusted input only through B33's body (core's bounded reader, 100 KiB) and path params that are
echoed into JSON (escaped by the writer); nothing is stored, no file is named by a request, and the only file read
is the fixed `progression_config.json` under the configured folder. The generators read host-side files (game
exports, host-supplied responses, captures hostkit fetched at a pinned commit and checked by hash) and write fixed
file names under hostkit's folders. A `docs/security/rustic-isles-gamedata.md` page is short; it may be a section of
core's `rustic-isles.md` instead (the coordinator's call).

- **Bounded work**: the window rewrite is linear in the paths plus the recent-season list; the generators are linear
  in their inputs except the Reward Cache sort (n log n over the plan) and the platinum pass over baked rows.
- **Regexes** (`regex` crate, linear time): the token pattern, the quest-rumour pattern, the entitlement pattern.
- **Texture references** are checked in §5.4 and §5.6, so a catalog icon cannot put `'` into the reference; the
  platinum pass's catalog fallback only requires the `/Game/` prefix (a weak spot to list; the client parses the
  reference).
- **Paths**: no generator joins a path from file content; backups are written beside their target only.
- **Known weak spots to list**: the unchecked platinum reference above; the storage-policy walk follows symbolic
  links inside the game data folder; the cached config never refreshes (core §14).

**Fuzz targets (`fuzz/fuzz_targets/`):** `rustic_isles_gamedata_windows`: arbitrary JSON (as `payload.paths`),
arbitrary served id and recent-season text, any `now`: never panics; non-pass elements are unchanged; the path count
grows by at most the recent-season count; every opened or added pass has the expected window strings.
`rustic_isles_gamedata_formats`: arbitrary bytes as a catalog line, a nested `customData`, a store-image row name, a
journey row and a Lady Luck source: the readers and the slug, decode and category rules never panic;
`JSON.parse(stringify_pretty(v, n)) == v` for any parsed `v` and `n` in 0..=4.

---

## 12. Open questions and behaviour that looks wrong but is kept

1. **Patch-notes prose** (§3.6). The welcome descriptions and six list items are upstream's expressive text (Paradox
   `48e3956`, renamed). As with the auth code mail (auth §15 item 7), proposed: own copy, keeping §3.6's structure,
   titles, types, date and permalink; restricted's two patch-notes golden cases then check structure only. The
   alternative (verbatim parity) needs the text supplied as data by the version crate.
2. **The Reward Cache plan** (§5.4) is 178 hand-picked catalog ids with prices, three name and description
   overrides, and two family orders: authored data over game identifiers. Proposed: an input (`RewardCachePlan`)
   that restricted's version crate supplies; whether it can ever be MIT (unit 12) is the maintainer's call. Without
   it, `RewardCache` cannot run on an MIT-only host.
3. **`src_tokens.txt`** (§6): not carried; restricted passes it as `Seeds` for parity. Proposed for later: a seed
   list of the catalog ids the MIT area crates name (starter manifest, grant windows, seasonal coin, tokens),
   reviewed by the maintainer; the policy may then drop ids only Paradox's TypeScript named.
4. **B33 reads auth before the body.** Node parses JSON before any route, so a malformed body is a 400 even without a
   token; here it is auth's 401 first (core §15 item 4). Kept.
5. **A missing `progression_config.json`** fails Node's start; here only B46 answers a 500 page (and logs). Kept.
6. **`resolve_platinum_export_file` prefers the `_1_12` export** whenever it exists, even beside a `_1_14_7` one
   (1.12.0 was dropped on 2026-10-31), and §5.8's policy always reads `catalog_1_14_7.jsonl`, so the platinum store
   and the policy can come from different catalogs. Parity; worth a row.
7. **The five settings of §7 are not in `host.toml`** (AGENTS.md's configuration rule). They are debug and A/B
   switches with safe defaults. Proposed: leave them unlisted until a host needs one, then add them through hostkit.
8. **hostkit's labels** name `isles_core::gamedata::*` (restricted's paths) in its "unavailable" error. A hostkit row
   may rename them to `rustic_isles_gamedata::generators::*`; not this crate's.
9. **Two meanings of "instanced"**: Reward Cache, Lady Luck and the storage policy treat anything not exactly
   `isStackable: true` as instanced; the platinum store treats only exactly `false` as instanced (a missing flag is
   stacked). Kept from Node.
10. **Restricted's Rust departs from Node in places seen**; the MIT crate follows Node (§5's text): (a) §5.5 prints
    the problem list before refusing (restricted drops it); (b) §5.6 and §5.7 leave out `quantity` for an item without
    one (restricted writes 1) and §5.7 an entitlement `name` that is absent (restricted writes `null`); (c) §5.7's
    display priority and duration follow `Number(..) || default` (restricted truncates fractions and ignores numeric
    strings). None occurs in real captures; restricted's byte-for-byte test does not reach them. The implementer
    flags any further gap it meets rather than guessing.
11. **`supplies_boost`** is in the tab table and priced, but no catalog id maps to `boosts`. Kept.
12. **Dead version branch**: §3.6's non-empty release version serves no shipped version. Kept, so the version table
    keeps its meaning.
13. **Report texts mention 1.12 and 1.4.4** (`live 1.12 table`, `tierA: sku id not baked into 1.12
    StoreItemsTable`): host terminal text from the 1.12 era. Kept for parity; rewording is a separate change.
14. **`generatedAt` is fixed at `2026-10-08`** and `sourceScope` names TypeScript globs the MIT crate never walks.
    Kept: bounties-store and inventory read neither, and a byte-identical file is easier to compare.

---

## 13. Coordinator decisions (2026-11-05, before implementation)

These settle §12 and override anything above that disagrees.

- **Item 1: own patch-notes copy.** Structure, titles, types and dates as specified; the prose is the implementer's
  own. Restricted's two patch-notes golden cases compare structure only.
- **Item 2:** `RewardCachePlan` is an input the version crate supplies; it stays in restricted until the maintainer
  rules on it.
- **Item 3:** `TokenSource::Seeds` as proposed; `src_tokens.txt` is not carried.
- **Item 4:** parity, as listed.
- **Item 5:** the Node behaviour, as specified (the oracle).
- **Items 6-7:** unchanged here; host.toml fields and hostkit labels are later hostkit rows.
- **Item 8:** its own page, `docs/security/rustic-isles-gamedata.md`, with both fuzz targets.
