<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: admin API (RX3)

The checklist for the admin API, `crates/admin` ([admin-api.md](../admin-api.md)). It is a new trust boundary:
a caller holding its key can kick, time out and ban players. `SECURITY_REVIEW.md` indexes it.

How to use it: before the admin API ships in a server release, copy this list into the release's review, check each
item in the code (not from memory), link the code or test that shows it, and sign at the end with the date and the
commit reviewed. An item that doesn't hold is a bug to fix before the release, not a box to leave open.

## What it is and where the boundary is

A GraphQL service for the host's tools, such as a Discord bot. It runs no moderation of its own. It calls the Node
backend's admin routes and the Director, so what it can do is bounded by what those routes do.

- **In, from callers:** HTTP on `127.0.0.1:61003` only. `POST /graphql` with `Authorization: Bearer <key>` (the
  owner's, or one per person, below), an
  optional `x-reforged-operator-id` (digits), and a JSON body. `GET /health` answers `{"ok":true}` without a key, for
  the host kit's ready probe. It reveals nothing.
- **In, from the host:** `admin.env` (written by `reforged-host setup` from `host.toml` and `secrets.env`) and the
  timeouts file it names.
- **In, from this machine's services:** the backend's answers (`/admin/v1/players`, `/admin/v1/online-players`,
  `/admin/v1/players/<id>/access`, `/reforged/api/Kick`, `/reforged/api/ServerStatus`) and the Director's
  `/api/instances`. They are local and trusted to be what they say, but each one is parsed defensively.
- **In, from game modules (DF4c):** the answers to kick, ban and lookup (`module_api::AdminResponse` or a
  `ModuleError`), through the controller's driver. A module out of process is another program: its frames are
  checked by `module-api`'s parser (fuzz target `module_frame`), and the admin API treats the text of its errors as
  untrusted.
- **Out:** signed `/admin/v1` calls (the manager-operator credential,
  `modules/rustic-isles/reference/1.14.7/ParadoxBackend/src/security/managerOperator.ts`), the admin key to `/reforged/api/Kick`, `AdminCall`s to game
  modules, and GraphQL answers to the caller.

### Which game a player is in

The admin API sends kick, ban and lookup to the game the player is in (`modules.rs`, `schema.rs` `locate`). The Node
backend answers for 1.14.7 until a module for 1.14.7 is loaded, so a server with no modules works exactly as before.
Each game module that registered admin actions is attached to the `AdminPlugin` core plugin by the controller; it
must answer lookups, and one module per game version answers admin actions. A registration names no accounts and
every game keeps its own until the network's directory (DD17), so the admin API asks every game at once whether it
has the name. A name found in two games is refused (`AMBIGUOUS_PLAYER`) until the caller names the game
(`game: "1.4.4"`).

Every call to a module carries `ActorContext { actor: admin { key: "<key id>" }, request: "admin-<ms>-<n>" }`: the id
of the key that asked (`owner` for `REFORGED_ADMIN_API_KEY`, DD23), and the request id is logged beside the operator
id, so a change in a module's log leads back to the request and the person who asked (DH2).

### Keys and roles (DH2)

| Role | May |
| --- | --- |
| viewer | Reads: `player`, `serverStatus`, `huntStatus`, `activeTimeouts`, `modules`, `module` |
| moderator | + `kick`, `timeout`, `ban` |
| admin | + `unban`, `liftTimeout`, `loadModule`, `unloadModule`, `restartModule`, `adminKeys`, `issueKey` for viewer and moderator |
| owner | + `issueKey` for admin and owner, `revokeKey`; the history revert (DH3b) requires `Role::Owner` through `schema::allow` |

- `REFORGED_ADMIN_API_KEY` (`secrets.env`) is the owner's key, id `owner`. Every other key is `RRA_<8 hex id>_<64 hex
  secret>` (256 random bits), stored as the SHA-256 of the whole key in `admin_keys` in the admin API's own database
  (`admin.db` beside the node's SQLite file; the node's database on Postgres). The key is printed once by
  `reforged-host admin key issue` (or returned once by `issueKey`) and is not recoverable.
- A request is authenticated on every call against the database, so `key revoke` refuses the key's next request. The
  owner's key is checked first and needs no database. An unknown id, a wrong secret and a malformed key are the same
  401, and only a key whose secret matches can learn it was revoked. A key's shape is checked before any lookup.
- The role is checked at the top of each mutation in `schema.rs` (`Admin::kick`, `ban`, `timeout`, `unban`,
  `lift_timeout`, `issue_key`, `revoke_key`, the module roots) with `allow`. A refusal is `FORBIDDEN`, names the
  required role, changes nothing and is logged with the key id, the role, the role needed and the action.
- Every write is `Database::write` under the actor that made it: moderation actions are `admin_actions` rows under
  `admin:<key id>`; issuing and revoking are under the issuer (`admin:<id>` or `host:admin key issue|revoke`). The
  row is written after the action succeeded; a write that fails is logged with all its fields rather than hiding the
  action.

Code: `crates/admin/src/` (`input.rs` holds every check of outside input, `app.rs` the HTTP side, `schema.rs`
the fields and what they call, `upstream.rs` the calls and the signature, `modules.rs` the module registry side,
`hmac.rs`, `timeouts.rs`, `config.rs`, `server.rs`). Tests: `tests/api.rs` runs the API end to end against a fake
backend and Director that check the signature and the admin key as the Node code does, and runs RD25's lookup, kick
and ban cases against the Node backend, a stub 1.14.7 module and a stub 1.4.4 module (`games`).

Fuzz target in `fuzz`:

- `admin_request`: the caller's headers and body, the query text (through async-graphql's parser), a username, a
  reason, a timeout's `minutes`/`until`, a `game`, a module's error text, and a Director answer. Checks that a
  request passes the header check only with exactly one `Bearer <key>` and no proxy header. Checks that an accepted
  name, reason or timeout end is within its rule, that an accepted game has one spelling, that a module's text comes
  out one capped line, and that an instance's fields stay within their caps. It also feeds the key parser: only a
  77-character `RRA_<8 hex>_<64 hex>` key has an id to look up, a key's name is one capped line and a role name
  round-trips. Also that a guarded action is found only by 16 lower-case hex digits and that a guard rule is one of
  `off`, `delay` or `approval` (DH6).

## Checklist

### Who can call it

- [ ] Every `BIND_HOST` entry is `localhost`, a `127.x` address or `::1`. Anything else stops the start, and there is
      no override (`config.rs`, `is_loopback_host`; `listens_and_calls_on_loopback_only`).
- [ ] A peer that is not loopback is refused even so (`app.rs`, `graphql`).
- [ ] The gateway never routes to port 61003: it forwards only to `GATEWAY_BACKEND_URL` and `GATEWAY_CONTENT_URL`.
      In Docker the `admin` service publishes no port (`compose.yaml`).
- [ ] A request carrying `Forwarded`, `Via`, `X-Real-IP`, `X-Reforged-Gateway` or any `X-Forwarded-*` header is
      403, before the key is looked at (`input.rs`, `check_headers`).
- [ ] The key is compared as SHA-256 digests with a constant-time fold, so neither its length nor where it differs
      shows (`hmac.rs`, `same_secret`). A repeated `Authorization` header is refused. Keys and roles: below.
- [ ] The key, the backend's admin key and the operator secret are at least 32 characters, or the start fails
      (`config.rs`). setup makes each as 32 random bytes in hex (`commands/setup/secrets.rs`).
- [ ] The body must be `application/json`, at most 16 KiB, and one request, not a batch. A browser page on the host
      therefore can't send a "simple" cross-site request, and the bearer key is needed in any case.
- [ ] GET runs no query: `/graphql` answers 405 to anything but POST.

### What a query can do

- [ ] Depth is capped at 6 and complexity at 200 (`schema.rs`, `build`). Each field makes at most a few backend
      calls; a name search reads at most 100 pages of 100.
- [ ] Usernames are checked with the launcher's rule (`protocol::account::is_valid_username`) before any call.
      Account ids sent on in a path or `?ids=` match `[A-Za-z0-9_-]{1,64}` (`is_account_id`). The search term is
      percent-encoded.
- [ ] Reasons are 3 to 400 characters, control characters turned into spaces (`input.rs`, `reason`). The backend
      caps them at 500 and the timeout prefix fits inside that.
- [ ] A timeout ends in the future and within a year. `minutes` overflow is refused (`timeout_end`).
- [ ] Accounts with the backend's `admin` role can't be kicked, timed out or banned (`schema.rs`, `target`).
- [ ] `activeTimeouts` reads the API's own `timeouts.json` and asks the backend nothing; it needs the same
      bearer token as every query.
- [ ] A timeout is refused for an account that is banned for good or disabled, so lifting it never undoes someone
      else's ban. When it runs out, it is lifted only if the account still carries its exact reason (`sweep`;
      `a_timeout_never_lifts_a_ban_made_since_or_one_it_did_not_make`).
- [ ] When the timeouts file can't be written, the ban just made is undone rather than left permanent
      (`schema.rs`, `timeout`). A damaged file stops the start instead of forgetting every timeout (`timeouts.rs`).
- [ ] The file is written whole to `<file>.tmp` and renamed. It lives in `.workspace/config`, next to `secrets.env`.

### Keys and roles (DH2) checklist

- [ ] A key is stored only as `SHA-256(key)`; the key is in no log line, no error, no list (`keys.rs`, `list`;
      `a_key_is_shown_once_and_only_its_hash_is_kept` scans the database files and the change log for it).
- [ ] The key and the hash are compared in constant time (`hmac::same_secret`), and only a key of the issued shape is
      looked up (`keys::key_id`; fuzz target `admin_request`).
- [ ] A revoked key is refused on its next request (`a_revoked_key_is_refused_on_its_next_request`).
- [ ] The owner's key cannot be revoked through the API or the command, and changing it is in `secrets.env`.
- [ ] Each role does only its table's rows, tested for every role and every mutation
      (`each_role_does_what_it_does_and_the_refusal_names_the_role_it_lacks`); an admin issues only viewer and
      moderator keys (`an_admin_issues_only_keys_below_its_own_role`).
- [ ] Every change names its key in the change log (`every_change_names_the_key_that_made_it`) and in the module's
      `ActorContext` (`a_module_is_told_which_key_asked`).
- [ ] A refusal is logged with the key id and never the key (`allow`, `app.rs` `authenticate`).
- [ ] `admin key` commands run on the host with `admin.env` and no key; they are as trusted as the host.

### Guarded actions (DH6) checklist

The destructive and bulk actions wait (DD23 layer 4): a ban or timeout over `ban_limit` an hour from one key,
`revokeKey` of an admin or owner key, and a history revert over `revert_limit` changes. They are rows of
`guarded_actions` in the admin database, written by `Admin::gate`, which each of those actions calls after its role
and input checks (`schema.rs`; `guarded.rs`, `guard.rs`). Not guarded, with the reason in `guarded.rs`: `kick`,
`unban`, `liftTimeout`, the module calls and `issueKey`.

- [ ] The owner is held like any key: there is no role or key that skips `gate`, and the owner key cannot approve its
      own action (`the_owner_waits_too_and_needs_a_second_owner`).
- [ ] An approval comes from a different key (`Caller.key`) with at least the initiator role and the action own role;
      the same key is `FORBIDDEN` and nothing changes (`approval_by_the_same_key_or_a_lesser_role_is_refused`). A delay
      is never approved early (`a_bulk_ban_waits_out_its_delay_and_then_runs_once`).
- [ ] The host kit (`admin guarded approve`) counts as the key `host`, an owner no key can equal. It is as trusted as
      the host, which can edit the database anyway; its decisions are `host:admin guarded <command>` in the change log.
- [ ] Cancel is the key that asked or an owner (`a_second_operator_cancels_a_bulk_action_while_it_waits`); a canceled,
      done, failed or expired action cannot be approved or canceled again.
- [ ] Executed once: a held action moves `pending`/`approved` to `running` in one conditional `UPDATE`; only the
      caller that changed the row runs it (`guard::claim`; `guarded_actions_on_sqlite` claims from eight tasks at once
      on both engines). One the server stopped in the middle of is marked `failed` at the next start and never run
      again (`an_action_the_server_stopped_in_the_middle_of_is_failed_and_never_run_again`).
- [ ] A restart keeps what waits (`a_restart_keeps_what_waits_and_runs_it_once`, `what_waits_survives_a_restart`).
- [ ] It runs as the key that asked, if that key is still active and still has the role (`Admin::execute`); a key
      revoked meanwhile fails the action (`a_held_action_does_not_run_for_a_key_that_was_revoked_meanwhile`). The
      change log names that key, and `admin_actions.approved_by` the approver (`APPROVED_BY`, set only while a held
      action runs).
- [ ] `params` hold what the action needs (a name, a reason, a key id, a plan id), never a key or a secret; the API
      shows `summary`, not `params`.
- [ ] An unapproved action expires (`expire_hours`); an expired one never runs, also if the server was down past it
      (`a_held_approval_expires_when_nobody_comes`).
- [ ] The thresholds are `host.toml` `[guard]`, written to `admin.env` by setup and read by `Config::from_env`, which
      refuses a value out of range (`config.rs` test, `host/tests/config.rs`). Left out, the defaults hold only the
      bulk cases (`everyday_moderation_is_immediate_and_old_installs_behave_as_before`).
- [ ] The count behind `ban_limit` is per key, from `admin_actions`: several keys each stay under it. A team that
      wants a ceiling on the whole server sets `ban_guard = "approval"` and a small `ban_limit`.

### Game modules (DF4c)

- [ ] A server with no module for 1.14.7 sends 1.14.7's kick, ban and lookup to the Node backend exactly as before,
      and asks no module when none is loaded (`schema.rs`, `locate`; `rd25_cases_on_the_node_backend`). Once a 1.14.7
      module is attached, the Node backend is never asked for a player
      (`rd25_cases_on_a_1_14_7_module_which_takes_over_from_the_node_backend`).
- [ ] `AdminPlugin::attach` refuses a plugin's admin actions, a game whose actions don't include lookup, and a second
      module for a game version already attached (`modules.rs`;
      `the_plugin_takes_game_modules_that_answer_lookups_one_per_version`).
- [ ] A module is sent only an action it registered, and each call is given up after 5 s (`AdminPlugin::call`).
- [ ] Every call carries the actor `admin:<key id>` (`admin:owner` for the owner's key) and a request id, valid by `ActorContext::is_valid`. The request id
      is logged with the operator id and the action (`module_act`; `sent_as_owner`). The operator id stays the
      caller's word, as for the backend.
- [ ] A name found in more than one game is refused with `AMBIGUOUS_PLAYER` until the caller names the game
      (`a_name_in_two_games_needs_the_game`). `game` must be a game version in its one spelling (`input.rs`, `game`).
- [ ] A kick, ban or timeout is refused with `BACKEND_UNAVAILABLE` while a game that could also have the name
      doesn't answer, so it never lands on a namesake. A lookup still answers with what it found. A name found nowhere
      while a game was silent is an error, not "no such player"
      (`a_game_that_does_not_answer_stops_an_action_but_not_a_lookup`).
- [ ] A module's error text reaches the caller as one line of at most 200 characters (`input.rs`, `module_message`),
      and the error kind decides the code. The details go to the log.
- [ ] A timeout in a module's game is a ban with `until`: the module lifts it, and `timeouts.json` holds the Node
      backend's timeouts only. `unban` and `liftTimeout` answer `NOT_SUPPORTED` for a module's player, since
      `module-api` has no request for them yet (`a_module_keeps_its_own_timeouts_and_lifts_nothing_yet`).
- [ ] What `module-api`'s answers don't carry (status, restriction, the admin role, location, connections closed)
      shows as `UNKNOWN`, null, false or 0 for a module's player, never as a guess. Refusing a module's admin is the
      module's job, and it answers `BACKEND_REFUSED` with the module's reason rather than `PROTECTED_ACCOUNT`.

### What it sends on

- [ ] The backend and Director URLs must be `http://` on a loopback address with no path (`config.rs`, `local_url`).
      The admin key and signatures never leave the machine.
- [ ] The `/admin/v1` signature is the manager-operator contract: HMAC-SHA256 over method, request target,
      timestamp, operator id and the body's SHA-256, joined by LF. It is checked against a value recorded from
      `managerOperator.ts` (`upstream.rs`, `signs_as_the_backend_verifies`). Timestamps never repeat, so two
      identical calls in one millisecond aren't refused as a replay (`next_timestamp`).
- [ ] The operator id is 1 to 32 digits, the caller's or `0`. The backend records it as `manager:<id>` in every
      audit record. It is the caller's word, not proof: anyone with the key can name any id.
- [ ] Every outgoing call has a 5 s limit and reads at most 1 MiB of answer.
- [ ] Backend answers are parsed into typed structs, and a shape that doesn't fit is `BACKEND_UNAVAILABLE`. The
      Director's list is checked field by field as `ParseDirectorInstances` does (`input.rs`, `parse_instances`).
- [ ] Error messages to the caller name the problem without the backend's addresses or keys. The details go to the
      log (`schema.rs`, `upstream`).
- [ ] Nothing logs a key, the operator secret or a signature. Moderation logs the account, the operator id and the
      reason.

### What changed on the backend's side

- [ ] `RUSTIC_ISLES_MANAGER_ADMIN=on` in `backend.env` makes `/admin/v1/*` accept the signed credential: from
      loopback only, with no forwarding header, a timestamp within 60 s, and a signature not seen in 120 s. The
      gateway already refuses `/admin` (`crates/gateway/src/policy.rs`).
- [ ] `RUSTIC_ISLES_MANAGER_OPERATOR_SECRET` is in `secrets.env` (owner-only), `backend.env` and `admin.env`;
      `setup` moves it from `MYSTICPARADOX_MANAGER_OPERATOR_SECRET` with the same value, and the admin API reads
      the old name for one release. The
      env files are owner-only too (`write_secret_file`).
- [ ] `POST /reforged/api/Kick` sits behind `HasAdminKey` (the admin key, a loopback peer, no gateway or
      `X-Forwarded-For` header). The gateway refuses it like every `/reforged/api` route not on its public list
      (`PUBLIC_REFORGED_API`).

## paradox/ change

RD2 makes fixes the baseline for `paradox/`, not a rule. The kick needs a way to drop a live session, and nothing in
the Node backend did that: `SessionRegistry` holds the chat connections, but no route closed them on demand. The Rust
backend can't start before its gate (RD17). So `paradox/` gained one small route (2026-10-24, decision RD25):

- `modules/rustic-isles/reference/1.14.7/ParadoxBackend/src/routes/reforgedApi.ts`: `POST /reforged/api/Kick {Username}`, behind `HasAdminKey`, answers
  `{UserId, Username, Closed}` (404 `not_found`, 400 `bad_request`).
- `modules/rustic-isles/reference/1.14.7/ParadoxBackend/src/launcherApiRules.ts`: `DisconnectAccount` revokes the account's refresh sessions and unused exchange
  codes, then closes each chat connection with WebSocket code 1008 `kicked by the host`. Tested in
  `launcherApiRules.test.ts`.
- The game's access token isn't revocable in the Node backend and stays valid until it expires. The kick doesn't
  ban, so the player can sign straight back in with their key. To keep someone out, use a timeout or a ban.
- RB7 ports this route with the rest of `/reforged/api`, and RB1's inventory has to list it.

## Changes to section 6

Section 6 keeps the host kit's `.env` files as the Node kit wrote them. This row adds to them and changes nothing
already there: two keys at the end of `secrets.env`, two lines in `backend.env` (after `REFORGED_ADMIN_KEY`), and a
new `admin.env`. `tests/node_fixtures.rs` still checks every Node-kit line byte for byte and leaves out only these.

DH6 changes no existing file: `admin.env` gains the `REFORGED_GUARD_*` lines only when `host.toml` has a `[guard]`
table, the admin database gains a table and a column through a migration, and the schema gains `guarded`, `approve`
and `cancel`. What does change for a client is that a ban or timeout over the limit, the revoking of an admin or
owner key and a big revert answer `GUARDED` instead of running (the owner included): that is the row, and the limits
are settings.

## Fuzzing

A short run of the target is in CI. The 24-hour run before a release that ships it, from `fuzz` (on Windows
the AddressSanitizer runtime must be on PATH, see `fuzz/Cargo.toml`):

```bash
cargo +nightly fuzz run admin_request -- -max_total_time=86400
```

Signed: ________ (date, commit)
