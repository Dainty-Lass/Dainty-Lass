<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# metagame

Dauntless Revived's metagame for Dauntless 1.4.4, the backend of a **legacy** server (RD5 reversed): accounts,
characters, inventory, progression, the store, matchmaking through the deploy server, parties, guilds, friends and
the XMPP chat. Copied from DR's `rust/services/metagame` at `24b7624` with its structure kept, so it diffs against
DR; the Reforged changes are listed at the end.

```text
metagame serve --env-file <metagame.env>     # runs until `shutdown` on stdin, Ctrl+C or SIGTERM
metagame migrate --env-file <metagame.env>   # creates or adopts the database, applies every migration, exits
metagame db ...                              # database tools (backup, check, journal mode, keys, invites)
metagame offline-save export|import ...      # the account key is read from stdin, never from the arguments
```

## Listeners

Both bind loopback only; the gateway is the only listener off loopback, and a `METAGAME_BIND_HOST` that is not
loopback stops the start.

| Listener | Setting | Port the host kit should write | Compiled default |
| --- | --- | --- | --- |
| HTTP (the game client, game servers, the launcher, admin tools) | `METAGAME_PORT`, `METAGAME_BIND_HOST` | 61010 | none: `METAGAME_PORT` is required, as in DR (empty: a port the system picks) |
| XMPP chat over WebSocket | `METAGAME_CHAT_PORT`, `METAGAME_CHAT_BIND_HOST` | 61099 | 61099, 127.0.0.1 |

61010 keeps clear of the reborn services (backend 61000, Director 61001, content 61002, admin 61003, allowlist
61005), so both games can run on one host.

## Routes

Every route DR's Node metagame had, in its order: the full table is
[`node/UndauntedMetagame/routes.json`](node/UndauntedMetagame/routes.json), read with `/undaunted/api` as
`/reforged/api`, and `tests/parity.rs::route_table_matches_node` keeps the Rust table equal to it. The gateway
forwards everything it does not answer itself; what it must refuse or keep back:

- `/reforged/api/*`: only `Register` (POST), `GetUserInfo`, `ServerStatus`, `RegistrationStatus` (GET, HEAD),
  `HuntRegion` (GET, HEAD, POST) and `DeviceSignals` (POST) from outside, as DR's gateway allowed
  (`PUBLIC_UNDAUNTED_API`). The rest are admin routes.
- `/internal/backup` and `/internal/key-recovery/:action`: host-local only (the metagame also refuses them from
  anything but a direct local caller).
- The header `x-mysticparadox-gameserver-apikey` (game servers): refused from outside.
- `x-reforged-gateway: <METAGAME_GATEWAY_SECRET>`: added by the gateway; the metagame trusts `X-Forwarded-For`
  only from loopback with that secret.
- The chat listener: a WebSocket upgrade on any path is a chat connection; any other request is a 404.

Game-facing names stay the game's: the 1.4.4 client's routes (`/account/api/...`, `/candidate/...`, `/QoS`...),
`x-dauntless-game-session-id` and `x-dauntless-connected-player-ids`.

## Settings

The metagame reads one `--env-file`, and the process environment wins over it (`service_kit::env`). Every setting
is DR's name with the prefix `METAGAME_` (`METAGAME_PORT` is DR's `PORT`); the code keeps DR's names and strips the
prefix once at start (`config::ENV_PREFIX`). An unprefixed variable is never read. Meanings and values in more
detail: DR's `docs/reference/configuration.md` at `24b7624`, under the unprefixed name.

### Server

| Variable | Default | Meaning |
| --- | --- | --- |
| `METAGAME_PORT` | required | HTTP port. Empty: a port the system picks |
| `METAGAME_BIND_HOST` | `127.0.0.1` | HTTP address; must be loopback |
| `METAGAME_NODE_ENV` | development | `production`: JSON log lines, `AUTH_MODE=NONE` has no effect, no stack traces |
| `METAGAME_LOG_LEVEL` | `info` | `trace`, `debug`, `info`, `warn`, `error`, `fatal`, `silent` |
| `METAGAME_LOG_REQUESTS` | on | `0` turns off the one line per request (path only, never the query) |
| `METAGAME_DB_FILENAME` | required | The SQLite database; its folder must exist. Migrations run at every start |
| `METAGAME_STORE_IMAGES_DIR` | `store-images` | Folder of store icons (`<catalog id>.png`, extracted from the 1.4.4 client); missing: no icons |
| `METAGAME_BACKUP_ROOT` | unset (off) | Where `POST /internal/backup` writes |

### Accounts and authentication

| Variable | Default | Meaning |
| --- | --- | --- |
| `METAGAME_AUTH_MODE` | required | `APIKEY` (account keys) or `NONE` (development: anyone logs in as anyone) |
| `METAGAME_AUTH_SIGNING_PRIVKEY_B64` | required | **Secret.** Base64 of the PEM RSA private key (PKCS#8) that signs session tokens (RS256, 24 h) |
| `METAGAME_AUTH_SIGNING_PUBKEY_B64` | required | Base64 of the matching PEM public key (SPKI) |
| `METAGAME_REGISTRATION_MODE` | unset (registration answers 500) | `NONE`, `INVITECODE` or `OPEN` |
| `METAGAME_ACCOUNT_KEY_RECOVERY` | off | `1` keeps an encrypted copy of each player's key for admin read-back |
| `METAGAME_KEY_RECOVERY_SERVICE_SECRET` | unset (off) | **Secret**, at least 32 characters: DR's Discord bot's key recovery (`/internal/key-recovery`) |
| `METAGAME_DISCORD_KEY_STATE_FILE` | unset | DR's Discord bot's state file, for `DiscordKeyStats` |
| `METAGAME_DEVELOPER_ACCOUNT_IDS` | empty | Comma-separated account ids marked as developers in the account directory |
| `METAGAME_DEVICE_SIGNAL_HMAC_KEY` | unset (claims refused) | **Secret**, at least 64 hex digits: the key the launcher's device claims are hashed with |
| `METAGAME_ACCOUNT_DISPLAY_NAME` | on | `0`: `displayName` is `{}` again, as upstream sent it |
| `METAGAME_ACCOUNT_MAPPING` | on | `0`: `POST /account/mapping` maps nothing |
| `METAGAME_ACCOUNTINFO_PUBLIC_LEGACY` | off | `1`: upstream's `POST /accountinfo/public` reply (a rollback) |
| `METAGAME_VERIFY_STUB_ACCOUNT` | off | on/off: the token check names the placeholder account for everyone (a rollback) |

### Public mode and origin

| Variable | Default | Meaning |
| --- | --- | --- |
| `METAGAME_GATEWAY_SECRET` | unset (public mode off) | **Secret**, the gateway's: turns on public mode; `x-reforged-gateway` must carry it. Needs `AUTH_MODE=APIKEY` |
| `METAGAME_GAMESERVER_ALLOW_FROM` | empty | Comma-separated IPs, besides this machine, allowed to present the game-server key |
| `METAGAME_QOS_TARGET_URL` | unset | The one region the client pings before matchmaking, `http://127.0.0.1:<port>/QoS` in public mode |
| `METAGAME_BACKEND_HEALTH` | off | `1` tracks request health for `GET /reforged/api/BackendHealth` |

### Server status

| Variable | Default | Meaning |
| --- | --- | --- |
| `METAGAME_SERVER_NAME` | `Dauntless Revived` | Name in the status replies and the in-game welcome (64 printable ASCII) |
| `METAGAME_SERVER_VERSION` | the crate's version | Version in the status replies (32 characters) |
| `METAGAME_GIT_COMMIT` | `unknown` | The commit the running code came from |
| `METAGAME_SOURCE_URL` | `https://github.com/mixutin/dauntless-revived` | The AGPL source offer: set it to this repository (or the host's fork) |
| `METAGAME_CONTENT_PORT` | unset | The content server's port, reported to launchers |
| `METAGAME_STATUS_EXTRA` | on | `0`: `/dauntless-status` answers the game's nine fields only |

### Matchmaking

| Variable | Default | Meaning |
| --- | --- | --- |
| `METAGAME_MATCHMAKING_MODE` | unset (every request refused) | `DEPLOYSERVER` or `DISABLED` |
| `METAGAME_DEPLOYSERVER_URL` | unset (every launch fails) | `host:port` of the deploy server, no scheme; keep it on 127.0.0.1 |
| `METAGAME_MATCHMAKING_DEPLOY_TIMEOUT_MS` | `240000` | 1000-900000: bound on one allocation request |
| `METAGAME_MATCHMAKING_CANCEL` | off | `1` answers the client's matchmaking cancel (experimental) |
| `METAGAME_TARGET_CHANGELIST` | unset | Build id sent to the client: keep `239827` |
| `METAGAME_PARTY_SOLO_STUB` | on | `0`: a party of one answers like a real party that isn't queued |
| `METAGAME_GERMANY_REGION`, `METAGAME_US_REGION` | off | `1` turns on that hunt region |
| `METAGAME_REGION_<MAIN\|GER\|US>_PROBE_HOST`, `..._PORT` | unset, port 443 | A region's probe endpoint; an invalid one fails the region list |

### Progression, inventory, history

| Variable | Default | Meaning |
| --- | --- | --- |
| `METAGAME_PROGRESSION_MODE` | real | `real` or `stub` (upstream's fake max ranks) |
| `METAGAME_PROGRESSION_REAL_ACCOUNTS` | empty | With `stub`: comma-separated account ids that get real progression anyway |
| `METAGAME_PROGRESSION_GRANT_CAP` | `5000` | Most XP one request adds to one track |
| `METAGAME_PROGRESSION_CONFIRM` | on | `off`: the rank-confirm route answers 404 again |
| `METAGAME_PROGRESSION_ALLOW_DELETE` | off | `1` lets game servers reset a progression track |
| `METAGAME_PROGRESSION_REPLAY_WINDOW_S` | `5` | Seconds a byte-identical repeated grant is answered, not added; `0` off |
| `METAGAME_PROGRESSION_CONFIRM_ENTITLEMENTS` | off | on/off: a rank confirm also grants the rank's permanent entitlements |
| `METAGAME_PROGRESSION_CONFIG_DIR` | unset (the bundled config) | Folder of `.json` progression paths overriding the bundled ones |
| `METAGAME_ACTIVE_HUNT_PASS` | `season09b` | The Hunt Pass an account gets when none is stored; must be a loaded track |
| `METAGAME_ENTITLEMENTS_DEFAULT` | `season09b_premium,season_premium_any,season_free_any` | Entitlements every real-mode account owns; empty means none |
| `METAGAME_ESCALATION_MODE` | `stub` | `real`: real Escalation saves |
| `METAGAME_ESCALATION_STRICT` | off | on/off: soft Escalation rules refuse (409) instead of warning |
| `METAGAME_INVENTORY_REFUSE_OVERSPEND` | off | `1` refuses (409) a transaction that removes more than the player has |
| `METAGAME_INVENTORY_REPORT_REMOVALS` | on | `0`: upstream's additions-only inventory reply |
| `METAGAME_PATROL_BONUS_DAILY` | `10` | Daily patrol bonus stack |
| `METAGAME_BOUNTY_TOKENS_DAILY` | `6` | Daily bounty tokens |
| `METAGAME_MIDDLEMAN_FUSION_GUARDS` | off | on/off: a finished Middleman exchange may only be claimed as started |
| `METAGAME_SAVE_HISTORY_KEEP` | `100` | Newest versions of each character's data kept for rollbacks (2 or more) |
| `METAGAME_SAVE_HISTORY_HOURLY` | `48` | Hours one version per hour is kept beyond those; `0` off |
| `METAGAME_SAVE_HISTORY_DAILY` | `30` | Days one version per day is kept; `0` off |
| `METAGAME_BALANCE_FROM_INVENTORY` | on | on/off: `/balance` and `/reconcile` report the currencies the character holds |

### Store and Trials

| Variable | Default | Meaning |
| --- | --- | --- |
| `METAGAME_STORE` | `off` | `free`: the free in-game store |
| `METAGAME_STORE_REPEATABLE_TOKENS` | off | on/off: the bounty-token bundle can be bought any number of times |
| `METAGAME_STORE_CATALOG_PROFILE` | full catalog | `curated30`: the curated 30-offer catalog |
| `METAGAME_STORE_RAMS` | off | `1`: webstore prices in Rams, the main shop only for launchers that speak it |
| `METAGAME_STORE_RAMS_TEST_ACCOUNT` | unset | The one account Rams pricing is on for |
| `METAGAME_STORE_RAMS_EVENT_TRANSPORT` | off | `1`: Rams prices sent as the Event01 currency the client can show |
| `METAGAME_STORE_WEEKLY_ROTATION` | off | `1`: with Rams, a weekly selection of up to three offers per category |
| `METAGAME_STORE_ROTATION_SEED` | empty | Seed of that weekly selection |
| `METAGAME_TRIALS_STORE` | on | on/off: Lady Luck |
| `METAGAME_MIDDLEMAN_STORE` | on | on/off: the Middleman's slots and weekly cells |
| `METAGAME_TRIALS_LEADERBOARDS` | on | on/off: the Trials leaderboard routes and run submissions |
| `METAGAME_TRIALS_SCHEDULE` | off | on/off: the tuning schedule advertises this week's Trials |
| `METAGAME_TRIAL_ROTATION_START` | `2019-07-18T18:00:00.000Z` | The first weekly Trials rotation |

### Social and chat

| Variable | Default | Meaning |
| --- | --- | --- |
| `METAGAME_MISC_ROUTES` | on | `0`: the friends list, block list and other routes 404 again, as upstream |
| `METAGAME_GUILDS` | on | `0`: the old guild stubs |
| `METAGAME_GUILD_MAX_MEMBERS` | `100` | Members per guild (1-10000) |
| `METAGAME_GUILD_INVITE_TTL_DAYS` | `7` | Days a guild invite stays open (up to 365) |
| `METAGAME_GUILD_NAME_DENYLIST` | empty | Comma-separated words refused in guild names, on top of the built-in list |
| `METAGAME_GUILD_RESERVED_NAMES` | on | `0` allows staff and project words in guild names |
| `METAGAME_GUILD_CREATE_ACTIVITY_FALLBACK` | off | `1`: a weaker check on guild creates |
| `METAGAME_SLAYER_LINKS` | on | on/off: the `/slayerlink` routes |
| `METAGAME_CHAT` | on | Unset, empty or `1`: in-game chat; `0` off |
| `METAGAME_CHAT_PORT` | `61099` | Chat listener port |
| `METAGAME_CHAT_BIND_HOST` | `127.0.0.1` | `127.0.0.1`, or `::1` outside public mode |
| `METAGAME_CHAT_NICK_CHECK` | `enforce` | `log` admits nicknames that fail the rules, with a warning (a rollback) |
| `METAGAME_CHAT_TRACE` | off | `1` logs every chat frame, redacted (first live runs only) |
| `METAGAME_CHAT_PRESENCE` | on | on/off: friends' online status |
| `METAGAME_EXPERIMENTAL_CHAT` | | No longer read; set, it only logs a warning |

### Diagnostics

| Variable | Default | Meaning |
| --- | --- | --- |
| `METAGAME_LOG_BODIES` | off | `1` appends request bodies of unfinished routes to `BODY_LOG_FILE`. Development only: the host kit must keep it off in public mode |
| `METAGAME_BODY_LOG_FILE` | `bodies.log` | Where `LOG_BODIES` writes; never rotated |
| `METAGAME_BODY_LOG_PER_PATH` | `0` (no cap) | Most lines per method and path in one run |

The secrets (`AUTH_SIGNING_PRIVKEY_B64`, `GATEWAY_SECRET`, `KEY_RECOVERY_SERVICE_SECRET`, `DEVICE_SIGNAL_HMAC_KEY`)
belong in the host kit's `secrets.env`, never in `host.toml` or on a command line.

## Data from DR's Node metagame

[`node/`](node/) holds the files of DR's Node metagame (`legacy/UndauntedMetagame`, and one from
`legacy/UndauntedDeployServer`) at `24b7624` that this crate builds in or tests against: the Drizzle migrations
an older DR database is brought forward with, the vendor JSON (catalogs, progression, Trials), `routes.json` and
the store art list. They are DR's, unchanged; a DR update replaces them. Not copied: the store icons (game art; the
tests use empty stand-ins named in `tests/fixtures/store-images.txt`) and the Node test sources (the
`tests/ported.rs` ledger skips without them). `tests/parity/record.cjs` and `tests/chatparity/record.cjs` re-record
from a DR checkout.

## Changes from DR

- Settings are read under `METAGAME_` (`service_kit::env::Env::strip_prefix`).
- Section 3 of `docs/port/migration-guide.md`: `/undaunted/api` is `/reforged/api`, `x-undaunted-user-api-key` is
  `x-reforged-user-api-key`, `x-undaunted-gameserver-apikey` is `x-mysticparadox-gameserver-apikey`,
  `X-Dauntless-Gateway` is `x-reforged-gateway`, and new account keys start `RRK_` (`UUK_` keys in an adopted DR
  database keep working, and both are redacted from logs).
- A non-loopback `BIND_HOST` is refused (DR only warned in public mode).
- Invites from `metagame invite` are `dauntless-revived://` ones (`Game::Legacy`) and read
  `METAGAME_DB_FILENAME`.
- The status answer keeps DR's `ServerStatus` shape (a local struct), since Reforged's adds fields and moves
  `contentPort`.
