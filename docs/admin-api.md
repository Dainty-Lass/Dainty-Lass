<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Admin API

A GraphQL API for the host's own tools, such as a Discord bot: kick, time out and ban players (and undo it), and
look up a player, the server and its hunts. It is the `admin` service (`crates/admin`). It listens on
`127.0.0.1:61003` only, wants its key on every request, and the gateway never forwards to it, so players can't reach
it. What it trusts and why is in [security/admin-api.md](security/admin-api.md).

## Running it

`dainty-host setup` makes its key and writes `.workspace/config/admin.env`. `dainty-host start` runs it in its
own process after the content server (every release's binary has it; `dainty-host admin serve --env-file <file>`
runs it alone). In Docker it is the `admin` service in `compose.yaml`. Nothing needs setting in `host.toml`.

What it needs, all written by setup:

| Variable | In | What |
| --- | --- | --- |
| `REFORGED_ADMIN_API_KEY` | `secrets.env`, `admin.env` | The key callers send as `Authorization: Bearer <key>` |
| `RUSTIC_ISLES_MANAGER_OPERATOR_SECRET` | `secrets.env`, `admin.env`, `backend.env` | Signs the API's calls to the backend's `/admin/v1` routes |
| `RUSTIC_ISLES_MANAGER_ADMIN=on` | `backend.env` | Lets the backend accept those signed calls |
| `REFORGED_ADMIN_KEY` | `secrets.env`, `admin.env`, `backend.env` | The backend's existing admin key, for the kick |
| `PORT`, `BIND_HOST` | `admin.env` | 61003, 127.0.0.1. Any non-loopback address stops the start |
| `BACKEND_URL`, `REFORGED_DIRECTOR_URL` | `admin.env` | `http://127.0.0.1:61000` and `:61001`. Loopback only |
| `REFORGED_ADMIN_API_TIMEOUTS_FILE` | `admin.env` | `.workspace/config/admin-timeouts.json`, which holds when each timeout ends |
| `REFORGED_MAX_HUNT_SERVERS`, `REFORGED_SERVER_MODE` | `admin.env` | From `host.toml`, for `huntStatus` and `serverStatus` |
| `REFORGED_GUARD_*` | `admin.env` | From `[guard]` in `host.toml`, only the ones it sets: when a destructive or bulk action waits (below) |

A host moving from an older setup gets the new secrets on the next `setup` (or `update`). The secrets it already has
stay as they are.

## The schema

The full schema, with every field's description, is
[crates/admin/schema.graphql](../crates/admin/schema.graphql) (`admin schema` prints it; a test keeps
the file current). In short:

```graphql
type Query {
  player(username: String!, game: String): Player   # null when there is no such account
  serverStatus: ServerStatus!                  # name, mode, registration, version, commit, uptimeSeconds, onlineCount
  huntStatus: HuntStatus!                      # maxHuntServers, running, free, hunts { title, players { username } }
  activeTimeouts: [ActiveTimeout!]!            # the timeouts this API will lift, soonest first: username, until, reason, by, at
  guarded(all: Boolean): [GuardedAction!]!     # the destructive and bulk actions that wait (below)
  history(filter: HistoryFilter, cursor: String, limit: Int): HistoryPage!   # admin: the change logs, newest first
  historyCheck: [HistoryCheck!]!               # admin: each log's hash chain, walked from its start
  revertPlan(selection: HistoryFilter!): RevertPlan!   # owner: what a revert would do; changes nothing
}

type Mutation {
  kick(username: String!, game: String): KickResult!     # userId, username, connectionsClosed, game
  timeout(username: String!, minutes: Int, until: String, reason: String!, game: String): Player!
  liftTimeout(username: String!, game: String): Player!
  ban(username: String!, reason: String!, game: String): Player!
  unban(username: String!, reason: String, game: String): Player!
  approve(id: String!): GuardedAction!         # a second operator approves a held action
  cancel(id: String!): GuardedAction!
  revertApply(planId: String!, selection: HistoryFilter): RevertDone!   # owner: applies exactly that plan
}

type Player {
  userId: String!  username: String!  status: AccountStatus!    # ACTIVE, BANNED, DISABLED
  restriction: Restriction       # kind BAN | TIMEOUT | DISABLED, reason, since, by, until
  online: Boolean!  location: Location    # kind HUB | HUNT | TRAINING | TUTORIAL, instanceId, title
  admin: Boolean!  createdAt: String  lastLoginAt: String  game: String!    # the game version, such as 1.14.7
}
```

Which game a player is in: the Node backend answers for 1.14.7 until a module for 1.14.7 is loaded, and each game
module that registers admin actions answers for its own game. The API asks every game for the name at once. A name
that more than one game has is refused with `AMBIGUOUS_PLAYER` until `game` names one (`game: "1.4.4"`); a game this
server doesn't run is `UNKNOWN_GAME`. A kick, ban or timeout waits for every game to answer: while one doesn't, it is
refused with `BACKEND_UNAVAILABLE` unless `game` is given.

For a player in a module's game, the module does the work and the answer carries what `module-api` reports: the
name, the account id, whether they're online and, when the module reports one, the ban, timeout or disabling that
keeps them out of play (`restriction`, with a timeout's `until`; `status` is then `BANNED` or `DISABLED`). Without
one `status` is `UNKNOWN` and `restriction` null; `location` is null, `admin` false and `connectionsClosed` 0. A
timeout is sent as a ban with its end, which the module keeps and ends itself: the 1.14.7 module lets a timed-out
player sign in to the launcher but not play. A module that doesn't register `unban` and `liftTimeout` answers
`NOT_SUPPORTED` to them. The module refuses to act on its own admins, which comes back as `BACKEND_REFUSED` with its
reason.

How the moderation works:

- **kick** closes the player's chat connections and revokes their sign-in tokens (`POST /dainty/api/Kick` on the
  backend). They aren't banned and can log in again with their key. Nobody has yet checked what the 1.14.7 client
  does after losing its chat connection mid-hunt.
- **timeout** bans the player with the reason `Timed out until <time>: <reason>`, which the player sees in the
  launcher. The API records the end time and lifts the ban within 30 s of it, also after a restart. `minutes` or
  `until` (RFC 3339), not both, ending within a year. It replaces an earlier timeout. It is refused with
  `ALREADY_BANNED` for a player who is banned for good or disabled, since lifting it would undo that too.
- **ban** bans until someone unbans, and replaces a timeout. The backend audits it as `manager:<operator id>` and
  revokes the player's refresh tokens. A game that is running stops at its next request, because the backend checks
  bans on every call.
- **unban** lifts a ban or a timeout, and **liftTimeout** lifts only a timeout.
- A timeout whose ban changed in the meantime (lifted, or replaced from the backend's dashboard) is only forgotten
  when it runs out, never lifted.
- Accounts with the backend's `admin` role are refused (`PROTECTED_ACCOUNT`).
- Names follow the launcher's rule (3 to 16 letters, digits or `_`) and match regardless of letter case. Reasons are
  3 to 400 characters on one line.

Errors are GraphQL errors with a code in `extensions.code`: `BAD_INPUT`, `NOT_FOUND`, `PROTECTED_ACCOUNT`,
`ALREADY_BANNED`, `NOT_BANNED`, `NOT_TIMED_OUT`, `BACKEND_UNAVAILABLE` (the backend, the Director or a game's module
didn't answer), `BACKEND_REFUSED` (its message says when the secrets don't match: run `setup` and restart),
`STATE_NOT_SAVED`, `AMBIGUOUS_PLAYER`, `UNKNOWN_GAME`, `NOT_SUPPORTED` (the player's module can't do that yet),
`FORBIDDEN` (the key's role is too low; see above), `GUARDED` (the action was held, not done: see Guarded actions),
`NOT_WAITING` (a guarded action that is not waiting), `SETTINGS_UNAVAILABLE`, `SETTINGS_UNREADABLE` and
`SETTINGS_REFUSED` (`logSettings` and `setLogSettings`: see [logging.md](logging.md#from-the-dashboard)), and for the history `PLAN_CHANGED`, `PLAN_EXPIRED`,
`HISTORY_UNAVAILABLE` and `PARTIAL_REVERT` (see [History and revert](#history-and-revert)). A
request refused before any GraphQL runs gets a 4xx with the same shape: 401 `UNAUTHENTICATED`, 403 `FORWARDED`,
400 `BAD_OPERATOR` or `BAD_REQUEST`, 415 `NOT_JSON`, 503 `KEYS_UNAVAILABLE` (the keys' database didn't answer; the
owner's key still works). A revoked or unknown key is 401 `UNAUTHENTICATED`.

## Keys and roles

One key per person, each with a role (DD23). `REFORGED_ADMIN_API_KEY` in `secrets.env` is the owner's key and
nothing about it changes: a server with only that key behaves as it always did, with every right. Everyone else gets
a key of their own, which the host (or a key that may) issues and revokes, so one person leaving is one revoke and
nobody else's key changes.

| Role | May |
| --- | --- |
| `viewer` | Read: `player`, `serverStatus`, `huntStatus`, `activeTimeouts`, `modules`, `module` |
| `moderator` | Viewer, and `kick`, `timeout`, `ban`, and `guarded`, `approve`, `cancel` (a held action: see Guarded actions) |
| `admin` | Moderator, and `unban`, `liftTimeout`, `loadModule`, `unloadModule`, `restartModule`, `adminKeys`, `issueKey` for viewer and moderator keys, `logSettings`, and reading the change logs: `history`, `historyCheck` |
| `owner` | Everything: `issueKey` for any role, `revokeKey`, `setLogSettings` (a collector receives every line the server logs), and the history revert: `revertPlan`, `revertApply` (a big one is held first: see Guarded actions) |

A call above the key's role is refused with `FORBIDDEN`; the message names the role it needs
(`unban needs the admin role; this key's role is moderator`) and `extensions.requiredRole` carries it. Nothing
changes, and the log gets the key's id.

From the host (no key needed; run it on the server):

```bash
dainty-host admin key issue --name Maya --role moderator   # prints RRA_<id>_<secret> once
dainty-host admin key list                                  # id, role, name, who issued it, active or revoked
dainty-host admin key revoke 58ef5419                       # its next request is refused
```

The key is shown once and only its SHA-256 is kept, so a lost key is revoked and a new one issued. One active key per
name. The owner's key is listed as `owner` and is changed in `secrets.env` (then `setup` and a restart), never
revoked here. In a container: `docker compose run --rm host admin key list --env-file /opt/dainty-lass/.workspace/config/admin.env`.

From the API: `issueKey(name, role)` (admin for viewer and moderator, owner for any), `revokeKey(id)` (owner) and
`adminKeys`. Revoking takes effect on the key's next request.

The change log (DD23) names the key: each admin API change is a row in `admin_actions` under `admin:<key id>`
(`admin:owner` for `REFORGED_ADMIN_API_KEY`), the keys' own changes are under whoever issued or revoked them
(`admin:<id>`, or `host:admin key issue`), and a game module is told the same actor with every call. The keys live in
the admin API's own database: `admin.db` next to the node's SQLite file, or the node's Postgres database.

## Guarded actions (DH6)

Some actions cannot be taken back with one more call, or do a lot at once. They do not run when they are asked. They
are written to the admin API database and wait, either for a **delay** to end or for a **second operator** to
approve, and then they run once, as the key that asked. Until then the person who asked and any owner can cancel
them.

| Action | Held when | Waits for (setting) |
| --- | --- | --- |
| `ban`, `timeout` | one key has made more than `ban_limit` of them in the last `ban_window_minutes` (10 in an hour) | `ban_guard` (a delay) |
| `revokeKey` | the key is an admin or owner key and not the caller own. Viewer and moderator keys go at once, so a leaked one is not live for ten minutes | `revoke_guard` (a delay) |
| the history revert (DH3b) | the plan has more than `revert_limit` changes (100) | `revert_guard` (a delay) |

Not guarded, on purpose: `kick` (nothing is lost), `unban` and `liftTimeout` (they give back), `loadModule`,
`unloadModule` and `restartModule` (a stopped game is started with one call and nothing is erased) and `issueKey` (it
adds access, and only the owner issues the high roles). Everything below the limits runs at once, exactly as before:
a server that never sets `[guard]` behaves as it did except for the bulk and destructive cases in the table.

**The owner waits too.** `REFORGED_ADMIN_API_KEY` is a key like any other here. A held action of the owner needs
the approval of a second key with the owner role, or of the host (below), or it waits out its delay. There is no way
around it through the API.

Each rule is `delay` (the action runs by itself `delay_minutes` after it was asked, 10 by default), `approval`
(another operator approves it; a delay is never approved early) or `off` (never held). An action nobody approved is
dropped after `expire_hours` (48), and so is one a stopped server could not run in that time; it is then `EXPIRED`.
An approver must hold **a different key** than the one that asked, with **at least the role of the key that asked**
and at least the role the action itself needs (an owner for a revoke or a revert); the approval of the same key is
refused. The action runs as the key that asked, so the change log names that key, and `admin_actions.approved_by`
names the second operator. If the key that asked was revoked while the action waited, or lost the role it needs, the
action does not run (`FAILED`, with the reason).

What it looks like to a caller: the call that is held answers **no data and a GraphQL error with the code
`GUARDED`**, which is not a failure but a receipt:

```json
{"errors": [{"message": "ban is held as 3f9c1a07d2b4e865: it runs by itself at 2026-10-27T12:10:00Z, unless it is canceled (cancel(id: \"3f9c1a07d2b4e865\"))",
  "extensions": {"code": "GUARDED", "guardedId": "3f9c1a07d2b4e865", "mode": "DELAY", "releaseAt": "2026-10-27T12:10:00Z",
                 "expiresAt": "2026-10-29T12:10:00Z", "needsRole": "moderator", "already": false}}]}
```

The same action asked again while it waits answers the first one (`already: true`). A bot that wants to show it
branches on `extensions.code`. The rest is three fields (all need the moderator role or higher):

```graphql
type Query {
  guarded(all: Boolean): [GuardedAction!]!       # waiting, oldest first; all: the latest 100, newest first
}
type Mutation {
  approve(id: String!): GuardedAction!           # a second operator; runs it now and answers how it went
  cancel(id: String!): GuardedAction!            # the key that asked, or an owner
}
# GuardedAction: id, kind, summary, state (PENDING APPROVED RUNNING DONE FAILED CANCELED EXPIRED), mode (DELAY APPROVAL),
# initiatedBy, needsRole, createdAt, releaseAt, expiresAt, approvedBy, canceledBy, endedAt, result
```

`NOT_WAITING` answers an `approve` or `cancel` of an action that already ran, was canceled or expired, or that waits
out a delay. An id is 16 hex digits.

From the host, which counts as an owner no key can be (so it may approve what an owner asked, and cancel anything):

```bash
dainty-host admin guarded list [--all] [--json]   # what waits, and for what
dainty-host admin guarded approve 3f9c1a07d2b4e865
dainty-host admin guarded cancel 3f9c1a07d2b4e865
```

These only record the decision, as `host:admin guarded approve` or `cancel` in the change log; the running server
carries an approved action out within 30 seconds, and a stopped one at its next start. The server GUI Host page
does the same through the launcher (`guarded_list`, `guarded_approve`, `guarded_cancel`).

**A restart keeps what waits**: the actions are rows in the admin API database. One the server was running when
it stopped is marked `FAILED` at the next start and never run again, because whether it took effect is not known;
look at the player or the key, and ask again.

The settings, in `host.toml` (each is left out for its default; nothing needs setting):

| Key | Default | What |
| --- | --- | --- |
| `[guard]` `ban_limit` | 10 | Bans and timeouts one key may make in the window before the next is held, 1 to 1000000 |
| `ban_window_minutes` | 60 | The window, 1 to 1440 |
| `ban_guard` | `"delay"` | `"delay"`, `"approval"` or `"off"` for the held bans and timeouts |
| `revert_limit` | 100 | A history revert of more changes than this is held |
| `revert_guard` | `"delay"` | The same three words |
| `revoke_guard` | `"delay"` | The same three words, for revoking an admin or owner key |
| `delay_minutes` | 10 | How long a delay waits, 1 to 1440 |
| `expire_hours` | 48 | How long an action nobody approved stays, 1 to 720 |

`approval` needs a second person with a key; on a server with one owner key nothing can approve, so it expires.
Leave the rules on `delay` until the team has two keys of the right role. `dainty-host setup` writes the settings
into `admin.env` as `REFORGED_GUARD_BAN_LIMIT`, `_BAN_WINDOW_MINUTES`, `_BAN`, `_REVERT_LIMIT`, `_REVERT`, `_REVOKE`,
`_DELAY_MINUTES` and `_EXPIRE_HOURS`, only the ones `host.toml` sets.

## History and revert

The change logs (DD23) behind a server's databases, read and undone through the API; the same engine as
`dainty-host history` and `revert` (`restricted/docs/hosting.md`, "Undoing a change"). Reading the logs needs the
`admin` role, since a row's before and after are in them; a revert needs the `owner`.

```graphql
{ history(filter: { actor: "admin:58ef5419", since: "2d" }, limit: 50) {
    entries { id database at actor table key op before after }   # newest first
    next } }                                                      # pass it as `cursor` for older changes

{ revertPlan(selection: { actor: "admin:58ef5419", since: "2d" }) {
    id actionCount conflictCount
    actions { database table key kind current target }            # kind: INSERT (put back), UPDATE, DELETE, SOFT_DELETE
    conflicts { table key reason by at } } }                      # rows that stay: reason "changed_since", ...

mutation { revertApply(planId: "<id>") { rowsChanged databases { database rows } conflictCount } }
```

- A filter's `actor`, `account`, `since` and `until` all have to match. An actor is `admin:<key id>`, `host:console`,
  `player:<account>` or `system:<task>` (a bare key id or `owner` works too); a time is RFC 3339, a date, or how long
  ago (`6h`, `2d`). `database` is `node` (the game), `admin` or `chat` (on Postgres, all `node`).
- `revertPlan` changes nothing and keeps the plan on the server for ten minutes (the last 16). `revertApply` applies
  that plan: it plans again under the write lock and runs only if the plan still has the same `id`, otherwise
  `PLAN_CHANGED` and nothing is written. If the server no longer has the plan, `revertApply` answers `PLAN_EXPIRED`;
  give it the same `selection` and it plans again and applies only if the result has `planId`.
- A row someone else changed after the selection did stays as it is and comes back in `conflicts` with `reason`
  `changed_since`, `by`, `at` and the `entry`. A row already as it was is not touched.
- The revert is logged as `admin:<key id>` (so it shows in `history` and can be reverted), and leaves a `revertApply`
  row in the admin API's record. A revert that spans databases is atomic in each; if a later one fails the earlier
  ones stay reverted and the error is `PARTIAL_REVERT`, naming them.
- A revert of more changes than `revert_limit` is held (see Guarded actions): `revertApply` answers `GUARDED` and
  nothing is written. When its delay ends, or a second owner approves, it plans the same selection again and
  applies it only if the plan still has `planId`, otherwise the held action ends `FAILED` ("the history changed
  since the dry run") and nothing is applied. A small revert runs at once.
- Lists are cut at 500 rows (`truncated`; the counts are the real ones). An admin key's `key_hash` is shown as
  `<hidden>` in every row.
- `historyCheck` walks each log's hash chain: `ok: false` and `broken` name the first entry edited or removed (DH5).

## Calling it

`POST http://127.0.0.1:61003/graphql`, with `Content-Type: application/json` and a body of
`{"query": ..., "variables": ...}`. The headers:

- `Authorization: Bearer <key>`: `REFORGED_ADMIN_API_KEY` (the owner's), or the person's own key
- `x-dainty-operator-id: <Discord user id>`: who the moderation is for, digits only. The backend's audit log records
  it as `manager:<id>`. Leave it out and it is `0`, the host.

From the server itself:

```bash
curl -s http://127.0.0.1:61003/graphql -H "Authorization: Bearer $REFORGED_ADMIN_API_KEY" \
  -H 'Content-Type: application/json' -d '{"query":"{ serverStatus { name onlineCount uptimeSeconds } }"}'
```

## Metrics

`GET http://127.0.0.1:61003/metrics` with any admin key (`Authorization: Bearer <key>`, no operator id) answers every
component's metrics in Prometheus' text format, read at the moment of the scrape:

| Metric | What |
| --- | --- |
| `dainty_http_requests_total`, `dainty_http_request_duration_ms` | Requests by `service` (`gateway`, `admin`), `route` (the gateway's upstream: `backend`, `content`, `ws`, or `node`, `network`, `enroll`, `refused`), method and status class; time to the response head |
| `dainty_component_phase`, `_restarts`, `_uptime_seconds` | Each service and module the controller runs |
| `dainty_process_cpu_percent`, `_memory_bytes`, `_uptime_seconds` | The host process (`role="host"`) and every process it started (`role="child"`): the game servers, and mongod or turn-rs when the kit runs them. CPU reads 0 at the first scrape |
| `dainty_players_online`, `dainty_hunts_active`, `dainty_hunts_max` | The server's own counts |
| `dainty_game_server_players`, `_max_players`, `dainty_director_up` | Each running game server, from the Director |
| `dainty_chat_messages_total`, `dainty_chat_dashboard_posts_total` | Chat lines by channel and filter verdict; posts to the staff dashboard |
| the backend's own, and `dainty_backend_metrics_up` | `/internal/v1/metrics`, fetched with `RUSTIC_ISLES_METRICS_TOKEN` (setup writes it to `admin.env`) |

A Prometheus job for it:

```yaml
scrape_configs:
  - job_name: dainty-lass
    authorization: { credentials_file: /etc/prometheus/dainty-admin-key }
    static_configs: [{ targets: ["127.0.0.1:61003"] }]
```

## A Discord bot calling it

The bot runs on the server itself, because the API is loopback only. A bot on another machine reaches it through an
SSH tunnel (`ssh -L 61003:127.0.0.1:61003 host`), never by changing `BIND_HOST`. Give the bot the key from
`secrets.env` in its own environment, and never put it in the bot's repository.

A held action (`GUARDED`) comes back as an error whose message already says what to do; the sketch shows it as
`Refused: ...`, and a bot that wants a nicer line reads `error.extensions.guardedId` and offers `/approve` and
`/cancel` the same way.

A discord.js sketch in the style of Dauntless Revived's `UndauntedDiscordBot`: `/status`, `/player`, `/hunts` for
everyone, and `/kick`, `/timeout`, `/ban`, `/unban` only for members who can ban in Discord.

```js
import { Client, Events, GatewayIntentBits, MessageFlags, PermissionFlagsBits, SlashCommandBuilder } from 'discord.js';

const API = 'http://127.0.0.1:61003/graphql';
const KEY = process.env.REFORGED_ADMIN_API_KEY;

async function admin(query, variables, operatorId) {
  const answer = await fetch(API, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${KEY}`, 'x-dainty-operator-id': operatorId },
    body: JSON.stringify({ query, variables }),
    signal: AbortSignal.timeout(15000),
  });
  const body = await answer.json();
  if (body.errors?.length) throw Object.assign(new Error(body.errors[0].message), { code: body.errors[0].extensions?.code });
  return body.data;
}

const name = o => o.setName('username').setDescription('Their launcher username').setRequired(true).setMinLength(3).setMaxLength(16);
const reason = o => o.setName('reason').setDescription('Shown to the player in the launcher').setRequired(true).setMinLength(3).setMaxLength(400);
const moderator = b => b.setDefaultMemberPermissions(PermissionFlagsBits.BanMembers);

export const commands = [
  new SlashCommandBuilder().setName('status').setDescription('Server status'),
  new SlashCommandBuilder().setName('hunts').setDescription('Running hunts'),
  new SlashCommandBuilder().setName('player').setDescription('Look up a player').addStringOption(name),
  moderator(new SlashCommandBuilder().setName('kick').setDescription('Drop a player now').addStringOption(name)),
  moderator(new SlashCommandBuilder().setName('timeout').setDescription('Ban for a while').addStringOption(name)
    .addIntegerOption(o => o.setName('minutes').setDescription('How long').setRequired(true).setMinValue(1).setMaxValue(525600))
    .addStringOption(reason)),
  moderator(new SlashCommandBuilder().setName('ban').setDescription('Ban until unbanned').addStringOption(name).addStringOption(reason)),
  moderator(new SlashCommandBuilder().setName('unban').setDescription('Lift a ban or timeout').addStringOption(name)),
].map(c => c.toJSON());

const PLAYER = `{ username status online location { title } restriction { kind reason until } }`;

const handlers = {
  status: async (_, op) => {
    const { serverStatus: s } = await admin(`{ serverStatus { name mode version onlineCount uptimeSeconds } }`, {}, op);
    return `**${s.name}** (${s.mode.toLowerCase()}, ${s.version}): ${s.onlineCount} online, up ${Math.floor(s.uptimeSeconds / 3600)} h`;
  },
  hunts: async (_, op) => {
    const { huntStatus: h } = await admin(`{ huntStatus { running free maxHuntServers hunts { title players { username } } } }`, {}, op);
    const lines = h.hunts.map(x => `• ${x.title}: ${x.players.map(p => p.username ?? '?').join(', ') || 'nobody yet'}`);
    return [`${h.running}/${h.maxHuntServers} hunts running, ${h.free} free`, ...lines].join('\n');
  },
  player: async (i, op) => {
    const { player: p } = await admin(`query($u: String!) { player(username: $u) ${PLAYER} }`, { u: i.options.getString('username') }, op);
    if (!p) return 'No such player.';
    const where = p.online ? (p.location ? `in ${p.location.title}` : 'online') : 'offline';
    const r = p.restriction;
    return `**${p.username}**: ${where}${r ? `, ${r.kind.toLowerCase()}${r.until ? ` until ${r.until}` : ''} (${r.reason})` : ''}`;
  },
  kick: async (i, op) => {
    const { kick } = await admin(`mutation($u: String!) { kick(username: $u) { username } }`, { u: i.options.getString('username') }, op);
    return `Kicked ${kick.username}.`;
  },
  timeout: async (i, op) => {
    const v = { u: i.options.getString('username'), m: i.options.getInteger('minutes'), r: i.options.getString('reason') };
    const { timeout } = await admin(`mutation($u: String!, $m: Int, $r: String!) { timeout(username: $u, minutes: $m, reason: $r) ${PLAYER} }`, v, op);
    return `${timeout.username} is timed out until ${timeout.restriction.until}.`;
  },
  ban: async (i, op) => {
    const v = { u: i.options.getString('username'), r: i.options.getString('reason') };
    const { ban } = await admin(`mutation($u: String!, $r: String!) { ban(username: $u, reason: $r) ${PLAYER} }`, v, op);
    return `${ban.username} is banned.`;
  },
  unban: async (i, op) => {
    const { unban } = await admin(`mutation($u: String!) { unban(username: $u) ${PLAYER} }`, { u: i.options.getString('username') }, op);
    return `${unban.username} can play again.`;
  },
};

const client = new Client({ intents: [GatewayIntentBits.Guilds] });
client.on(Events.InteractionCreate, async interaction => {
  const handler = interaction.isChatInputCommand() && handlers[interaction.commandName];
  if (!handler) return;
  await interaction.deferReply({ flags: MessageFlags.Ephemeral });
  try {
    await interaction.editReply({ content: await handler(interaction, interaction.user.id), allowedMentions: { parse: [] } });
  } catch (error) {
    // The API's messages are written for the host; NOT_FOUND, PROTECTED_ACCOUNT and the like read fine as they are
    await interaction.editReply(error.code === 'BACKEND_UNAVAILABLE' ? 'The server is not running.' : `Refused: ${error.message}`);
  }
});
client.login(process.env.DISCORD_TOKEN);
```

Register `commands` with `rest.put(Routes.applicationGuildCommands(appId, guildId), { body: commands })`, or the
create-or-patch pattern of `UndauntedDiscordBot/commands.mjs`. `setDefaultMemberPermissions` hides the moderation
commands from members who can't ban. A server admin can change that per role in Discord's integration settings,
and the API still never acts on an account with the backend's `admin` role.
