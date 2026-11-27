<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the launcher's network client (RX3)

The checklist for how the launcher talks to a game server: the invite that names it, the pinned TLS that proves it
is that server, the HTTP client every launcher request goes through, the account key header, and what it accepts
back. The code is:

- `crates/dainty-protocol/src/invite.rs`, the invite parser.
- `crates/dainty-relay/src/pinned.rs` and `endpoint.rs`, shared with the local relay. The launcher re-exports them
  from `net/pinned.rs` and `net/http.rs`.
- `crates/dainty-launcher-kit/src/net/http.rs` and `http/to_file.rs`, the client.
- `crates/dainty-launcher-kit/src/net/hostapi.rs` and `hostapi/parse.rs`, the requests and their replies.
- `net/versions.rs` and `dainty_epic::download::cdn`, the HTTPS-only fetches to third parties.

The local relay ([launcher-relay.md](launcher-relay.md)), region ping ([region-ping.md](region-ping.md)), the OAuth
redirect listener ([oauth.md](oauth.md)) and the runtime download ([runtime-download.md](runtime-download.md)) have
their own lists; this one covers the client they share. Written from the MIT code (RD27); `SECURITY_REVIEW.md`
indexes it.

How to use it: before a launcher release, copy this list into the release commit, check each item in the code (not
from memory), link the code or test that shows it, and sign at the end with the date and the commit reviewed.

## What it is and where the boundary is

- **In, from outside (untrusted):** invite links (pasted, or a `dainty:` or `dauntless:` link the OS hands over), and
  every byte a server answers: status, probe, register, user info, the login queue, the game ticket, playtime, OAuth
  replies, the runtime manifest, news, branding and its images, and the game version lists.
- **In, from third parties (untrusted):** the `https` profile and manifest sources in `versions.rs`, and Epic's CDN.
- **Out:** HTTP/1.1 requests, one per connection, to the endpoint the stored server names. The account key goes only
  in the `x-dainty-user-api-key` header (`KEY_HEADER`).

The launcher's client listens on nothing; listeners are the relay's and OAuth's. It holds no CA store for game
servers: a public server is trusted only by the SHA-256 of its certificate from the invite.

Fuzz targets in `fuzz`: `protocol_input` (invites, statuses, usernames, pasted keys, runtime manifests),
`launcher_hostapi` (every host API reply above, and a branding image's first bytes) and `launcher_oauth` (OAuth's
replies and the redirect listener's request head).

## Checklist

### Invites

- [ ] `parse_invite` refuses any unknown, empty, repeated or `%`/`+`-named query parameter, and refuses `v=1` with
  `mode` or `fp`, or `v=2` with `share` (`InviteError::Params`).
- [ ] A private invite (`v=1`) must name a loopback, Tailscale CGNAT (`100.64.0.0/10`) or `*.ts.net` host
  (`is_private_mode_host`). A public invite (`v=2`, `mode=public`) must carry a 64 lower-case hex `fp`.
- [ ] Hosts are lower-case DNS names or canonical dotted-quad IPv4 (`is_valid_host`): no IPv6, no userinfo, no
  numeric look-alikes such as `0x7f.1`.
- [ ] `dauntless-revived://` and Revived's 1.4.4 are a typed refusal (`InviteError::Revived`, RD5), not a second
  path. A saved server marked `revived` is refused by `switch_server`.
- [ ] Invite, status, username, key and manifest cases are shared tables in `crates/dainty-protocol/tests/cases/`, which
  the servers check too. Fuzzed by `protocol_input`.

### Pinned TLS

- [ ] `check_endpoint` runs before any connect: port 0, an invalid host, or a pin that is not 64 lower-case hex is
  refused. A public endpoint with no pin is refused, and only a private-mode host may go without one (test
  `endpoints_are_checked_before_connecting`).
- [ ] `pinned_connect` trusts exactly one certificate: `PinVerifier::verify_server_cert` compares the SHA-256 of the
  end-entity DER with the pin and ignores the chain, the name and the CA. The TLS 1.2 and 1.3 signatures are still
  checked, so a copied certificate without its key fails (test `a_matching_certificate_signed_by_another_key_fails`).
- [ ] The pin is checked in the handshake, before any request byte is sent. A mismatch is `HttpError::Pin` with the
  fingerprint the server showed, never retried, and never sends the key (tests
  `connects_when_pin_matches_and_rejects_mismatch`, `a_pin_mismatch_is_reported_and_no_route_sees_the_key`).
- [ ] A server that does not speak TLS is a TLS failure, not a pin mismatch, so the page never offers to accept a new
  certificate for it (test `a_server_that_does_not_speak_tls_is_a_tls_failure_not_a_pin_mismatch`).
- [ ] The connect and the handshake share one deadline (`CONNECT_TIMEOUT`, 10 s; test
  `the_deadline_covers_the_handshake`). An IP host is sent as `ServerName::IpAddress`, so no SNI is sent.

### The HTTP client

- [ ] One request per fresh connection (`http::open`). A redirect is answered, never followed (test
  `a_redirect_is_answered_not_followed`). The third-party fetches in `versions.rs` and `cdn.rs` are `https://` only,
  use the webpki roots, follow no redirect, and treat anything but 200 as a failure.
- [ ] Every request has a byte cap and a deadline over the whole exchange, connect included (`RequestOptions`;
  tests `a_body_over_the_limit_is_too_large`, `the_deadline_covers_the_whole_exchange`). The caps are set per route
  in `hostapi.rs`: 4 KiB for the queue, 15 MiB for a branding image.
- [ ] A cancel ends a request in flight, or before it starts (test `a_cancel_aborts_in_flight_and_before_starting`).
- [ ] A body-less GET or HEAD that fails on the way is retried once, to the same endpoint and pin. A timeout or a pin
  mismatch is never retried, and neither is anything with a body (tests
  `a_get_that_breaks_is_sent_once_more_to_the_same_server`, `a_write_is_never_sent_twice`).
- [ ] A download to a file (`http/to_file.rs`) creates nothing on another status or a declared length over the cap. It
  removes the file on a running total over the cap, an idle body, or a cancel (tests `another_status_writes_nothing`,
  `a_declared_length_over_the_limit_creates_nothing`, `a_running_total_over_the_limit_removes_the_file`,
  `an_idle_body_times_out_and_removes_the_file`, `a_cancel_mid_body_removes_the_file`).
- [ ] A path must start with `/` (test `a_path_must_start_with_a_slash`). `Host` names the port unless the endpoint
  is pinned on 443.

### The account key

- [ ] `key_headers_as` adds the key only when it is plausible and `check_endpoint` passes: over a pinned connection,
  or in clear only to a private-mode host (tests `status_carries_the_key_in_clear_only_to_loopback`,
  `status_over_pinned_tls_carries_the_key`).
- [ ] The routes that need the key refuse to send without the header. `oauth_link` and `upload_crash_report` answer
  `KeyInvalid` when `key_headers` is empty. `fetch_user_info`, `fetch_game_ticket` and `fetch_playtime` refuse an
  implausible key without asking (test `an_implausible_key_is_rejected_without_asking`).
- [ ] The key is never sent to the probe route, register, OAuth start or finish, the runtime manifest, news or
  branding (tests `a_missing_status_route_asks_the_probe_without_the_key`, `register_posts_the_request_without_a_key`,
  `content_routes_never_carry_the_key`). The Epic manifest route in `versions.rs` sends it only through `key_headers`.
- [ ] The key the client sends is the one `KeyStore::load` gave for this server's slot and pin
  ([keystore.md](keystore.md)). `key_headers` itself checks only that some valid pin is set, so this order matters.
- [ ] No header value reaches a log or a `Debug`: `RequestOptions` prints header names only, and failures log the kind
  and error, which never carry the key (test `debug_text_never_shows_header_values`; `logged_failure`). The one-time
  game ticket goes on the log's secret list as soon as it arrives.

### Answers from a server

- [ ] Status: a 200 must parse as a `ServerStatus`. A 404 asks the older probe route (`/dauntless-status`) without
  the key, and only a 200 object with `show-status` there reads as an older, unsupported server (`status_reply`,
  `probe_reply`).
- [ ] Register and OAuth finish take a key only from a 200 whose key is plausible. A register error message is
  cleaned and cut to 200 units before the page shows it (`register_reply`, `clean_text`;
  asserted in `launcher_hostapi`).
- [ ] User info: the id is 1–128 of `[A-Za-z0-9_-]` and the username at most 32 UTF-16 units (asserted in
  `launcher_hostapi`).
- [ ] News is at most `MAX_NEWS_ITEMS`, and a bad answer keeps the old list (test
  `news_keeps_the_old_list_on_a_bad_answer`).
- [ ] Branding paths must be plain file names under the branding prefix with an image extension and no `..`
  (`is_branding_path`; test `a_path_outside_branding_is_never_asked`). An image is kept only when its first bytes are
  JPEG, PNG or WebP; the server's content type is never trusted (`sniff_image`). The accent must be `#rrggbb`.
- [ ] JSON is read from at most the route's cap, with invalid UTF-8 replaced; an empty body is no JSON
  (`json_body`, `parse_json_body`).

## Open points

- **No SSRF block on public hosts.** A public invite may name `127.0.0.1`, a LAN address or a name that resolves to
  one. `dainty_relay::endpoint::is_private_or_loopback` exists and is re-exported from `net/http.rs`, but nothing
  calls it. The pin stops any HTTP byte from being sent, but the TCP connect and the TLS ClientHello still reach the
  address. Decide whether public mode should refuse private and loopback addresses.
- A `*.ts.net` name gets the key in clear and is resolved by the system resolver. That is safe only while MagicDNS
  answers for it, and the launcher does not check that Tailscale is up.
- TLS 1.2 is still offered next to 1.3 (`pinned_connect`). Decide whether every server can move to 1.3 only.
- There is no fuzz target for `pinned.rs` or `http.rs`: they parse nothing of their own beyond what hyper and rustls
  parse.
- Triage 2026-11-26: none of these is changed. A public server on a LAN address is a real setup (a host testing
  public mode at home), and the pin already stops any HTTP byte, so the SSRF block would cost more than the TCP
  connect and ClientHello it saves; the other three are decisions about Tailscale and the servers' TLS, not gaps.

## Sign-off

| Date | Commit reviewed | Reviewer | Notes |
| --- | --- | --- | --- |
| | | | |
