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
15. **Rewritten after the similarity review** (precautionary, coordinator follow-up): `rules.rs`'s `qos_plan` port
    parse was redone from spec §9 alone without looking at the earlier text: the setting or the version's default,
    trimmed, then a byte-by-byte digit accumulation that gives up on a non-digit or a sixth digit, and a `u16`
    conversion with 0 refused (the off/`0` early return and the two `QosPlan` literals are §9's). The Director test's
    request id is now `trace-0042-alpha`. After merging `feat/rustic-isles-clean-room`, the solo-cancel test also
    checks the spec's amended §17.2: the cancelled candidate keeps the search's id, and a re-join makes a second
    Director launch.

## 6. Similarity

Measured by the RD27 reviewer (who may read the AGPL code) with `similarity.md` §1's method, on the crate as committed
at `5d43bb3e` (unchanged at `1fe2e697`): every `.rs` file in `src/` and `tests/` against each of the 17 files of
restricted `e0e238b`'s `modules/rustic-isles/crates/isles-core/` that hold matchmaking (`src/areas/matchmaking.rs` and
the 12 files of `src/areas/matchmaking/` other than economy's `multipliers.rs`; and the tests that drive them,
`tests/{matchmaking_controller,matchmaking_routes,rifts_core,deploy_http}.rs`), the best single file named. "Any"
counts a line matched by a 3+-line run in any of the 17. Paths in the third column are under `isles-core/`; a file
with no match names none.

| File | New lines | Best restricted file (old lines) | Matched | % | Any, % |
| --- | ---: | --- | ---: | ---: | ---: |
| `src/candidate.rs` | 181 | `src/areas/matchmaking/state.rs` (291) | 45 | **24.9** | 29.3 |
| `src/controller.rs` | 735 | `src/areas/matchmaking/controller.rs` (857) | 58 | **7.9** | 8.3 |
| `src/director.rs` | 221 | `src/areas/matchmaking/deploy.rs` (255) | 52 | **23.5** | 24.4 |
| `src/hooks.rs` | 54 | `src/areas/matchmaking/deploy.rs` (255) | 3 | **5.6** | 5.6 |
| `src/lib.rs` | 33 | none | 0 | **0.0** | 0.0 |
| `src/outbox.rs` | 131 | `src/areas/matchmaking/outbox.rs` (134) | 22 | **16.8** | 19.1 |
| `src/party.rs` | 238 | `src/areas/matchmaking/party.rs` (314) | 64 | **26.9** | 26.9 |
| `src/rifts_core.rs` | 152 | `src/areas/matchmaking/rifts_core.rs` (123) | 40 | **26.3** | 26.3 |
| `src/routes.rs` | 650 | `src/areas/matchmaking.rs` (812) | 61 | **9.4** | 10.0 |
| `src/rules.rs` | 512 | `src/areas/matchmaking/qos.rs` (182) | 29 | **5.7** | 15.4 |
| `src/startup.rs` | 113 | `src/areas/matchmaking/qos.rs` (182) | 9 | **8.0** | 15.9 |
| `src/state.rs` | 277 | `src/areas/matchmaking/state.rs` (291) | 17 | **6.1** | 7.2 |
| `src/status.rs` | 132 | `src/areas/matchmaking/status.rs` (171) | 18 | **13.6** | 13.6 |
| `src/test_support.rs` | 193 | `src/areas/matchmaking/testkit.rs` (294) | 15 | **7.8** | 10.9 |
| `tests/common/mod.rs` | 148 | `tests/matchmaking_controller.rs` (805) | 3 | 2.0 | 2.0 |
| `tests/controller.rs` | 479 | `tests/matchmaking_controller.rs` (805) | 38 | 7.9 | 7.9 |
| `tests/routes.rs` | 339 | `tests/matchmaking_controller.rs` (805) | 3 | 0.9 | 0.9 |
| `tests/services.rs` | 348 | `tests/matchmaking_controller.rs` (805) | 9 | 2.6 | 6.9 |
| The 14 `src` files | 3622 | | 433 | **12.0** | 14.4 |
| The 4 test files | 1314 | | 53 | **4.0** | 5.2 |
| All 18 | 4936 | | 486 | **9.8** | 11.9 |

Two files reach 25 % against their best single file (`party.rs` 26.9, `rifts_core.rs` 26.3); `candidate.rs` also does
on the "any" column (29.3), and `director.rs` comes close (23.5). The fuzz targets
(`fuzz/fuzz_targets/rustic_isles_matchmaking_{inputs,director_answer}.rs`) have no restricted counterpart and were not
measured.

**What the matches are.** Every run of 5 lines or more against any of the 17 files was read: 50 of them over 40
distinct spans of the crate, 12 of which (over 4 spans, at `candidate.rs:151-158` and `director.rs:211-218`) are the
`} } #[cfg(test)] mod tests { use super::*; #[test]` boilerplate. The rest:

- **The candidate record** (`candidate.rs`, 45): §5.1's eleven fields in its order (`25-34`), the matching
  constructor's literal (§5.1's "Matching" bullet, `43-56`), the period's `1000`/`10_000` (`77-81`) and the reader's
  `text("HuntId")?`-style lines over §5.1's key list (`135-144`).
- **Parties** (`party.rs`, 64): the constants §14.2 names (`INVITE_TTL_MS`, `MAX_PARTY_SIZE`, `PLATFORM`), `Parties`'
  fields, and the operations of §7's table as calls of the store spec's §11.2 repository (`create_invite`,
  `accept_invite` with `AcceptInvite`'s fields, `promote`) inside the `write` retry; `invite`'s guard (`145-147`) is
  the table's "empty recipient or the sender itself: nothing". The matched lines are signatures, struct literals and
  `.await` tails; the retry, the account lookups and `username`'s fallback chain do not match.
- **The rifts core** (`rifts_core.rs`, 40): `rifts_seam::IslesCore` method heads (an MIT trait), `isles_party_id`
  (§11's "`party:` + the text parsed as a `GlobalId`", a one-liner with no other form), `cancel_travel` (§11's row over
  §5.3), the `INVALID_CANDIDATE` refusal text (§11) and the fallback forwards (§11's table, decision item 11).
- **The Director client** (`director.rs`, 52): §10's request head (`121-127`: method, path, `Host`, `Connection:
  close`, the headers), its answer rules (`38-50`: the blank line, the status as the second field, the `fetch
  failed: ...` texts; `68-72`: §10's de-chunking ending at a 0 or short chunk), and §10's log and warn texts with the
  `Refused { reason: None }` of a non-200.
- **Spec-given log texts and records**: the outbox's `[PartyOutbox]` lines (§8, `outbox.rs:122-141`), the join's
  `Matchmaking is disabled, refusing MM!`/`Unsupported MATCHMAKING_MODE!` arms (§6.1, `controller.rs:139-147`), the
  `NO_DEDICATED_SERVERS` refusal (§6.10), the join and party-mismatch warnings (§3.3, `routes.rs:374-432`), the
  `MatchmakingMap` 404 and the stale-Ramsgate warning (§3.6, `status.rs:58-63`, `110-113`), the QoS start-up texts
  (§9, `startup.rs:54-59`), `HuntActivity`/`NoHuntActivity` (§14.2 gives both methods with their arguments,
  `state.rs:23-35`), §6.4 step 6's state chain (`controller.rs:367-381`), the leave's key sanitising (§3.4 step 1,
  `rules.rs:308-330`), and the `ConnectInfo` peer read and the `rifts_seam` error variants destructured in
  `routes.rs:103-106`, `303-322`.
- **Test harness and vectors**: the stand-in auth layer (`test_support.rs:224-239`: header to `AuthData`, the stock
  `map_request` shape the crate's chat tests already use), the fake Director's `DedicatedServers` heads, §17.2's
  launch-body vector (`tests/services.rs:211-215`), and `.await.body.unwrap()` tails in `tests/controller.rs`.

No comment was copied: three comment lines equal restricted ones (`controller.rs:33`, `315`, `464`), all three the
spec's own words (§5.7's online window, §6.5's Node name, §6.6 step 1's sentence). Of the 351 string literals of 12
characters or more that the crate shares with the 17 files, all but a handful are spec texts; the rest are the JSON
pointer `/serverInfo/gameSessionId` over §3.6's field, `x-mystic-party-revision` (§3.7's header in lower case),
tokio's `multi_thread`, and one that no spec gives: the test request id `req-12345678` (`tests/services.rs:216`,
`230`). 111 of the crate's 276 function names and 28 of its 81 type and constant names also occur in restricted. Those
the spec does not spell out are its terms in snake or camel case (`pop_queue` for `PopQueue`, `player_candidate` for
`GetPlayerCandidate`, `touch_realm_presence` for §4.2, `record_hub_session` for §3.5's "record a hub session",
`seam_party` for §11's "the seam party", `rifts_party_id`/`isles_party_id` for §11's two directions, `StatusAnswer`
for §3.6, `JoinRequest`/`LeaveOutcome`, `QUEUE_SIZE`/`QUEUE_WAIT_MS`/`PARTY_INSTANCE_REUSE_MS` for §5.7's rows,
`is_hub_destination`, `js_number`, `set_ramsgate` for §14.2's "settable Ramsgate hub") or plain names other MIT crates
already use (`dechunk`, `require_account`, `empty_object`, `string_field`, `user_of`). Their signatures differ from
restricted's, and none brings a matching body beyond the lines above. The large files (`controller.rs`, `routes.rs`,
`rules.rs`, `state.rs`, the black-box tests) match at 1 to 10 %: the join, poll, party hunt, immediate instance, queue,
pop and launch, the route handlers, the state tables, the presence and the outbox loop share no logic beyond them.

**The two spans that go beyond the spec.**

1. **`src/rules.rs:257-278` (`qos_plan`'s tail).** §9 gives the rule and the problem text, so the early return for
   `off`/`0` and the two `QosPlan` literals are forced. But the port is also read with the same chain (`bytes()`,
   `all(is_ascii_digit)`, `then` a parse, `flatten`, a filter above 0) followed by the same two-arm `match`, in the
   same order. Each piece is stock; together they are the closest the crate comes to an expressive match.
2. **`tests/services.rs:216`, `230` (`req-12345678`).** A placeholder request id identical to one in restricted's
   tests that no spec gives. Not a wire value, but the only shared literal outside the specs.

**Verdict: clean of structural or expressive copying, with two small rewrites asked for as a precaution.** Everything
else is spec-given wire and log texts, the candidate and party records in the spec's order, MIT seam signatures
(`rifts_seam`, `module_api`, the store's repositories), pinned test vectors and boilerplate. The rewrites, from §9 and
§17.2 only and without looking at the current text: `qos_plan`'s port parse in the implementer's own form (for
instance a `u16` parse checked against 0 under an `if let`, or one `match` over the trimmed value), and a different
request id in the Director test. `party.rs`, `rifts_core.rs`, `candidate.rs` and `director.rs` need nothing: their
matched lines are §5.1's, §7's, §10's and §11's contracts and the seam's signatures in the only idiomatic form.
