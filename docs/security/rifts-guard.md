<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: Rustic Rifts' Guard (RD27)

The checklist for `crates/rifts-guard`, Guard in observe mode: the Hunt Guard evidence chain a host's transport
posts, whom a hosted runtime may write player state for, which items a player host may grant, the bounds on Guard
traffic, the host route census and two admin operations routes. Written with the clean-room MIT rewrite from
`docs/port/spec-rd27-rifts-guard.md` (and the umbrella's D3 renames). `SECURITY_REVIEW.md` indexes it.

How to use it: before the rewrite replaces the AGPL crate on a server, copy this list into the cutover commit, check
each item in the code (not from memory), link the code or test that shows it, and sign at the end with the date and
the commit reviewed.

## What it is and where the boundary is

A player host runs the hunt; its runtime and transport talk to the server with a capability token. Guard decides
**what that host may write for other players** and **how much evidence traffic it may cost the server**. It grants
nothing: the evidence chain is only observed, and in observe mode every host policy refusal is only logged.

- **In, from a player host:** the hunt routes' bodies (`/rustic/v1/hunts/:sessionId/event-batches`, `/completion`),
  the `x-rustic-host-capability` header, the inventory transaction bodies and host write requests isles forwards.
- **In, from a launcher:** `GET /rustic/v1/hunts/:sessionId/reward/self`.
- **In, from an admin:** the reward rollout and alert acknowledgement bodies.
- **In, from the server:** the hunts (realms' `HuntSessions`), isles' catalogue, stacked items and policy, settings.
- **Out:** replies, the evidence and host write rows in `store`, log lines.

Fuzz targets in `fuzz`: `guard_evidence` (the body parser, batches, completions and the chain they move),
`guard_grants` (item classes, stored balances, the draft token cap).

## Checklist

### Bodies and evidence

- [ ] Hunt route bodies are parsed before any authentication, at most 72 KiB, objects or arrays only
  (`json::tests::body_parser_*`, `tests::bodies_are_bounded_and_strict_before_authentication`).
- [ ] A batch takes only its six fields, at most 128 events and 64 KiB of events, 8 KiB per payload; payloads are
  never stored (`tests::every_malformation_is_refused`, `evidence::apply_batch`).
- [ ] The batch hash covers the whole body in document order (`serde_json` `preserve_order` in the crate's manifest),
  pinned to a known value (`tests::known_body_hashes_to_the_pinned_value`).
- [ ] A batch id is used once per hunt generation: a resend answers the stored batch, other evidence under the same id
  is refused; a chain write loses to a concurrent one instead of overwriting it (`tests::full_chain_flow_through_the_routes`,
  `sql_store` `exercise`).
- [ ] A chain is capped at 262 144 events and 64 MiB, and closed once completion is requested or the hunt aborted
  (`tests::monotonic_regression_and_limits`, `tests::closed_chains_and_completions`).
- [ ] Evidence is taken only from the hunt's own host at its current generation, never for a hub
  (`tests::fenced_and_hub_hunts`).

### Capabilities

- [ ] The capability header is used only when it is 43 characters of base64url; only its SHA-256 is looked up or
  cached, and neither is logged (`tests::rate_limit_buckets_are_cached_and_never_logged`).
- [ ] An anonymous caller with no valid capability is refused before anything is charged or read
  (`tests::bodies_are_bounded_and_strict_before_authentication`).

### Traffic

- [ ] 64 requests and 1 MiB in flight across all hosts, each released once (`traffic::tests::budget_releases_exactly_once`,
  `tests::evidence_off_collects_nothing_and_returns_the_budget`).
- [ ] 180 requests and 8 MiB a minute per host, 120 and 4 MiB per hunt, answered 429 so the transport backs off
  (`tests::local_windows_per_host_and_session`, `tests::route_level_rate_limit`).
- [ ] The local limiter holds at most 4096 keys; the distributed one stores only hashed keys and fails closed (503)
  when the store fails (`traffic::tests::*`, `tests::distributed_limit_then_unavailable`).

### Host writes and grants

- [ ] A host writes only for admitted members of its running hunt, or for players present in its hub; the realm and
  hub tags of a recorded write come from the stored hunt (`writes::tests::*`,
  `tests::recorded_writes_carry_the_stored_hunts_tags`).
- [ ] `P2P_HOST_WRITES=off` refuses every write (`tests::the_kill_switch`).
- [ ] Every grant of an authorized transaction is checked again against the hunt then, inside the transaction
  (`tests::fence_rechecks_presence`).
- [ ] Premium currency, Hunt Pass, cosmetic and entitlement items are refused in enforce mode and logged in observe
  mode; a no-reward hunt grants nothing (`grants::tests::*`, `tests::grant_classes_observed_or_enforced`).
- [ ] The daily draft token is one whole token with a balance cap of 1, decided again from the transaction's own read
  (`grants::tests::draft_decision_table`, `tests::draft_over_the_cap_early_and_inside_the_transaction`).

### Admin operations

- [ ] Both routes need an admin caller; a reason of at least 8 characters is audited with the caller's key id and
  address (`tests::reward_rollout_replies`, `tests::alert_acknowledgement`).
- [ ] Enabling tier 1 grants needs a fresh step-up code; a refused attempt is audited
  (`tests::reward_rollout_replies`).

### Failures

- [ ] An internal failure is logged with the request id and answered `INTERNAL` with a fixed message, never its
  detail (`tests::internal_errors_are_logged_and_hidden`).
- [ ] A stored row that does not map back (unknown kind or state, negative count, bad JSON) is an error, never
  skipped (`sql_store` `rows_that_do_not_map_back_are_failures`).
- [ ] The shadow comparison key is never read or logged (`EvidenceIntake::record_shadow_comparison`).

Reviewed by: \_\_\_\_\_\_ on \_\_\_\_\_\_ at commit \_\_\_\_\_\_
