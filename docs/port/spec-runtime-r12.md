<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime slice R12, replication graph and channel guards (RD27, plan item 11)

R12 is the replication slice of the MIT game runtime (`spec-runtime-surface.md` section 3.8 and section 8's R12 row,
approved 2026-11-13 with rulings RT-1 to RT-18 in `rulings-2026-11-13.md`). On a server it drives the game's
replication graph once a frame (the engine never does on this build, so without it no client receives its game state
or player state), and it keeps one invariant at every place an actor channel is filled, bound, sent or used for an RPC:
**a PlayerController is never replicated onto another player's connection**. A remote controller arriving on a
client makes the stock engine hand that client the other player's controller (`OnActorChannelOpen`), and the client's
own controller, pawn and HUD come apart. It owns the hooks RG-1, RG-4, RG-5, RG-6 and RG-7 and the feature id
`SP-3@R12`; RG-3 moves to R14 (section 12.2) and RG-2 is R13's (section 1).

An implementer builds from this page, `spec-runtime-r0.md`, `spec-runtime-r9.md` section 8, `spec-runtime-r11.md`
section 16, section 3 of `spec-runtime-r17.md` and the surface page alone, and never reads `restricted/`. What crossed
is facts: RVAs into `Dauntless-Win64-Shipping.exe` (1.14.7, CL 647472), instruction bytes, `.pdata` entries and one
virtual table (each checked against the maintainer's copy of that executable on 2026-11-13), engine layout offsets,
engine and game names, flag names, and behaviour. The AGPL side stands down through seam patch 0041 and R9's patch
0052 alone (section 14); R12's reserved patch number 0056 is not used.

Conventions are R0's: an RVA is `+0x0XXXXXXX` from the executable's image base; **S** server. **(call)** marks a
decision this page makes; **(ambiguity)** something the source did not settle; **(departure)** a place the MIT
runtime deliberately behaves differently from the AGPL one. All three are collected in section 18.

---

## 1. Scope

| R12 owns | Role | Section |
| --- | --- | --- |
| The identity tests every guard shares (is it a PlayerController, a connection, an actor channel; is the controller remote) | S | 5 |
| The controller channel quarantine table | S | 6 |
| RG-4: the graph's per-actor, per-connection replication step (`ReplicateSingleActor`), the pre-channel guard | S | 7 |
| RG-5: `UActorChannel::SetChannelActor`, the bind guard | S | 8 |
| RG-6: `UActorChannel::ReplicateActor`, the final guard | S | 9 |
| RG-7: `UNetDriver::ProcessRemoteFunctionForChannel`, the unbound-channel RPC guard (patch 0039) | S | 10 |
| `SP-3@R12`: the quarantine's stale sweep and the replication-graph drive, after the engine tick | S | 11 |
| RG-1: owned, not installed | S | 12.1 |

**Where the split with R13 falls.** RG-2 (`RouteAddNetworkActorToNodes`, `+0x01BFE430`) decides which graph node an
actor is filed in: it normalises every PlayerController before the graph files it and routes player roles and weapons
(patches 0015, 0016). That is relevance, and it is R13's with the weapon gap log and dormancy wake (`SP-3@R13`, 0014,
0015), the relevance settings pass and base-actor routing. R12 does not decide what is relevant to whom; it drives the
graph that does, and refuses one kind of result (a controller on the wrong connection) wherever the graph or any other
caller would produce it. R12's guards and R13's routing are independent: neither reads the other's state, and either
may be MIT while the other is AGPL.

**Not R12, though the surface page puts them near it:**

- RG-3 (`RouteRemoveNetworkActor`): its only behaviour that is not a log purges the player role's caches, which are
  R14's. It moves to R14 (12.2).
- The manual net tick and the replication graph's construction (NETD-3) are R11's; R12 drives the graph R11 made sure
  exists.
- The listen and the hub's net driver are R9's.
- No slice creates or pushes an actor channel by hand (R9 8.3): the driven graph opens every channel.

---

## 2. Where it goes

### 2.1 Files

| Crate | Path | Holds |
| --- | --- | --- |
| `rustic-isles-runtime-core` | `src/repgraph.rs` (a folder `src/repgraph/` past ~400 lines) | everything pure: the identity decisions over read values (5), the quarantine table (6), the four guards' decisions (7–10), the drive's checks (11), the line builders and budgets, the "once per pair" set, the flag table (13) |
| `rustic-isles-runtime` | `src/ffi/repgraph/` (`mod.rs`, `identity.rs`, `guards.rs`, `drive.rs`) | the four detours, the SP-3 handler, the reads and writes of game memory, the call into the graph; under `ffi` because all of it is FFI (R0 2, RT-1) |

`core/src/lib.rs` gains `pub mod repgraph;`, `ffi/mod.rs` gains `pub mod repgraph;`.

### 2.2 Shared lines R12 adds

| File | Edit |
| --- | --- |
| `core/src/registry.rs` | `LANDED` gains `"R12"` **only once it holds `"R11"`** (14.2). The `RG-3` line's slice becomes `"R14"` (12.2), and the `surface_ids` test's RG rule becomes "RG-2 → R13, RG-3 → R14, the rest → R12". RG-1, RG-4..7 and `SP-3@R12` exist already. One new test (16.1) |
| `core/src/offsets.rs` `TARGETS` | four lines, RG-4..7, all `FoundBy::RvaAndBytes` (17.1) |
| `core/src/offsets.rs`, a new `pub mod repgraph` | the data RVAs and layout offsets of 17.2 |
| `core/src/offsets.rs` test | RG-1 joins NETD-8 on the list of registered detour ids with no `TARGETS` line on purpose (12.1) |
| `src/ffi/memory.rs` | `pub fn frame_epoch() -> u32` (the epoch of R0's frame cache, `FRAME.epoch()`), if no slice has added it yet |
| `src/lifecycle.rs` | before `handlers::freeze()`: `ffi::repgraph::register()` (the SP-3 handler, server only). In the install list: R12's results at the positions of 4.5. Nothing else changes there |

`owned_count()` then counts 6 more ids (RG-1, RG-4..7, `SP-3@R12`).

### 2.3 What is pure and what is Windows-only

Pure (core, `cargo test` on every platform): every decision on this page over values read through R0's `Memory`
trait (a fake memory in tests), the quarantine table, every log text, the budgets, the flag table. Windows-only (DLL
crate): the detours, the call into the graph, `RtlCaptureStackBackTrace`, writes of game memory. Nothing in R12 runs
against the real executable in CI (surface 9).

---

## 3. What R12 consumes from other slices

| From | What | Used in |
| --- | --- | --- |
| R0 | the broker (`broker::hook`, `original`), `handlers::register_tick`, `ffi::memory` (`readable`, `readable_this_frame`, `read`, `verify_entry`, `function_entry`, `frame_tick`, and the `frame_epoch` getter of 2.2), `is_live_object` (R0 5.3), `sane_pointer_array`, the object names (`engine`), `flags::flag`, the log, the detaching flag | everywhere |
| R3 | the address text `frame_text` and the own-module stack trim (R3 3.2) | RG-5's caller (8) |
| R15 / R17's reflection kit (`reflect.rs`, R17 section 3) | `find_class(name)`, `is_a`, the checked `write<T>` (R17 3.5) | 5, 7 |
| R9 | `net_driver() -> Option<usize>` (R9 8.1): the hub's `GameNetDriver`, `Some` whenever R9 is listening | 11 |
| R11 | nothing called. R11 makes the graph exist (NETD-3) and drives the driver after R12's drive (its manual tick at SP-3 after `order = 800`, R11 8.2); R12's drive keeps below 800 (11.1) and bumps the frame epoch after calling game code (R11 8.3's rule) | 11 |

R11 offers R12 `net::sanitize`, `net::graph_class()` and `net::graph_self_constructed()` (R11 16). R12 needs none of
them: it never walks `ClientConnections`, and it identifies the graph by the driver's own pointer to it (11.2)
**(call)**.

R9's `net_driver()` is the only listen state R12 reads. Because R12 lands with or after R11 (14.2), and R11 with or
after R9, it is always the MIT's.

---

## 4. Common rules for every R12 hook and handler

### 4.1 Before anything else

1. **Detaching** (R0 3.3): if set, the detours call `next` with their arguments and return its answer; the tick
   handler returns.
2. Call R0's `lifecycle::write_status_once()` (first use only).
3. The work runs inside `catch_unwind`; a panic logs `[Panic] <id> ...` once (R0 2), then the detour calls `next` with
   its arguments (the guard fails open for that call), the handler returns.

Every game read goes through R0's checked reads; a read that fails ends that piece of work as the section says. No
R12 code catches a hardware fault **(departure: section 11.4)**.

### 4.2 Install

RG-4..7 are `broker::hook`ed with `order = 0`, band `Mit`, after `verify_entry(rva, bytes)` of 17.1 succeeds. An id
whose check fails is not hooked and reports `prologue mismatch`; one gated off by its flag reports `flag <NAME>`. The
AGPL hooked RG-4 only after the same kind of check and the other three by bare RVA **(departure: R12 checks all four)**.

When RG-4's check fails, RG-5 and RG-6 still hold the invariant (the AGPL's own note on that hook), so a failed RG-4
logs and the others install.

### 4.3 Lines

Every R12 line carries the tag `[MitRepGraph]`. Addresses print as `0x<HEX>` (upper case, no padding; `null` for 0),
counts in decimal. A line that can repeat has a **budget**: the first `B` lines of that kind go out, the `B+1`-th
becomes `[MitRepGraph] <kind>: further lines suppressed`, the rest are dropped. Budgets are per process, relaxed
atomics. Each section names its budget **(departure: the AGPL logs several of these on every occurrence)**.

### 4.4 Threads

Every R12 detour and the handler run on the server's game thread: the graph's replication runs inside R12's drive
(11) and the engine's net flush, RPCs inside the game's `ProcessEvent`. R12's tables (the quarantine, the "logged
pair" sets, the class memo of 5.2) are single-thread state: a thread-local or a game-thread-only cell, never a lock
on the hot path. **(call)** A call on another thread (none is expected) sees an empty quarantine and an empty memo,
and its guard decides from its own reads alone.

### 4.5 Install order (RT-17 for R12)

**Settled:** no R12 id depends on another id's install order; all four guards must be in place before the first
client connects, which every install in `DllMain` is. The MIT build keeps the AGPL's positions (surface 3.11): after
R11's NETD-8 line and before NETD-3, in the order RG-1 (its "not ported" line), RG-4, RG-5, RG-6, RG-7. The SP-3
handler is registered before `handlers::freeze()`. Each id gives one install line through R0's status
(`[MitRuntime] install RG-<n> ok|<reason>`). R12 installs nothing on a client.

---

## 5. Identity: who is a controller, and when it is remote (S)

### 5.1 Layout

| Object | Fact | Value |
| --- | --- | --- |
| any `UObject` | class | pointer at `+0x10` (R0) |
| `AActor` | the flags byte logged with a block | `u8` at `+0x58` (readable span `0x60`) |
| `APlayerController` | `NetConnection` (the connection it belongs to) | pointer at `+0x418`; readable span `0x420` |
| `UNetConnection` | `PlayerController` (inherited from `UPlayer`) | pointer at `+0x30`; readable span `0xA0` |
| `UNetConnection` | `OwningActor` | pointer at `+0x98` |
| `UChannel` | `Connection` | pointer at `+0x28` |
| `UActorChannel` | `Actor` | pointer at `+0x70`; readable span `0x78` (R9's `CHANNEL_ACTOR`) |
| the graph's per-connection manager (`UNetReplicationGraphConnection`) | its `UNetConnection` | pointer at `+0x28`; readable span `0x30` |
| a connection's cached actor info (`FConnectionReplicationActorInfo`) | its actor channel | pointer at `+0x00` (8 bytes) |

The classes, by the reflection kit's `find_class` (short names): `PlayerController`, `NetConnection`, `ActorChannel`.
Each is found once, on first need, and kept; while one is not found yet, the tests that need it answer false (a guard
then fails open) and the search is retried at most once a second.

### 5.2 The four tests

- **live connection** `conn(p)`: `p` non-null, `readable(p, 0xA0)`, R0's `is_live_object(p)`, `is_a(p, NetConnection)`.
- **live actor channel** `chan(p)`: `p` non-null, `readable(p, 0x78)`, `is_live_object(p)`, `is_a(p, ActorChannel)`.
- **live controller** `pc(p)`: `p` non-null, `readable(p, 0x420)`, `is_live_object(p)`, `is_a(p, PlayerController)`.
- **live or null** `live_or_null(p)`: `p` when `is_live_object(p)`, else 0 (used for a connection's controller and
  owning actor).

`is_a` here is by class pointer: walk the object's class and its supers (`SuperStruct`, `+0x40`, at most 64 steps,
each class pointer `readable` for `0x48`) and compare with the found class pointer. If the kit has only the by-name
form, R12 keeps this walk in its own core module over R0's `Memory` **(call)**.

**The hot path (call).** RG-4 runs for every actor on every connection every frame, and RG-6 for every channel sent.
The AGPL keeps them cheap with an O(1) liveness test before anything else; R0's `is_live_object` costs up to three
`VirtualQuery` calls. So RG-4 and RG-6 first ask a cheaper question: `readable_this_frame(actor, 0x60)`, read the
class pointer, and look it up in a **class memo** (class pointer → "is a `PlayerController`"; a fixed open-addressed
table of 1 024 entries, filled by the class-chain walk, emptied whenever `GWorld` (R0 6.2) differs from the value seen
when the memo was last used, because classes can only be unloaded across a map change). Only an actor whose class the
memo says is a controller goes on to the full `pc(actor)` test. A full table computes without memoising. The verdict
is the same as the AGPL's; only its cost changes.

### 5.3 Remote

For a controller `actor` and a target connection `target` (already `conn(target)`), with
`actor_conn` = the pointer at `actor + 0x418`, `target_pc` = `live_or_null(target + 0x30)` and `target_owner` =
`live_or_null(target + 0x98)`:

- **remote by connection:** `conn(actor_conn)` and `actor_conn != target`;
- **remote by identity:** not `conn(actor_conn)`, `target_pc` or `target_owner` is non-null, and `actor` equals
  neither.

**Remote** is either. Neither case (the controller's own connection, or no identity to compare) is not remote: the
guards **fail open** when they cannot prove the controller is another player's, since blocking a controller's own
initial channel strands every controller-owned state on its client. The reason texts are
`actor-netconnection-mismatch` (by connection) and `target-identity-mismatch` (by identity). This decision is pure
(core) over the read pointers and the four test answers.

---

## 6. The quarantine table (S)

A channel that was refused (or found serving a controller on the wrong connection) is **quarantined**: RG-5 refuses
any later bind on it, RG-6 sends nothing on it, RG-7 sends no RPC on it. Nothing closes it **(as in the AGPL on
1.14.7: its deferred close ran from a hook this build never installs)**; an entry leaves the table only when the
sweep (11.3) finds it stale.

| Field | Meaning |
| --- | --- |
| `channel` | the quarantined channel (non-null marks the entry used) |
| `actor` | the controller it carried or was to carry |
| `wrong_connection` | the connection it must not serve |
| `correct_connection` | the controller's own connection, or 0 when unknown |
| `caller` | the caller's RVA (RG-5 only, else 0) |
| `queued_ms` | `GetTickCount64` at queueing |
| `source` | `conn-actor-info-mismatch`, `remote-controller-pre-channel`, `set-channel-actor` or `replicate-actor-final` |

- **64 entries**, and a count of used entries kept current on every change, so "is it quarantined" costs one compare
  while the table is empty (the normal state).
- **queue(channel, actor, wrong, correct, source, caller) -> bool:** a null channel answers false. An entry for the
  same channel exists: fill only its fields that are still 0 (null), answer true, no line. Else take the first free
  entry, fill it, set `queued_ms`, log `[MitRepGraph] quarantine: queued channel=<p> actor=<p> wrong=<p> correct=<p>
  source=<source> caller=+0x<RVA>` (budget 64), answer true. No free entry: log `[MitRepGraph] quarantine: table full
  channel=<p> actor=<p> wrong=<p> correct=<p>` at most once a second, answer false.
- **contains(channel) -> bool:** the count is 0 → false; else a linear search.
- **stale(entry)** (pure over the reads): with `live = chan(channel)`, `cur_conn` and `cur_actor` the channel's
  `+0x28` and `+0x70` when `live` (else 0): stale when not `live`, or `wrong_connection != 0` and
  `cur_conn != wrong_connection`, or `cur_actor != 0`, `actor != 0` and `cur_actor != actor`. A stale entry is
  cleared and logs `[MitRepGraph] quarantine: dropped stale channel=<p> expectedActor=<p> currentActor=<p>
  expectedConnection=<p> currentConnection=<p>` (budget 64). Why: an entry that outlives its channel would make a new
  channel later allocated at the same address be refused for an unrelated actor.

**Full table (departure).** The AGPL's bind guard (RG-5), when it could not queue, closed the channel at once through
the channel's virtual `Close` (virtual table `+0x288`, reason 3) inside a fault catch. R12 does not: the bind is still
refused, so the channel carries no actor, RG-7 drops its RPCs ("channel without actor") and RG-6 refuses it by its own
identity test if an actor is ever bound. A full table logs as above.

The table, `queue`, `contains` and `stale` are pure (core) and tested with fake pointers.

---

## 7. RG-4: the pre-channel guard on `ReplicateSingleActor` (S)

| | |
| --- | --- |
| Target | `+0x00F2C230`, the replication graph's per-actor, per-connection step. Reached from the drive: the Archon graph's `ServerReplicateActors` (`+0x01C034C0`) → the base `UReplicationGraph::ServerReplicateActors` (`+0x00F2EB60`) → `+0x00F2AFF0` → here. It reads the actor's destroy bit (`+0x5C`), reuses the cached channel in the connection's actor info when there is one, otherwise creates one (`CreateChannelByName`, `+0x03F62E90`, on the manager's connection), binds it (`SetChannelActor`, RG-5) and sends it (`ReplicateActor`, RG-6) |
| Signature | `extern "C" fn(graph, actor, conn_actor_info, global_info, actor_info_map, conn_manager, frame: u32) -> u64` |
| Returns | the bits written; the graph adds it to the connection's bandwidth counters and its own total (its epilogue returns it in full `RAX` at `+0x00F2C6C9`). **A blocked call returns 0** ("nothing written") |
| Gate | none (13) |

Why here: the bind and send guards (RG-5, RG-6) are too late on their own. Measured in a two-player escalation: the
send guard logged its block, yet about 100 ms later the receiving client still entered the stock
`OnActorChannelOpen` → `HandleClientPlayer` and adopted the other player's controller; its relic screen's owning
player went null and its clicks could never send `ServerSelectRelicOption`. A cached channel can also belong to the
other player's connection, so the cached one needs the same check as a new one.

Behaviour:

1. Is `actor` a controller (5.2, memo first, then `pc(actor)`)? No: call `next` with every argument and return its
   answer.
2. Read: `conn_manager` valid when `readable(conn_manager, 0x30)` and `is_live_object(conn_manager)`; `target` = its
   `+0x28` when valid, else 0; `target_ok` = `conn(target)`. `actor_conn` = `actor + 0x418`; `actor_conn_ok` =
   `conn(actor_conn)`.
3. **The cached channel.** `cached` = the pointer at `conn_actor_info` when `readable(conn_actor_info, 8)`, else 0.
   `cached_ok` = `chan(cached)`; when `cached_ok`, `cached_conn` and `cached_actor` are its `+0x28` and `+0x70`, else 0;
   `cached_conn_ok` = `conn(cached_conn)`.
   - `conn_mismatch` = `cached_ok` and `target_ok` and `cached_conn_ok` and `cached_conn != target`;
   - `actor_mismatch` = `cached_ok` and `cached_actor != 0` and `cached_actor != actor`;
   - `unusable` = `cached != 0` and (not `cached_ok` or not `cached_conn_ok`).

   If any holds:
   - when `conn_mismatch`, `cached_ok`, `cached_conn_ok` and (`cached_actor` is 0 or `actor`): `queue(cached, actor,
     cached_conn, actor_conn_ok ? actor_conn : target, "conn-actor-info-mismatch", 0)`;
   - **clear the slot:** if the pointer at `conn_actor_info` still equals `cached`, write 0 there (checked write);
     `cleared` = 1 when written, 0 when it no longer matched, -1 when the write's check failed;
   - log `[MitRepGraph] pre-channel: reset cached channel actor=<p> actorConnection=<p> target=<p> channel=<p>
     channelConnection=<p> channelActor=<p> cleared=<n> reason=<cached-channel-connection-mismatch|
     cached-channel-actor-mismatch|cached-channel-unusable>` (the first reason that holds, in that order; budget 64).
4. **Target unproven:** `target_ok` false → fail open: log `[MitRepGraph] pre-channel: target connection unproven,
   allowed actor=<p> manager=<p> target=<p> frame=<n>` at most once a second (budget 64), then call `next` and return.
5. **Remote** (5.3, with `target`)? No: call `next` and return its answer.
6. Remote: re-read the slot (`before` = the pointer at `conn_actor_info`, 0 when unreadable). If `chan(before)`: when
   its `+0x28` equals `target` and its `+0x70` is 0 or `actor`, `queue(before, actor, target, actor_conn_ok ?
   actor_conn : 0, "remote-controller-pre-channel", 0)`; then clear the slot as in step 3 when it still equals `before`.
7. Log once per `(actor, target)` pair (a 64-pair set; when it is full, budget 64 for the rest):
   `[MitRepGraph] pre-channel: blocked actor=<p> actorConnection=<p> target=<p> targetController=<p> targetOwner=<p>
   channelBefore=<p> frame=<n> reason=<5.3 reason>`.
8. **Absorb:** return 0 without calling `next` (no channel is created for that controller on that connection this
   frame).

The step order and every decision above are pure (core) over the read values; the DLL crate reads, writes and calls.

---

## 8. RG-5: the bind guard on `UActorChannel::SetChannelActor` (S)

| | |
| --- | --- |
| Target | `+0x03D8FB20` |
| Signature | `extern "C" fn(channel, actor, flags: u64)`, no return value (`flags` is passed through untouched) |
| Why | every native caller of the bind, including the graph's multicast RPC path (`UReplicationGraph::ProcessRemoteFunction` binds a channel it just created at `+0x00F27D40`) outside RG-4 |

Behaviour:

1. `caller` = the RVA of the first stack frame outside this DLL (`RtlCaptureStackBackTrace` of up to 4 frames, R3's
   own-module trim; 0 when none lies inside the executable).
2. `contains(channel)`: log `[MitRepGraph] bind: refused quarantined channel=<p> actor=<p> caller=+0x<RVA>` (budget
   64) and **absorb** (return without calling `next`).
3. If `chan(channel)` and `pc(actor)`: `chan_conn` = `channel + 0x28`, `actor_conn` = `actor + 0x418`, `previous` =
   `channel + 0x70`; `chan_conn_ok` = `conn(chan_conn)`, `actor_conn_ok` = `conn(actor_conn)`; when `chan_conn_ok`,
   `chan_pc` = `live_or_null(chan_conn + 0x30)` and `chan_owner` = `live_or_null(chan_conn + 0x98)`, else both 0.
   - wrong by connection: `chan_conn_ok`, `actor_conn_ok`, `chan_conn != actor_conn`;
   - wrong by identity: `chan_conn_ok`, not `actor_conn_ok`, (`chan_pc` or `chan_owner` non-null), `actor` equals
     neither.

   (This is 5.3 with the channel's connection as the target, which here must itself be proven.) Wrong:
   `queued = queue(channel, actor, chan_conn, actor_conn_ok ? actor_conn : 0, "set-channel-actor", caller)`; log
   `[MitRepGraph] bind: refused actor=<p> actorConnection=<p> channel=<p> channelConnection=<p> channelController=<p>
   channelOwner=<p> previousActor=<p> flags=0x<HEX> caller=+0x<RVA> reason=<5.3 reason> queued=<0|1>` (budget 64);
   **absorb**.
4. Otherwise call `next(channel, actor, flags)`.

A refused bind leaves the channel with no actor; that is why RG-7 exists.

---

## 9. RG-6: the final guard on `UActorChannel::ReplicateActor` (S)

| | |
| --- | --- |
| Target | `+0x03D8A2D0` (the function holding the engine's "ReplicateActor called while ..." texts) |
| Signature | `extern "C" fn(channel) -> i64`, the bits written (its epilogue returns them in full `RAX` at `+0x03D8BDAB`; RG-4's caller adds them up) |
| Returns when blocked | **0**. An earlier AGPL version declared this hook `bool`, then `void`, and handed the graph whatever `RAX` held |

The last boundary before a controller's state is serialised. It changes nothing in the graph's lists: changing them
while `ServerReplicateActors` iterates caused disconnects in an earlier build, so it only refuses and queues.

Behaviour:

1. `contains(channel)`: return 0 (absorb), no line.
2. `channel` non-null and readable for `0x78`: `actor` = its `+0x70`. If `actor` is a controller (5.2, memo first,
   then `pc(actor)`):
   - `target` = `channel + 0x28`; `target_ok` = `conn(target)`; `target_pc`, `target_owner` from `target` when
     `target_ok`, else 0; `actor_conn` = `actor + 0x418`; `known` = `conn(actor_conn)`.
   - remote by connection: `known`, `target_ok`, `actor_conn != target`; remote by identity: not `known`, (`target_pc`
     or `target_owner` non-null), `actor` equals neither. (5.3, except that by identity the target need not be proven
     live: the AGPL's rule, kept.)
   - Remote: `queue(channel, actor, target, known ? actor_conn : 0, "replicate-actor-final", 0)`; log once per
     `(actor, target)` pair (its own 64-pair set; budget 64 past it) `[MitRepGraph] send: blocked actor=<p>
     actorConnection=<p> target=<p> targetController=<p> targetOwner=<p> flags=0x<HEX of the byte at actor+0x58>
     reason=<5.3 reason>`; return 0.
3. Otherwise return `next(channel)`.

Not ported: the AGPL's per-class send frequency tally (`REPGRAPH_DIAG.flag`) and its per-connection bit counters for
the performance log **(departure, RT-3)**.

---

## 10. RG-7: the unbound-channel RPC guard on `UNetDriver::ProcessRemoteFunctionForChannel` (S, patch 0039)

| | |
| --- | --- |
| Target | `+0x03FA09B0`, the public wrapper; its private body calls the channel's actor's `CallPreReplication` (`+0x03FA0CE2` reads the channel's `+0x70`) before the channel's first bunch, then `ReplicateActor` |
| Signature | `extern "C" fn(driver, channel, class_cache, field_cache, target_obj, connection, function, parms, out_parms, stack, is_server: u8, send_policy: i32, remote_function_flags: *mut c_void)`, no return value (UE 4.26's order; the graph's caller at `+0x00F27DDF` passes `is_server` as a byte, `send_policy` as a 32-bit value and the flags by pointer) |
| Gate | off-flag `DISABLE_RPC_UNBOUND_CHANNEL_GUARD.flag`, read once at install |

Why (measured, two-player escalation, 21–22 Oct 2026): the amp fires a multicast on the chooser's controller (or a
component it owns). The graph's multicast path opens a channel for that controller on the other player's connection,
binds it (RG-5 refuses: a remote controller channel), and then always calls this function. The private body
dereferences the channel's null actor (`+0x03F90A77` reading `null+0x10`), the fault unwinds the whole server RPC, and
the rest of the amp choice, including the round change that spawns the geyser to the next arena, never runs. A second
multicast in the same frame finds the same channel cached and faults again. A controller is only relevant to its
owner, so the stock engine never delivers its multicasts to another connection: dropping exactly this case changes
nothing a player should see, and the owner's copy goes out on its own channel.

Behaviour:

1. `channel` null, or `is_server == 0`: call `next` with every argument and return.
2. **Drop** when `contains(channel)` (`reason=quarantined-channel`), or else when the pointer at `channel + 0x70` reads
   as 0 or cannot be read (`reason=channel-without-actor`).
3. A drop logs `[MitRepGraph] rpc: dropped fn=<function's name> target=<target_obj's name> channel=<p>
   connection=<p> reason=<reason> count=<n>` for the first 64 drops and every 256th after (`n` counts drops from 1;
   names by R0's `engine` names, `null` or `unreadable` when they cannot be read). Then **absorb**.
4. Otherwise call `next` with every argument.

With the flag present: install line `flag DISABLE_RPC_UNBOUND_CHANNEL_GUARD` and nothing hooked. Either way one line
`[MitRepGraph] rpc guard <on|off (DISABLE_RPC_UNBOUND_CHANNEL_GUARD.flag)>`.

---

## 11. `SP-3@R12`: the stale sweep and the graph drive (S)

### 11.1 Registration and position

`handlers::register_tick`, `slice: "R12"`, `order: 500` **(call)**, `after` only. When R12 is in `LANDED`, `SP-3@R12`
is owned and the AGPL skips both of its own blocks (seam 0041).

**Position (cross-slice).** The AGPL ran, right after the engine tick: the stale sweep, then R13's weapon gap log and
dormancy wake (`SP-3@R13`), then the drive, then R7's progression resume, the diagnostics, and last its listening block
(R8's expected-player sync and connection count, R9's driver maintenance, R11's manual tick). As an MIT after handler
the sweep and the drive run after every AGPL post-tick block (R0 8.2), so while R13 and R7 are AGPL their blocks run
before the drive instead of R7's after it; the progression resume does not touch the graph, so the change has no
effect **(call)**. Among MIT after handlers: R12 at 500, below R8's listening-block steps (below 700), R9's maintenance
(700), R11's manual tick (800) and R9's listen (900), which is the AGPL's order. R13's per-tick weapon blocks should
take orders below 500 to keep theirs (section 15).

### 11.2 Each frame

1. **Stale sweep** (6): when the quarantine's count is non-zero, test every used entry with `stale` and clear the stale
   ones. Any thread-local "processing" guard of the AGPL is not needed: nothing in the sweep calls game code.
2. `driver` = R9's `net_driver()`; `None`: return silently.
3. `readable_this_frame(driver, 0x6F0)`; else return.
4. `graph` = the pointer at `driver + 0x6E8` (`UNetDriver::ReplicationDriver`); require it non-null and
   `readable_this_frame(graph, 0x4C8)`; require the pointer at `graph + 0x30` (the graph's own net driver) to equal
   `driver`. Any failure: return (the graph does not exist yet, or belongs to another driver).
5. `vtable` = the pointer at `graph + 0x00`; `fn` = the pointer at `vtable + 0x2F8` (readable for 8). That slot is
   the graph's `ServerReplicateActors`; for the game's `ArchonReplicationGraph` it holds `+0x01C034C0` (its virtual
   table is at `+0x05497AF0`; checked on 2026-11-13, as is slot `+0x308` holding RG-2's `+0x01BFE430`).
6. **The call check (call, 11.4):** `fn` must lie inside the executable image and be a `.pdata` function entry
   (`function_entry(rva) == rva`). Checked once per distinct `fn` and remembered. A failure logs `[MitRepGraph]
   drive: slot +0x2F8 of graph=<p> is <p>, not a function of the game; not called` once per distinct `fn`, and returns.
7. **Re-entry:** a thread-local flag (const-initialised, no destructor). When set, return: a nested engine tick inside
   the graph must not drive again. Set it, call `fn(graph, delta)` (`extern "C" fn(graph, delta: f32) -> i32`, the
   number of actors replicated), clear it on every exit path.
8. R0's frame epoch bump (`frame_tick(true)`): game code ran.
9. The first 6 drives log `[MitRepGraph] drive: graph=<p> class=<short name> fn=+0x<RVA> -> <result>
    connections=<i32 at graph+0x40> deferred=<i32 at graph+0x4C0>` (each count `-1` when unreadable).

`delta` is the frame's delta the SP-3 after handler receives. Steps 3 to 6 are pure decisions over the read values.

### 11.3 Why the drive (a fact of this build)

`UNetDriver::TickFlush` (`+0x03FAE700`) walks the client connections and ticks each, but never calls the driver's
replication driver. The 1.12 runtime compensated by hooking the driver's `ServerReplicateActors`; on 1.14.7 that hook's
target was never resolved, so the port lost the drive. Undriven, the graph still files actors into its nodes and a
connection's connect-time actors still arrive, but the connection's actor-info map is never filled by a gather, so no
actor that only an always-relevant route delivers, `AGameStateBase` and `APlayerState` among them, reaches the client.
Measured 2026-10-05: with the drive, the hub's connection went from 2–5 open channels to 89–100, and the client from a
120 s "Loading timeout while joining the server" to a playable Ramsgate. The Archon override walks its deferred
owner-only list (data `graph+0x4B8`, count `graph+0x4C0`) and then tail-calls the base `ServerReplicateActors`
(`+0x00F2EB60`).

The call goes through the virtual table slot, not a fixed RVA, as in the AGPL: the graph class is the game's choice
(R11 10.1 may fall back to `BasicReplicationGraph`, whose slot holds another function), and step 6 keeps that safe.

### 11.4 No fault catch **(departure, call)**

The AGPL calls the slot inside structured exception handling and logs up to six faults, then keeps going. R12 does
not catch: a fault inside the graph ends the server through R3's crash path (EXIT-12 logs it, EXIT-14 bounds the
wait), and the Director restarts it. The reasons are R11's (R11 8.4): no MIT structured exception handling (R0 call 7),
a caught fault in the engine's replication leaves a half-dead hub, and the faults the AGPL saw here came from a wrong
slot it no longer uses. The check of step 6 replaces the catch for the one thing the AGPL guarded against (a slot
that is not a function). This follows R11's ruling (R11 19.2, call 5); if the maintainer rules for a recovery there,
it applies here too.

---

## 12. RG-1 and RG-3

### 12.1 RG-1: owned, not installed **(call)**

RG-1 is the graph's `ServerReplicateActors` itself (`+0x01C034C0`). The AGPL hooks it only to count and log its first
eight calls with their caller, a diagnostic that answered whether the engine drives the graph (it does not; 11.3).
R12 owns RG-1 (so the AGPL never installs it) and installs nothing: install line `not ported` **(RT-3)**. The drive's
own first lines (11.2 step 9) say the same thing. `TARGETS` has no RG-1 line; its RVA and bytes are recorded in
`pub mod repgraph` (17.2) for the drive's log and for the record.

### 12.2 RG-3 moves to R14 **(call, ruling)**

RG-3 (`RouteRemoveNetworkActor`, `+0x01BFE4C0`, `extern "C" fn(graph, info)`, where `info` is the engine's
`FNewReplicatedActorInfo` with the actor at `+0x00`) does two things in the AGPL, both for player roles:

- **a purge:** when the actor is a live `ArchonPlayerRole`, it clears every entry for that role in the AGPL's player
  role caches (role routes, role channel states, two Tempest tables and the Frank drone state) **before** the graph
  removes it. The role can still be a registered object while its destroying bit (`+0x5C`, `0x08`) is set, so waiting
  for the next liveness sweep leaves a dying pointer in those caches;
- a membership log (diagnostic).

Those caches and every reader of them are R14's (player role routing, retries, Tempest modifiers). If R12 owned RG-3,
the AGPL would stop installing its hook while its R14 caches are still live, and the purge would be lost until R14
lands. So **RG-3's registry line names R14** (2.2), the AGPL keeps the whole hook until then (exactly today's
behaviour, no patch), and R14's spec decides what its MIT side needs on route removal for its own caches.

**If the maintainer keeps RG-3 in R12** instead: R12 owns it and installs nothing, and a runtime patch 0056 makes the
AGPL install a purge-only RG-3 wrapper through `rir_chain_add_inner` while `SP-3@R14` is not owned. Recommended: the
move; it needs no AGPL change. Facts for R14: RG-3's first bytes are `48 89 5C 24 08 57 48 83 EC 20` (a `.pdata`
entry, checked 2026-11-13).

---

## 13. Flags (RT-14 for R12)

Appendix A gave R12 six flags. Settled:

| Flag | Kept? | Meaning in the MIT runtime |
| --- | --- | --- |
| `DISABLE_RPC_UNBOUND_CHANNEL_GUARD` | kept | read once at install: RG-7 not installed (10) |
| `DISABLE_PC_CHANNEL_GUARD` | **dropped** | an A/B switch (2026-10-22) that bypassed RG-4 and RG-5 (not RG-6) to compare a two-player escalation with and without them; its own note says without them a remote controller may be adopted. A switch that can only reintroduce the controller swap is not kept **(ruling)** |
| `DISABLE_GRAPH_DRIVE` | **dropped** | returns to the undriven graph, where no client ever receives its game state or player state (11.3); it can only make a server unjoinable **(ruling)** |
| `REPGRAPH_DIAG` | **dropped** | diagnostics only (RG-6's frequency tally, the AGPL's per-tick graph dumps) **(RT-3)** |
| `SRA_RATE_CAP` | **dropped** | dead on 1.14.7 already: its only reader is a hook the AGPL never installs on this build |
| `DIAG_NATURAL` | **dropped** | not R12's code: a 2026-07 diagnostic that turned off the server's readiness and restart overrides (R7's LD-1, R20's `PlayerCanRestart`, R14's input activation) to watch the engine's natural lifecycle. R7's spec left the decision here. Dropping it means no MIT slice reads it, and R7, R14 and R20 need no check for it **(RT-3)** |

A dropped flag left in a game folder has no effect on the MIT side. While a slice that reads `DIAG_NATURAL` is still
AGPL, the AGPL still honours it there.

---

## 14. The AGPL side and the other slices

### 14.1 How the AGPL stands down (seam patch 0041, no new patch)

- RG-1, RG-4, RG-5, RG-6 and RG-7 are in 0041's RVA table. When R12 owns them, the AGPL makes no wrapper and its
  "original" pointers become `rir_original(target)`; its enable calls succeed as no-ops. Its RG-4 install still runs
  its prologue check first, which is harmless.
- `SP-3@R12` gates both AGPL blocks: the stale sweep and the drive.
- The quarantine table is only read and written by the AGPL's RG-4..7 bodies, the sweep and a deferred-cleanup routine
  that only a never-installed hook calls. With R12 owned, no AGPL code touches the AGPL's table; the MIT keeps its
  own. No state crosses.
- RG-3 is R14's (12.2): the AGPL keeps it.
- What stays AGPL and is harmless: its install lines (the seam answers `MH_OK`, so it still logs `create=MH_OK` for
  hooks it no longer installs); its once-only `[RpcChannelGuard] DISABLE_RPC_UNBOUND_CHANNEL_GUARD.flag ...` line at
  install (it reads the flag whether or not it installs); its `SRA_RATE_CAP` notice; and its performance log's
  replication counters, which stop counting (they were fed by its RG-6 body; diagnostics, RT-3).
- The AGPL's own driver pointer is R9's (patch 0052), which R12 reads through R9's interface.

So nothing beyond 0041 and 0052 is needed, and **R12's reserved patch number 0056 is unused** (unless the maintainer
rules against the RG-3 move, 12.2).

### 14.2 R12 lands with or after R11 **(call)**

R12's drive needs R9's `net_driver()`; R11 already requires R9 (R11 15.2). The surface makes R12 depend on R11, and
the drive's position assumes R11's manual tick flushes after it. So `LANDED` holds `"R12"` only when it holds `"R11"`,
and a registry test enforces it (16.1). The R12 implementer's branch adds `"R12"` to `LANDED` only if `"R11"` is
already there on `plan`; otherwise the integration pass adds it when it merges R12 after R11. Until then R12's code
builds and its pure tests run, and it installs nothing.

**R12 alone with R11 (R13, R14, R20 still AGPL) is safe:** the AGPL's RG-2 routing, weapon wake, relevance pass and
player role work read the driver and graph from R9's export (patch 0052) and never read the quarantine.

Row for R0 7.5's cross-slice table:

| State | Set by | Read by | Rule |
| --- | --- | --- | --- |
| the player role caches' purge on route removal | RG-3 | the AGPL's R14 blocks | **RG-3 is R14's** (12.2): it moves with the caches it purges |

### 14.3 Rulings settled here

- **RT-14 for R12:** section 13.
- **RT-17 for R12:** section 4.5; no order dependency.
- **RT-18 for R12:** R12 registers no `ProcessEvent` handler and matches no names.
- RT-3: RG-1, RG-6's tally and the graph dumps are not ported.
- RT-12, RT-13, RT-15 and RT-16 do not touch R12.

---

## 15. What other slices need from R12

| Slice | Needs |
| --- | --- |
| R11 | nothing called; R12's drive at SP-3 after `order = 500` < R11's manual tick (800), as R11 16 asked |
| R8 | its listening-block steps keep orders between 500 and 700 if it wants the AGPL's order (the drive before them); nothing depends on it |
| R13 | `repgraph::graph() -> Option<usize>`: the graph of R9's driver after the checks of 11.2 steps 2 to 4 (computed on call, game thread), for its relevance pass and routing; `repgraph::is_quarantined(channel) -> bool` should a weapon or pawn routing step need to skip a refused channel; its SP-3 weapon blocks at orders **below 500** (before the drive, as in the AGPL); the layout of 17.2 (`GRAPH_CONNECTIONS`, `CONN_MANAGER_CONNECTION`, `CHANNEL_CONNECTION`) to share rather than repeat. RG-2 normalises every controller before the graph files it; R12's guards are the backstop behind that and must stay in place whatever R13's routing does |
| R14 | RG-3 (12.2) with the facts there; `repgraph::graph()`; the identity tests of 5.2 (`pc`, `conn`, `chan`, exposed from `repgraph`) for its role channel states, rather than a second copy |
| R20 | nothing called. Its possession and post-login behaviours rely on the drive running (a late joiner receives the game state only through it) and on the guards (a respawned controller is never sent to another player) |

---

## 16. Tests

### 16.1 In CI

Core (`cargo test -p rustic-isles-runtime-core`), over a fake `Memory`:

- **Remote:** every row of 5.3: own connection (not remote), other live connection (remote by connection, its
  reason), unknown controller connection with the target's controller equal / owner equal / neither / no identity
  (not remote, not remote, remote by identity, not remote).
- **Quarantine:** queue fills the first free slot and logs; a second queue of the same channel only fills zero fields
  and does not log; the 65th distinct channel answers false with the "table full" line, not twice within a second;
  `contains` false on an empty table without searching; `stale` for a dead channel, a live channel on another
  connection, another actor, and a still-wrong binding (kept).
- **RG-4 decisions:** a non-controller passes through; each of the three cached-channel reasons, with the queue only
  for the connection mismatch with a matching or null actor; the slot cleared only while it still holds the channel;
  unproven target fails open with its line once a second; a remote controller queues the channel before, clears the
  slot, logs once per pair and answers 0; the 65th pair falls back to the budget.
- **RG-5 decisions:** a quarantined channel is refused with its caller; wrong by connection and by identity refuse and
  queue; a controller on its own channel, a non-controller and an unproven channel connection pass.
- **RG-6 decisions:** quarantined answers 0 silently; remote answers 0 and queues; by identity with an unproven target.
- **RG-7 decisions:** client calls (`is_server == 0`) and null channels pass; quarantined and actor-less (0 or
  unreadable) channels drop; the log schedule (lines 1–64, then 256, 512, ...).
- **Drive checks:** no driver, unreadable driver, null or foreign graph (`+0x30` mismatch), unreadable slot, slot
  outside the image or not a `.pdata` entry (one line per distinct value), re-entry.
- **Class memo:** filled on the first lookup, used on the second, emptied on a world change, bypassed when full.
- **Lines:** every text on this page for fixed values.
- **Registry:** if `LANDED` contains `"R12"` it contains `"R11"`; with both, `owns` is true for RG-1, RG-4..7 and
  `SP-3@R12`, false for RG-2 and RG-3; RG-3's slice is R14.
- **Offsets:** RG-4..7 each have one `RvaAndBytes` line with the bytes of 17.1; RG-1 joins NETD-8 as a registered
  detour id with no `TARGETS` line.

DLL (`cargo test -p rustic-isles-runtime`, Windows):

- RG-6's detour on a quarantined test pointer returns exactly 0 in a full `i64` (call it directly).
- RG-4's detour with a non-controller actor (a test-allocated object block whose class chain does not reach the
  controller class) calls a test `next` and returns its value unchanged.
- RG-7's detour with `is_server = 1` and a channel block whose `+0x70` is 0 does not call `next`.
- The drive's call check refuses a pointer into a test allocation.
- The drive's call into the graph and every hook against the game are not exercised.

### 16.2 What CI cannot show

Every hook against the real executable, whether the graph delivers the game state, the controller swap and the amp
choice in a two-player escalation, and the cost of the guards on a full hub: the play-test.

---

## 17. Offset-table lines (R12)

### 17.1 `TARGETS` (`FoundBy::RvaAndBytes`: `.pdata` entries with these first bytes, checked 2026-11-13)

| Id | RVA | Bytes | What |
| --- | --- | --- | --- |
| RG-4 | `0x00F2_C230` | `40 55 53 56 57 41 55 41 56 48 8D 6C 24 E9 48 81 EC D8 00 00 00` | the graph's `ReplicateSingleActor` |
| RG-5 | `0x03D8_FB20` | `48 89 5C 24 18 55 56 57 41 54 41 55 41 56 41 57` | `UActorChannel::SetChannelActor` |
| RG-6 | `0x03D8_A2D0` | `4C 8B DC 55 57 49 8D AB E8 FD FF FF 48 81 EC 08 03 00 00` | `UActorChannel::ReplicateActor` |
| RG-7 | `0x03FA_09B0` | `4C 8B DC 48 83 EC 78 48 8B 84 24 E0 00 00 00` | `UNetDriver::ProcessRemoteFunctionForChannel` |

### 17.2 `pub mod repgraph`

| Name | Value | Use |
| --- | --- | --- |
| `ARCHON_SERVER_REPLICATE_ACTORS` | `0x01C0_34C0`, bytes `48 8B C4 56 57 48 83 EC 68` (RG-1's target; not hooked) | 11.2, 12.1 |
| `GRAPH_SERVER_REPLICATE_ACTORS` | `0x00F2_EB60` (the base; not hooked, not called) | 11.3 |
| `ARCHON_GRAPH_VTABLE` | `0x0549_7AF0` (for the record and the play-test's reading) | 11.2 |
| `ROUTE_REMOVE` | `0x01BF_E4C0`, bytes `48 89 5C 24 08 57 48 83 EC 20` (RG-3, for R14) | 12.2 |
| layout | `NETDRIVER_REPLICATION_DRIVER 0x6E8`, `NETDRIVER_SPAN_FOR_GRAPH 0x6F0`, `GRAPH_SPAN 0x4C8`, `GRAPH_NET_DRIVER 0x30`, `GRAPH_CONNECTIONS 0x38` (data; `Num` `0x40`, `Max` `0x44`), `GRAPH_DEFERRED_NUM 0x4C0`, `GRAPH_VTABLE_SERVER_REPLICATE_ACTORS 0x2F8`, `CONN_MANAGER_CONNECTION 0x28`, `CONN_MANAGER_SPAN 0x30`, `CHANNEL_CONNECTION 0x28`, `ACTOR_CHANNEL_SPAN 0x78`, `PLAYER_CONTROLLER_NET_CONNECTION 0x418`, `PLAYER_CONTROLLER_SPAN 0x420`, `CONNECTION_PLAYER_CONTROLLER 0x30`, `CONNECTION_OWNING_ACTOR 0x98`, `CONNECTION_SPAN 0xA0`, `ACTOR_FLAGS_58 0x58`, `STRUCT_SUPER 0x40` | throughout |

`CHANNEL_ACTOR` (`0x70`) is R9's; `GWorld` is R0's. Where R9 or R17 already defines a name for the same fact, use
theirs.

---

## 18. The maintainer's play-test, calls and ambiguities

### 18.1 Play-test (surface section 8, R12 row; collected for the one hands-on pass at the end)

"Two-player hunt: both see each other, nobody's controller swaps; RPC guard quiet." Needs an MIT build with R9, R11
and R12 landed (14.2), both DLLs injected as in R0 11.1.

1. **Hub boot.** Let the Director start Ramsgate. In `rustic_isles_dll_port<port>.log`: `install RG-1 not ported`,
   `install RG-4 ok` through `install RG-7 ok`, `[MitRepGraph] rpc guard on`, and the AGPL's `[MitSeam] ... <n> hooks
   left to it` with `n` 5 larger than with R11 alone (RG-1, RG-4..7; RG-3 stays AGPL).
2. **One client joins.** `[MitRepGraph] drive: graph=<p> class=ArchonReplicationGraph fn=+0x1C034C0 -> <n>
   connections=1 ...` (the first six drives). The client reaches a playable Ramsgate (no 120 s loading timeout): the
   drive delivers its game state and player state.
3. **Two players, a hunt.** Both join one hunt from the airship. Each sees the other move, the HUD of each shows their
   own character (no controller swap), and both finish the hunt. Count `[MitRepGraph] pre-channel: blocked`, `bind:
   refused`, `send: blocked` and `quarantine:` lines: a few `pre-channel: blocked` or `bind: refused` are the guards
   working; any `send: blocked` means the earlier guards missed one (note it). No `[MitRepGraph] rpc: dropped` is
   expected outside an escalation.
4. **Two-player escalation, amp choice.** Pick an amp on both clients through at least two arenas. The geyser to the
   next arena spawns each time and both relic screens accept clicks. `rpc: dropped ... reason=channel-without-actor`
   lines here are the 0039 guard working ("quiet" means: no fault, no stuck choice).
5. **Leave and rejoin.** One player quits mid-hunt and rejoins. No `quarantine: dropped stale` storm (a handful is
   fine); the rejoined player sees the other and is not given their controller.
6. **Off switch.** With `DISABLE_RPC_UNBOUND_CHANNEL_GUARD.flag` beside the server executable: `install RG-7 flag
   DISABLE_RPC_UNBOUND_CHANNEL_GUARD` and `[MitRepGraph] rpc guard off (...)`. Do not play an escalation with it.
7. **Cost.** On a hub with both players idle for 5 minutes, compare the server's frame time against the AGPL-only build
   (the AGPL's performance log is still there): no clear rise (5.2's hot path).

Only CI passed until these steps are done.

### 18.2 Calls, ambiguities and departures

Calls (for the maintainer to confirm; the ones marked **ruling** need one):

1. **Ruling: RG-3 moves to R14** (12.2): its only live behaviour purges R14's caches; the AGPL keeps it until R14,
   with no patch. Recommended: yes. Otherwise patch 0056 (a purge-only AGPL wrapper while `SP-3@R14` is not owned).
2. **Ruling: three switches dropped** (13): `DISABLE_PC_CHANNEL_GUARD`, `DISABLE_GRAPH_DRIVE`, and `DIAG_NATURAL` for
   every MIT slice. Recommended: yes. (`REPGRAPH_DIAG` and `SRA_RATE_CAP` go under RT-3 and as dead.)
3. **No fault catch around the drive** (11.4), following R11's ruling on the manual tick.
4. RG-1 owned, not installed (12.1).
5. **R12 lands with or after R11** (14.2), enforced by a registry test.
6. The drive at SP-3 after `order = 500` (11.1); the progression resume moving before the drive while R7 is AGPL
   changes nothing.
7. The graph found through the driver's own pointer, not R11's `graph_class()` (3).
8. The class memo on the hot path (5.2), emptied on a world change.
9. A guard on a non-game thread uses no table (4.4).
10. The slot check replaces the fault catch for "slot is not a function" (11.2 step 6).
11. Patch 0056 unused (14.1).

Departures:

1. All four guard targets checked against their first bytes (4.2).
2. New tag `[MitRepGraph]` and new texts; budgets on every repeating line (4.3).
3. No emergency channel close when the quarantine is full (6).
4. RG-6's frequency tally and performance counters not ported (9).
5. No fault catch around the drive (11.4).

Ambiguities:

- `graph+0x4C0` is the count of the Archon graph's deferred owner-only list (the override reads it first thing); the
  AGPL's diagnostics also called it "actors without connection". Only logged, so the name does not matter.
- `SetChannelActor`'s third argument is UE 4.26's `ESetChannelActorFlags`, a 32-bit enum; passed through as the full
  register, which is what the AGPL does.
- The drive's slot for `BasicReplicationGraph` (R11's fallback) was not checked; 11.2 step 6 refuses it only if it is
  not a function entry, and the play-test only ever sees `ArchonReplicationGraph`.
- The engine's `ReplicateActor` with a refused (actor-less) channel, when the quarantine is full and RG-6 cannot
  prove the identity, was not measured; the AGPL has the same path.

No open question remains beyond the two rulings (calls 1, 2).

---

## 19. Size

Rust, without tests: the DLL crate about 550 lines (identity reads and the memo 120, RG-4 130, RG-5 70, RG-6 60, RG-7
50, the handler and drive 90, install 30), the core crate about 450 (decisions 220, quarantine 80, lines and budgets
110, flags and offsets 40). Tests about 550. Smaller than the surface's 1 800: the diagnostics, the frequency tally,
the dead rate cap and the never-reached deferred cleanup are not ported, and RG-3 is R14's.
