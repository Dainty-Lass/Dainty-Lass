<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the legacy metagame (RX3)

The checklist for Dauntless Revived's metagame, the backend of a legacy (Dauntless 1.4.4) server:
`rust/services/metagame` in the legacy-revived submodule (`modules/legacy-revived`, DS6), DR's at `24b7624` with this
project's catch-up (RD5 reversed); this checklist moves there with RV1. `SECURITY_REVIEW.md` indexes it. Its fuzz
targets (`metagame_chat`, `metagame_token`) left this repository's `rust/fuzz` with the code; RV1 carries them over.

How to use it: before the first legacy server goes public, copy this list into the cutover notes, check each item in
the code (not from memory), link the code or test that shows it, and sign at the end with the date and the commit
reviewed. An item that does not hold is a bug to fix first, not a box to leave open.

## What it is and where the boundary is

The metagame holds every legacy account: key hashes, session-token signing, saves, inventory, progression, the
store's receipts and the chat. Players reach it through the gateway (public mode) or straight over Tailscale
(private mode); the 1.4.4 game servers and the deploy server call it from the host. Whoever gets a game-server or
admin capability can rewrite any player's data; whoever gets the signing key can sign in as anyone.

- **In, HTTP:** `127.0.0.1:<METAGAME_PORT>` (61010 from the host kit). Players' requests come from the gateway, with
  `x-reforged-gateway` and `X-Forwarded-For`; game servers' from the host with `x-mysticparadox-gameserver-apikey`.
- **In, chat:** XMPP over WebSocket on `127.0.0.1:<METAGAME_CHAT_PORT>` (61099), behind the gateway.
- **Out:** the deploy server (`METAGAME_DEPLOYSERVER_URL`, loopback) and the region probes, nothing else.
- **Files:** the SQLite database, the backup folder, the body log (development only), the progression folder.

Code: `config.rs` (settings, the loopback rule), `origin.rs` (who is calling), `auth.rs` (the extractors every
route takes), `crypto.rs` (tokens, key hashes, recovery blobs), `redact.rs` and `bodylog.rs` (what reaches a log),
`limits.rs` (rate limits), `chat/` (the XMPP listener), `routes/backup.rs` and `routes/keyrotation.rs` (the two
host-local routes).

Fuzz targets in `rust/fuzz`: `metagame_token` (bearer tokens and recovery blobs never verify unless signed, and the
log redaction never lets an `RRK_` or `UUK_` key through) and `metagame_chat` (the XMPP frame parser, JIDs and the
nickname rule: no panic, linear time, nothing in the tree that was not in the input). Both are DR's (`auth_token`,
`chat_frame`), with the redaction check added.

## Checklist

### Who can reach it

- [ ] HTTP binds `METAGAME_BIND_HOST`, which must be loopback: anything else stops the start
      (`Config::from_env`; test `config::tests::required_settings_and_defaults`). DR only warned.
- [ ] Chat binds `127.0.0.1` (or `::1` outside public mode); anything else keeps chat off with an error
      (`chat::Settings::from_env`; test `reads_chat_chat_port_and_chat_bind_host_with_127_0_0_1_the_only_host_in_public_mode`).
- [ ] `X-Forwarded-For` is believed only from loopback with `x-reforged-gateway` equal to `METAGAME_GATEWAY_SECRET`,
      compared in constant time (`Origin::is_trusted_gateway`; test
      `the_gateway_is_trusted_only_from_loopback_with_its_secret`).
- [ ] Public mode (`METAGAME_GATEWAY_SECRET` set) refuses to start without `AUTH_MODE=APIKEY`, and warns on a short
      secret, a non-production `NODE_ENV` or a QoS URL off loopback (`Config::gateway_checks`; test
      `public_mode_checks`).

### Credentials

- [ ] The game-server key is accepted only from a direct local caller (loopback, the host's own addresses or
      `METAGAME_GAMESERVER_ALLOW_FROM`) and never through a proxy (`Origin::is_direct_local`, `auth.rs`; tests in
      `tests/m31/permissions.rs` and `gameserveronly.rs`).
- [ ] Admin keys never work through a proxy (`AdminKey`: 403 through a proxy).
- [ ] Account keys are stored only as SHA-256 hashes; a new key is `RRK_` and 48 hex digits from the OS RNG
      (`accounts.rs`, `offline_save.rs`), 64 for a recovery rotation (`keyrotation.rs`). `UUK_` keys in an adopted DR
      database still work.
- [ ] Session tokens are RS256 only, 24 hours, zero leeway, issuer and audience checked, and void once the
      account's epoch moves or it is banned (`crypto.rs`, `auth.rs`; tests in `tests/auth.rs`, `tests/crypto.rs`).
- [ ] `AUTH_MODE=NONE` has no effect with `NODE_ENV=production` (`AuthConfig::dev_logins`), and development
      logins refuse proxied requests.
- [ ] Recovery blobs (`ACCOUNT_KEY_RECOVERY=1`) are AES-256-GCM sealed under a key derived from the signing key,
      with the account id as associated data and a random nonce; a forged or moved blob never opens (`crypto.rs`;
      fuzz `metagame_token`).
- [ ] `/internal/key-recovery/:action` needs a loopback, unproxied caller and `x-key-recovery-service` equal to
      `METAGAME_KEY_RECOVERY_SERVICE_SECRET` (at least 32 characters, compared by hash); unset, it is off.
- [ ] `/internal/backup` needs a direct local caller and a valid game-server key, writes only under
      `METAGAME_BACKUP_ROOT` with a `YYYY-MM-DD_HHMMSS` name, and is off without the setting.
- [ ] The signing key, the gateway secret, the key-recovery secret and the device-signal key come only from the env
      file (host kit: `secrets.env`) and are never logged.

### What it logs

- [ ] Request lines carry the path only, never the query or a body, through `redact`: JWTs become `<token>`,
      `RRK_`/`UUK_` keys and any run of 64 or more word characters `<redacted>` (`redact.rs`; tests
      `matches_node`, `credentials_never_reach_the_log`; fuzz `metagame_token`).
- [ ] The body log is off unless `METAGAME_LOG_BODIES=1`, and the host kit never turns it on in public mode.
- [ ] Chat logs never carry message text, passwords or tokens, `CHAT_TRACE` included (`chat::xmpp::redact_frame`,
      `log_text`; tests in `tests/m31/chatwire.rs`).

### Untrusted input

- [ ] Every route takes one of the `auth.rs` extractors or is meant to be public; `tests/m31/permissions.rs`
      checks each route's refusals as Node's did.
- [ ] A panicking handler becomes a 500 (`CatchPanicLayer`), never a dead process.
- [ ] Chat: 32 KiB frames, 512 sockets, 8 unbound connections per address, 4 frames before login, 10 failed logins
      per address in 10 minutes, 2 bound sessions per account, 256 KiB of unsent output, per-connection stanza
      buckets (`chat/mod.rs` constants; tests in `tests/m31/chat.rs`). The frame parser is linear and refuses
      markup declarations (fuzz `metagame_chat`).
- [ ] Room nicknames must be the caller's own `<name>:<account id>:<resource>` unless `CHAT_NICK_CHECK=log`, and
      another account's id is refused either way (`chat/nick.rs`).
- [ ] Store icons are served only for `[a-z0-9_]+.png` names listed in the folder at start, so no path reaches
      the disk unchecked (`storeimages.rs`; test in `tests/m31/ladyluck.rs`).
- [ ] Rate limits (`limits.rs`) cover logins, token checks, player and admin mutations, device signals, store
      images, health reads and the backup route.

## Open points

These must be settled, or accepted in writing, before a legacy server goes public:

- **The gateway's list.** The metagame trusts the gateway to refuse `x-mysticparadox-gameserver-apikey` from
  outside, to keep `/internal/*` and the admin `/reforged/api` routes back (DR's `PUBLIC_UNDAUNTED_API`), and to
  forward chat only to the chat port. The legacy gateway routing must carry those rules.
- **Shared names with the reborn backend.** The legacy metagame now answers `/reforged/api/*` and reads the same
  header names as Mystic Paradox's backend; a host running both games needs the gateway to route by game, never by
  guess.
- **Body size.** HTTP bodies are capped at `service_kit`'s default of 50 MiB, as DR had; JSON is parsed whole.
- **Private mode.** Chat is not supported over Tailscale yet (DR), and with HTTP now loopback-only a private-mode
  legacy server needs the gateway (or another loopback forwarder) in front, as reborn servers have.
- **Key format.** New keys are `RRK_`; the legacy launcher path must accept `RRK_` keys for a legacy server, and
  DR's own launcher would see an `RRK_` key it did not make.

## Fuzzing

A short run of both targets happens on every PR (CI's `fuzz` job). The run before going public, from `rust/fuzz`:

```bash
cargo +nightly fuzz run metagame_token -- -max_total_time=86400
cargo +nightly fuzz run metagame_chat -- -max_total_time=86400
```

## Changes from DR

- Settings under `METAGAME_`; routes and headers renamed by the migration guide's section 3; `RRK_` keys.
- A non-loopback `BIND_HOST` is an error, not a warning.

Signed: ________ (date, commit)
