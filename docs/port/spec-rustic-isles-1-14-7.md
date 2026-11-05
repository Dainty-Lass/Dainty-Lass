# Clean-Room Behaviour Specification: `crates/rustic-isles-1-14-7` and `crates/rustic-isles-1-14-7-plugin` (RD27)

> **Scope and purpose (RD27):** this is the clean-room behaviour specification for unit 12 of
> [`spec-rustic-isles-core.md`](spec-rustic-isles-core.md) §16: the MIT **version crate** `rustic-isles-1-14-7`
> (1.14.7's tables, hooks and Director values, the module spec, the composer that wires every `rustic-isles-*` area
> crate and dainty-lass's MIT `rifts-*` crates into one module, its start-ups and its `ModuleHooks`) and the MIT
> **plugin crate** `rustic-isles-1-14-7-plugin` (the same module as a library the core loads). Its sources are
> dainty-restricted's `modules/rustic-isles/crates/isles-1-14-7` and `isles-1-14-7-plugin`, the module skeleton of
> `isles-core/src/module.rs` that composes the areas today, `areas/auth/wiring.rs`'s rifts core, and how
> restricted's `crates/host` loads the module. All are AGPL-3.0-only with Mystic Paradox's section 7 terms; an
> implementer who never sees them writes the MIT crates from this document and the area specs alone.
>
> Everything below is stated from observable contracts: the manifest, the wire payloads of every hook, the table
> values, the order in which the composition fills core's seams, the settings it reads, the files it loads and the
> tests. No implementation code is reproduced and no private helper is named. Node and restricted file names appear
> only as provenance.
>
> Written against dainty-restricted `e0e238b4434316bb210677d88bc59e3448688794` (worktree
> `splashloader-integration-90130d`; the two crates last changed in `4efc8b5`, the flat-`crates/` move) and dainty-lass
> `d54366d2b0bc1962f735f8ad0b9015ce1a777135` (branch `feat/rustic-isles-clean-room`), reading every
> `docs/port/spec-rustic-isles-*.md` at that commit, including each one's closing coordinator decisions.

---

## 1. Provenance and licensing classification

### 1.1 Evidence used

| Source | What it says |
| --- | --- |
| `modules/rustic-isles/NOTICE.md` | Every crate and file of the module is AGPL-3.0-only with Mystic Paradox's section 7 terms |
| File headers (`//!`) and comments of `isles-1-14-7/src/lib.rs` | DaintyLass rows (DD4, DD8, DD14, DD16, DD25, RU3c, RF2) for the shape; Node provenance for values: `controllers/candidateStatus.ts` (Ramsgate), `weaponBaseLevel.ts`, `tuning.ts`'s `BountyGrantWindows`, `listenerPlan.ts` (QoS 80), the captured `epiceos` contract |
| `git log` of both crates | One author (the maintainer); history flattened by the 2026-11-01 merge and the `4efc8b5` move |
| `isles-1-14-7/tests/*.rs`, `isles-1-14-7-plugin/tests/load.rs` | Authored tests (manifest, 501 sweeps in and out of process, Director arguments, game-data goldens, realms with a database, chat before and after start, the library load) and the golden replays of five areas (described only, §12) |
| `isles-core/src/module.rs` (`start_services`, `chat`, `admin`, `event`, `stop`) | The composition order restricted runs today |
| `isles-core/src/areas/auth/wiring.rs` (`AuthReforgedCore`) | The rifts fallback core (§5.6) |
| `isles-core/src/areas/characters/{data,starter}.rs`, `areas/bounties_store/wallet.rs`, `game_data/generators.rs`, `isles-db/src/transforms.rs` | Where the game-derived inputs of §4 live today (read for classification only; no value is copied here) |
| `crates/host/src/compose.rs`, `crates/host/src/lib.rs` (`isles_service_main`) | How restricted's server links the module and builds the hidden `backend` and `director` subcommands |
| `launcher/src-tauri/src/controller/realms.rs`, `native/runtime/p2p/PlayerHost.cpp`, `modules/rustic-rifts/src/admission_client.cpp`, `modules/rustic-rifts/crates/rifts-seam/src/http.rs`, `isles-core/src/http.rs` | Which host-capability header each side sends or reads (§7) |
| dainty-lass `crates/rifts-*`, `crates/rustic-isles`, `crates/module-api` (`plugin.rs`, `child.rs`), `crates/plugin-host`, `crates/controller/src/registry.rs`, `docs/port/spec-rd27-rifts.md` D3 | The MIT side the composition is written against |

### 1.2 Per file: category and where it goes

Category A is code written for DaintyLass with no Paradox code in it; B ports Paradox TypeScript. As core decided
(core §17 item 11), everything is rewritten from this spec; nothing moves as it is.

| Restricted file | Cat. | Rewritten as |
| --- | --- | --- |
| `isles-1-14-7/src/lib.rs`: `Hooks` | A (shape), wire facts (payloads) | §3.3 |
| `isles-1-14-7/src/lib.rs`: `TABLES`, `DIRECTOR` | wire facts and authored values | §3.2, §3.4, classified in §4 |
| `isles-1-14-7/src/lib.rs`: `rifts`, `rifts_with_core` | A | §6 |
| `isles-1-14-7/src/lib.rs`: `version`, `spec`, `module` | A | §3.1, §9 |
| `isles-1-14-7/src/main.rs` | A | §9.3 (the child executable) |
| `isles-1-14-7-plugin/src/lib.rs` | A (one line) | §9.4 |
| `isles-core/src/module.rs` composition (`start_services`, hooks) | A | §5 (the composer; core §11.4 already took the skeleton) |
| `isles-core/src/areas/auth/wiring.rs` `AuthReforgedCore` | B (wraps `services/operationsPolicy.ts` reads and the step-up rule) | §5.6.1 |
| The auth-to-rifts glue in `areas/auth/{middleware,launcher,account,game}.rs` | A | §5.6.2 |
| `isles-1-14-7/tests/*`, `isles-1-14-7-plugin/tests/load.rs` | A (authored tests), goldens | §12 (described only; goldens stay) |

### 1.3 What stays in dainty-restricted

- `native/`, `tools/RuntimeLoader`, `reference/`, every golden and golden harness (core §16 row 12).
- The **values** of the game-derived inputs of §4.2 (`CharacterData`, `StarterKit`, the legacy starter defaults, the
  Reward Cache plan, the storage-policy token seeds, and the starter manifest's item lists until §15 item 1 is
  ruled): restricted ships them as `version_data.json` (§4.3) and may pass them directly when it compiles the module
  in (§9.2).
- The runtime patch and launcher change of the host-capability cutover (§7.4): `PlayerHost.cpp`, the rustic-rifts
  native client and `launcher/src-tauri/src/controller/realms.rs` are restricted's.
- `crates/host`'s compile-in and the hidden `reforged-host backend` / `director` subcommands, now over the MIT crate
  (§9.2); the gamedata and db host hooks until hostkit calls the MIT crates.
- Restricted's AGPL `isles-1-14-7`, `isles-1-14-7-plugin`, `isles-core`, `isles-chat`, `isles-director`, `isles-db`
  and rustic-rifts' server crates are deleted once restricted's host links this crate (the integration pass's call).

---

## 2. Conventions

- **JSON** is written as `JSON.stringify` writes it (core §2.1): keys in insertion order (`serde_json`'s
  `preserve_order`, enabled in each crate's own manifest, core §17 item 8), numbers as JavaScript prints them (an
  integral value has no fraction: `3`, never `3.0`; a non-finite number is `null`), strings escaped as JSON does.
- **Text** comparisons are exact and case-sensitive unless a row says otherwise; "trimmed" is Rust's `trim` unless
  "JS-trimmed" (`service_kit::js::trim`) is said.
- **Settings** are read through core's `Settings` (core §6.1): `raw` is the value as set (empty included), `get` is
  `raw` without empties; `rifts_seam::Settings::get` answers `raw`. The process environment is never read.
- **No legacy names**: no `MYSTICPARADOX_*` alias, no `x-mysticparadox-*` header copy, no `MysticParadox.dll`
  mapping (core §17 item 2, auth §15 item 9). Rifts names follow spec-rd27-rifts D3 (§7, §8).
- **Secrets never in `Debug` or a log**: every type here that holds or can reach a key, token, secret or account key
  prints names only (core §17).
- **Clocks and ids** are core's replay-aware ones (core §9) unless a row says "system clock" (parity, §14).

---

## 3. The version's public values (the contracts)

### 3.1 Identity and manifest

| Item | Value | Class (§4) |
| --- | --- | --- |
| Module id | `isles-1-14-7` (kept, core §15 item 13: `host.toml`, the gateway's routing and invites name it) | (c) DaintyLass |
| Display name | `Rustic Isles 1.14.7` | (c) DaintyLass |
| Game version | `1.14.7` | (a) |
| Invite aliases | `reforged` (the manifest answers both `reforged` and `1.14.7`) | (c) DaintyLass, live contract |
| Kind | Game; `api` is `module_api::API_VERSION`; no client profile | — |
| Capabilities | `realms`: true when built with the `realms` feature (default on), else false; `network` true; `chat` true; `patch_set` `Accepts` (rustic-freeza registers against it, DD25); `theme` none | (c) |
| `unverified` | false | (c) |
| Ports, in this order | `backend` TCP 61000, `director` TCP 61001, `qos` TCP 61010 (the QoS pong's loopback side), `game` UDP 8770-8777 | (c) DaintyLass |
| Services, in start order | core's `BackendService`, then `rustic-isles-director`'s `DirectorService` over §3.4's config | — |

The manifest must pass `ModuleManifest::validate`; core builds it from the spec (core §7.1).

### 3.2 Tables (`rustic_isles::VersionTables`)

| Field | 1.14.7 value | Class (§4) | Read by |
| --- | --- | --- | --- |
| `target_changelist` | `647472` | (a) the client's build | auth, characters, Director, rifts |
| `game_version` | `1.14.7` | (a) | everyone; `build_id()` is `647472_1.14.7_shipping` |
| `active_huntpass_id` | `season43` | (a) the client's season id | gamedata, progression |
| `recent_huntpass_seasons` | `season43`, `season44`, `season45`, `season46`, `season47`, `season48`, `season49`, `season50`, in that order | (a) | gamedata |
| `patchnotes_release_version` | empty text (the 1.14.7 runtime collapses the line) | (a) | gamedata |
| `bounty_item_grant_data` | two windows, in order: `TOKEN_BOUNTY_DRAFT_PREMIUM` amount 1, and `CURRENCY_PRESTIGE` amount 5, each from `2026-01-01T00:00:00.000Z` to `2027-12-31T00:00:00.000Z` | ids (a); amounts and dates (c) upstream `tuning.ts`, dates moved forward for DaintyLass | gamedata, bounties |
| `store_sections` | `ladyluckstore`, `season_store`, `webstore`, `dyes`, `hp_level_skip_rank`, `huntpass_store`, `exchange_vendor_slot_2`, `exchange_vendor_slot_3`, `fountain_daily_free_bundle`, in that order | (a) section tags the client asks for | bounties |
| `seasonal_coin` | catalog id `CURRENCY_SEASONAL_COIN`, at most 1000 per claim | id (a); cap (c) upstream | inventory, bounties |
| `masked_journey_nodes` | one: node `Gauntlet_Unlock`, unmasked by the setting `RUSTIC_ISLES_GAUNTLET` | node (a); switch (c) DaintyLass | progression |
| `starter_manifest.bootstrap_version` | `starter-1.12-v5` (stored in existing rows; must stay byte-equal) | (a) a stored-data contract | characters, db |
| `starter_manifest` item lists | 24 stacked catalog ids and a nine-entry quick-emoji wheel (eight ids then one empty text) | **disputed**, §4.2 and §15 item 1; values not in this spec | characters |
| `switch_defaults` (core §6.2 keys) | `qos_port` `80` (the 1.14.7 QoS fix: unset is 80), `reforge` `on`, `mastery_persistence` `on`, `bounty_persistence` `on`, `cooldown_persistence` `on` | (c) DaintyLass | core settings |
| `session_payload` | §3.3 row 1 | (a) | auth |

Core's `quick_emoji_wheel` field is `Option<&'static str>` at `d54366d`, but characters §6.2 and §6.3 read nine texts.
The field must hold nine texts (an interface fix for the core owner, §15 item 2).

### 3.3 Hooks (`rustic_isles::VersionHooks`, plus the table's session payload)

One stateless value (`Hooks`, `Copy`, `Default`, `Debug`). Every JSON answer below is byte-exact under §2.

| # | Hook | Answer |
| --- | --- | --- |
| 1 | session payload (`VersionTables::session_payload`, token `t`) | `{"error_code":null,"sessionid":"t","sessiontoken":"t","sessionToken":"t"}`: the captured `epiceos` contract, the token as the session id under both spellings. `rustic_isles_auth::session_payload_1_14_7` already answers exactly this and may be the table's value |
| 2 | `no_player_policy()` | `ConfiguredAccount` (core §5.3) |
| 3 | `weapons_at_base_level(settings)` | false only when `raw("KEEP_WEAPON_ITEM_LEVELS")`, trimmed, equals `true` ASCII case-insensitively; true otherwise (unset included) |
| 4 | `reforge_rules()` | true |
| 5 | `mastery_persistence()` | true |
| 6 | `huntpass_payload(id)` | `{"huntpass_id":"<id>","season":"<id>"}` (the same text twice) |
| 7 | `objective_row(user, row)` | `{"phx_account_id":"<user>","objective_id":"<row.objective_id>","progress":<row.value>,"completed_count":<row.completed_count>,"created_date":"<row.created>","last_modified_date":"<row.last_modified>"}` |
| 8 | `progression_config()` | `RollingHuntpassWindows` |
| 9 | `survey_config()` | status 200, body `{"code":null,"message":"OK","payload":{"Triggers":[]}}` |
| 10 | `entitlements_payload(list)` | `{"entitlements":[...]}`, one object per entry in the order given: `{"activatedDate":<text or null>,"duration":<number>,"name":"<name>"}` (keys in that order) |
| 11 | `candidate_destination(hunt, realm_hub)` | §3.3.1 |

`objective_row`'s numbers: restricted stores them as integers; core's `ObjectiveRow` carries `f64`, so an integral
value must be written without a fraction (`{"progress":3,...}`, never `3.0`). The same holds for `duration`.

#### 3.3.1 The candidate destination

Checked in this order:

| Hunt id | Realm hub | Destination |
| --- | --- | --- |
| exactly `ShatteredIsles_ReturnToRamsgate` | either | game mode `CITY`, hunt id none (`null` on the wire) |
| contains `TrainingDojo` (case-sensitive substring) | true | `SHARED`, the hunt id |
| contains `TrainingDojo` | false | `ISLAND`, the hunt id |
| anything else (empty included) | either | `ISLAND`, the hunt id |

### 3.4 Director values (`rustic_isles_director::DirectorConfig`, `const`)

`target_changelist` 647472; standard switches in this order `-EpicPortal`, `-server`, `-nullrhi`, `-warp`,
`-nosound`, `-RepDriverEnable`; default `-LogCmds` `LogPhoenixCharacter VeryVerbose, LogOnline Verbose, LogLoad Off`
(director §12.4). With default settings the Director's `launch_args` joined by one space is
`-EpicPortal -server -nullrhi -warp -nosound -RepDriverEnable -LogCmds="LogPhoenixCharacter VeryVerbose, LogOnline Verbose, LogLoad Off"`.
1.14.7 has the Trials tables, so the Director's file list is core §8.1's with the two `_new` arena tables.

---

## 4. Game-derived values: classification and how the MIT crate gets each

### 4.1 The classes

- **(a) Wire constant the 1.14.7 client (or data the client already wrote) requires**: an identifier or a fixed
  protocol shape, small and functional, which the client sends, asks for or reads back exactly. May be committed,
  each with a one-line justification beside it (the comment points here). Precedent in dainty-lass: the 1.14.7
  executable's SHA-256 is already committed in `crates/epic`, `hostkit`, `launcher-kit` and `rifts-realms`.
- **(b) Game data**: read out of the game's own files (exports, captures, asset GUIDs, a captured preset). Never
  committed; generated on the host from its own install or supplied at run time.
- **(c) Authored data**: a choice someone made. **DaintyLass-authored** values (the maintainer's rows) may be
  committed. **Upstream-authored** values (Paradox or the Reforged Revival line, in the AGPL tree) reach dainty-lass
  only as run-time inputs restricted supplies, unless the maintainer rules otherwise.

### 4.2 Every value the area specs made a version input

| Value (spec) | What it is | Class | How the MIT crate gets it |
| --- | --- | --- | --- |
| §3.1-§3.4 identifiers and payloads (core §5.2, director §12.4) | build, season, section, node and catalog ids the client uses; payload shapes; engine switches | (a) | Committed in the version crate (core §5.2 already ruled the tables wire facts) |
| Grant window amounts and dates, seasonal coin cap, `LogCmds`, switch defaults, ports, identity | policy and DaintyLass choices | (c) DaintyLass (dates moved by DaintyLass; amounts and cap from upstream `tuning.ts`, small policy numbers) | Committed, as core §5.2 has them |
| The 1.14.7 executable SHA-256 `f3f95fbf...fbb66b` (§6 step 9) | a hash of the shipped client | (a) | Committed (already in dainty-lass); `rifts-realms`' `KNOWN_GAME_EXE_SHA256` may be the source |
| `starter_manifest` item lists (24 stacked ids, nine quick emojis) | catalog and emoji ids a new account must hold | (a) by core §5.2's ruling, but the same kind as `StarterKit` (characters decision item 3) | **Disputed** (§15 item 1). Until ruled: an input (`version_data.json` `starterManifest`), which needs the core seam change of §15 item 1 |
| `CharacterData` (characters §6.1): the fill list, forced flags, forced login flag, Dojo series key, blob, node and trigger | save-data key names and default blobs (protocol shapes), client flag names, quest-series GUIDs from the game's data, upstream defaults | mixed (a)/(b)/(c upstream); one input as the coordinator decided (characters decisions item 3) | Input: `version_data.json` `characterData` (§4.3) |
| `CharacterData.default_appearance` | a character-creator preset as the client writes it (mesh slots, colour values) | (b) captured | Input (§4.3) |
| `StarterKit` (characters §6.2): instanced ids (23), default weapon parts (8), equipment map, weapon instance data, flask, banner, glider, banner customization, intro emote, flare, nine default quick chats, the wallet | an upstream selection (`controllers/starterManifest.ts`, `controllers/wallet.ts`) over game ids, and upstream amounts | (c) upstream over (a) ids | Input: `version_data.json` `starterKit` |
| Legacy starter defaults (db §14.3: the default dye and sigil) | two catalog ids an upstream migration script wrote | (c) upstream | Input: `version_data.json` `legacyStarter` (read by the db tool, not the module) |
| Reward Cache plan (gamedata §5.4, 178 selections) | hand-picked catalog ids with prices and overrides | (c) upstream | Input: `version_data.json` `rewardCachePlan`, in `RewardCachePlan::from_json`'s shape (gamedata §8.2); stays restricted's (gamedata decisions item 2) |
| Storage-policy token seeds (gamedata §6) | the effective part of `src_tokens.txt`, a derivative of AGPL source | (c) upstream | Input: `version_data.json` `storageTokenSeeds` (gamedata decisions item 3: `TokenSource::Seeds`); §15 item 6 proposes an MIT list |
| Patch-notes prose (gamedata §3.6) | upstream prose | (c) upstream | Not an input: gamedata writes its own copy (gamedata decisions item 1) |
| Hunt tables, progression config, Slayer's Path, stores, storage policy, mastery objectives, escalation tuning, fountain bundle, tester kit (core §8.1) | exports and generator output | (b) | Already run-time files under `game_data.backend_dir` / `director_dir`, generated by `reforged-host gamedata` (unchanged) |
| Ramsgate, Training Dojo ids and maps (matchmaking, director) | client hunt and map ids | (a) | In their area crates (already specified there) |

### 4.3 `version_data.json` (the run-time inputs)

**Where**: a file named `version_data.json` in the module's backend game data folder (`[modules.isles-1-14-7.game_data]
backend_dir`, core §8.1). No new setting, `host.toml` field or env line: the folder is already configured.

**Who writes it**: restricted, which holds the values. Its release carries the document, and its setup (or its
`reforged-host gamedata` hook, until hostkit gains a step) copies it into the backend game data folder. A host
running only dainty-lass's binaries gets it the same way it gets every other backend file: generated or supplied,
never from this repository (AGENTS.md "Nothing from the game is committed"). Restricted's compiled-in host may skip
the file and hand the values directly (§9.2).

**Format**: one JSON object, read by core's loader (core §8.2: lossy UTF-8, one BOM dropped, `JSON.parse` rules).
Unknown top-level members are refused (a typo must not silently fall back).

| Member | Type | Required | Meaning |
| --- | --- | --- | --- |
| `format` | number | yes | `1` |
| `gameVersion` | text | yes | must equal the tables' `game_version` (`1.14.7`) |
| `characterData` | object | with a database | characters §6.1, camelCase: `fill` (array of `[key, value]` pairs, in order), `defaultAppearance` (object with `Data` and `StringData` arrays), `forcedCharacterFlags` (array of `[flag, value]` text pairs), `forcedLoginFlag` (`[flag, value]`), `dojoSeriesKey`, `dojoSeries` (object), `dojoNodeId`, `triggerSeriesKey`, `triggerNode` (texts), `triggerStatus` (number) |
| `starterKit` | object | with a database | characters §6.2, camelCase: `instanced`, `defaultWeaponParts` (arrays of texts), `equipment` (array of `[slot key, catalog id]`), `instanceData` (array of `[catalog id, object]`), `flask`, `banner`, `glider`, `introEmote`, `flare` (texts), `bannerCustomization` (object), `defaultQuickChats` (nine texts), `wallet` (array of `[catalog id, number]`) |
| `starterManifest` | object | only under §15 item 1's option B | `stacked` (texts), `quickEmojis` (nine texts or absent) |
| `legacyStarter` | object | no | `defaultDye`, `defaultSigil` (texts) |
| `rewardCachePlan` | object | no | gamedata §8.2's `RewardCachePlan::from_json` document |
| `storageTokenSeeds` | array of texts | no | gamedata §6 `TokenSource::Seeds` |

**Validation** (each failure is the text `version_data.json: <reason>`; the first wins): `format` is not 1;
`gameVersion` differs; a required member is missing or of the wrong type; `instanced` has a duplicate; an
`equipment` catalog id, `flask`, `banner`, `glider` or a `defaultWeaponParts` entry is not in `instanced`; an
`equipment` slot key is empty or repeated; `defaultQuickChats` (and `quickEmojis` when given) is not exactly nine
texts; a wallet amount is not a finite number; `triggerStatus` is not a finite number. (Restricted fails later, at
the first character creation, with characters §6.2's `Missing inventory instance ID ...` 500s; refusing the file at
start is a departure only for a broken file, §14 item 18.)

**When it is read**: at compose (§5.1 step 2), only when a database is wanted. Missing: the start fails with core's
required-file text (core §8.2) naming `<backend_dir>/version_data.json` (or `<no backend game data folder>/version_data.json`).
Unreadable or invalid: the start fails with the error. Without a database it is not read and the areas get empty
inputs (every route that needs them answers "no database" first).

**`Debug`** of the parsed value is safe (no secret), but prints counts, not values, so logs never carry it.

---

## 5. The composition (`rustic_isles::Composer`)

`VersionComposer` implements core's `Composer` (core §11.4). It is built with its data source: "read
`version_data.json`" (the default) or given `VersionData` (§9.2). One composer per module; it keeps no state between
starts.

### 5.1 `compose(inputs)`, in order

1. **Database.** When `inputs.database_wanted`: `rustic_isles_store::IslesStore::open_configured` with the settings'
   `get`, core's context home over `inputs.context` (core §5.4) and `inputs.node`, run off the async threads. An error
   fails the start with its text. `engine_is_postgres` is whether the opened configuration is Postgres.
2. **Version data** (§4.3), only with a store.
3. **Per-module state**: `Characters::new(character data, starter kit)` (empty inputs without a store);
   `MatchmakingState::new()`; `HubSessions::new()`; `ReforgedState::new()`; and a **late-bound cell** for the
   module's `Shared`, empty until the first start-up fills it (§5.8). Everything is per module; nothing is a process
   global (core §17 item 14).
4. **Rifts cores**: `AccountsCore` (§5.6.1) over the store, the settings and the cell; then
   `MatchmakingCore::new(store, node, state, accounts core)` (matchmaking §11).
5. **Rifts** (§6), built off the async threads from the settings and the matchmaking core. Without the `realms`
   feature: `Rifts::dedicated_only()`.
6. **Handlers** (§5.3) on a new `Handlers`.
7. **Slots** (§5.2).
8. **Auth**: `auth_store` is `IslesAuthStore::new(store)` (none without a store); `auth_seams` per §5.4.
9. **Observer**: the Reforged crate's `RequestObserver` over the state's metrics.
10. **Admin authenticator**: the Reforged admin half's `AdminAuthenticator` (reforged-admin §16.2), so core can make
    the `Admin` caller for the `/admin/` rifts rows.
11. **Hooks** (§5.7) and **start-ups** (§5.8).

Any failure in 1, 2 or 5 is `ModuleError::failed(<text>)` and core marks the start failed (core §7.3).

### 5.2 Slots (core §5.1 typed area state)

| Slot | Value | With a store only |
| --- | --- | --- |
| `IslesStore` | the opened store | yes |
| `Characters` | step 3's | no |
| `MatchmakingState`, `HubSessions` | step 3's (matchmaking records hub sessions, chat reads them) | no |
| `ReforgedState` | step 3's (both Reforged halves) | no |
| Inventory's `RingInstances` | matchmaking's implementation over its state (matchmaking decisions item 12) | no |
| Bounties' `BountiesSeams` | `foreign`: §5.6.4's adapter; `activity`: §5.6.3's claim adapter | no |
| Reforged's `MatchmakingPresence`, `MatchmakingCandidates` | adapters: `touch(user, at)` is the state's `presence().touch`; `has_candidate(user)` the state's | no |
| The economy's activity slot(s) | §5.6.3's adapters | no |
| The Reforged admin half's `TesterFunding` | §5.6.5's adapter | yes |

### 5.3 Handlers

Each area registers its own rows by their exact table paths (core §5.7); `Handlers::on` replaces, so two areas must
never register one row. Calls, all on the backend service: `rustic_isles_gamedata::register`;
`rustic_isles_characters::register`; `rustic_isles_chat::register`; the economy's inventory `register(handlers,
ports)` with the ports of §5.6.3 and matchmaking's `RingInstances`, progression's `register(handlers, ports)` with its
activity adapter, the same ring and the inventory ports, and `bounties::register`; `rustic_isles_matchmaking::register`;
`rustic_isles_reforged::register_public` and `register_admin`. Core adds auth's 49 itself (core §10).

| Registrar | Rows | Count |
| --- | --- | --- |
| auth (core) | B6-B24, B91, B92, B145-B172 | 49 |
| gamedata | B26-B34, B36, B37, B40-B42, B45-B51, B62, B63, B82-B87 | 29 |
| characters | B65-B67, B132-B136 | 8 |
| chat | B35, B80, B81, B137-B144 | 11 |
| economy: inventory | B68-B73, B130, B131 | 8 |
| economy: progression | B43, B44, B52, B108-B129 | 25 |
| economy: bounties | B38, B39, B53-B61, B74-B79 | 17 |
| matchmaking | B88-B90, B93-B107 | 18 |
| Reforged public | B1-B3, B5, B25, B64, B173-B176, B182-B198, B210-B220, B222-B227 | 44 |
| Reforged admin | B4, B177-B181, B199-B209, B221 | 18 |
| **isles handlers** | the 219 isles-side backend rows and the 8 rifts-side Guard rows isles keeps | **227** |
| rifts (§6) | B228-B285 on the backend, D5 on the Director | 58 + 1 |
| Director | D1-D4 | 4 |

The sets are disjoint and their union is every row of core's table (290).

### 5.4 Auth seams (`rustic_isles_auth::Seams`)

| Seam | Filled with |
| --- | --- |
| `activity`, `operations`, `notices`, `erasure` | `ReforgedSeams` (reforged-public §13.1) |
| `realms` | §5.6.2's adapter over step 5's rifts |
| `game_login` | progression's `SessionSupplies` (progression §10.4) |
| `home` | core's context home over the module context (core §5.4) |
| `mail`, `discord` | auth's defaults (`SmtpTransport`, `HttpsDiscord`); the Reforged admin half is given the same `DiscordApi` |

### 5.5 Who needs the `Shared` late

Several parts take the module's `Shared`, which core builds only after `compose` returns (core §7.3 step 4): the
accounts core's operations calls, the Reforged admin state, the chat source, the activity adapters, and the session
supplies if its constructor wants it. They read it from step 3's cell. Start-up 1 fills the cell before any listener
binds, so no request sees it empty; a call that still finds it empty answers as a module without a database (the
default policy, no activity, "not running" for chat). Matchmaking's `MatchmakingCore::dedicated_servers_configured`
reads core's dedicated-servers slot, which also lives on `Shared`; §15 item 5.

### 5.6 Adapters the composer writes (MIT glue, specified here)

#### 5.6.1 `AccountsCore` (rifts' fallback `IslesCore`; restricted's `AuthReforgedCore`)

Accounts cross as their userId text; a refusal is `SeamError::Refused { code, message }`.

| Method | Answer |
| --- | --- |
| `display_name(account)` | No store: none. Else one read: if the text parses as an account global id, the launcher account of that id: its display name. Else the game account with that userId: its launcher account's display name, else the game account's name. Else none. A store error: refusal `ACCOUNT_READ_FAILED` with its text |
| `find_by_display_name(name)` | No store: none. Else the launcher account whose normalized display name equals the name trimmed and lower-cased: its userId text. Store error as above |
| `launcher_account_eligible(account)` | No store: true. Else the launcher account found as in `display_name` (by global id, else through the game account's userId): auth's `is_account_eligible`. None found: **true** (§14 item 12). Store error as above |
| `party`, `get_or_create_party`, `set_candidate`, `cancel_travel`, `touch_game_session`, `record_hunt_activity`, `healthy_guard_session`, `published_guard_artifacts` | `rifts_seam::NoCore`'s answers (matchmaking answers the party ones before the fallback is asked) |
| `dedicated_servers_configured()` | false |
| `guard_mode()` | `GuardMode::from_setting(raw("RUSTIC_ISLES_GUARD_MODE"))`: empty or `observe` observes, anything else enforces |
| `operations_policy()` | the Reforged crate's `operations_policy_json(shared)` (reforged-public §9.4): the stored policy normalized, else the default, with `GUARD_ENFORCEMENT_OVERRIDE` exactly `OBSERVE` forcing `guard_enforcement` to `OBSERVE`; the JSON text |
| `verify_operations_step_up(key, code)` | the Reforged crate's `verify_operations_step_up(shared, key, code)`: the secret is `ADMIN_OPERATIONS_TOTP_SECRET` JS-trimmed when non-empty, else `ADMIN_TOTP_SECRET`; at most 6 tries per 15 minutes under the key `admin-operations-step-up:<key>`; true only with a secret, not limited, and a valid TOTP |
| `host_grant_catalog`, `stacked_items`, `accepted_inventory_replay` | the seam's defaults |

#### 5.6.2 The realms adapter (auth's `Realms` over rifts)

| Auth call | Rifts call and mapping |
| --- | --- |
| `authenticate_runtime(request)` | `rifts.realms().authenticate(RequestInfo)` with the request's path, headers (valid text only), peer and request id. `Authenticated(runtime)`: auth's `RuntimeCaller` with the runtime's roster and, in the opaque handle the characters unit adds to `RuntimeCaller` (characters decisions item 2), the `rifts_seam::RuntimeAuth` itself; `Refused(reply)`: `Refused { status, body }` from the reply; `NotMine`: `NotMine`; an error: `Err(<its text>)` (auth answers 500 and logs `rifts authenticate: ...`) |
| `launcher_policy_fields(account)` | `rifts.realms().launcher_policy_fields()`: each `(name, JSON text)` as given; a `SeamError::Refused` is auth's `LauncherError` with its code and message, any other error the `INTERNAL` launcher error |
| `authorize_game_session(account, realm)` | `rifts.entry().authorize_game_session_realm(<userId>, realm JSON text or none)`: some entry gives its realm id text, none gives none; `Refused` is `RealmError::Refused { code, message }`, other errors `RealmError::Failed(<text>)` |
| `bind_realm_presence(account, realm id)` | the realm id parsed as a `RealmId` (a failure is `Err(<its text>)`), then `rifts.entry().bind_realm_presence_at_login(<userId>, realm)`; an error is `Err(<its message>)` (auth answers 503) |

Under `Rifts::dedicated_only()` these answer `NotMine`, no fields, no entry and nothing, which is auth's default
behaviour too.

#### 5.6.3 Activity adapters (monitoring only; never fail a request)

| Port (owner spec) | Forwarded to |
| --- | --- |
| Inventory `Observer::committed(shared, tx)` (inventory §11.3) | bounties' `economy_ledger::commit_transaction(shared, tx)` (bounties decisions W9), then the Reforged `record_inventory_transaction(tx)` |
| Inventory `Observer::reforge_reset(shared, account, at)` | the Reforged `record_reforge(<user text>)` |
| Progression `Activity` (progression §9.5) | the Reforged `record_xp_grant(user, track, amount)` and `record_slayers_path_unlocks(user, keys, source)` |
| Bounties `ClaimActivity::challenges_claimed(user, n)` | the Reforged `record_challenge_claims(user, n)` (nothing for 0 or a blank id) |
| Matchmaking `HuntActivity` (set on the state only with a store) | the Reforged `dedicated_hunt_starts(users, hunt, ref)` and `dedicated_hunt_ends(users, reason)` |

#### 5.6.4 Bounties' `ForeignAccounts`

`foreign_account(shared, user)`: a global id text of kind account as it is, else the live local account's id found
by that userId; answered only when its owner node is not this node; a lookup failure is none (bounties §6.4).

#### 5.6.5 The Reforged admin half's `TesterFunding` (reforged-admin §15.3)

Over the economy crate: `is_approved_xp_track` and the curves (`track_cap_xp`, `rank_for_progress`,
`required_xp_for_rank`) from progression's `tracks`; `is_known_catalog_id` from inventory's `StoragePolicy`;
`max_xp_grant` progression's `MAX_PROGRESSION_GRANT_AMOUNT` (100 000); `apply_operator_test_stacks` through
inventory's `run_inventory_transaction` with `operator_test_funding` set, `NotOwned` and errors mapped to the port's
answers. Without a store the port is not built.

### 5.7 `ModuleHooks`

| Hook | Answer |
| --- | --- |
| `chat(shared, ctx, query)` | Chat's `IslesChat` over a `StoreChat` (built once from the shared state and the context's module id, chat §9.3) and the `HubSessions` slot, **with or without a database**: without one the rules answer for themselves (a `Jid` query is `Unsupported`, as restricted's test pins; core answers `Unavailable` before start and after stop) |
| `admin(shared, call)` | the Reforged admin state's `admin_action` (reforged-admin §10). Core has already refused without a database and validated a `Ban`'s `until` |
| `home_operation(shared, name, account, actor, body)` | For a name in core §7.8's table (`progression.grant`, `progression.read`, `inventory.write`, `inventory.read`, `bounty.write`, `bounty.read`, `character.load`, `account.write`, `chat.login`, `chat.player`, `chat.party`): the Reforged admin state's `home_operation` for the invocation the four parts make (it delegates `chat.*` to chat), its value as `Some`. Any other name: `Ok(None)` (core answers `NotFound unknown home operation {name}`) |
| `place_hunt_players(shared, players)` | matchmaking's `place_hunt_players` |
| `record_ring_instance(shared, instance, host, port, players)` | matchmaking's `record_ring_instance(shared, instance, players)` (host and port are not kept) |
| `player_offline(shared, user)` | matchmaking's `player_offline` (the party leave; errors ignored) |
| `stop()` | the store's `database().checkpoint()`; no store: `Ok`. Core logs a failure as `checkpoint: {error}` |

### 5.8 Start-ups, in this order

Core runs them before auth's own start and before the services bind (core §7.3 steps 5-6).

1. **Bind**: fill the late cell with `Shared`; put matchmaking's `HttpDirector::from_settings(settings)` into core's
   dedicated-servers slot (the Director service replaces it with its in-process one when it can launch, director
   §12.3); build the Reforged admin state (`ReforgedAdmin::new(shared, store, discord, tester funding)`, with a store)
   and the chat source; log the test multipliers' start-up line (`read_test_multipliers(settings).startup_line()`,
   warn when the multipliers are on, info otherwise; on the process log, as restricted does).
2. The Reforged admin half's `startup()` (reforged-admin §3.3's check).
3. The Reforged public half's `public_startups()` (the log and TTL purge jobs; only with a store).
4. Matchmaking's `startups()`: the QoS pong (its listener's `qos` and `qos_public_port` addresses), then the party
   outbox (only with a store).

### 5.9 Services and stop

The spec's services (§3.1) are core's to run. On stop core calls each service's `stopping()` (the Director lets go
of its game servers), aborts the start-ups' tasks, stops the QoS listener and calls the hooks' `stop()`.

---

## 6. The rifts composition (`rifts_with_core(settings, core) -> Rifts`, feature `realms`)

`Rifts::load(settings, parts)` builds the parts only when rifts-seam's `p2p_module_enabled` is true
(`RUSTIC_RIFTS_P2P_MODULE` not one of its "no" words); off, the module is dedicated-only and every `rifts` row is
unmounted (core §4.2). The parts, in this order:

1. **Realms database wanted**: `REFORGED_DB_FILE` or `REFORGED_DB_ENGINE` is set (through `rifts_seam::Settings::get`,
   that is `raw`: set-but-empty counts, §14 item 10).
2. **Node**: `REFORGED_NODE_ID` parsed as a node id, else `local` (§14 item 3).
3. **Open** (when wanted, with a node): `store::DatabaseConfig::from_lookup` over `raw` and `REFORGED_DB_FILE` (empty
   when unset); open `rifts_guard::SCHEMA` on it (the result ignored), then `RealmsDb::open(config, default options,
   node)` (an error: no realms database). The isles schema is already open from §5.1 step 1. A configuration error
   also means no realms database. No line is logged for either failure (§14 item 11).
4. **Clock**: rifts-realms' system clock. **Realms enabled**: `realms_enabled_or_default(settings)`
   (`RUSTIC_RIFTS_REALMS_MODULE`).
5. **Hosting policy**: `HostingPolicy::from_settings`; unless `RUSTIC_RIFTS_P2P_HOSTING` is set, its `enabled` is
   "realms enabled" (§14 item 1).
6. **With a realms database**:
   1. **Tickets**: `Es256Tickets::from_settings` (`P2P_ADMISSION_KEY_ID`, `P2P_ADMISSION_SIGNING_PRIVKEY_B64`,
      `P2P_ADMISSION_VERIFY_PUBKEY_B64`); when that fails, a fresh P-256 pair with key id `default-k1`, in memory
      only (§14 item 4). The private key never reaches a log or `Debug`.
   2. **Admission store**: `MemoryAdmissionStore` (§14 item 4). **Capabilities**: `HostCapabilities` over the realms
      database.
   3. **Hosting settings**: `HostingSettings::from_settings`; unless `RUSTIC_RIFTS_REALMS_HOSTING` is set, realm
      hosting is "realms enabled"; unless `RUSTIC_RIFTS_P2P_HOSTING` is set, the policy's `enabled` is too; when
      neither `P2P_ALLOW_ALL_ACCOUNTS` nor `P2P_CANARY_ACCOUNTS` is set and the trust tier is 0, `allow_all_accounts`
      is true (§14 item 2).
   4. **Expected artifacts** (asked at each use): `read_artifact_policy(settings, empty sources)`'s hashes when it
      succeeds; else, when the core's guard mode is `Observe`, hashes from settings: `game_exe` `P2P_GAME_EXE_SHA256`
      else the 1.14.7 executable's SHA-256 (§4.2); `internal_server` `P2P_INTERNAL_SERVER_SHA256`; `winmm_proxy`
      `P2P_WINMM_SHA256`; `steam_bridge` `P2P_TRANSPORT_MODULE_SHA256` else `P2P_STEAM_BRIDGE_SHA256`;
      `steam_runtime` `P2P_TRANSPORT_RUNTIME_SHA256` else `P2P_STEAM_RUNTIME_SHA256`; `admission_public_key`
      `P2P_ADMISSION_PUBLIC_XY_SHA256`; each missing one is 64 zeros. Else the error `artifact policy feed not
      configured` (§14 item 7).
   5. **Backend URL** for hosts: `METAGAME_URL`, else `BACKEND_PUBLIC_URL`, else `http://127.0.0.1:61000` (§14 item 8).
      **Host server UDP port**: `P2P_HOST_SERVER_UDP_PORT` trimmed as a port above 0, else 8791.
   6. **Hosting**: rifts-realms' `Hosting` over the database, the hosting settings, the clock, the core's operations
      view and guard sessions, a `RealmsRecovery` (database, core, clock, stderr log), the expected artifacts and a
      stderr log; with its runtime (capabilities, tickets, the admission store, the core, the node, the backend URL,
      the UDP port, realms enabled).
   7. **Directory**: `RealmDirectory` over the database, the core's accounts, hosting as its event sink, the clock,
      **its own** sliding-window limiter (a new `rustic_isles_auth::RateLimiter`, at the system clock, not the
      module's auth limiter; §14 item 13), random ids, `RealmLimits::from_settings` and a stderr log.
   8. **Control channel** over the database and clock.
   9. **Relay config**: `RelayConfig::from_settings`; without `P2P_TRANSPORT_KEY_SECRET` a fresh random secret made
      at start (at least 128 bits, held as `Secret`; §14 item 5).
   10. **Admission**: `Admission::new(tickets, admission store, hosting as its hunts, the transport secret, the clock)`.
   11. **Capability resolver** (for the routes where the HTTP layer leaves the caller anonymous): the capabilities'
       `check(header, session, scope, now)`. A realms refusal stays a refusal; any other error is the refusal
       `AUTH_UNAUTHORIZED` with the error's text as message (§14 item 9). The host peer is the record's host Steam id
       when non-empty, else rifts-realms' `derive_ice_peer_id(host account)`, read as a number, else rifts-admission's
       `derive_ice_peer_id`. The session is `seam_session_id(record session, node)`, else the refusal
       `P2P_VALIDATION_FAILED` `sessionId must be a valid session identifier.`. The generation is the record's, at
       least 1. The bridge instance id is the record's.
   12. **Admission routes** `AdmissionRoutes::new(admission, resolver, node)`.
   13. **Relay**: `RelayGrants { config, http: a provider client that always answers unreachable, usage: a new cache }`
       (§14 item 6), `Ice::new(hosting as its hunts, grants, hosting as its operations policy, the clock)`, and
       `RelayRoutes::new(ice, node)`.
7. **Without a realms database**: no directory, control, hosting or capabilities; rifts-admission's and rifts-relay's
   `stub_routes()`.
8. **Guard**: with a realms database `SqlGuardStore` over its database and `RealmsHuntSessions` (database, clock),
   else `StubGuardStore` and `NoHuntSessions`; `Guard::new` with `GuardConfig::from_settings`, that store and those
   sessions, `CoreOperationsPolicy` over the core, the core, rifts-guard's system clock and stderr log.
9. **Service**: `RealmsService::new` (directory, capabilities, hosting, clock, core, the policy of step 5, realms
   enabled, node); **routes**: `RealmsRoutes::new` (directory, control, hosting, capabilities, clock, realms
   enabled) and `GuardRoutes::with_store(guard, whether a realms database exists)`.
10. **Parts**: `realms` and `entry` both the service; `guard`; routes in the order admission, relay, realms, guard.

Without the `realms` feature, `rifts_with_core` answers `Rifts::dedicated_only()` and the capability is off.

**Precondition**: with rifts loaded, core fails the backend's build when a `rifts` row has no rifts route of the same
method and path text (core §4.2). The eleven `/mystic/v1/hunts/...` rows (B228-B235, B237-B239) have no such route in
dainty-lass's rifts crates, which serve `/rustic/v1/hunts/...` (D3). The composed module therefore cannot start with
rifts on until core's table is renamed (§7.3).

---

## 7. The host-capability header and the hunt routes (settled)

### 7.1 Evidence

| Side | Sends | Reads |
| --- | --- | --- |
| Game server runtime (`native/runtime/p2p/PlayerHost.cpp`, a runtime patch shipped into hosts' installs) | `x-mystic-host-capability` | — |
| rustic-rifts native transport (`admission_client.cpp`) | `X-Mystic-Host-Capability` | — |
| Launcher (`launcher/src-tauri/src/controller/realms.rs`, host-ready and ticket consume to `/mystic/v1/hunts/...`) | both `x-mystic-host-capability` and `x-rustic-isles-host-capability` | — |
| Restricted backend (`isles-core/src/http.rs` budget bucket; auth through rifts) and restricted rifts (`rifts-seam` `HOST_CAPABILITY_HEADER`, admission, guard, realms) | — | `x-mystic-host-capability` only |
| Node reference (`hostCapabilities.ts`, `hostRuntimeAuth.ts`, `hostRouteCensus.ts`) | — | `x-mystic-host-capability` |
| dainty-lass rifts (`rifts-seam` `HOST_CAPABILITY_HEADER`, `rifts-transport` `CAPABILITY_HEADER`) | `x-rustic-host-capability` | `x-rustic-host-capability`; routes `/rustic/v1/hunts/...` |
| dainty-lass core `rustic-isles` (`pipeline.rs`) | — | `x-mystic-host-capability` for the budget bucket, passed to the guard under that name; table rows and the budget prefix `/mystic/v1/hunts` |

Nothing reads `x-rustic-isles-host-capability`. Nothing sends `x-rustic-host-capability` today. spec-rd27-rifts D3
(maintainer decision 2026-11-02) renames the header and the routes, forbids aliases in the new code ("not even as
aliases"), and says server, launcher and hosted runtime cut over together.

### 7.2 Verdict

The composed module accepts **`x-rustic-host-capability` only**, and serves the hunt routes at
**`/rustic/v1/hunts/...` only**.

- Every reader of the capability inside the composed module is a dainty-lass rifts crate (runtime authentication,
  the admission resolver, the guard's bucket and route census), and each reads only the D3 name. Core's budget
  bucket must ask the guard with the same name the guard reads, or a valid host falls into the client bucket.
- Accepting both would need an alias in the rifts crates, which D3 forbids, and would leave the 501/404 split of §6's
  precondition unsolved for the paths.
- The cost is D3's: the MIT module cannot serve hosted hunts to today's runtime patch or launcher. It ships in the
  same cutover as a runtime patch and a launcher that send the new header to the new paths; until then restricted's
  AGPL composition (old names) stays the shipping one. Dedicated hunts, hubs and every non-hosted route are
  unaffected.

### 7.3 What changes outside this unit (seam requests, not this row's)

- **Core** (`rustic-isles`): `HOST_CAPABILITY_HEADER` becomes `x-rustic-host-capability`; the eleven B-rows'
  paths become `/rustic/v1/hunts/:sessionId/...` in place (registration order unchanged); the counted-first prefix
  becomes `/rustic/v1/hunts`. Core §15's "kept wire" list loses `x-mystic-host-capability` and `/mystic/v1/hunts/`.
  Its tests that send `x-mystic-host-capability` (`tests/pipeline.rs`) follow.
- **Characters** §15 item 1 already names `x-rustic-host-capability`; no change. Auth §4.7's bucket text follows core.
- **Restricted** (the cutover, hands-on): the runtime patch for `PlayerHost.cpp` (header, D3's pipe name and `MGE1`),
  the launcher's two request sites (send only `x-rustic-host-capability`, to `/rustic/v1/hunts/...`; drop
  `x-rustic-isles-host-capability`), and the hosting docs' rifts settings (`RUSTIC_RIFTS_*`, §8).

---

## 8. Settings

Read through core's settings (§2). None is new: no `host.toml` field, `setup` option or env line comes with this unit.
Rifts' own settings are their crates' (`P2P_*`, `RUSTIC_RIFTS_*`); listed here are those the composition itself reads.

| Setting | Reader | Secret |
| --- | --- | --- |
| `KEEP_WEAPON_ITEM_LEVELS` | hook 3 (§3.3) | |
| `REFORGED_DB_FILE`, `REFORGED_DB_ENGINE` and `store`'s other `REFORGED_DB_*` | §5.1 step 1 (`get`), §6 steps 1 and 3 (`raw`) | `REFORGED_DB_PASSWORD` |
| `RUSTIC_RIFTS_P2P_MODULE`, `RUSTIC_RIFTS_REALMS_MODULE` | §6 (through rifts-seam and rifts-realms) | |
| `RUSTIC_RIFTS_P2P_HOSTING`, `RUSTIC_RIFTS_REALMS_HOSTING` (presence only) | §6 steps 5 and 6.3; the D3 names of restricted's `RUSTIC_ISLES_P2P_HOSTING` and `RUSTIC_ISLES_REALMS_HOSTING` | |
| `P2P_ALLOW_ALL_ACCOUNTS`, `P2P_CANARY_ACCOUNTS` (presence only) | §6 step 6.3 | |
| `REFORGED_NODE_ID` | §6 step 2 | |
| `P2P_ADMISSION_KEY_ID`, `P2P_ADMISSION_SIGNING_PRIVKEY_B64`, `P2P_ADMISSION_VERIFY_PUBKEY_B64` | §6 step 6.1 | the private key |
| `P2P_GAME_EXE_SHA256`, `P2P_INTERNAL_SERVER_SHA256`, `P2P_WINMM_SHA256`, `P2P_TRANSPORT_MODULE_SHA256`, `P2P_STEAM_BRIDGE_SHA256`, `P2P_TRANSPORT_RUNTIME_SHA256`, `P2P_STEAM_RUNTIME_SHA256`, `P2P_ADMISSION_PUBLIC_XY_SHA256` | §6 step 6.4 | |
| `METAGAME_URL`, `BACKEND_PUBLIC_URL`, `P2P_HOST_SERVER_UDP_PORT` | §6 step 6.5 | |
| `P2P_TRANSPORT_KEY_SECRET` | §6 step 6.9 (through `RelayConfig`) | yes |
| `RUSTIC_ISLES_GUARD_MODE` | §5.6.1 | |
| `GUARD_ENFORCEMENT_OVERRIDE`, `ADMIN_OPERATIONS_TOTP_SECRET`, `ADMIN_TOTP_SECRET` | §5.6.1 (through the Reforged crate) | the two secrets |

Restricted reads `RUSTIC_ISLES_P2P_HOSTING` and `RUSTIC_ISLES_REALMS_HOSTING` for steps 5 and 6.3 while its rifts read
`MYSTICPARADOX_*`; the MIT composition reads the D3 names only, so a host's env file that set the old names must be
renamed at the cutover (a line in restricted's hosting docs; hostkit writes none of these).

---

## 9. Entry points and how the module is loaded

### 9.1 The library functions

- `version()`: `IslesVersion { hooks: Hooks, tables: &TABLES, routes: RouteSet::V1147 }`.
- `spec()`: the module spec of §3.1 with `version()`, the two services and a `VersionComposer` reading
  `version_data.json`. `spec_with(data)`: the same with given `VersionData`. Both answer `Result<ModuleSpec,
  ModuleError>` because the id must parse (`ModuleId::new`); the version crate's test proves they succeed.
- `module()`: `IslesModule::new(spec()?)`, as a `Result` (core's constructor also parses the game version).
- `rifts_with_core(settings, core)` (§6), public for restricted's and this crate's tests.

### 9.2 Restricted's host (compiled in)

Restricted's `host` pushes a factory answering this crate's module onto the controller's game modules (today
`isles_1_14_7::module()`), and its hidden `reforged-host backend` / `director` subcommands take `spec()` (or
`spec_with`) and replace `services` with only core's `BackendService` or only `DirectorService::new(DIRECTOR)` before
building the module. So `ModuleSpec`'s `services` and `composer` stay public fields (they are, in core at
`d54366d`), and `DIRECTOR` is a public constant. Restricted may call `spec_with(VersionData)` with its own values
instead of shipping the file.

### 9.3 The child executable

A binary `rustic-isles-1-14-7` in the version crate: `module_api::child::main(module)` over the module. stdout is the
frame pipe, so nothing prints there. If `module()` fails, the error goes to stderr and the exit code is 1.

### 9.4 The plugin (`rustic-isles-1-14-7-plugin`)

A `cdylib` (lib name `rustic_isles_1_14_7_plugin`) whose whole source is `module_api::export_plugin!` over the version
crate's module. Its `realms` feature (default on) passes through to the version crate's. `export_plugin!` needs a
module value, not a `Result`; module-api's library `open` builds it inside its unwind guard, so the one construction
failure that cannot happen (a constant id that does not parse) may surface as a panic carrying the error text, which
the core sees as a failed load (`dainty_plugin_v1` answers null), never as undefined behaviour. The workspace's
`panic = "unwind"` is what makes that hold.

### 9.5 dainty-lass's core

The core loads either artifact from `host.toml`'s `[[modules.external]]` (`id = "isles-1-14-7"`, `path`, `sha256`):
`plugin_host::is_library` picks the library driver for the cdylib and the child driver for the executable; the file
is checked against its pinned SHA-256 before anything in it runs, and the module's id must equal `id`
(`docs/security/plugin-host.md`). The core links no rustic-isles code itself.

---

## 10. The public seam

### 10.1 Shape

Two crates, MIT, in dainty-lass `crates/rustic-isles-1-14-7` (lib and bin) and `crates/rustic-isles-1-14-7-plugin`
(cdylib). The version crate depends on dainty-lass MIT crates only: `rustic-isles`, `rustic-isles-auth`,
`rustic-isles-store`, `rustic-isles-gamedata`, `rustic-isles-director`, `rustic-isles-characters`,
`rustic-isles-chat`, `rustic-isles-economy`, `rustic-isles-matchmaking`, `rustic-isles-reforged`, `rifts-seam`,
`module-api`, `store`, `service-kit`, and with the `realms` feature (default) `rifts-admission`, `rifts-relay`,
`rifts-realms`, `rifts-guard`; workspace `serde_json` (`preserve_order` in its own manifest), `tokio`, `tracing`,
`thiserror`. The plugin depends on the version crate (`default-features = false`) and `module-api`.

**Seam changes for the integration pass** (not this row's): both crates as workspace members and in
`[workspace.dependencies]`; the fuzz target's entry in `fuzz/Cargo.toml`; core's header and route rename (§7.3);
core's `quick_emoji_wheel` type (§15 item 2); and, under §15 item 1's option B, the starter manifest's move. The
migration guide §2 line this row reports: restricted `modules/rustic-isles/crates/isles-1-14-7{,-plugin}` and the
composition of `isles-core/src/module.rs` now live in dainty-lass `crates/rustic-isles-1-14-7{,-plugin}`.

### 10.2 Interface (names are the contract restricted's host and the tests are written against)

| Item | Contract |
| --- | --- |
| `pub const ID`, `DISPLAY_NAME`, `INVITE_ALIASES`, `DEFAULT_PORTS` | §3.1 |
| `pub struct Hooks` | implements `rustic_isles::VersionHooks` (§3.3) |
| `pub static TABLES: VersionTables` | §3.2 |
| `pub const DIRECTOR: rustic_isles_director::DirectorConfig` | §3.4 |
| `pub fn capabilities() -> module_api::Capabilities` | §3.1 (realms follows the feature) |
| `pub fn version()`, `pub fn spec()`, `pub fn spec_with(VersionData)`, `pub fn module()` | §9.1 |
| `pub struct VersionData` | §4.3: `from_json(&Value) -> Result<VersionData, String>`, `load(&GameDataRoot) -> Result<VersionData, String>` (core's required read); `character_data()`, `starter_kit()`, `legacy_starter() -> Option<LegacyStarter>` (db §14.3's type, built with the kit), `generator_inputs() -> Option<rustic_isles_gamedata::VersionInputs>` (when both generator members are present); `Debug` prints counts; `pub const DATA_FILE: &str = "version_data.json"` |
| `pub struct VersionComposer` | implements `rustic_isles::Composer` (§5); `reading_file()`, `with_data(VersionData)` |
| `pub struct AccountsCore` | §5.6.1; implements `rifts_seam::IslesCore` |
| `pub struct RealmsAdapter` | §5.6.2; implements `rustic_isles_auth::Realms` |
| `pub fn rifts_with_core(&Settings, Arc<dyn rifts_seam::IslesCore>) -> rifts_seam::Rifts` | §6 |
| bin `rustic-isles-1-14-7` | §9.3 |
| feature `realms` (default) | §6; off: dedicated only, `realms` capability false |
| feature `test-support` | a synthetic `VersionData` builder (made-up ids that satisfy §4.3's rules, never the real values) |
| plugin crate | §9.4 |

---

## 11. What stays in dainty-restricted

§1.3. After restricted's host links this crate, restricted keeps nothing of the module but `native/`,
`tools/RuntimeLoader`, `reference/`, the goldens and their harnesses, the `version_data.json` values (until §15 item 1
and characters decisions item 3 are ruled), and the host's compile-in and hooks.

---

## 12. Test cases

### 12.1 What may be copied

Nothing recorded. `isles-1-14-7/tests/*.rs` and `isles-1-14-7-plugin/tests/load.rs` are authored test code (described
below only); the goldens they read (`isles-core/tests/golden/*`) and the game-data fixtures stay in restricted. The
values of §4.2's inputs are not copied: tests use `test-support`'s synthetic `VersionData`.

### 12.2 Behaviour the authored tests fix

| Area | Cases |
| --- | --- |
| Manifest | validates; id `isles-1-14-7`, display name `Rustic Isles 1.14.7`, game version `1.14.7`; answers invites `reforged` and `1.14.7`, not `1.14`; `realms` equals the feature; `network`, `chat` true; `patch_set` `Accepts`; `unverified` false; ports in order `backend` 61000, `director` 61001, `qos` 61010, `game` UDP 8770-8777 |
| Tables | every value of §3.2; `build_id()` `647472_1.14.7_shipping`; `switch_default("qos_port")` `80`, `("reforge")` `on`, an unknown key empty |
| Hooks | §3.3's table byte for byte: session payload for `abc`; `huntpass_payload("season43")`; `objective_row("u", {o, 3, 1, c, m})` writes `"progress":3,"completed_count":1`; `survey_config` 200 and its body; entitlements `[]` → `{"entitlements":[]}`, one with `activatedDate` none and duration 30 → `{"entitlements":[{"activatedDate":null,"duration":30,"name":"n"}]}`; order of a two-entry list kept |
| Weapons at base level | unset → true; `true`, ` TRUE ` → false; `yes`, `1`, empty → true |
| Destination | §3.3.1's rows: Ramsgate with hub true and false → `CITY`/none; `Arena_TrainingDojo_01` hub true → `SHARED`, false → `ISLAND`; `trainingdojo` hub true → `ISLAND`; `Hunt_X` → `ISLAND` |
| Director | default settings give §3.4's line exactly |
| Version data | the synthetic document parses; each rule of §4.3 fails with its text (one test per rule); an unknown top-level member is refused; a document without `rewardCachePlan` has no generator inputs |
| Coverage | the routes the areas register (each crate's `routes()` where it has one) plus auth's are disjoint and equal §5.3's 227 rows; with realms loaded every `rifts` row has a rifts route of the same method and path (fails until §7.3 lands, then passes) |
| Module, no database | core's `drive` (core §13.3) compiled in and out of process (the bin) with `RUSTIC_ISLES_ADMIN_API=on`: every backend row answers per `sweep` with the mounts of §5.3, the Director's `/health/ready` 503 and `/api/instances` `200 []`, status `Ready`, unload |
| Chat | before start `Unavailable`; running without a database a `Jid` query is `Unsupported`; after unload `Unavailable` |
| Version data at start | a database wanted and no file: start fails with core's required-file text naming `version_data.json`; a bad file: fails with the file's error; no database: starts without the file |
| Rifts | `RUSTIC_RIFTS_P2P_MODULE=off`: `loaded()` false, a `rifts` row 404; with a SQLite database and rifts on (after §7.3): `loaded()` true; create a realm `horizon` as a host whose display name is `SlayerHost` → 200 and the realm's name and host display name; the listed realms have it; a capability-gated hunt route for an unknown session → `P2P_SESSION_NOT_FOUND`; the realm directory's limiter answers `AUTH_RATE_LIMITED` (429 on the launcher route) once its window is used up; without a database the admission and relay routes answer their stubs |
| Header | with rifts on: a request with `x-rustic-host-capability` lands in the guard's host bucket; one with only `x-mystic-host-capability` is a client request |
| Accounts core | no store: display name none, eligible true; a launcher account by global id and by userId; an unknown account eligible true; `RUSTIC_ISLES_GUARD_MODE` empty and `Observe` observe, `off` enforces; step-up with no secret false |
| Realms adapter | under dedicated-only: `NotMine`, no fields, no realm, `bind` with none ok |
| Plugin | after `cargo build -p rustic-isles-1-14-7-plugin`, `plugin_host::DylibModule::load` of the built library (pinned by its SHA-256, expected id `isles-1-14-7`): its manifest equals the compiled-in one as JSON; `register` gives routes; unload. A missing library fails the test with the build command in the message (a nested cargo would wait on the outer lock) |
| Features | built without `realms`: the capability is false and the module is dedicated-only |

### 12.3 How restricted replays its goldens against the composed module

Restricted keeps every golden (`characters`, `progression`, `inventory_reforge`, `bounties_store`, `game_data`,
`reforged_api`, `chat`, `request_budget`, the auth `golden.json` and `scenario.json`) and adds this crate as a git
dev-dependency at the integration rev. Each golden test:

1. Writes a temporary env file (a fresh SQLite `REFORGED_DB_FILE`, or `REFORGED_TEST_POSTGRES_URL`'s database when
   set, the golden's signing keys and game server key, `RUSTIC_ISLES_REPLAY_HEADERS=on` so pinning works, core §17
   item 1, and the area's switches) and a backend game data folder holding the golden's fixtures and restricted's
   `version_data.json` (or uses `spec_with` with restricted's values).
2. Builds `rustic_isles_1_14_7::module()` (or `spec_with`), drives it in process with core's test support on port 0,
   and replays the recorded requests against the started backend (and Director, and chat through the chat plugin
   where the golden has stanzas) over HTTP, with `rustic-isles-replay`'s library where the corpus is a gateway
   capture.
3. Compares with the recorded answers using the golden's own normalization. A difference is a bug on the MIT side
   (parity tests are the contract, AGENTS.md), except where a coordinator decision departs (gated replay headers,
   the departures listed in each area spec), which the golden records as expected.

The goldens that today call area internals (`areas::characters::golden::replay`, `areas::progression::testkit`) move
to the module's HTTP surface; the goldens themselves do not change.

---

## 13. Security behaviour (for `docs/security/rustic-isles-1-14-7.md` and RX3)

The version crate opens no listener of its own; it decides what core's listeners serve and what the module trusts.

- **Trust boundary is core's**: every service binds 127.0.0.1 (core §14); the gateway refuses the host-only prefixes
  core registers. The composition adds no route of its own and no host-only path.
- **Loading**: the library and the executable run only after the core checked their pinned SHA-256 and the id
  (plugin-host). A construction failure is a failed load, never a half-built module (§9.4).
- **Secrets**: the admission signing key (given or generated), the transport key secret, the step-up TOTP secrets,
  the database password and every settings value reach no log and no `Debug` (the composer, the adapters and the
  rifts parts print names only). The generated fallback admission key and transport secret live in memory only.
- **The capability header**: accepted under one name only (§7.2); its value is never logged (rifts' census tags only
  its presence).
- **Fail-open spots, listed as known weak spots** (parity, §14): an account rifts cannot find is eligible (§14 item
  12); with no allow-list and trust tier 0 every account may host (§14 item 2); the capability resolver echoes an
  internal error text to the client (§14 item 9); a realms database failure degrades silently to stubs (§14 item 11);
  the Observe-mode artifact fallback accepts all-zero hashes for unset roles (§14 item 7).
- **Input files**: `version_data.json` is read only from the configured backend folder by its fixed name, through
  core's loader; unknown members are refused; the values are identifiers handed to the store as data, never paths or
  SQL.
- **Replay headers** are core's to gate (core §17 item 1); the composition never turns them on.

**Fuzz target** (`fuzz/fuzz_targets/rustic_isles_1_14_7_version_data.rs`): arbitrary bytes through core's JSON reader
and `VersionData::from_json` never panic and either refuse with a `version_data.json:` text or give a value whose
invariants hold (every referenced id in `instanced`, nine quick chats, finite wallet amounts, unique instanced ids);
arbitrary text through `candidate_destination`, `huntpass_payload` and `objective_row` never panics and gives JSON of
§3.3's fixed shape with the input echoed unchanged.

---

## 14. Behaviour that looks wrong but is kept (parity)

1. **Hosting switches are overridden by the realms switch**: unless the otherwise unread `RUSTIC_RIFTS_P2P_HOSTING` is
   set (even empty), `P2P_HUNTS_ENABLED`'s policy flag is replaced by "realms enabled"; likewise realm hosting unless
   `RUSTIC_RIFTS_REALMS_HOSTING` is set.
2. **Open hosting by default**: with neither `P2P_ALLOW_ALL_ACCOUNTS` nor `P2P_CANARY_ACCOUNTS` and trust tier 0, every
   account may host.
3. **Rifts' node** comes from `REFORGED_NODE_ID` (else `local`), not the module's node identity, so on a networked
   node rifts ids may name another node than core's.
4. **Admission is not durable**: the admission store is in memory and a missing signing key is generated per start,
   so tickets and admissions die with the process.
5. **A missing transport key secret** is random per start: pair keys change at each restart.
6. **The relay provider is never reachable**: the provider HTTP client always answers unreachable, so a
   Cloudflare-issued TURN credential is never minted; TURN REST and STUN work.
7. **The artifact policy ignores the runtime feed** (empty sources), and in Observe mode unset roles default to 64
   zeros.
8. **The backend URL default** is `http://127.0.0.1:61000` even when the backend port was assigned elsewhere.
9. **The capability resolver's refusal** carries an internal error's text as its message.
10. **Rifts' "database wanted"** counts a set-but-empty variable, unlike core's (core reads `get`).
11. **A failed rifts database open** (guard schema or realms) is silent and leaves the stubs.
12. **Eligibility fails open**: no store, or no launcher account found, is eligible.
13. **The realm directory's limiter** is its own, at the system clock, not the module's replay-aware auth limiter.
14. **`KEEP_WEAPON_ITEM_LEVELS`** keeps levels only for exactly `true` (trimmed, any case); `1` or `yes` do not.
15. **`TrainingDojo`** is matched case-sensitively and only for realm hubs; elsewhere it is an `ISLAND` hunt.
16. **The bounty grant windows end on 2027-12-31**: after that the bounty component finds no season until the table
    is moved.
17. **`bootstrap_version` reads `starter-1.12-v5`** on a 1.14.7 module (stored in existing rows).
18. **One departure**: a broken `version_data.json` fails the start (§4.3) where restricted, holding the values in code,
    could not have a broken file; a correct file behaves as restricted.

---

## 15. Open questions

1. **The starter manifest's item lists (blocking for the characters wiring).** Core §5.2 ruled the tables wire facts
   the version crate commits; characters decisions item 3 keeps the same kind of value (`StarterKit`) an input. The
   24 stacked ids and nine quick-emoji ids sit in `VersionTables` (`&'static`), so they cannot come from a file
   without a seam change. Option A (recommended): the maintainer rules starter identifier lists (these, and
   `StarterKit`'s ids) wire facts, committed with a one-line justification, and only upstream amounts and captured
   blobs stay in `version_data.json`. Option B: the lists move from core's `StarterManifest` into characters'
   `StarterKit` (owned, loaded at compose; `version_data.json` `starterManifest`), a change to core §5.2 and characters
   §6.2 and §8.2 for their owners. Until ruled, the implementer builds everything else, leaves the two lists out of
   `TABLES`, and reports the characters wiring as blocked rather than guessing.
2. **Core's `quick_emoji_wheel`** is `Option<&'static str>`; characters reads nine texts. Proposed: the field becomes
   nine texts (or a slice). For the core owner.
3. **D3 cutover** (§7): the core rename is a seam change and a hands-on cutover (runtime patch, launcher, hosting
   docs). Until it lands, the composed module can run only with `RUSTIC_RIFTS_P2P_MODULE=off` or without the `realms`
   feature. The coordinator schedules it with the rifts integration.
4. **Restricted's odd rifts wiring** (§14 items 1-11): candidate decision rows. The strongest candidates for a fix
   are items 2 and 12 (fail-open), 3 (node identity: use `ComposeInputs.node`), 6 (a real provider client, which
   needs an HTTPS client the relay crate can take) and 7 (pass the Reforged runtime feed as the artifact source,
   reforged-public §21 item 12).
5. **`MatchmakingCore`'s `dedicated_servers_configured`** reads core's dedicated-servers slot, which lives on
   `Shared`, but its constructor takes no `Shared` (matchmaking §14.2). Proposed: matchmaking takes the composer's
   late cell (§5.5) or a `Shared` accessor; for the matchmaking owner.
6. **An MIT token seed list** (gamedata §12 item 3): a list of the catalog ids the MIT crates name (starter, grant
   windows, seasonal coin, tokens), reviewed by the maintainer, would let `inventory_storage_policy.json` be
   generated without restricted's seeds. Until then `storageTokenSeeds` comes from restricted.
7. **Who installs `version_data.json`**: restricted's setup copying it is the proposal; a hostkit `gamedata` step
   ("version data") would make it visible in `reforged-host gamedata`'s report. A hostkit row if wanted.
8. **The child executable's name** (`rustic-isles-1-14-7`) differs from the module id (`isles-1-14-7`); `host.toml`
   names the path, so either works. Kept as proposed unless the coordinator prefers the id.
9. **`ReforgedAdmin::home_operation` takes a `HomeInvocation`** while core's hook passes its parts (§5.7); the
   adapter rebuilds one. If the Reforged unit prefers the parts, the adapter shrinks.

---

## Coordinator decisions (2026-11-05, before implementation)

These settle the open questions and override anything above that disagrees.

- **Item 1: the starter manifest's identifier lists are wire facts (maintainer's ruling, 2026-11-05).** The 24
  stacked ids and the quick-emoji texts stay in core's `&'static` table, committed in dainty-lass; option A. The
  characters spec's decisions are amended to match.
- **Item 2:** core's `quick_emoji_wheel` becomes the list of nine texts characters reads (a core change).
- **Item 3: the D3 cutover lands in core now.** The 11 `/mystic/v1/hunts` rows become `/rustic/v1/hunts`, the early
  budget prefix follows, and the host capability is read from `x-rustic-host-capability` (rifts-seam's constant),
  with no aliases (D3, 2026-11-02). The runtime patch and launcher release that send the new name are the
  maintainer's hands-on work in dainty-restricted.
- **Item 4 and §14:** parity; the listed rifts wiring oddities are candidate decision rows.
- **Item 5:** `MatchmakingCore` reads `Shared` through the composer's late cell, as the other late-bound parts do.
- **Item 6:** later; restricted supplies the seeds.
- **Item 7:** restricted ships and installs `version_data.json`; a hostkit step is a later row.
- **Items 8-9:** as proposed (executable `rustic-isles-1-14-7`, module id `isles-1-14-7`; the composer adapts core's
  hook arguments into a `HomeInvocation`).

## Coordinator decisions, amended after implementation (2026-11-05): starter manifest wire facts

By the maintainer's ruling (decision item 1), these are wire facts committed in dainty-lass. They fill core §5.2's
`starter_manifest` (`rustic_isles::StarterManifest`) in `TABLES`, and replace §3.2's "values not in this spec". Order
matters in both lists, and each is given in order: the stacked ids are written to a new account's stacked items, and
appended to an account that lacks some, in this order (both are stored and read back as they are written); the wheel's
index is the in-game slot.

| Field | 1.14.7 value |
| --- | --- |
| `bootstrap_version` | `starter-1.12-v5` (§3.2, §14 item 17) |
| `stacked_item_ids` (24, in order) | 1 `TOKEN_DAILY_CHALLENGE_DRAFT`, 2 `QI_BASIC_FLARE_DURABLE`, 3 `EM_INTRO_BEGINNER_01`, 4 `EM_END_BEGINNER_01`, 5 `EM_PLAYER_BEGINNER_01`, 6 `EM_PLAYER_BEGINNER_02`, 7 `EM_PLAYER_BEGINNER_03`, 8 `EM_PLAYER_BEGINNER_04`, 9 `BNC_MESH_BEGINNER_00`, 10 `BNC_FABRIC_BEGINNER_00`, 11 `BNC_SIGIL_BEGINNER_00`, 12 `BNC_ANIMATION_BEGINNER_00`, 13 `BNC_VFX_BEGINNER_00`, 14 `DYE_GREEN04_DURABLE`, 15 `DYE_BROWN07_DURABLE`, 16 `EJ_SYSTEM_TYPING`, 17 `EJ_PLAYER_BOSUN_JOY`, 18 `EJ_PLAYER_KAT_PRAY`, 19 `EJ_PLAYER_MOYRA_MUSCLE`, 20 `EJ_PLAYER_GREGARIO_QUESTION`, 21 `EJ_PLAYER_MIDDLEMAN_HMM`, 22 `EJ_PLAYER_PRIYANI_WINK`, 23 `EJ_PLAYER_ZAI_ANGRY`, 24 `EJ_PLAYER_ZAI_SWEAT` |
| `quick_emoji_wheel` (9, in slot order) | 0 `EJ_PLAYER_BOSUN_JOY`, 1 `EJ_PLAYER_KAT_PRAY`, 2 `EJ_PLAYER_MOYRA_MUSCLE`, 3 `EJ_PLAYER_GREGARIO_QUESTION`, 4 `EJ_PLAYER_MIDDLEMAN_HMM`, 5 `EJ_PLAYER_PRIYANI_WINK`, 6 `EJ_PLAYER_ZAI_ANGRY`, 7 `EJ_PLAYER_ZAI_SWEAT`, 8 `""` (the empty text) |

1.14.7 has the wheel, so the list is the nine texts, never empty (empty means a version without quick emojis,
characters §6.3). Every wheel id is also a stacked id (17 to 24), so a new account holds what its wheel shows.
