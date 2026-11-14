<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Distributed mode: failover across nodes

A network of Dainty Lass servers (DD16) already shares one directory of accounts and usernames. With a fixed leader
(`[network] leader`, or the alphabetically first node) that directory has one weak point: while the leader is down,
no node can register an account or rename one. `[network] failover = true` removes it. The nodes elect the leader
among themselves, elect another within seconds when it goes down, and acknowledge a change only once most of them
hold it, so no acknowledged registration is lost with the node that took it.

The nodes talk over the node links they already use (mutual TLS under the cluster CA, `docs/security/network.md`),
and each peer can be reached over a private back channel such as Tailscale instead of its public address.

## What fails over and what doesn't

| Part | With `failover = true` |
| --- | --- |
| Directory (accounts' homes, usernames) | Elected leader, majority commit, automatic repair. Survives any minority of nodes going down |
| Logging in, playing, hunts | Unchanged: they run on each player's home node and never needed the leader. A node that is down takes its own players' sessions with it |
| An account's game data | Stays on its home node. `[history] ship_to_peer` keeps a copy of the change log and snapshots on a peer for recovery (DH7); it does not serve players |
| Chat's network-wide rooms | Still on the fixed leader (`effective_leader`): a follow-up |
| Remote management | Unchanged: managed installs keep their last good settings while the controller is away (DD27) |

## How it works

`crates/dainty-network/src/election.rs` is Raft's leader election (Ongaro's dissertation, chapters 3 and 9) and
`crates/dainty-network/src/directory.rs` its commit rule and log repair, on the directory the network already had.

- **Terms and votes.** Time is cut into numbered terms with at most one leader each. A node votes once per term,
  saves the vote to `.workspace/config/network-election.json` before answering, and votes only for a node whose
  directory log is at least as new as its own. A node needs the votes of a majority of every node listed, itself
  included, to lead.
- **Heartbeats.** The leader sends one to every node each `heartbeat_ms` (1 s). A follower that hears nothing for
  `election_timeout_ms` to twice that (5-10 s, random so two don't stand together) stands for election.
- **Pre-vote.** A node first asks whether it would win, without raising its term. A node cut off by a partition can't
  come back with a term that unseats a healthy leader.
- **Check-quorum.** A leader that hasn't heard back from a majority for the longest election timeout steps down, so
  the side of a partition without a majority never keeps one.
- **Majority commit.** A registration on the leader is written there, then every node is told to pull
  (`DirectoryCall::Notify`). The home gets its answer only when a majority holds the entry; otherwise it gets
  `no_quorum` and may ask again. A new leader first writes a `lead` entry of its own term and gets it onto a
  majority, which settles whatever its predecessor left half-done.
- **Repair.** A node that comes back holding entries the elected leader doesn't (writes it took but never got
  acknowledged) finds the newest entry both share, drops the rest and pulls again.
- **Fencing.** Every entry carries its term. A notify from an older term is refused, so a deposed leader can't count
  anyone towards a majority, and its writes fail instead of splitting the directory.

A failover takes the election timeout plus one round of votes: about 6-11 s with the defaults. Reads (who holds a
name, an account's home) never wait for it; registrations and renames get `no leader` until it is over.

## How many nodes

| Nodes | Can lose | Notes |
| --- | --- | --- |
| 1 | 0 | Leads itself; failover changes nothing |
| 2 | 0 | Both must be up to write. Don't: add a third |
| 3 | 1 | The smallest useful network |
| 4 | 1 | No better than 3 |
| 5 | 2 | |

The third node can be small: it needs the network module and its database, not players. Put the nodes in different
places, or one outage takes the majority.

## Setting it up

Every node needs the same `failover` setting and the same list of nodes (each lists the others).

1. Issue each node's certificate with every address it is reached at, the back channel's included:
   `dainty-host network issue-node-cert --ca cluster-ca.pem --node uk-1 --region uk --host uk.example.org --host 100.64.0.2 --host uk-1.tail1234.ts.net`.
   A node link checks that the certificate carries the address it dialled.
2. In each node's `settings.toml`:

   ```toml
   [network]
   node = "oregon-1"
   region = "oregon"
   failover = true            # leave `leader` out
   # heartbeat_ms = 1000
   # election_timeout_ms = 5000

   [[network.nodes]]
   id = "uk-1"
   region = "uk"
   host = "uk.example.org"    # what players and redirects use
   port = 443
   link = "100.64.0.2:443"    # node links dial this instead: the Tailscale address or MagicDNS name

   [[network.nodes]]
   id = "germany-1"
   region = "germany"
   host = "de.example.org"
   port = 443
   link = "germany-1.tail1234.ts.net:443"
   ```

3. Run `dainty-host setup` on each, then start them. `dainty-host network status` shows every peer up; the logs show
   `election: won; leading` on one node and `election: leadership changed` on the others.

A public gateway listens on every interface by default (`GATEWAY_BIND`), so its Tailscale address answers on the same
port. Node links are mutual TLS whichever way they come, so the back channel is for reachability and latency, not
for trust: a node still accepts only the nodes it lists, with certificates from the cluster CA.

On a server that already ran a fixed leader, switch every node in one go: stop them all, set `failover = true`,
remove `leader`, run `setup`, and start them. The directory's log is kept; the first leader elected is one whose log
is the newest. A node without failover answers neither elections nor notifies, so a half-switched network elects
nobody rather than two leaders.

### Timings

Keep `election_timeout_ms` at least three heartbeats and well above the worst round trip between two nodes;
`setup` refuses less. Over Tailscale between continents (100-300 ms) the defaults are right. Lower them only on a
LAN, where `heartbeat_ms = 200` and `election_timeout_ms = 1000` fail over in about 2 s.

### Adding or removing a node

Change one node at a time: add it to every node's list (and its list of every other), restart the nodes one by one,
and let the cluster settle (`network status`) before the next change. Two changes at once can leave two majorities
that don't overlap.

## When something goes wrong

- **The leader goes down.** The others elect one in about 10 s; registrations wait for it and then go on. When the old
  leader comes back it follows, drops anything it wrote but never got acknowledged, and catches up.
- **A partition.** The side with a majority keeps a leader and writes; the other side reads and refuses writes with
  `no leader`. Healed, the minority catches up.
- **More than half the nodes are down.** Nobody leads and nothing is written anywhere. Bring nodes back until a
  majority is up.
- **A node's disk is lost.** Start it with an empty directory and it pulls the whole log again, but it votes as soon
  as it is up, and a vote from a node that forgot what it held can help elect a leader missing an entry it had
  acknowledged. Lose and rebuild one node at a time, and give it a minute to pull the whole log before touching
  another. Keep `network-election.json` with the rest of `.workspace/config`: a node that
  lost it should stay down until the others have elected a leader without it, or it may vote twice in one term.
- **Clocks.** Elections use local timers, not wall time. The 30 s skew bound of every node link still applies.

The checklist for all of this is in [security/network.md](security/network.md#election-and-failover-docsdistributed-modemd);
the tests are `crates/dainty-network/tests/failover.rs` and the `election` module's own.
