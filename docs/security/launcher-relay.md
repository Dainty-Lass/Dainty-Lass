<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the launcher's local relay and its owner check (RX3)

The checklist for the relay the launcher runs while the game plays: `net::relay` and `net::relay_owner` in
`crates/launcher-kit` (RL11, spec `docs/port/spec-rd27-launcher.md` §2.6.4 and §2.6.5, RD21). The gateway it talks to
has its own list in [gateway.md](gateway.md).

How to use it: before the MIT relay replaces the shell's, copy this list into the review, check each item in the code
(not from memory), link the code or test that shows it, and sign at the end with the date and the commit reviewed.
An item that does not hold is a bug to fix first, not a box to leave open.

## What it is and where the boundary is

The game is pointed at `http://127.0.0.1:<port>` and believes it is talking to the server. The relay carries what it
sends, the account key (`x-reforged-user-api-key`) included, to the invite's server. Any program on the PC, and any web
page in a browser on it, can also open 127.0.0.1, so the relay is where a local stranger would try to borrow the
player's key or session.

- **In:** plain HTTP/1.1 on `127.0.0.1:<port>` (61100, or `REFORGED_RELAY_PORT` when it passes the override rule),
  and best effort a QoS-only listener on `127.0.0.1:80`. Never another address.
- **Out, public mode:** TLS 1.2/1.3 pinned to the invite's certificate fingerprint (`net::pinned`, through
  `net::connector`), to the gateway's host and port.
- **Out, private mode:** plain HTTP, only to a private-mode host (Tailscale or loopback; `net::http::check_endpoint`).
- **Out, away from home:** after a redirect (DN6), another node of the network, with a session ticket instead of the
  key.
- **Process table (Windows):** `System32\netstat.exe -ano -p tcp`, read hidden.

Code: `crates/launcher-kit/src/net/relay.rs`, `relay/local.rs`, `relay_owner.rs`, `relay_owner/table.rs`; tests in
`relay/tests.rs` and each file's unit tests.

Fuzz targets in `fuzz` (to be added by the integration pass; RL11 does not touch `fuzz/`):

- `relay_input`: the local rules (`local::check`, `host_is_local`, `origin_is_local`, `authority_host`,
  `is_qos_target`) and the table parser (`relay_owner::table::owner`) take any bytes without a panic, and no Host or
  Origin that passes names anything but `localhost`, `::1` or `127.x.y.z`.

## Checklist

### Listening

- [ ] Both listeners bind `127.0.0.1` only (`relay.rs`, `bind`). No setting changes the address.
- [ ] The port override takes only 4 or 5 digits without a leading zero, 1024 to 65535, trimmed (`port_override`);
      anything else falls back to 61100.
- [ ] A port in use or denied is `PortBusy`; a QoS port that won't bind is logged at info and the relay runs without
      it.
- [ ] `stop` cancels both listeners, every connection and every tunnel, and the ports are free when it returns
      (`starts_logs_and_stops_freeing_both_ports`, `upgrades_tunnel_both_ways_plain_and_pinned_until_stopped`).
      Dropping the relay cancels too.

### Who is served

- [ ] On Windows, each connection to the game's port is checked once before its first request: the TCP table must
      name the running game's pid (> 0) as the owner of the connection's **client** end, both ends loopback IPv4
      (`relay_owner.rs`, `GameOwnerCheck::check`; `the_owner_check_serves_only_the_games_connections`).
- [ ] The table is read fresh for each batch of waiting checks; nothing is cached. A missing row gets one more read
      25 ms later; a second miss, two rows for one connection, a non-numeric pid or an unreadable table refuse
      (`table.rs`, `relay_owner.rs` tests).
- [ ] At most 64 checks wait at once; more are refused, so a flood of connections can't queue process reads.
- [ ] A refused connection answers every request 403 `{"error":"local_process_denied"}` with `connection: close`, and an
      upgrade a bare 403. Nothing is forwarded.
- [ ] Every request, on both listeners, passes the local rules first, in order (`relay/local.rs`, `check`):
      loopback peer (IPv4-mapped counts); exactly one Host whose host part is `localhost`, `::1` or a dotted
      `127.x.y.z` (DNS rebinding names fail here); no `sec-fetch-*` header on a plain request (a browser's fetch); no
      Origin, or one Origin whose host is loopback. A refusal is 403 `{"error":"forbidden"}` (upgrades: bare) and a
      log line naming the rule (`every_local_refusal`).
- [ ] `CONNECT` is never forwarded (405).

### What reaches the server

- [ ] Paths and queries pass as sent, `/__origin/...` included; the gateway judges them (`origin_paths_pass_as_sent`).
- [ ] Hop-by-hop headers, those `Connection` names, `host`, `http2-settings` and any `x-reforged-session-ticket` the
      game sent are dropped (`proxy::end_to_end`, `Route::headers`); `host` is set to the server's.
- [ ] A `transfer-encoding` other than `chunked` is 501 and nothing is sent. A chunked request goes on chunked
      without its `content-length`, so the server never sees both framings.
- [ ] Upgrades use a fresh connection, never the pool, with `connection: Upgrade` and the game's `upgrade` value
      (`upgrades_never_use_the_pool`). A refused upgrade passes back its status and at most 64 KiB of body, then
      closes.
- [ ] Tunnels end after 24 hours without a byte either way, or when the relay stops.

### The account key and the pin

- [ ] Public mode never sends a byte before the certificate's SHA-256 matches the pin (`net::pinned`); a mismatch
      reaches the server with nothing, calls the pin callback with the fingerprint seen, logs an error and answers 502
      `{"error":"certificate_mismatch"}` (upgrades: bare 502) (`a_pin_mismatch_is_reported_as_one`).
- [ ] A relay whose home is public refuses to start without a valid pin (`Relay::start` through `check_endpoint`),
      and a private one refuses a host that is not private-mode (`targets_are_checked_before_binding`).
- [ ] The key goes only home. Away from home, or with a ticket, `x-reforged-user-api-key` is removed and
      `x-reforged-session-ticket` set (`a_redirect_re_points_mid_session_with_a_ticket_and_home_takes_the_key_again`).
- [ ] A redirect needs `x-reforged-redirect-host` and a numeric `x-reforged-redirect-port`; any other node than home
      needs a ticket; a pinned relay never moves to a node without a pin; a bad value is logged and ignored
      (`bad_redirects_are_logged_and_ignored`). "Home" is the same host and port, with the same pin or none.
- [ ] The redirect headers and the ticket are stripped from the answer before the game sees it.
- [ ] No relay log line carries a header value: not the key, not the ticket, not a cookie. Lines name peers, rules,
      hosts, ports and the fingerprint seen (`the_account_key_never_reaches_the_log`). Upstream errors go through
      `platform::log::describe_error`, which redacts registered secrets.

## Open points

- Off Windows there is no owner check (RD21): any local program that sets a loopback Host and no Origin is served.
  Linux players run the game under Proton or Wine, whose connections can't be matched to the game's pid the same way.
- The QoS listener has no owner check. It answers only the fixed QoS page and forwards nothing, so a stranger learns
  only that the relay runs.
- A program that shares the game's pid (code injected into the game) passes the owner check. The relay can't tell;
  RD9's loader checks are the defence there.
- The owner check runs once per connection; a connection the game hands to another process keeps its pass. Windows
  does not let a socket change owner without the game's help.

## Sign-off

Reviewed at commit `________` on `________` by `________`.
