<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the network (RX3, DX2)

The checklist for the network core plugin, `rust/plugins/network` (`docs/dainty-lass-plan.md` DD16-DD19, DN3-DN11).
`SECURITY_REVIEW.md` indexes it. DX2 gives it five parts: node certificates and the cluster CA, session tickets, the
directory, the claim protocol and chat federation. Each row adds its part with its code; this page has the directory
(DN4) so far. Region ping, which the gateway answers for the launcher, is `region-ping.md`'s.

How to use it: before the network's canary (DN14), copy this list into the review, check each item in the code (not
from memory), link the code or test that shows it, and sign at the end with the date and the commit reviewed. An
item that does not hold is a bug to fix before the canary, not a box to leave open.

## What it is and where the boundary is

The nodes of one operator's cluster trust each other (DD16), but each node is a separate server with its own host,
database and admins, and a node's link to another crosses the internet. What one node sends another must not let it
take what belongs to a third: another home's accounts, a name already claimed, the account key.

- **Node links:** one call is one request body and one answer body, JSON, at most `MAX_MESSAGE` (1 MiB) each way
  (`link.rs`). DN3 carries them over mutual TLS on host-only gateway routes and gives the service the caller's node
  name from its certificate; until then `inprocess.rs` carries them inside one process for tests. A service trusts
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
- **No transport yet.** DN3 puts these messages on mutual TLS; until it does, only the in-process link exists, and
  `[network]` in `host.toml` (DF3) doesn't create a directory.
- **Joining an existing server** (DN13) needs its accounts registered through the leader, with the clash renames,
  and its standalone log replaced by the leader's.

## Node certificates and the cluster CA (DN3)

To be written with DN3.

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
```

Signed: ________ (date, commit)
