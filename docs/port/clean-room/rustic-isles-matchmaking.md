# Clean-Room Provenance Record: `crates/rustic-isles-matchmaking` (RD27)

- **Component**: `crates/rustic-isles-matchmaking` (crate `rustic-isles-matchmaking`, `license = "MIT"`, `Copyright
  (c) 2026 Silent Spud`)
- **Behaviour specification**: `docs/port/spec-rustic-isles-matchmaking.md` (its closing "Coordinator decisions"
  override the rest), with `spec-rustic-isles-core.md` (§5.5, §5.6, §7.7, §7.8, §11, §17),
  `spec-rustic-isles-economy-inventory.md` (`RingInstances`, §11.3-§11.4), `spec-rustic-isles-chat.md` (§5.10) and
  `spec-rustic-isles-1-14-7.md` (§3.3.1, the destination rule the test support mirrors) for the contracts it depends
  on
- **Clean-room worktree**: `R:\source\dauntless\dainty-lass\.claude\worktrees\agent-a855dbc656aded15c` (branch
  `feat/rustic-isles-matchmaking`, reset to `feat/rustic-isles-clean-room` at `fb3484f8`)
- **Implementer**: one Claude Code subagent session, briefed by the RD27 coordinator with the rules below

## 1. Isolation

### 1.1 Rules the implementer worked under

It never read, listed, grepped, globbed or opened anything under `R:\source\dauntless\dainty-restricted` (any
worktree of it, including the session's additional working directory `splashloader-integration-90130d`),
`legacy-revived`, any `rustic-isles` or `rustic-rifts` tree outside dainty-lass, or any other repository. It ran no
`git show` or `git log -p` of another branch (only `git log --oneline` of its own branch after the reset). It searched
nothing on the web and did not look for golden files or captures.

### 1.2 Other sources

None beyond this repository: no crate sources from the cargo registry were opened.

## 2. Files read

- The coordinator's brief; `AGENTS.md`; `docs/port/spec-rustic-isles-matchmaking.md` (whole)
- `docs/port/spec-rustic-isles-core.md`: §5.2 (the hooks list), §5.5-§5.7, §6.1, §7.7, §7.8, §11, §12, §17 (by
  section and grep)
- `docs/port/spec-rustic-isles-economy-inventory.md`: the `RingInstances` row and §11.4 (by grep)
- `docs/port/spec-rustic-isles-chat.md`: the `HubSessions` lines (by grep)
- `docs/port/spec-rustic-isles-1-14-7.md`: §3.3.1
- `crates/rustic-isles/`: `Cargo.toml`, `src/lib.rs`, `shared.rs`, `handlers.rs`, `composer.rs`, `dedicated.rs`,
  `body.rs`, `replay.rs`, `season.rs`, `version.rs`, `pipeline.rs` (whole); `settings.rs` (lines 100-260 and its
  public items); `module.rs` (lines 75-145, 165-420, 440-600); `testing.rs` (lines 1-120, 330-451);
  `ids.rs` (`AccountId`, `UserId`); `routes/table.rs` (the matchmaking rows, by grep); `routes/mod.rs` and
  `pattern.rs` (public items by grep)
- `crates/rifts-seam/src/`: `core.rs`, `lib.rs`, `hooks.rs` (whole); `id.rs` (lines 85-280); `error.rs` (lines
  60-157); `dedicated.rs` (public items, lines 60-160 and 228-300); `http.rs` (`Json`, `Reply`)
- `crates/module-api/src/network.rs`: `NodeIdentity`, `IdKind`, `GlobalId`, `SessionTicket`, `PeerGateway`,
  `InstanceRecord`, `JoinDecision`, `PublishedTicket`, `TicketState`, `HomeError`, `HomeApi`'s method list,
  `StandaloneHome` (by grep and lines 46-160, 880-1000); `chat.rs` (`HEADLINE_EVENT`, `Headline`); `module.rs`
  (`Event`, `EventName`, `ModuleContext::emit`); `actor.rs` (`ActorContext`); `version.rs` (`GameVersion`)
- `crates/rustic-isles-store/`: `Cargo.toml`, `src/lib.rs` (lines 1-240), `test_support.rs`, `repo/mod.rs` (lines
  1-150); `repo/parties.rs` (lines 1-240, 800-880); `repo/auth.rs` (`AccountRecord`, `LauncherAccountRecord`, the
  repository declarations)
- `crates/rustic-isles-auth/src/`: `lib.rs` (`AuthData`), `game.rs` (`game_auth`)
- `crates/rustic-isles-chat/`: `Cargo.toml`, `src/lib.rs`, `hubs.rs`, `headline.rs`, `ids.rs` (whole);
  `tests/common/mod.rs`, `tests/routes.rs` (lines 1-140, for the harness form)
- `crates/rustic-isles-economy/`: `Cargo.toml`, `src/lib.rs`, `test_support.rs`, `inventory/ports.rs` (lines
  325-387); the `slot::<` uses (by grep)
- `crates/service-kit/src/`: `serve.rs` (lines 1-140), `jsjson.rs` (lines 100-200), `body.rs` (`error_page`), the
  public items of `json.rs` and `js.rs`
- `crates/store/src/database.rs` (`Tx::conn`, by grep); the root `Cargo.toml`
- `fuzz/Cargo.toml`, `fuzz/fuzz_targets/rustic_isles_economy_text.rs` (for the form)
- `docs/security/rustic-isles-chat.md`, `docs/port/clean-room/rustic-isles-chat.md` (format)

## 3. Files written

### 3.1 The crate, from the spec

| File | Spec |
| --- | --- |
| `Cargo.toml` | §14.1 (`serde_json` `preserve_order` in the crate's manifest; feature `test-support`) |
| `src/lib.rs` | §14.2's re-exports |
| `src/candidate.rs` | §5.1 (`MatchCandidate`, `CandidateSource`, Node's object) |
| `src/rules.rs` | §3.6 (client address, travel host), §3.3 step 4 and §3.4 (log shapes and keys), §5.4-§5.6, §7 (`make_party_id`), §9 (`qos_plan`) |
| `src/state.rs` | §5.2, §5.3, §5.8, §5.9, the decision on item 12 (`MatchmakingState`, `GamePresence`, `MatchClock`, `HuntActivity`, `RingInstances`) |
| `src/controller.rs` | §6 (`Ctx`: join, poll, party instance, cancel, party hunt, immediate, queue, pop, launch, ring registration) |
| `src/status.rs` | §3.6 (the status answer, Ramsgate verification) |
| `src/party.rs` | §7 (`Parties`, `PartyError`, the retry on `PARTY_REVISION_CONFLICT`) |
| `src/routes.rs` | §3, §4 (`register` and the 18 handlers) |
| `src/outbox.rs` | §8 (`outbox_pass`, `run_outbox`, `HeadlineSink`, `ContextSink`) |
| `src/startup.rs` | §8, §9 (`QosListener`, `OutboxDispatcher`, `startups`, `startups_on`, `QOS_PONG`) |
| `src/director.rs` | §10 (`HttpDirector`, `read_answer`) |
| `src/rifts_core.rs` | §11 (`MatchmakingCore`) |
| `src/hooks.rs` | §12, §7's offline hook |
| `src/test_support.rs` | §14.2's feature `test-support` |
| `tests/controller.rs`, `tests/routes.rs`, `tests/services.rs`, `tests/common/mod.rs` | §17.2's tables |

### 3.2 Elsewhere

- `Cargo.toml`: `rustic-isles-matchmaking` in `[workspace.dependencies]`; `Cargo.lock`. A seam change in its own
  commit.
- `fuzz/fuzz_targets/rustic_isles_matchmaking_inputs.rs`, `rustic_isles_matchmaking_director_answer.rs`, their
  `[[bin]]`s and the dependency in `fuzz/Cargo.toml`, `fuzz/Cargo.lock`
- `docs/security/rustic-isles-matchmaking.md`

## 4. Verification

`cargo fmt --all --check` and `cargo clippy --workspace --all-targets --locked -- -D warnings` clean (also with
`--all-features` for this crate); `cargo test -p rustic-isles-matchmaking --locked --no-fail-fast`: 49 tests pass (20
unit, 18 controller, 2 route, 9 service). The fuzz targets build with `cargo check` in `fuzz/` and have not run here.
Nothing was played against the 1.14.7 client; restricted's `tests/golden/{scenario,matchmaking-parties}.json`
replay is still owed (spec §1.3).

## 5. Deviations and ambiguities, and how they were resolved

1. **The coordinator's departures** are implemented as decided: the member-left hook after a kick runs only when the
   kick removed the target (`Parties::kick` answers whether the target's party changed, read in the same
   transaction); a launch that fails during a pop clears the queue's launching mark; the join log's mode, args and
   hunt id are cut to 120 characters with control characters escaped (`rules::log_text`); the three defaulted
   `IslesCore` methods forward to the fallback; `MatchmakingState` implements economy's `RingInstances`
   (`ring_instances_slot` builds the `RingInstancesSlot`). This crate therefore depends on `rustic-isles-economy`,
   which §14.1 said it would not.
2. **A spec vector that disagrees with its rule.** §17.2's "Solo hunt cancelled before its release" expects the leave
   to answer `canceled`/`matching`. By §6.4 step 6 a candidate that is ready on the server but not yet released is
   `go-now`, and a solo `ISLAND` hunt with no queue is launched at once (§6.7), so the code answers
   `canceled`/`go-now` and the test asserts that. Left for the coordinator.
3. **The rifts core and the Director slot.** `MatchmakingCore::new` has no `Shared` (rifts is built before it), yet
   `dedicated_servers_configured` follows core's slot. The state keeps a weak `Shared` that the first handler call
   or the QoS start-up binds (`MatchmakingState::bind_shared`); unbound, the answer is false.
4. **The QoS bind port.** A `Startup` sees only `Shared`, which does not carry the config's assigned `qos` range, so
   `startups()` binds 1.14.7's default 61010 and `startups_on(port)` takes the assigned port from the composer. A core
   change (the ports in `ComposeInputs` or `Shared`) would remove the second function.
5. **The ring calls' home** is `Shared::home()`; under `test-support` `MatchmakingState::set_home` puts a recording one
   in its place (the ring test).
6. **Party operations without a store**: the routes answer 501 before the controller runs; inside the controller's
   leave, a missing store is "no party" rather than an error, so a bare module (the controller tests) cancels alone.
7. **`HuntActivity`** is a settable field of the state (`set_hunt_activity`), defaulting to `NoHuntActivity`.
8. **Ring instances are keyed by text**: a player's userId or global id text as the Director invocation and §6.11 name
   them; `RingInstances::ring_instance_for_player(&GlobalId)` looks the id's text up.
9. **Huge integers in a candidate** (`Port`, `ReadyAt`) are rounded as `JSON.parse` would and read 0 at or beyond
   2^63, so a candidate always writes back and reads the same (the fuzz invariant).
10. **The outbox's recipient lookup failing** counts as a failure building the headline (released, the pass stops),
    not as a failed poll.
11. **A short chunk** in the Director's answer ends the body without the partial chunk.
12. **`record_ring_instance`** is a plain function of `(shared, instance, players)`; core's hook also passes the host
    and port, which the composer drops.
13. **An empty userId** after game auth is treated as missing (the 500 page), as Node's `!userId` would.
14. **Dependencies**: `uuid` and `thiserror` from §14.1's list are not needed (the replay source makes UUIDs; the
    outbox owner is `replay::random_uuid`); `base64` is an existing workspace dependency.

## 6. Similarity

To be measured by the coordinator against restricted `e0e238b`.
