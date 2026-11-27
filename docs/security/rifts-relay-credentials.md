<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: Rustic Rifts' relay credentials (RX3)

The checklist for what the server hands a co-op transport so its peers can reach each other: the ICE servers and TURN
credentials, the pair keys that bind a joiner to its host, and the ICE signals it carries between launchers, with the
relay-only mode. The code is `crates/rustic-rifts-relay` (`config.rs`, `credentials.rs`, `rules.rs`, `service.rs`,
`routes.rs`). The admission service's hub peer list in `crates/rustic-rifts-admission/src/service.rs` (`hub_peers`)
also hands out pair keys. Both are composed by `crates/rustic-isles-1-14-7/src/rifts.rs`. Written from the MIT code
(RD27); `SECURITY_REVIEW.md` indexes it.

This list also covers the TURN topics: the turn-rest shared secret, credential lifetimes and relay-only mode. The
server runs no TURN listener of its own. There is no turn-rs service in the host kit and no relay port range to open,
so there is no separate TURN checklist. The launcher's side of these values is [rifts-host.md](rifts-host.md), and
tickets and control frames are [rifts-transport.md](rifts-transport.md).

How to use it: before realms leave canary, copy this list into that commit, check each item in the code (not from
memory), link the code or test that shows it, and sign at the end with the date and the commit reviewed.

## What it is and where the boundary is

- **In, from a signed-in launcher (untrusted body, authenticated account):**
  `POST /launcher/v1/p2p/hunts/:sessionId/transport-config` and `.../signal` with a JSON signal.
- **In, from an admin:** `PATCH /admin/v1/operations/coop-transport` on the admin mount.
- **In, from settings (the host's, trusted):** the `P2P_` values `RelayConfig::from_settings` reads.
- **In, from Cloudflare (untrusted answers, when that provider is used):** `iceServers` and the analytics usage.
- **Out:** each transport config (ICE servers with a TURN username and credential, the peers with their pair keys,
  `forceRelay`), and the signals queued for the other launcher as `ICE_SIGNAL` control messages.

The crate listens on nothing and holds no `unsafe`. HTTPS to a provider is the module's `ProviderHttp`. The 1.14.7
module passes `Unreachable`, so today no Cloudflare call is ever made and only turn-rest can issue a TURN credential.

Fuzz target in `fuzz`: `relay_signal` (`parse_ice_signal` and `filter_relay_only` for each kind).

## Checklist

### Credentials

- [ ] The provider is `P2P_TURN_PROVIDER`: `cloudflare` (the default), `turn-rest` or `none`. A provider without its
  settings issues nothing, and the transport gets STUN only with `RELAY_NOT_CONFIGURED` (`RelayGrants::issuer`,
  `grant`; test `stun_only_until_configured`).
- [ ] The turn-rest secret (`P2P_TURN_REST_SECRET`) must be at least 16 UTF-16 units and comes with at least one valid
  `turn:` URL, or turn-rest is off (`Reader::secret`; test `providers_need_their_keys`). Cloudflare ids are 8–64
  alphanumerics and tokens 8–512 of a token alphabet (`is_account_like`, `is_api_token`).
- [ ] A turn-rest credential is the TURN REST scheme: username `<expiry seconds>:<tag>`, password base64 of
  HMAC-SHA1 under the secret (`mint_turn_rest`; test `turn_rest_credentials`). The TURN server checks the expiry; the
  backend keeps no list and can't revoke one early.
- [ ] A provider never sees an account id. The tag is the first 16 bytes of SHA-256 of `rustic-relay-tag-v1\n<id>`
  (`relay_account_tag`; test `relay_tag_hides_the_account`).
- [ ] Lifetimes: `relay_ttl_seconds` gives the hunt's time left plus ten minutes, at least 300 s, and never over
  the cap. The cap is `P2P_TURN_MAX_TTL_SECONDS` (default 10800) for a hunt and `P2P_REALM_HUB_TURN_TTL_SECONDS`
  (default 86400) for a hub. Both are bounded to 300–172800, and a value out of range or not a whole number falls
  back to the default (`Bounded`; tests `numbers_fall_back`, `month_window_and_ttls`).
- [ ] A config is issued only while hosting is switched on and not emergency-stopped, for an ICE session at the
  asked generation in an ICE-active state, to an account that has a role in it (`config_for`, `live_ice_hunt`,
  `resolve_role`). A hub member needs a live presence row.
- [ ] Each account may ask `P2P_ICE_CONFIGS_PER_HOUR` configs an hour (default 30, bounded to 1–1000; at least 120 for
  a realm session). A refused call is not counted (`HourlyCounter::allow`).
- [ ] A Cloudflare answer is read defensively: STUN and TURN URLs must pass `is_stun_url` and `is_turn_url`, port 53
  is dropped, and the username and credential must be 1–512 visible ASCII characters
  (`normalize_provider_ice_servers`; tests `provider_answers_in_both_shapes`, `bad_provider_credentials_are_skipped`).
- [ ] A monthly quota (`P2P_RELAY_MONTHLY_QUOTA_GB`) is enforced only with Cloudflare analytics. A figure that can't be
  read does not block (`over_quota`; test `quota_from_analytics_is_cached`).
- [ ] Every secret is a `rustic_rifts_seam::Secret`, which prints `Secret(<redacted>)`. A TURN credential is written
  out only by `IceServer::to_json`, and `summary_json` has no secret (test `summary_has_no_secret`). The provider
  trait's contract is to log neither the token nor the body.

### Pair keys

- [ ] A pair key is HMAC-SHA256 under `P2P_TRANSPORT_KEY_SECRET` of `rustic-ice-pair-v1`, the wire session, the
  generation, the host account and the joiner account. It is base64url with no padding and is derived on demand,
  never stored (`derive_pair_key`; test `pair_keys`).
- [ ] A configured secret must be at least 32 UTF-16 units, or it reads as unset. Unset, the 1.14.7 module makes a
  random one for each start, so pair keys change at every restart (`rifts.rs`).
- [ ] A joiner gets only its key with the host. The host gets one per active joiner: the hunt's seated members, or at
  most `MAXIMUM_HUB_PEERS` (64) live hub members, never itself (`ice_peers_for`, `active_joiners`; tests
  `roles_in_a_hunt`, `hub_members_come_from_presence`).
- [ ] Admission's `hub_peers` hands keys only to the hub's host, at the current generation, while ICE-active, and at
  most `PEER_LISTS_PER_HOUR` times a generation.

### Signals and relay-only

- [ ] A signal is accepted only with a non-negative safe-integer `generation`, a `toAccountId` of 8–64
  `[A-Za-z0-9-]`, a known `kind`, and `data` of at most 4096 bytes of printable ASCII and line breaks. `data` may
  be blank only for `GATHERING_DONE` (`parse_ice_signal`; test `signal_parsing`; fuzz target `relay_signal`).
- [ ] Signals go host to joiner and back only: never joiner to joiner and never to oneself (`signal_route_allowed`;
  test `routing_is_host_to_joiner_only`). The sender is the authenticated account, never a body field.
- [ ] Each sender may send 300 signals an hour per session generation, or 3000 for a hub's host. A filtered signal
  still counts. A queued signal lives at most 120 s and never past the hunt (`SIGNAL_LIFETIME_MS`).
- [ ] With `P2P_FORCE_SDR=true` (`force_relay`), every transport config says `forceRelay`, and `filter_relay_only`
  drops every candidate line that is not `typ relay`. A description keeps only its ICE credentials and options. A
  signal with nothing left is answered `FILTERED_RELAY_ONLY` and not delivered (test `relay_only_filter`).
- [ ] **Known gap:** in relay-only mode a `GATHERING_DONE` signal is passed on unchanged (`filter_relay_only` returns
  `data` as sent, and the fuzz target asserts exactly that). A launcher can put host or reflexive candidate lines in
  that kind's `data` and they reach the peer. The joiner's transport refuses a ready route that is not `route=relay`
  ([rifts-host.md](rifts-host.md)), but the addresses are still disclosed. Decide whether `GATHERING_DONE` data should
  be forced blank.

## Open points

- `P2P_TURN_PROVIDER` falls back to Cloudflare for any unknown word, and the module can't reach Cloudflare. A typo
  in `turn-rest` therefore turns the relay off without a warning beyond `iceRelayConfigured` in the start summary.
- `SECRET_SETTINGS` says `setup` keeps the four `P2P_` secrets in `secrets.env`, but the host kit's `SECRET_KEYS` has
  none of them, and `setup` never makes them. Today they reach the module only from the service's environment.
  Decide whether `setup` should write `P2P_TURN_REST_SECRET` and `P2P_TRANSPORT_KEY_SECRET`, and add them to
  `SECRET_KEYS` so they are kept out of logs and overrides.
- `TurnRelayPorts` (3478, relays on 49152–65535) is exported but used only by a test. No firewall rule or listener
  depends on it.
- There is no fuzz target for `RelayConfig::from_settings` or `normalize_provider_ice_servers`. Both are covered by
  unit tests only.

## Sign-off

| Date | Commit reviewed | Reviewer | Notes |
| --- | --- | --- | --- |
| | | | |
