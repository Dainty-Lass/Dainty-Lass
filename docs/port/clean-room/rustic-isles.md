# Clean-Room Provenance Record: `crates/rustic-isles` (RD27)

- **Component**: `crates/rustic-isles` (crate `rustic-isles`, `license = "MIT"`, `Copyright (c) 2026 Silent Spud`)
- **Behaviour specification**: `docs/port/spec-rustic-isles-core.md` (§17's coordinator decisions override the rest),
  with `docs/port/spec-rustic-isles-auth.md` for the hosted crate's contract
- **Clean-room worktree**: `R:\source\dauntless\dainty-lass\.claude\worktrees\hopeful-volhard-1b89b9` (branch
  `feat/isles-auth-clean-room`), shared with other agents writing `docs/port/spec-*.md`
- **Implementer**: one Claude Code subagent session, briefed by the RD27 coordinator with the rules below

## 1. Isolation

### 1.1 Rules the implementer worked under

It never read, listed, grepped, globbed or opened anything under `R:\source\dauntless\dainty-restricted` (any
worktree of it, including the session's additional working directory `splashloader-integration-90130d`),
`legacy-revived`, any `rustic-isles` or `rustic-rifts` tree, or any other repository. It ran no `git show` or `git log
-p` of another branch (one `git show --stat` of this branch's own seam commit `d1a8fe8`, for its message form). It
searched nothing on the web and did not look for the golden files §13.1 keeps in dainty-restricted. It did not read
the other `spec-rustic-isles-*.md` files that appeared in the tree during the session.

### 1.2 Other sources

None beyond this repository: no crate sources from the cargo registry were opened.

## 2. Files read

- `docs/port/spec-rustic-isles-core.md` (whole); from `docs/port/spec-rustic-isles-auth.md` the heading list, §4
  (lines 487-628) and §11 (lines 1092-1160); `AGENTS.md`
- `docs/port/clean-room/rustic-isles-auth.md` and `docs/security/rustic-isles-auth.md` (format)
- `Cargo.toml`, `crates/rustic-isles-auth/Cargo.toml`
- `crates/rustic-isles-auth/src/`: `lib.rs`, `routes.rs`, `limiter.rs`, `client.rs` (whole); `runtime.rs` (first 120
  lines), `errors.rs` (first 80), `launcher.rs` (lines 76-131 and the signatures of `launcher_cors`,
  `launcher_auth`, `launcher_auth_bearer`, `require_active_account` by grep), `seams.rs` (`Seams` and its `Default`),
  `store.rs` (`StoreError`, the `AuthStore` head and the `Arc<T>` impl by grep), `game.rs` and `jobs.rs` (the
  `game_auth` and `start` signatures by grep), `util.rs` (`random_bytes`, `random_uuid`, `read_body`, `body_kind`)
- `crates/rustic-isles-auth/tests/game.rs` (first 80 lines)
- `crates/rifts-seam/`: `Cargo.toml`, `src/lib.rs`, `http.rs`, `error.rs`, `dedicated.rs` (whole), `hooks.rs` (the
  `Settings`, `Lifecycle`, `RealmsHooks`, `RealmsEntry`, `GuardHooks` and `InventoryDecision` declarations), `core.rs`
  (public items by grep)
- `crates/module-api/`: `Cargo.toml`, `src/lib.rs`, `module.rs` (whole), `registrar.rs` (types and checks),
  `manifest.rs` (`ModuleManifest`, `PortRange`, `PortDefault`, `Capabilities`), `network.rs` (`NodeId`,
  `NodeIdentity`, `DirectorInvocation`, `SessionTicket`, `HomeError`, `HomeForward`, `HomeApi`, `dispatch_home`,
  `HomeInvocation`, `StandaloneHome`), `chat.rs` (lines 100-190), `actor.rs` (`Actor`, `ActorContext`), `driver.rs`,
  `stub.rs` (lines 240-345), `ids.rs` (the name-type macro), `version.rs` (`API_VERSION`, `GameVersion` by grep)
- `crates/service-kit/`: `Cargo.toml`, `src/body.rs`, `serve.rs`, `json.rs` (whole), `env.rs` (first 60 lines), `js.rs`
  (lines 130-200), `jsjson.rs` (lines 150-210), public items by grep
- `fuzz/Cargo.toml`, `fuzz/fuzz_targets/rustic_isles_auth_text.rs` (first 60 lines)

## 3. Files written

### 3.1 The crate, from the spec

| File | Spec |
| --- | --- |
| `src/lib.rs` | §11.2's re-exports |
| `src/routes/mod.rs`, `src/routes/table.rs` | §4.2, Appendix A (the 290 rows, generated from the appendix's table) |
| `src/pattern.rs` | §4.1 |
| `src/pipeline.rs` | §3.1-§3.8, §3.6's 501 |
| `src/body.rs` | §3.9 |
| `src/handlers.rs` | §5.7, §10 (the auth routes as handlers) |
| `src/settings.rs` | §6.1-§6.3, §17 (`RUSTIC_ISLES_REPLAY_HEADERS`, names-only `Debug`) |
| `src/version.rs` | §5.2, §5.3 |
| `src/ids.rs` | §2.2 |
| `src/shared.rs` | §5.1, §5.4, §5.5, §10 |
| `src/dedicated.rs` | §5.6 |
| `src/observe.rs` | §3.1 step 4 |
| `src/game_data.rs` | §8.1-§8.3 |
| `src/season.rs` | §8.4 |
| `src/replay.rs` | §9.1 |
| `src/composer.rs` | §11.4 |
| `src/module.rs` | §6.4, §7 |
| `src/testing.rs` | §13.3 |
| `tests/pipeline.rs`, `tests/module.rs`, `tests/replay.rs` | §13.2's table |

### 3.2 Elsewhere

- `Cargo.toml`: `rustic-isles` in `[workspace.dependencies]`; `Cargo.lock`. A seam change in a commit of its own. No
  new external dependency (`uuid` is not in the workspace; ids are formatted from `rand` and `sha2`).
- `tracing-subscriber` (already a workspace dependency) as a dev-dependency of the crate, to capture the CORS line
  in that test
- `fuzz/fuzz_targets/rustic_isles_{routes,pipeline_text}.rs`, their `[[bin]]` entries and the `rustic-isles` and
  `service-kit` dependencies in `fuzz/Cargo.toml`, `fuzz/Cargo.lock`
- `docs/security/rustic-isles.md`, this record

## 4. Verification

- `cargo test -p rustic-isles -p rustic-isles-auth --locked --no-fail-fast`: see the commit report
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean
- `cargo check` of the two new bins in `fuzz/`: clean (not fuzzed; the targets need nightly)
- Not run against the real 1.14.7 client or restricted's golden replays; those run in dainty-restricted once its
  version crate composes this crate (§13.1)

## 5. Deviations and ambiguities, and how they were resolved

1. **The session payload.** `AuthConfig::session_payload` is a plain `fn(&str) -> Value`, which a trait object's
   method cannot become, so §5.2's `game_session_payload` hook is a field of `VersionTables` (`session_payload`)
   instead of a `VersionHooks` method.
2. **Table shapes.** §5.2 names the tables but not their types; `BountyItemGrant`, `SeasonalCoin`,
   `MaskedJourneyNode` and `StarterManifest` are `&'static` records with the fields §5.2 lists.
   `ObjectiveRow`'s numbers and `Entitlement`'s duration are `f64` (JS numbers).
3. **`IslesModule::new` and `Shared::new`/`bare` return `Result`**: the game version text must parse (§7.1), and
   nothing may panic.
4. **`isles.listening`'s payload** is the JSON object's text as a string (§7.3 "as text, in order"), so the order
   survives any map; `testing::listening` also takes an object.
5. **Admin auth for the `/admin/` rifts rows** (auth §4.4 "the Reforged API's admin auth") had no seam in §11.4.
   Coordinator addition: a slot trait `AdminAuthenticator` (headers and peer in, `AdminCaller { key_id }` or a
   refusal `Response` out), in `SharedParts`/`Composition` and settable later (`Shared::set_admin_authenticator`).
   With the slot empty the row is refused 401 `AUTH_UNAUTHORIZED` / `Sign in required.` (`admin_refusal`).
6. **`Ban` validation first**: the `until` checks run before the "no database" answer, so §13.2's two `Invalid` cases
   hold on a bare module (§15 item 15).
7. **"No database"** (§7.6, §7.8) is §5.1's "database wanted"; the auth's `start` runs when a database is wanted or the
   composer gave a store.
8. **The host capability header to the guard.** The request summary carries the header under the name the request
   used, `x-mystic-host-capability` (§3.4); rifts-seam's own constant is `x-rustic-host-capability`. Restricted's
   guard adapter must read the former, or the coordinator renames one side.
9. **Count, then CORS** (§3.2 steps 3 and 5, §3.5, auth §4.5). The first cut asked launcher CORS before the count
   (a request ref may not live across the count's await), so a request the budget refused could still log
   `[CORS] Rejected launcher origin`. Coordinator review restored the order: the count's input is taken from the
   request first, then the count, then CORS; `the_budget_counts_before_launcher_cors` pins it by status and by the
   log line.
10. **`HEAD`** answers drop the body in the dispatcher as well, so a test through `oneshot` sees what hyper sends.
11. **Patterns**: a `:name` is letters, digits and `_`; a literal holding `:` is refused like the other pattern
    characters. The gauntlet check comes after the bad-param 400, in §3.2 step 4's order.
12. **Replay time**: an integer is `-?digits`; anything else goes to `service_kit::js::date_parse`. A seeded id's
    counter is per scope; `ReplayScope::enter` carries the scope into blocking closures.
13. **Rifts relay**: a body read error other than over-limit is an empty 400; a launcher refusal is the launcher
    envelope with a fresh UUID (auth §2.3, §15 item 6); the rifts route is found by path text and method (`ALL`
    matches any).
14. **The game data cache** lives in the `GameDataRoot` (one per module), not the process (§15 item 14).
15. **Home operations**: the hook answers `Ok(None)` for an operation it doesn't have, which the module turns into
    `NotFound` `unknown home operation {name}`; its default is "no database".
16. **Stop order**: listeners are told to stop and awaited (5 s), then the services are signalled (their own 2 s
    drain) and awaited up to 5 s, then the hooks' checkpoint.
17. **Test support**: `drive` takes a `ModuleDriver` and the `RecordingHost` behind it and sets every port, the UDP
    range included, to 0; `Mounts` counts the auth rows as handled; `sample_version()` is a version with 1.14.7's
    numbers and placeholder hook answers, for this crate's tests only (the wire payloads are the version crate's).
18. **Bodies** follow §3.9: "has a body" includes a missing `host` header (unlike auth's own reader), and a parsed body
    is left as `ExpressBody` in the request so the auth crate reuses it.

## 6. Similarity

Left for the RD27 coordinator.
