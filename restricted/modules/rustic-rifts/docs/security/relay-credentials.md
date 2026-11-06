<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: relay credentials (RX3)

> **Retired** (RD27): this checklist is for the AGPL `rifts-*` crate, which was replaced by the root's MIT
> clean-room crate and removed. Kept for reference; the root's `docs/security/rifts-*.md` is the live checklist.

The checklist for `crates/rifts-relay` (DaintyLass plan RF4, DD14, DD24, DX2): TURN credentials (`turn-rest` for
turn-rs and coturn, Cloudflare TURN), the per-pair transport keys, the ICE transport config and signal routes, and
the `coop-transport` admin route. The relay process itself (turn-rs, its listener exception and port range) is the
host kit's and has its page in DaintyLass (`docs/security/turn-relay.md`, DX2).

How to use it: before the 1.14.7 module serves realms from Rust, copy this list into the cutover notes, check each
item in the code (not from memory), link the code or test that shows it, and sign at the end with the date and the
commit reviewed. An item that does not hold is a bug to fix before the cutover, not a box to leave open.

## What it is and where the boundary is

A relay credential lets its holder push traffic through the server's TURN relay, which the host pays for in
bandwidth; a pair key decrypts one host-joiner pair's packets; a signal carries ICE candidates, which name IP
addresses. Whoever gets credentials beyond their share runs up the relay, whoever gets another pair's key reads
that pair's game, and whoever learns a candidate learns another player's address, which the Privacy Policy says
players do not see when `P2P_FORCE_SDR` is on.

- **In, launcher:** `POST /launcher/v1/p2p/hunts/:sessionId/transport-config` and `.../signal`, with an account
  isles authenticated; the body's `generation`, and the signal's `toAccountId`, `kind` and `data`.
- **In, admin:** `PATCH /admin/v1/operations/coop-transport`, behind isles' admin gate.
- **Secrets:** `P2P_TURN_REST_SECRET` (turn-rs' `static-auth-secret`), `P2P_CLOUDFLARE_TURN_API_TOKEN`,
  `P2P_CLOUDFLARE_ANALYTICS_API_TOKEN`, `P2P_TRANSPORT_KEY_SECRET`, all in `secrets.env` (`SECRET_SETTINGS`).
- **Out:** Cloudflare's credential and analytics APIs through `ProviderHttp` (isles' HTTPS client), realms' hunts,
  presence and control queue (`IceHunts`), realms' operations policy (`OperationsPolicy`), and the reply to the
  participant, the one place a credential and its pair keys are written out.

Code: `config.rs` (settings), `credentials.rs` (both providers, the quota, the grant), `rules.rs` (roles, pair keys,
signal parsing, the relay-only filter), `service.rs` (the three routes), `wire.rs`.

Fuzz target: `relay_signal` in `fuzz/` (signal bodies and the relay-only filter). It checks that an accepted signal
is bounded printable SDP and that the filter passes no candidate but a relay one and no line the sender did not
send.

## Checklist

### Credentials

- [ ] `turn-rest` is the TURN REST scheme turn-rs and coturn check: user `<expiry>:<relay tag>`, password
      base64(HMAC-SHA1(secret, user)); `tests/node_vectors.rs` matches Mystic's own output and the HMAC is checked
      against RFC 2202.
- [ ] The provider never sees an account id: the relay tag is the first 32 hex digits of SHA-256 of a fixed label
      and the account (`no_request_to_the_provider_carries_the_raw_account_id`).
- [ ] A credential lives the hunt's remaining time plus 10 minutes, within 300 seconds and the cap (3 hours, a hub's
      24), never longer than Cloudflare's 48 hours (`P2P_TURN_MAX_TTL_SECONDS` is bounded at 172800).
- [ ] Credentials go only to a participant of an active ICE hunt, at most `P2P_ICE_CONFIGS_PER_HOUR` per account
      (120 for a realm's sessions), and within the monthly quota when Cloudflare's analytics measure it.
- [ ] Provider trouble never fails the request and never logs a token: the answer carries STUN only, with
      `RELAY_PROVIDER_ERROR`; port-53 URLs and credentials outside printable ASCII are dropped
      (`provider_answers_both_shapes_port_53_dropped_bad_credentials_refused`).
- [ ] Every secret is a `Secret` (no `Debug`), and the startup summary names none (`the_summary_names_no_secret`).

### Pair keys

- [ ] A pair key is HMAC-SHA256(`P2P_TRANSPORT_KEY_SECRET`, label, session, generation, host, joiner), 43 base64url
      characters: derived, never stored, the same for host and joiner, different for every other pair and
      generation (`pair_keys_match_for_host_and_joiner_and_differ_between_pairs_and_generations`).
- [ ] The host gets every active joiner's key, a joiner only its own pair's; the hub's peer list (rifts-admission)
      goes to the hub's host only.
- [ ] Without a secret of at least 32 characters ICE is off (`P2P_ICE_UNAVAILABLE`).

### Signals and relay-only

- [ ] A signal is one of three kinds, to an account id of 8 to 64 letters, digits and dashes, at most 4096 bytes of
      printable ASCII and line breaks (`signals_are_well_formed_bodies_only`).
- [ ] Signals go only between the host and a joiner of the current generation, never between joiners or to
      oneself, 300 an hour per sender (3000 for a hub's host).
- [ ] With `P2P_FORCE_SDR`, a description keeps only the ICE credentials and relay candidates, a candidate signal
      only a relay candidate, and nothing is delivered when nothing is left
      (`relay_only_keeps_relay_candidates_and_the_ice_credentials_never_an_address`).
- [ ] The hunt's frozen transport decides, not the current policy: a dashboard switch never changes a running hunt.

## Open points

These do not block RF4 but must be settled, or accepted in writing, before the cutover:

- **The filter trusts the candidate type the sender wrote.** A modified client can label a host candidate
  `typ relay`; the other side then learns that address. Node has the same gap. Checking a relay candidate's address
  against the relay's own would close it, and needs the relay's public address in the config.
- **The rate counters are per process**, as Node's: a network with several backend processes multiplies the
  limits. The request rate limiter still covers the routes.
- **`ProviderHttp`** is isles'; it must apply the 5-second timeout, use rustls with aws-lc-rs
  (DaintyLass D10), follow no redirects and log neither the bearer token nor the body.
- **A hub keeps its credential for up to 24 hours**, because no transport exports `UpdateRelayCredentialH` (RF1,
  decision 6).

## Fuzzing

From `fuzz/`, on nightly (on Windows with `clang_rt.asan_dynamic-x86_64.dll`'s folder on PATH):

```bash
cargo +nightly fuzz run relay_signal -- -max_total_time=86400
```

## Changes from Node's relay code

- The settings are read once into `RelayConfig` rather than on every request; a change needs a restart, which the
  host kit's `update` does.
- Log lines and metrics are isles'; the grant reports its result label (`ISSUED` or the denial) for them.
- `mint-relay-file` (an example) is `tools/mint-relay-test-file.cjs` for `turn-rest`; the Cloudflare half stays in
  the cjs tool, which makes a live call.

Signed: ________ (date, commit)
