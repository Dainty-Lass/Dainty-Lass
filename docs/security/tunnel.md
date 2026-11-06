<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the split server's tunnel (`crates/network/src/tunnel.rs`, RX3)

The checklist for `Service::Tunnel`, which carries HTTP between the two halves of a split server
(`docs/split-roles.md`). `SECURITY_REVIEW.md` indexes it.

How to use it: check each item in the code (not from memory), link the code or test that shows it, and sign at the end
with the date and the commit reviewed. An item that does not hold is a bug to fix before merging.

## What it is and where the boundary is

- **In, loopback:** `127.0.0.1:61001` on the metagame half, `127.0.0.1:61000` on the game half (`controller::modules::tunnel`).
  Whatever can reach loopback could already reach those ports on a single server, so the trust is unchanged.
- **Across, the node link:** `POST /__node/tunnel` on the other half's gateway, mutual TLS under the cluster CA, the
  peer named by its certificate (`docs/security/network.md`). One message is at most 1 MiB.
- **Out, loopback:** the answering half replays the request on its own `127.0.0.1:<port>`, never anywhere else.

## Checklist

- [ ] Only the gateways listen off loopback: the tunnel's listener binds `127.0.0.1` (`modules::tunnel`), and the
      replay connects to `127.0.0.1` only (`tunnel::replay`).
- [ ] The answering half serves the one node `split.peer` names, and refuses every other node of the network with
      `Refused` (`TunnelService::answer`; test `answers_its_peer_only_and_its_ports_only`).
- [ ] It replays only on the port this half runs: the metagame half exposes the backend, the game half the Director
      (`serve.rs`, `split_tunnel`). A request for the other port is refused.
- [ ] The Director's port carries only its four routes, by method and path (`DIRECTOR_ROUTES`, `carries`; test
      `the_director_port_carries_its_routes_only`), checked on both halves.
- [ ] Nothing in a decoded request can split the replayed request line or add a header: the method is one of seven,
      the target is visible ASCII in origin form without `//` or `#`, header names are tokens and values hold no CR,
      LF or NUL (`TunnelRequest::check`; test `refuses_what_could_split_a_request`; fuzz target `network_tunnel`).
- [ ] Hop-by-hop headers, and any header `Connection` names, never cross (`end_to_end`, `check_headers`; test
      `hop_by_hop_headers_stay_on_their_connection`). The replay sets its own `Host`.
- [ ] Unknown fields and unknown ports are refused (`deny_unknown_fields`, `Port`).
- [ ] Bodies are capped at 512 KiB both ways before encoding, so a message stays under the link's limit
      (`MAX_BODY`; test `round_trip_and_limits`).
- [ ] The game server key travels only inside the node link; the public gateway's refusal of its header is unchanged
      (`crates/gateway/src/policy.rs`, `HOST_ONLY_HEADERS`).
- [ ] Nothing logs a request's headers or body: failures log the peer, the port and the error only.
- [ ] A public game half runs with open game ports only when `split.open_game_ports = true` says so
      (`SplitConfig::check`; test `split_reaches_the_backend_file`).

## Open points

- **The allowlist feed does not cross the tunnel.** A public game half's game ports are open to everyone
  (`docs/split-roles.md`, `open_game_ports`). Carrying the metagame gateway's heartbeat feed to the game half's
  allowlist closes this.
- **A compromised peer node** can call any backend route as loopback on the metagame half, game server routes included.
  That is the same trust a single server gives its own Director and game servers.

## Fuzzing

A short run of `network_tunnel` happens on every PR (CI's `fuzz` job). For a 24-hour campaign from `fuzz`:

```bash
cargo +nightly fuzz run network_tunnel -- -max_total_time=86400
```

Not yet signed.
