<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the network (RX3, DX2)

The checklist for the network core plugin, `rust/plugins/network` (`docs/dainty-lass-plan.md` DD16-DD19, DN3-DN11).
`SECURITY_REVIEW.md` indexes it. DX2 gives it five parts: node certificates and the cluster CA, session tickets, the
directory, the claim protocol and chat federation. Each row adds its part with its code; this page has the directory
(DN4) and the node certificates (DN3) so far. Region ping, which the gateway answers for the launcher, is
`region-ping.md`'s.

How to use it: before the network's canary (DN14), copy this list into the review, check each item in the code (not
from memory), link the code or test that shows it, and sign at the end with the date and the commit reviewed. An
item that does not hold is a bug to fix before the canary, not a box to leave open.

## What it is and where the boundary is

The nodes of one operator's cluster trust each other (DD16), but each node is a separate server with its own host,
database and admins, and a node's link to another crosses the internet. What one node sends another must not let it
take what belongs to a third: another home's accounts, a name already claimed, the account key.

- **Node links:** one call is one request body and one answer body, JSON, at most `MAX_MESSAGE` (1 MiB) each way
  (`link.rs`). DN3 carries them over mutual TLS on host-only gateway routes and gives the service the caller's node
  name from its certificate (below); `inprocess.rs` carries them inside one process for tests. A service trusts
  that name, never a name inside the body.
- **Outside a network:** the link is `NoPeers`, which refuses every call without sending anything.

## The directory (DN4)

The directory maps every account to its home node and holds username claims, unique network-wide ignoring case
(DD17). One node, the leader, writes; every change is an entry of an append-only log it numbers from 1. Every other
node is a replica that pulls the entries after the last one it holds. Homes register and rename their own accounts
through the leader.

- **In, on the leader:** `register` and `rename` from a home, `pull` from any node (`messages.rs`, `Service::Directory`).
- **In, on a replica:** pages of the leader's log, in answer to its pulls.
- **Out:** the directory holds account ids, usernames and home node names. No account key, session or address.

Code: `rust/plugins/network/src/directory.rs` (the leader's rules, the replica's pull and checks, the tables),
`messages.rs` (the messages and their decoder), `username.rs` (the claim's case rule), `link.rs` and `inprocess.rs`.
Tests: `rust/plugins/network/tests/directory.rs`, on SQLite and, with `REFORGED_TEST_POSTGRES_URL`, Postgres.

Fuzz target: `network_directory` in `rust/fuzz` (both decoders and the username rule). It checks that a message is
read whole or refused, that what is read holds only usernames the protocol's rule accepts and account ids of one
spelling, and that it writes back as the same message.

### Checklist

#### Who can change what

- [ ] A register is applied only when the caller, as the link authenticated it, owns the account id
      (`account:<ULID>@<node>`) and becomes its home; a rename only when the caller is the account's home in the
      directory. Anything else is `not_home`, and an id of another kind is `not_an_account` (`propose`). Test:
      `peers_change_only_their_own_accounts_through_the_leader`.
- [ ] A replica answers every call with `not_leader` and the leader's name; it never writes for a peer (`handle`).
- [ ] The actor a home names is parsed as `store`'s change log writes it, and a request with one it can't record is
      refused (`bad_actor`), so the leader's change log names who asked at the home. It is the home's claim, not
      something the leader can check.

#### One name, one account

- [ ] A username passes `protocol::account::check_username` (the shared `username.json` table) on the leader before
      anything is written, and again when any entry is read from a peer (`Username`'s deserialiser). Test:
      `usernames_follow_the_protocol_table`.
- [ ] The claim is the name in ASCII lower case, which folds every name the rule accepts; it is `UNIQUE` in
      `network_directory_accounts`, and the leader checks it in the same write that takes it. Writes are serial on
      both engines (`store`), so two regions asking at once get one answer each. Test:
      `a_name_taken_in_one_region_is_refused_in_every_other`.
- [ ] A replica never decides a claim from its own copy: registration and rename always go to the leader, and are
      refused (`LeaderUnreachable`) while it can't be reached, rather than risk one name claimed twice.
- [ ] A register repeated after a lost answer gets the entry that made it, not a second one, and a different name for
      a registered account is `account_exists`.

#### Replication

- [ ] A replica applies entries only in order, each the one after its newest, and checks each against the same rules
      the leader applied (`check_change`); an entry that breaks one stops replication as `Diverged` and nothing of
      that page is written. Test: `a_replica_ahead_of_its_leader_stops_replicating`.
- [ ] A pull carries the puller's newest entry; the leader refuses it as `diverged` unless it holds the same entry at
      that number, so a leader restored from an older backup can't feed a replica a different history under old
      numbers. A leader whose newest entry is older than the replica's is `Diverged` too.
- [ ] A page is at most `MAX_PULL` (256) entries; a page longer than asked for, a page that adds nothing or an empty
      page from a leader that claims more are protocol errors, so a broken leader can't keep a replica looping.
- [ ] Partitions: a replica keeps answering reads from its copy, and its next pull after the link heals catches up
      in pages, restarts included. Test: `a_replica_catches_up_after_a_partition`.
- [ ] Every directory write goes through `store`'s `Database::write`, so each node's change log (DD23) holds it with
      its actor; replicated entries are logged as `system:directory-replication`.

#### Messages

- [ ] Requests and answers carry `version`; another version is refused as `unsupported_version` before its body is
      read as this one's (DD16: versioned for later federation).
- [ ] Every message type has `deny_unknown_fields`; a body over `MAX_MESSAGE` is refused before it is parsed, by the
      link and by the decoder.
- [ ] A database failure on the leader answers `unavailable` and is logged there; its text never crosses the link.

#### Outside a network

- [ ] A standalone server is its own leader with `NoPeers`: registration, renames and lookups work the same, and no
      call is made. Test: `a_server_outside_a_network_keeps_its_directory_to_itself` (section 6).

### Still open

- **The leader is fixed by configuration.** There is no election; if the leader is down, registrations and renames
  everywhere wait for it, and reads go on. Moving the leader is an operator's step: the new one must hold the whole
  log first.
- **A diverged replica stays stopped** until an operator resets it (a fresh table, pulled again from 1). No command
  does that yet.
- **Nothing creates a directory from `host.toml` yet.** DN3 gave the messages their transport (below); wiring a
  `Directory` into the controller, with the leader and the link from `[network]`, is still to do.
- **Joining an existing server** (DN13) needs its accounts registered through the leader, with the clash renames,
  and its standalone log replaced by the leader's.

## Node certificates and the cluster CA (DN3)

Nodes reach each other through their gateways, on the public TLS port, with mutual TLS under a CA the operator
runs. The gateway stays the only listener off loopback.

- **The CA** (`cluster.rs`, `reforged-host network ca-init`): one file with the CA's certificate and key, made and
  kept on the operator's own machine. `issue-node-cert --ca <file>` reads it each time and stores nothing of it. A
  node holds three files in `.workspace/tls`: its own certificate and key and the CA's certificate. A CA file on a
  node (a key in the file the node reads as the CA's) stops the gateway at start.
- **A node certificate** is ECDSA P-256, valid 365 days by default, `serverAuth` and `clientAuth`, not a CA. It says
  who the node is in two URI names, `urn:dainty-lass:node:<id>` and `urn:dainty-lass:region:<region>`, beside the IP
  addresses and DNS names its gateway answers on. The CA is constrained to sign leaves only (path length 0).
- **Who is a peer:** a client certificate that chains to the cluster CA, is valid now and names a node that this node
  lists under `[[network.nodes]]`. The CA's signature alone is not enough, which is how a node is cut off: remove it
  from the others' lists. There is no revocation list.
- **How a node link arrives:** a client that offers the ALPN name `dainty-lass-node/1` gets the node certificate and
  a TLS 1.3 handshake that requires a cluster client certificate (`nodelink.rs`, `Gateway::connection`). Every
  other client gets the gateway's public certificate and the public policy exactly as without node links, and
  `/__node` stays refused for it (`node_link`, wrapped in `/__origin` or not). On a node connection only
  `/__node/ping` and `/__node/<service>` are served; anything else is 404, nothing reaches the game, content or chat,
  and nothing is captured. The caller's name is the one in its verified certificate.
- **How a node calls:** `TlsPeerLink` (`tls.rs`) connects to the peer's listed address, requires the peer's
  certificate to chain to the CA and to carry that address, and then that it names the peer it meant to call (node
  and region): a node of another CA, or another node answering at the address, is refused before a request byte is
  sent.
- **Clocks:** every request and answer carries `x-node-time`. A peer whose clock is more than 30 seconds off
  (`MAX_CLOCK_SKEW`) has its requests refused with 409 `clock_skew` and its answers refused by the caller; the pings
  measure it against the middle of the round trip. Both sides log it, and the caller's `PeerHealth` shows it.
- **Health:** `TlsPeerLink::health()` has a state per peer (`unknown`, `up`, `down`, `clock_skew`, `refused`,
  `untrusted`) with the last round trip, skew and error; a background ping every 10 seconds
  (`spawn_health_checks`) and every call update it. A restarted peer is reached again on the next call: idle
  connections are dropped when they close and a call retries once on a fresh one if the kept one was already closed.

Code: `rust/plugins/network/src/cluster.rs` (CA, issuing, reading names, credentials, the TLS settings),
`wire.rs` (the routes, the ping and error answers), `endpoint.rs` (admission and the service call), `tls.rs`
(`TlsPeerLink`, health); `rust/services/gateway/src/nodelink.rs`, `gateway.rs` (`connection`, `node_request`),
`config.rs` (`GATEWAY_NODE_*`); `rust/services/host/src/commands/network.rs`, `hostconfig/modules.rs`.
Tests: `rust/plugins/network/tests/cluster.rs` (the CA and mutual TLS in memory),
`rust/services/gateway/tests/nodelink.rs` (several real gateways on 127.0.0.1:0).

Fuzz target: `network_link` in `rust/fuzz` (the route table, the time header, a ping and an error answer, a peer as
`host.toml` lists it, a certificate's names, and the endpoint's admission of a request head).

### Checklist

#### The CA and the files on a node

- [ ] The CA key is only in the CA file: no node file, `host.toml` or env file holds it, and the index next to the
      CA file (`<ca>.issued.json`) holds no key. Test: `the_owner_makes_a_ca_and_issues_a_nodes_files_from_it`.
- [ ] A node refuses a CA file that holds a key, a key that isn't its certificate's, and a certificate that doesn't
      chain to its CA or isn't valid now (`NodeCredentials::from_pem`). Test:
      `a_node_accepts_only_credentials_of_its_own_cluster`.
- [ ] The gateway refuses to start when its certificate names another node or region than `GATEWAY_NODE_ID` and
      `GATEWAY_NODE_REGION` (`NodeLinks::load`).
- [ ] The gateway warns 30 days before the node certificate expires, and `network list` says "expires soon" and
      "expired".

#### Who gets in

- [ ] A client certificate from another CA is refused in the handshake, and a client without one is refused. Tests:
      `a_node_with_another_clusters_certificate_is_refused_both_ways`,
      `a_client_without_a_certificate_is_refused`, `a_node_of_another_cluster_is_refused_and_refuses`.
- [ ] A certificate of the CA for a node this node doesn't list gets 403 `not_a_peer` and the service is never
      called. Test: `a_cluster_node_the_server_does_not_list_gets_403`.
- [ ] A player's connection gets 403 on `/__node/*`, wrapped in `/__origin` or not, and the public policy is the
      one it was. Tests: `a_player_connection_cannot_reach_the_node_routes`, and `modules.rs`'s
      `the_node_link_group_refuses_everything`.
- [ ] On a node connection only `/__node/ping` (GET) and `/__node/<service>` (POST) are answered, exactly spelled
      (`wire::parse_target`); `/__origin`, queries, case changes and every other path are 404. Test:
      `a_node_connection_serves_the_node_routes_and_nothing_else`.
- [ ] Node links need the TLS gateway: `GATEWAY_NODE_ID` with `GATEWAY_TLS=0` is a start error, and `setup` refuses
      `[[network.nodes]]` in a private server.
- [ ] Node traffic is not in the traffic capture, and its access lines carry the route `node`, the status and the
      refusal reason, never a body.

#### What a node trusts

- [ ] A service learns the caller from the verified certificate only, never from the body (`NodeService::serve`'s
      `from`). Test: `nodes_call_each_other_and_the_service_is_told_who_called`.
- [ ] The caller checks the peer's certificate chain, its address and that it names the peer it called, so another
      node answering at that address is refused. Tests: `a_peer_at_the_wrong_address_is_not_that_peer`,
      `the_server_must_be_at_the_address_its_certificate_names`.
- [ ] A body over `MAX_MESSAGE` is refused before it is parsed: by the caller before sending, by the gateway from
      its declared length (413) and while reading, and an answer over it is a failure (502). Test:
      `a_body_over_the_limit_is_refused_before_it_is_parsed`.
- [ ] Nothing in a request or an answer is trusted before `wire` has decoded it whole: unknown fields, other versions
      and odd times are refused.

#### Clocks, health and restarts

- [ ] A peer more than 30 s off is refused both ways and shown as `clock_skew`; one within the bound is not. Test:
      `a_peer_whose_clock_is_too_far_off_is_refused_and_reported`.
- [ ] A peer that restarts is reached again on the next call with no action, and the pings show it down and up.
      Tests: `links_survive_a_peer_restart`, `health_checks_follow_a_peer_down_and_back_up`.
- [ ] A server with no `[[network.nodes]]` writes no node settings, serves no node link, and answers as it did.
      Tests: `a_server_without_peers_writes_no_node_settings`, `a_server_outside_a_network_answers_as_before`.

### Still open

- **No revocation list.** A leaked node key is handled by removing the node from every other node's list; its
  certificate stays valid until it expires, so keep them short-lived once the canary has the renewal habit.
- **Renewal is by hand:** `issue-node-cert` again and copy the three files; the gateway reads them at start, so a
  renewal needs a restart.
- **A lost or leaked CA file** means a new CA and every node certificate again. Nothing detects a leak.
- **The controller doesn't attach a service yet.** `Gateway::node_links().endpoint().attach(..)` and
  `server::run_with_node_links` give the network plugin its hook; DN5 and DN10b attach their services.
- **A private (Tailscale) server can't be a node:** node links need the gateway's TLS port.

## Session tickets (DN5)

To be written with DN5.

## The claim protocol (DN8)

To be written with DN8.

## Chat federation (DN10b)

To be written with DN10b.

## Fuzzing

Short runs happen on every push (CI's `fuzz` job). The run before the canary, from `rust/fuzz`:

```bash
cargo +nightly fuzz run network_directory -- -max_total_time=86400
cargo +nightly fuzz run network_link -- -max_total_time=86400
```

Signed: ________ (date, commit)
