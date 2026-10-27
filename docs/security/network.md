<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the network (RX3, DX2)

The checklist for the network core plugin, `rust/plugins/network` (`docs/dainty-lass-plan.md` DD16-DD19, DN3-DN11).
`SECURITY_REVIEW.md` indexes it. DX2 gives it five parts: node certificates and the cluster CA, session tickets, the
directory, the claim protocol and chat federation. Each row adds its part with its code; this page has the directory
(DN4), the node certificates (DN3) and the session tickets and home API (DN5) so far. Region ping, which the gateway
answers for the launcher, is `region-ping.md`'s.

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
  `server::run_with_node_links` give the network plugin its hook; DN5 built the services (`ServiceMux` puts the
  directory's and the home API's behind the one attachment) and DN10b adds its own.
- **A private (Tailscale) server can't be a node:** node links need the gateway's TLS port.
- **The running gateway does not ping its peers yet.** `spawn_health_checks` is used by no service, so a node's health
  is what `network status` finds when it is run; nothing on a live node records a peer going down between runs.

## The Docker harness (DN12)

`compose.yaml`'s `network` profile runs four nodes on one machine for tests (`docs/hosting-docker.md`,
`docker/network/test.sh`). It is not a deployment: its CA and every key in it are made for the test and thrown away
with `docker compose down --volumes`. It keeps the rules above anyway, so what it proves is the real arrangement.

- **The CA key lives in one volume, `network-ca`, mounted only by `network-init`** (at `/ca`, directory mode 700, the CA
  file 600, root's). No node mounts it and no node file holds it; `network-init` has no network
  (`network_mode: none`), runs as root only to write the nodes' volumes (`cap_drop: ALL`, then `CHOWN`, `DAC_OVERRIDE`,
  `FOWNER`) and exits. The CA is made once and kept while the volume lives, so a second `up` keeps every certificate and
  fingerprint.
- **A node gets its own files only.** `network-init` writes into each node's workspace volume its public gateway
  certificate and key, `node-cert.pem`, `node-key.pem` (600, owned by uid 1000) and `cluster-ca-cert.pem`, which is the
  CA's certificate, and its `host.toml`. The gateway container mounts that volume read-only.
- **The node certificates name the addresses the harness fixes** (`172.28.77.11` to `.14`, and `<region>.cluster.test`),
  which is also how each peer is listed, so the checks that a peer answers at the address its certificate names are
  the ones that run. The `host.toml` lists every other node with the SHA-256 of its public gateway certificate.
- **CAP_NET_ADMIN is the `netem-*` containers' alone**, in each node's namespace; the gateways keep
  `cap_drop: ALL`, `no-new-privileges` and a read-only root. `tc` shapes only that namespace's outgoing traffic.
- **Published ports** are the gateways' 443 on `127.0.0.1` (`REFORGED_NETWORK_PUBLISH_ADDRESS`); the node links share
  that port as they do anywhere. Setting the address to `0.0.0.0` exposes four nodes whose CA key anyone with the
  Docker socket can read: don't.
- **`reforged-host network status`** (the check the harness runs in each node) calls the peers as the node itself, from
  the node certificate the gateway reads, and prints states and round trips; it takes no action on a peer and prints
  no key. Run in a container, it needs the node's `gateway.env` and nothing of the CA.

## Session tickets and the home API (DN5)

An account's data lives only at its home (DD17). A node that needs it, such as the one hosting a hunt, gets from the
home a short-lived **ticket** the home signed, and with it asks the home's game module to run **operations** on the
account. The plugin carries a module name, an operation name and a JSON body without reading them; the module at the
home decides what they mean. **The account key never crosses a node link.**

- **Ticket:** `dlt1.<claims>.<signature>`, both base64url without padding, at most 1024 bytes. The claims are JSON the
  home wrote once: `v`, `kid` (the signing key's id), `iss` (the home), `aud` (the node it is for), `sub` (the
  account's global id), `game` (the game version), `iat`, `exp` (unix ms), `since` (when the login began, kept by every
  renewal) and `jti` (128 random bits). The signature is Ed25519 over `dainty-lass/session-ticket/v1\n` and the claim
  bytes as written, so nothing is re-serialised before it is checked (`ticket.rs`). It holds no secret.
- **Keys:** each node has one Ed25519 signing key in a file of its own (`dlk1.<64 hex>`, made on first start, mode 0600,
  refused on Unix if other users can read it; `keys.rs`). Its public key is not secret: a peer fetches it with
  `public_keys` over the mutually authenticated link, so a key learned from node X was told by X. A node never takes
  keys for itself from a peer, a rotated key's predecessor stays in the list while tickets it signed can be alive, and a
  node fetches a given peer's keys at most every 10 seconds.
- **Lifetimes:** 5 minutes by default, never more than 15 (a verifier refuses a ticket that claims more); clocks may
  differ by 30 seconds (the node links' own bound); a login can be renewed for 6 hours from `since`.
- **Where tickets are made:** at the account's home. The home's own modules ask `NetworkedHome::session_ticket`. A
  node that holds a ticket for a player can **exchange** it at the home for one for another node (to hand the player
  on, DN6) or for itself (renewal); the home checks the one presented first and keeps `since`.
- **Where they are checked:** by the node they are for (`HomeApi::admit`), offline against the home's public key:
  shape, signature, issuer (must be the home the directory names for the account), audience (this node), game version,
  times, and not seen before. By the home, on every call it answers (below).
- **What a node does with one:** `admit` keeps it; the node's operations on that account present it, and the node
  renews it with the home when it has under a minute left. A node with no ticket for an account can do nothing to it.
- **Operations:** `Service::Home`, route `/__node/home`, requests `public_keys`, `check` (does the home still stand
  behind this ticket), `exchange` and `run` (module, operation, account, actor, request key, body). Versioned,
  `deny_unknown_fields`, every name and id typed, bodies at most 256 KiB (`protocol.rs`). The home finds the module's
  `HomeBackend` in its `HomeRouter` and calls it only for accounts the directory says are homed on this node.
- **Repeats:** a `run` carries a request key; the home runs one key once and answers a repeat with the first answer
  (kept for up to 1024 requests or 8 MiB), so a request whose answer was lost can be sent again without writing twice.

Code: `rust/plugins/network/src/home/` (`ticket.rs`, `keys.rs`, `protocol.rs`, `service.rs` for the home's side,
`client.rs` for `NetworkedHome`), `link.rs` (`Service::Home`, `ServiceMux`), and in
`rust/crates/module-api/src/network.rs` the seams (`HomeBackend`, `HomeInvocation`, `HomeForward`,
`HomeApi::forward` and `admit`).
Tests: `rust/plugins/network/tests/home.rs` (three regions in process, SQLite and, with `REFORGED_TEST_POSTGRES_URL`,
Postgres), unit tests in `home/ticket.rs`, `keys.rs` and `protocol.rs`, and `rust/services/gateway/tests/home.rs` (two
real gateways over mutual TLS).

Fuzz targets: `network_home` in `rust/fuzz` (requests and answers: read whole or refused, no account-key-shaped value
in any free-form field, bounds, written back the same) and `network_ticket` (tokens: never panic, nothing the fuzzer
writes verifies, a changed byte of a real ticket never verifies; public keys from a peer).

### Checklist

#### The account key

- [ ] No field of any home message can hold an account key. Typed fields are names and ids checked as read; the
      free-form ones (`HomeBody`, `WireActor`, `RequestKey`, `CleanText`) refuse anything shaped like `RRK_` and eight
      more key characters, in any case, in a string or an object key, when built and when deserialised; a ticket's
      token is bounded base64url whose claims are typed. `encode_*` and `decode_*` run `Validate` over what they
      carry. Tests: `no_message_type_can_carry_an_account_key` (every message type),
      `the_account_key_never_crosses_a_node_link` (everything on the wire in a whole session is scanned, and a module
      that tries to write one is refused before anything is sent).
- [ ] A module's refusal text is cut and scrubbed before it crosses (`CleanText::lossy`); the signing key and tickets
      are not logged (`Debug` shows key ids and node names only), and a log line for a home call names the peer, module,
      operation and account id, never the body.

#### Who can ask for what

- [ ] Every `check`, `exchange` and `run` carries a ticket this node signed **for the caller as the link authenticated
      it**; a ticket presented by another node is `wrong_audience`. A node acts only for players whose home gave it a
      ticket, only until it expires or the home ends the session. Tests:
      `a_node_acts_only_for_players_whose_home_gave_it_a_ticket`,
      `tickets_that_are_foreign_expired_replayed_or_misaddressed_are_refused`.
- [ ] A `run` for an account that is not the ticket's, for a module the home doesn't run, for a ticket of another game
      version, or for an account the directory does not place at this node is refused before the module is called.
- [ ] An exchange only names this node or a peer it lists. The new ticket keeps the login's `since`, and a login older
      than 6 hours is renewed no more. Test: `a_held_ticket_is_renewed_for_as_long_as_the_login_lasts`.
- [ ] Ending a session at the home (`Tickets::revoke`: a ban, a logout; DN11) kills every ticket issued before it, at
      the home, and renewals fail. Test: `a_session_the_home_ends_stops_working_everywhere`.
- [ ] At most 64 operations run at once; one that outwaits its caller (20 seconds) still finishes and is remembered.

#### Verification (the audience)

- [ ] A ticket is accepted only when its signature verifies with a key the *issuer* told this node, the issuer is the
      home the directory names for the account, the audience is this node, the game version is the module's, and it is
      neither expired nor from the future (30 s allowed). Tests in `ticket.rs`:
      `a_ticket_nobody_but_the_homes_key_signed_is_refused`,
      `a_ticket_for_another_node_account_game_or_issuer_is_refused`, `tickets_expire_and_clocks_may_differ_a_little`.
- [ ] A ticket is admitted once: its id is remembered until it could no longer be accepted; a second presentation is
      `replayed`; a full memory (65 536) refuses rather than forgets; a ticket issued before this node started is
      refused (`before_start`), since a restart forgot what was used. Tests: `a_ticket_is_admitted_once`,
      `a_full_replay_memory_fails_closed`, `a_node_that_restarted_refuses_tickets_it_cannot_know_were_used`.
- [ ] A token is the only spelling of itself (padding, case and extra parts are `malformed`), and its outer fields must
      match its signed claims. Test: `nonsense_is_malformed_not_a_panic`.
- [ ] A rotated key is followed: an unknown key id makes the node ask the issuer, at most every 10 seconds. Test:
      `keys_are_fetched_from_the_home_and_a_rotation_is_followed`.

#### Writes

- [ ] A request key makes a write happen once: the repeat of a request whose answer was lost gets the first answer, a
      key reused for a different request is refused, and an operation that failed is not remembered (it can be
      sent again). Test: `a_request_lost_on_the_way_back_is_applied_once`.
- [ ] The home's module sees who asked: the calling node from the link, and the actor the caller claims (a claim; the
      module's change log records both).
- [ ] The loot and XP of a hunt hosted elsewhere are written at the player's home and nowhere else. Tests:
      `a_hunt_hosted_in_another_region_saves_loot_and_xp_at_the_players_home`, and over real mutual TLS
      `a_hunt_in_another_region_saves_at_the_players_home_over_mutual_tls`.

#### Outside a network

- [ ] A server with no peers keeps `StandaloneHome`: nothing to forward to, no tickets, nothing leaves it. A
      `NetworkedHome` with no peers works for its own accounts. Test: `a_server_outside_a_network_has_no_other_home`.

### Still open

- **Nothing builds these from `host.toml` yet.** The controller doesn't create the `Tickets`, `HomeService` and
  `NetworkedHome`, put them in a `ServiceMux` with the directory, attach that to the gateway's endpoint, or register
  a module's `HomeBackend` (`DriverBackend` wraps a loaded module) when it loads. The signing key file's place
  and the 5-minute and 6-hour settings are the caller's to choose in code; none is a `host.toml` key yet.
- **Revocations and the replay memory are in memory.** A restart of the home forgets who was banned until their ticket
  would have expired anyway (at most 15 minutes, but a holder can renew until the 6-hour limit unless the home's module
  checks bans on every operation); a restart of the audience refuses every ticket issued before it, so players ask
  again.
- **Admission is offline.** A node admits a ticket without asking the home, so a player whose session ended is admitted
  until the ticket expires; `NetworkedHome::check_session` asks the home and a module that cares calls it.
- **The cluster trusts its nodes with the tickets they hold.** A compromised node can act for the players it holds
  tickets for (and no others) until they expire or the home ends their session. There is no per-operation limit;
  that is the home's module's to apply to the invocation it receives.
- **One game per network.** Tickets are made for the game version in `NetworkedConfig` (1.14.7, DD16); a node serving
  several would need the version on `session_ticket`.
- **A replica that has not yet pulled a new account** answers `unknown_account` for it. A node that registered the
  account should `sync` before it hands the player on.

## The claim protocol (DN8)

To be written with DN8.

## Chat federation (DN10b)

To be written with DN10b.

## Admin actions across nodes (DN11)

- **Trust boundary**: an admin action relay message arrives over `/__node/admin` (`Service::Admin`) from an mTLS-authenticated peer node in `[[network.nodes]]`. The caller identity (`from`) comes from the verified client certificate's SAN/CN, never from the wire body.
- **Wire protocol (`network::relay`)**:
  - [ ] Versioned JSON (`ADMIN_RELAY_VERSION = 1`), `#[serde(deny_unknown_fields)]` on every request, response, action, and refusal struct, bounded to `MAX_ADMIN_WIRE = 64 KiB` inside `MAX_MESSAGE`.
  - [ ] Every free-form string (`reason`, `request_key`, `actor`, `approved_by`, refusal message) and JSON body (`HomeBody`) is scanned on both encode and decode by `holds_secret_key` (`looks_like_account_key` and `looks_like_admin_key`), refusing anything shaped like an account key (`RRK_...`) or an admin secret key (`RRA_...`) in any case.
  - [ ] Actor fields (`actor`, `approved_by`) carry only the public 8-hex key id (`^[0-9a-f]{8}$`) or `owner`, never a bearer secret or raw token.
  - [ ] Requests from a node not listed in `[[network.nodes]]` (`!is_peer(from)`) are refused with `AdminRelayRefusal::NotAPeer` before any state or ticket table is touched.
- **Authority, session revocation, and audit**:
  - [ ] When an operator kicks, bans, or times out a player whose home is another node, the originating node applies the action at the player's home node first (`Service::Admin`), then fans out to all remaining peers, and revokes session tickets (`Tickets::revoke`) on the originating node, the home, and each peer only after the authority accepts the action (`Admin` role protection and home validation cannot be bypassed to revoke an admin's session).
  - [ ] `Tickets::verify` checks the revocation floor (`revoked`) for every issuer, not only the local node, and `NetworkedHome` checks `Tickets::is_revoked_at` on held sessions before reusing or renewing a ticket, so a peer-relayed ban, timeout, or kick cuts off both new offline ticket admissions and already-held sessions immediately.
  - [ ] `unban` and `liftTimeout` clear the ticket revocation floor (`Tickets::allow`) across peers once applied.
  - [ ] Every relayed action is recorded in `admin_actions` with `via_node` (the authenticated peer name), `request_key`, and any second-operator `approved_by` id (satisfying guarded actions under `DH6` across nodes).
- **Idempotency and bounded down-peer queue**:
  - [ ] `AdminRelayService` deduplicates incoming `(from, request_key)` pairs in a bounded 4,096-entry cache (`Dedup`) and `Admin::apply` checks `Keys::has_relayed(via_node, request_key)` in `admin_actions` so replayed deliveries after a reconnect or restart are idempotent no-ops.
  - [ ] When a peer is unreachable, the action is persisted to `admin_relay_queue` via `Database::write`, bounded to `MAX_QUEUE_PER_PEER = 256` and `MAX_QUEUE_TOTAL = 2048` rows, surfaced immediately in the audit detail (`"pending on node <peer>"`) and the `pendingRelay` GraphQL query, and replayed by `AdminRelay::sweep()` when the peer returns.

## Fuzzing

Short runs happen on every push (CI's `fuzz` job). The run before the canary, from `rust/fuzz`:

```bash
cargo +nightly fuzz run network_directory -- -max_total_time=86400
cargo +nightly fuzz run network_link -- -max_total_time=86400
cargo +nightly fuzz run network_home -- -max_total_time=86400
cargo +nightly fuzz run network_ticket -- -max_total_time=86400
cargo +nightly fuzz run network_admin -- -max_total_time=86400
```

Signed: ________ (date, commit)
