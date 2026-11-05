<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: Rustic Isles module skeleton (RX3)

The checklist for `crates/rustic-isles`, the MIT skeleton of dainty-restricted's Rustic Isles module: the pipeline in
front of every backend and Director route, the route table, settings, start and stop, the game data loader and the
replay clock (RD27, `docs/port/spec-rustic-isles-core.md`). The sign-in area it hosts has its own page,
[`rustic-isles-auth.md`](rustic-isles-auth.md). `SECURITY_REVIEW.md` indexes it once the integration pass adds the
row.

How to use it: before restricted's version crate composes this crate, copy this list into the change, check each item
in the code (not from memory), link the code or test that shows it, and sign at the end with the date and the commit
reviewed. An item that does not hold is a bug to fix first, not a box to leave open.

## What it is and where the boundary is

One dispatcher per listener answers every request (`pipeline.rs`): the backend (61000) and the Director (61001), both
bound to `127.0.0.1` (`module.rs`, `start_inner`). The gateway is the only public listener, and it refuses the
host-only prefixes this module registers (`HOST_ONLY_PREFIXES`). The backend does not check the gateway's secret:
the loopback bind is the boundary (spec §15 item 5).

- **In:** HTTP from the gateway; the peer from `ConnectInfo<ConnectionInfo>`, `x-forwarded-for` counted only from a
  loopback peer (auth's `client_ip`). Controller calls through `module_api::GameModule` (commands, admin actions,
  events, chat).
- **Out:** the composer's handlers and hooks (restricted's areas today), rifts' route handlers and guard
  (`rifts_seam`), the home API through the module context (`shared.rs`, `ContextHome`), and the admin authenticator
  slot (`AdminAuthenticator`) in front of the `/admin/` rifts rows.
- **Files:** the env file (and `backend.env`'s `admin.env` and `director.env` siblings) named by `host.toml`, and the
  game data files by fixed names under the configured folders only (`game_data.rs`).

Code: `crates/rustic-isles/src/` (`pipeline.rs`, `pattern.rs`, `routes/`, `body.rs`, `settings.rs`, `replay.rs`,
`game_data.rs`, `module.rs`, `shared.rs`, `handlers.rs`, `composer.rs`).

Fuzz targets in `fuzz`:

- `rustic_isles_routes`: any path (lossy UTF-8) against every row of the table never panics, and every Express row
  agrees with a reference model of §4.1 (segment counts, ASCII case-insensitive literals, non-empty params, decode
  failures refused). Regex rows run on the `regex` crate, linear in the path.
- `rustic_isles_pipeline_text`: the origin strip, the request id normalizer, `decodeURIComponent`, the `/admin` and
  `/rustic/v1/hunts` prefix checks, the switches reader, the per-minute parser and the admin API reader never panic;
  a stripped target starts `/`, an accepted host is an allowed apex or ends in `.` and one, and a kept request id is
  safe.

## Checklist

### Boundary

- [ ] Every service binds `127.0.0.1:<port>`, never a wildcard (`module.rs`).
- [ ] `x-forwarded-for` is honoured only from a loopback peer; another peer is its own address
      (`tests/pipeline.rs`, `client_addresses`).
- [ ] The host-only prefixes are registered as host-only routes (`tests/module.rs`, `the_manifest_and_registration`).

### Input

- [ ] The origin strip accepts only `steelyard.ca`, `steelyard.online`, `ol.epicgames.com`, `api.epicgames.dev` and
      their subdomains, 1 to 253 bytes of `a-z0-9.-`, no leading, trailing or doubled dot; a `decodeURIComponent`
      failure is refused; a refusal is an empty 400 before anything is counted (`strip_origin`).
- [ ] A stripped target always starts `/` and parses as a URI (`strip_origin`, fuzz).
- [ ] A request id is kept only when it is 8 to 96 characters of a fixed set, so a client cannot inject log lines or
      headers through it; else a fresh UUID (`safe_request_id`).
- [ ] A param that is not valid percent-encoded UTF-8 is an empty 400 (`pattern.rs`, `tests/pipeline.rs`).
- [ ] Bodies a handler reads: 100 KiB by default, JSON's first character `{` or `[`, UTF-8 only, identity encoding
      only (`body.rs`); rifts bodies are capped at 100 KiB before relay (`relay`).
- [ ] The Unstubbed-route warning logs the method and the path, never the query.

### Limits

- [ ] One count per request, before routes; the probes before `GET /internal/v1/metrics` are never counted; the
      hunts prefix is counted first and once (`tests/pipeline.rs`, `budget_placement`).
- [ ] Launcher CORS runs only after the count: a refused request is a 429 and never reaches CORS
      (`the_budget_counts_before_launcher_cors`).
- [ ] The hunt rows and the early-budget prefix are `/rustic/v1/hunts` and the capability header
      `x-rustic-host-capability` (D3); the old `/mystic/v1/hunts` and `x-mystic-host-capability` are not aliases
      (`rifts_rows_relay`), and a player's own hunt reads stay behind launcher auth under the new name.
- [ ] Buckets per client `/56` or IPv4 address, per game server address, per host capability as rifts' guard names
      it; a guard error or empty name falls back to the client bucket (`budget_buckets`).
- [ ] The limiter is auth's, bounded at 16 384 keys.

### Admin

- [ ] The admin API is off by default; when off every `/admin` path is an empty 404 before any admin row, and the
      admin rows are not mounted (`the_admin_gate`). A bad value fails the start.
- [ ] The `/admin/` rifts rows go through the `AdminAuthenticator` slot; with it empty they are refused 401
      (`admin_rifts_rows_ask_the_authenticator`).
- [ ] A `Ban` `until` is checked (RFC 3339, in the future) before anything else (`tests/module.rs`, `admin_actions`).
- [ ] A home invocation for an account homed elsewhere, or for an id that is not an account, is refused
      (`commands_and_events`).

### Secrets

- [ ] `Debug` of `Settings` prints names only, never values (signing keys, `API_KEY_HASH_SECRET`, `METAGAME_API_KEY`,
      SMTP and Discord secrets); a deliberate departure from restricted, whose derived `Debug` prints them
      (`settings.rs`, `debug_names_only`).
- [ ] No log line carries a setting's value; errors name the variable (`SettingsError`, the env file errors name the
      path and dotenvy's message).
- [ ] No legacy names: no `MYSTICPARADOX_*` env aliases and no `x-mysticparadox-*` header copies (spec §17).

### Game data

- [ ] Files are read only from the `game_data` section's folders, by the fixed names of `BACKEND_FILES` and the
      Director lists; nothing a client sends becomes part of a path.

### Known weak spots

- [ ] Replay headers: `x-isles-replay-now-ms` and `x-isles-replay-seed` pin a request's clock and make its ids
      predictable. They count only with `RUSTIC_ISLES_REPLAY_HEADERS` on (spec §17; off by default, ignored as if
      absent otherwise). A host that turns it on outside a replay target lets any player choose their budget window,
      daily rules and ids; the gateway should drop the headers from the internet (a separate gateway row).
- [ ] A game server key header selects the higher game-server budget (6000/min per address) before the key is checked.
- [ ] The season window saturates on an extreme time, so a replay-pinned `i64::MIN` or `i64::MAX` cannot overflow
      (`season.rs`, `extreme_times_saturate`).
- [ ] The cached game data read never refreshes (`load_backend_cached`); a changed file needs a restart.

## Sign-off

Reviewed at commit `________` on `________` by `________`.
