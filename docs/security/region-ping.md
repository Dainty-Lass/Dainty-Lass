<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: region ping (RX3)

The checklist for region ping (`docs/dainty-lass-plan.md` DN7, DD19): the gateway's network routes and the
launcher's measurement and report. `SECURITY_REVIEW.md` indexes it. The pinned TLS connection and the HTTP client it
uses are `launcher-net.md`'s; the rest of the gateway is `gateway.md`'s.

How to use it: before the network's canary (DN14), copy this list into the review, check each item in the code (not
from memory), link the code or test that shows it, and sign at the end with the date and the commit reviewed. An
item that does not hold is a bug to fix before the canary, not a box to leave open.

## What it is and where the boundary is

The launcher asks the joined server, the player's home, for its network's nodes, times a few requests to each node's
gateway, and posts the regions, nearest first, back to the home. A node list from a malicious or compromised home
could point the launcher anywhere, so a node is only ever reached over TLS pinned to the fingerprint the home listed,
and nothing a node could use is sent to it. The account key goes only to the home, with the report.

- **Gateway, in:** `GET /network/v1/nodes`, `GET /network/v1/ping` and `POST /network/v1/regions` from anyone on
  the public port, answered by the gateway itself; nothing reaches the backend or the content server.
- **Gateway, out:** the node list (region id, host, port, fingerprint): public, empty outside a network.
- **Launcher, in:** the home's node list, over the connection pinned to the invite's fingerprint (or plain HTTP to a
  private-mode host); each node's ping answers.
- **Launcher, out:** to each node, TLS pinned to its listed fingerprint, then `GET /network/v1/ping` with only `host`
  in the headers. To the home, the report with the account key header.

Code: dainty-lass's `services/gateway/src/network.rs` (the node list's body, the report's parser), `policy.rs`
(`NetworkRoute`), `gateway.rs` (`Gateway::network`); `launcher/src-tauri/src/net/regionping.rs` (fetching,
timing, ordering, the report) and `net/regionping/parse.rs` (the node list's reader), `controller/region.rs`.

Fuzz targets in `fuzz`: `gateway_network` (the report's parser: read whole or refused, region ids and round
trips in range, each region once) and `launcher_region` (the node list: only valid region ids, hosts, ports and
fingerprints, each region once, at most 16). dainty-lass's `gateway_policy` covers the routing.

## Checklist

### The gateway

- [ ] The three routes match only without an `/__origin` wrapper: the game's wrapped calls go to the backend as
      before (`the_network_routes_are_answered_here_and_only_for_the_launcher`).
- [ ] The host-only headers are refused before a network route is matched; an upgrade to one is refused; any other
      method answers 405.
- [ ] The routes count against the general rate limit, as any public request.
- [ ] The node list carries no secret: region, host, port and the certificate fingerprint every invite already
      shows. At most 16 nodes are listed.
- [ ] A report without a key-shaped `x-reforged-user-api-key` is refused with 401 before its body is read; the body
      is read up to 4 KiB within the request timeout (413 past it, 408 on the clock) and refused whole unless it is
      exactly `{"regions":[{"region","rttMs"}...]}` with valid ids, round trips from 0 to 60 000 ms or null, each
      region once.
- [ ] The report is stored nowhere and logged only as the access line (route `network`), whose target is redacted
      as every other's; the key header is never logged and the capture redacts it.

### The launcher

- [ ] A node entry is kept only with a valid region id, a host `is_valid_host` accepts, a port from 1 and a
      64-hex fingerprint; anything else is dropped. Each region once, at most 16.
- [ ] Every connection to a node is pinned to the fingerprint its home listed (`pinned_connect`): a different
      certificate fails before any byte of a request is written
      (`a_node_with_another_certificate_is_refused_before_anything_is_sent`).
- [ ] No request to a node carries the account key or any header but `host`
      (`the_order_goes_to_the_home_with_the_key_and_to_no_node`).
- [ ] The report goes only to the joined server, through `key_headers`: pinned TLS or a private-mode host, or not at
      all.
- [ ] The node list's fetch carries no key.
- [ ] Every wait is bounded: 2 s to connect, 1.5 s per request and 4 s per node, four nodes at a time, 5 s for each
      call to the home; a ping answer's body is capped at 1 KiB and the node list at 16 KiB.
- [ ] A server outside a network (an empty list, or a 404 from one without the route) gets no report, and no node
      is contacted.

## Open items

- **The report's key is not checked.** The stub checks the header's shape only, since it stores nothing. DN8 moves
  the route to the home's network plugin, where the key (or a session ticket, DD17) is checked against the account
  before the order is kept.
- **The node list is empty everywhere** until DN2 and DF3 add `[network]` to `host.toml` and the gateway reads it.
  Whoever fills it decides whether a node lists itself.
- **Two copies of the shapes.** The region id rule and the routes are written in the gateway and the launcher; a
  launcher test reads the gateway's source to keep them equal until DN2's `crates/module-api` defines them once.

## Fuzzing

Short runs of both targets happen on every push (CI's `fuzz` job). The runs before the canary, from `fuzz`:

```bash
cargo +nightly fuzz run gateway_network -- -max_total_time=86400
cargo +nightly fuzz run launcher_region -- -max_total_time=86400
```

Signed: ________ (date, commit)
