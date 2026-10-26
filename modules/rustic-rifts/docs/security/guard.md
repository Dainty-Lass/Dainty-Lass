<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: Guard in observe mode (RX3)

The checklist for `crates/rifts-guard` (DaintyLass plan RF6, DD14, DX2): the Hunt Guard evidence chain intake, host
writes and the native inventory fence, the host policy mode, the Guard traffic limits, the host route census and
Guard's two admin operations routes. Launcher Guard and the Guard manifests are isles' (RF1, decision 1) and have
their own page there.

How to use it: before the 1.14.7 module serves realms from Rust, copy this list into the cutover notes, check each
item in the code (not from memory), link the code or test that shows it, and sign at the end with the date and the
commit reviewed. An item that does not hold is a bug to fix before the cutover, not a box to leave open.

## What it is and where the boundary is

Mystic's anticheat module is private, so Guard runs on the stubs in observe mode: it never refuses a hunt, a
progression or a reward because of evidence, and grants no rewards. What it does enforce is who may write for whom
and how much a host may send. Everything it reads from a host is untrusted: the host's PC runs the game server and
the transport, and the host can change both.

- **In, host runtime:** `POST /mystic/v1/hunts/:sessionId/event-batches` and `/completion`, with a host capability
  that isles (`RequireHostCapability`) has already checked; the body is raw bytes Guard parses itself (72 KiB,
  strict). The host's game server's own writes reach Guard through the hooks (`authorize_host_write`,
  `record_host_write`, `authorize_inventory`) with a checked `RuntimeAuth`.
- **In, launcher:** `GET /mystic/v1/hunts/:sessionId/reward/self` with an account isles authenticated.
- **In, anyone:** `POST /internal/v1/hunt-guard/shadow-comparisons`. The key header is never read.
- **In, admin:** `PATCH /admin/v1/operations/reward-rollout` and `/alerts/:alertId/ack`, behind isles' admin gate.
- **In, every request:** `rate_limit_bucket` hashes the `x-mystic-host-capability` header to give a live capability
  its own request budget.
- **Out:** the store (`GuardStore`), realms' hunt records (`HuntSessions`, read only), isles' operations policy and
  admin audit (`OperationsPolicyPort`), the step-up check (`IslesCore::verify_operations_step_up`) and log lines.

Code: `evidence.rs` (batch and completion readers, the chain), `json.rs` (the P2P body parser, `JSON.stringify`
text), `routes.rs` (the six routes, the budget and traffic order of `Handle()`), `writes.rs` (who may write),
`traffic.rs` (the windows and the budget), `lib.rs` (the hooks), `operations.rs`, `census.rs`, `config.rs`.

Fuzz target: `guard_evidence` in `fuzz/` (the P2P parser, the batch and completion readers and the chain update).
It checks that an accepted batch is bounded, gap-free, monotonic, hashed deterministically, and moves the chain to
exactly its own sequence and hash, and that every refusal is a `GUARD_*` code.

## Checklist

### Observe mode

- [ ] No path from evidence refuses a hunt, a progression or a reward: findings only flag the chain
      (`OBSERVE_FLAGGED`), `HuntVerdict::OBSERVED` applies nothing, `reward_receipt` is always `None` and
      `GET .../reward/self` answers `HUNT_REWARDS_UNAVAILABLE` to members. Tests:
      `what_observe_would_refuse_becomes_a_finding`, `the_evidence_chain_through_the_routes`,
      `the_routes_answer_as_node_without_the_module`.
- [ ] With `MYSTICPARADOX_HUNT_EVIDENCE` off (the default at trust tier 0) both evidence routes answer
      `HUNT_EVIDENCE_NOT_COLLECTED` (404) as Node does without the module, after the budget and traffic charge, and
      nothing is stored. Test: `the_routes_answer_as_node_without_the_module`.
- [ ] `rewards.policyStatus` is never ready (`ANTICHEAT_MODULE_ABSENT`), so a Tier 1 hunt routes to a dedicated
      server; `tier1_refusal` keeps the four Tier 1 gates. Tests:
      `without_evidence_hunts_collect_none_and_earn_no_rewards`, `tier1_requires_separate_gates`.

### The evidence chain

- [ ] A body over 72 KiB is `GUARD_BATCH_TOO_LARGE` (413) before it is parsed; anything but a JSON object or array
      is `GUARD_BATCH_INVALID` (400). Tests: `the_p2p_parser_is_strict_and_bounded`,
      `p2p_bodies_are_bounded_and_strict`.
- [ ] A batch with an unknown field, a schema other than 1, more than 128 events, more than 64 KiB of events, a
      payload over 8 KiB or not an object, a gap or a repeat in its sequences, a clock going back, a bad id or type,
      or a terminal event before the last is refused whole. Test: `malformed_batches_are_refused`.
- [ ] A batch is taken only when its `previousHash` is the chain's head and its first sequence follows the last
      one; anything else is `GUARD_CHAIN_CONFLICT` and leaves a finding. Test: `batches_chain_by_hash_and_sequence`.
- [ ] The batch hash covers the whole canonical batch, `previousHash` included, so a host cannot change evidence
      already chained without changing every later hash. A resent batch with the same id and hash is answered again
      (the transport retries after a lost answer); the same id with other content is `GUARD_BATCH_REPLAY`.
- [ ] After `COMPLETION_REQUESTED`, `HUNT_ABORTED` or an accepted completion the chain takes nothing more
      (`GUARD_CHAIN_FROZEN`). A completion must name the head exactly and a `NO_COMPLETION` hunt has none.
      Tests: `a_closed_chain_takes_nothing_more`, `completion_names_the_head_once`.
- [ ] A capability of another generation or host, or of a hub, gets no evidence in (`P2P_GENERATION_FENCED`,
      `HUNT_EVIDENCE_NOT_COLLECTED`). Test: `evidence_from_another_generation_or_a_hub_is_refused`.
- [ ] Per generation at most 262,144 events and 64 MiB (`GUARD_EVIDENCE_LIMIT`); payloads are not stored, only the
      batch summary.
- [ ] Every chain write names the revision it read (`apply_chain`), so two concurrent batches cannot both extend
      the same head.

### Who may write

- [ ] A host write or loot grant needs the runtime's own session and generation, a running hunt (or a suspect one
      that was running), and a target that is an admitted roster member, the host, or present in the hub. These
      refuse in both modes. Tests: the `writes.rs` tests from `hostWrites.test.ts`, `hubAuthorization.test.ts` and
      `p2pInventoryTransactions.test.ts`.
- [ ] Only the write-kind reasons are relaxed in observe mode, logged as `[HostPolicyObserve] would refuse`; a probe
      is never relaxed and never logged; dedicated parity is refused in enforce. Tests:
      `observe_lets_a_write_kind_through_and_logs_it`, `a_probe_stays_strict_and_quiet`,
      `parity_is_quiet_in_observe_and_refused_in_enforce`, `enforce_refuses_write_kinds`.
- [ ] `P2P_HOST_WRITES=off` refuses every host write. Test: `the_kill_switch_refuses_every_host_write`.
- [ ] The inventory fence reads the session again inside the transaction, so a member who left between the check
      and the commit is refused. Test: `inventory_is_fenced_again_inside_the_transaction`.
- [ ] The realm and hub tags on a host write come from the stored session, never from the request.

### Traffic

- [ ] Event batches, completions and shadow calls with a capability take a slot of the in-flight budget (64
      requests, 1 MiB) and charge the host's and the session's windows (180 and 120 requests, 8 and 4 MiB a minute);
      lifecycle calls do not. Tests: `only_guard_traffic_counts_against_the_windows`,
      `the_local_windows_bound_host_and_session`, `traffic_refusals_reach_the_route`.
- [ ] Distributed windows charge both or neither, and any storage failure is `GUARD_TRAFFIC_UNAVAILABLE` (503), never
      a local fallback. The window id holds the key only as a SHA-256. Tests:
      `the_distributed_windows_refuse_and_fail_closed`, `a_rejected_session_reservation_rolls_the_host_back`,
      `distributed_windows_are_stable_bounded_and_key_hashed`.
- [ ] The local limiter keeps at most 4096 keys and the bucket cache 1024, so neither grows without bound.

### Secrets

- [ ] The host capability is only ever hashed (`rate_limit_bucket`) and never stored, compared in clear or logged.
      Test: `only_a_live_capability_gets_its_own_bucket`.
- [ ] The shadow key arrives as a `Secret` (no `Debug`), and Guard never reads it.
- [ ] Log lines carry codes, account and session ids, sequences and a host write's `summary` (at most 400 UTF-16
      units, no secrets by the hook's contract); never a header or a body.

### Admin operations

- [ ] A rollout change needs the current policy version, a version above the current rollout's and an 8-character
      reason; turning Tier 1 grants on also needs a fresh step-up code, and a refusal is audited. Test:
      `the_reward_rollout_route`.
- [ ] An alert is acknowledged once, with its actor and an audit row. Test: `the_alert_acknowledgement_route`.

## Open points

These do not block RF6 but must be settled, or accepted in writing, before the cutover:

- **Grant classes and the draft token cap.** The seam's `authorize_inventory` carries no body and isles' catalog is
  not reachable, so `hostGrantPolicy.ts` and `hostDraftPremium.ts` are not ported. In observe mode that changes
  nothing a player sees (the refusals were only logged), but the `[HostPolicyObserve] would refuse grant` lines are
  missing, and `MYSTICPARADOX_HOST_POLICY=enforce` does not refuse those grants (a startup warning says so). Needs
  the body and the catalog through the seam (RU4).
- **The ledger annotator and the emergency stop fence.** `InventoryFence::on_grant` returns no annotator, so the
  `p2pMonitor` ledger tag is not written, and `FenceP2PEmergencyStop` is realms' and isles' (RF5, RU4).
- **The census hook.** `GuardHooks::census` sees the request before it is answered; counting needs the method, the
  route pattern, the status and the session kind, so isles calls `Guard::record_census` when the response finishes
  and `flush_census` every minute. A seam change could carry the sample instead.
- **Transactions.** Node acknowledges an alert, writes the policy and appends the audit row in one transaction. Until
  `store` has one, the in-memory store and the ports do them one after another.
- **Body parsing differences.** `serde_json` refuses lone UTF-16 surrogates, numbers beyond `f64` and nesting past
  128 levels, which `JSON.parse` accepts: those bodies get `GUARD_BATCH_INVALID` here. Charsets other than UTF-8 are
  decoded as UTF-8.
- **The operations alert dispatcher** (`operationsAlertDispatcher.ts`, the webhook and its token) is not ported: it
  needs an HTTP client in the workspace.

## Fuzzing

From `fuzz/`, on nightly with `cargo-fuzz` (on Windows the AddressSanitizer runtime from the Visual Studio install,
`VC/Tools/MSVC/<version>/bin/Hostx64/x64`, must be on `PATH`):

```bash
cargo +nightly fuzz run guard_evidence -- -max_total_time=86400
```

## Changes from Node's Guard

- The evidence chain intake is new: Node without the module answers `HUNT_EVIDENCE_NOT_COLLECTED` to every batch.
  Rifts does the same while `MYSTICPARADOX_HUNT_EVIDENCE` is off, and with it on takes the batches the transport
  sends, so `hunts.result` is `{progressionApplied:false,rewardsApplied:false}` after a completion instead of absent.
- `record_host_write` reads the session again rather than carrying the authorized one on the request.
- The census ties sort in byte order, not `localeCompare`; request, audit and alert ids are version 4 UUIDs from
  std's random hasher keys, not `crypto.randomUUID()` (they name records and guard nothing).

Signed: ________ (date, commit)
