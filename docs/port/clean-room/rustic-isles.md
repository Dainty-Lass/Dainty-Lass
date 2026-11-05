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

19. **Rewrites after the similarity review** (§6): `src/ids.rs` was rewritten from §2.2 and §11.2 alone. The impls
    are in §11.2's order. `FromStr` matches on the parsed id's kind. `AccountIdError` became an enum
    (`Unreadable`, `OtherKind`) with this implementer's own wording. `UserId` has a private field with `new` and
    `as_str`. The wording departs from §2.2's quoted refusal text, at the coordinator's direction; the `Invalid`
    kind of §7.8 is unchanged. `PatternError`'s fields are now `row` and `why`, with a reworded message.

## 6. Similarity

Measured by the RD27 reviewer (who may read the AGPL code) with `similarity.md` §1's method, on the crate as committed
at `083d3b95`: every `.rs` file in `src/` and `tests/` against each of the 29 files of restricted `e0e238b`'s
`modules/rustic-isles/crates/isles-core/` that hold the core (every `src/` file outside `areas/` and `repo/`: `lib.rs`,
`module.rs`, `http.rs`, `routes/mod.rs`, `routes/table.rs`, `settings.rs`, `bootstrap.rs`, `hooks.rs`, `ids.rs`,
`replay.rs`, `tables.rs`, `version.rs`, `legacy.rs`, `db.rs`, `testing.rs`, `game_data.rs` and the 9 files of
`game_data/`; and the tests that drive them, `tests/{pipeline,gamedata_generators,store}.rs` and
`tests/{common,support}/mod.rs`), the best single file named. "Any" counts a line matched by a 3+-line run in any of
the 29. Paths in the third column are under `isles-core/`; a file with no match names none.

| File | New lines | Best restricted file (old lines) | Matched | % | Any, % |
| --- | ---: | --- | ---: | ---: | ---: |
| `src/body.rs` | 165 | `src/http.rs` (717) | 10 | **6.1** | 7.9 |
| `src/composer.rs` | 131 | `src/module.rs` (1349) | 6 | **4.6** | 4.6 |
| `src/dedicated.rs` | 81 | `src/hooks.rs` (107) | 8 | **9.9** | 14.8 |
| `src/game_data.rs` | 184 | `src/game_data.rs` (58) | 30 | **16.3** | 31.0 |
| `src/handlers.rs` | 139 | `src/http.rs` (717) | 21 | **15.1** | 18.0 |
| `src/ids.rs` | 71 | `src/ids.rs` (69) | 61 | **85.9** | 85.9 |
| `src/lib.rs` | 43 | none | 0 | **0.0** | 0.0 |
| `src/module.rs` | 666 | `src/module.rs` (1349) | 170 | **25.5** | 26.9 |
| `src/observe.rs` | 19 | none | 0 | **0.0** | 0.0 |
| `src/pattern.rs` | 204 | `src/routes/mod.rs` (324) | 25 | **12.3** | 15.7 |
| `src/pipeline.rs` | 600 | `src/http.rs` (717) | 69 | **11.5** | 12.5 |
| `src/replay.rs` | 172 | `src/game_data/season_window.rs` (244) | 12 | **7.0** | 7.6 |
| `src/routes/mod.rs` | 187 | `src/routes/mod.rs` (324) | 130 | **69.5** | 70.1 |
| `src/routes/table.rs` | 316 | `src/routes/mod.rs` (324) | 9 | **2.8** | 2.8 |
| `src/season.rs` | 78 | `src/game_data/season_window.rs` (244) | 11 | **14.1** | 17.9 |
| `src/settings.rs` | 289 | `src/settings.rs` (243) | 43 | **14.9** | 18.0 |
| `src/shared.rs` | 365 | `src/db.rs` (293) | 63 | **17.3** | 23.3 |
| `src/testing.rs` | 400 | `tests/common/mod.rs` (447) | 33 | **8.3** | 18.0 |
| `src/version.rs` | 177 | `src/hooks.rs` (107) | 72 | **40.7** | 59.9 |
| `tests/module.rs` | 311 | `src/module.rs` (1349) | 11 | 3.5 | 4.5 |
| `tests/pipeline.rs` | 705 | `tests/pipeline.rs` (686) | 37 | 5.2 | 5.8 |
| `tests/replay.rs` | 18 | none | 0 | 0.0 | 0.0 |
| The 19 `src` files | 4287 | | 773 | **18.0** | 22.0 |
| The 3 test files | 1034 | | 48 | **4.6** | 5.3 |
| All 22 | 5321 | | 821 | **15.4** | 18.7 |

Four files reach 25 % against their best single file (`ids.rs` 85.9, `routes/mod.rs` 69.5, `version.rs` 40.7,
`module.rs` 25.5); `game_data.rs` also does on the "any" column (31.0). The fuzz targets
(`fuzz/fuzz_targets/rustic_isles_{routes,pipeline_text}.rs`) have no restricted counterpart and were not measured.

**What the matches are.** Every run of 5 lines or more against any of the 29 files was read: 115 of them over 79
distinct spans of the crate, 26 of which are, or end in, the `} } #[cfg(test)] mod tests { use super::*; #[test]`
boilerplate. The rest:

- **The route table's vocabulary** (`routes/mod.rs`, 130 matched lines): `Service`, `Method`, `Group`, `Side`, `When`
  and `Scope` with their `as_str` matches (`routes/mod.rs:8-41`, `66-127`, runs of 31 and 54), `RouteDef`'s six
  fields (`157-165`), `is_regex` and `name` (`176-183`). §11.2 names every type, variant and field and §4.2 every
  string, in that order; one enum per §4.2 column with a `const fn as_str` is the only idiomatic shape. The row table
  itself (`routes/table.rs`, 2.8 %) is written differently, and `scope()`, `Method::accepts`/`http` and `index` do
  not match.
- **The version's records and hooks** (`version.rs`, 72): `VersionTables` and its parts in §5.2's order
  (`version.rs:19-59`), `build_id` (§5.2's format), `NoPlayerPolicy` with `path_account`/`bearerless_account` over
  §5.3's two settings (`77-111`; the fallback chain is §5.3's sentence), `StubReply`, `Entitlement`, `GameMode` with
  §5.2's three strings, `Destination`, `ProgressionConfigRule` (`122-171`), and the `VersionHooks` method list
  (`176-185`, §5.2's names and arguments).
- **The module's manifest and registration** (`module.rs`, 170): `HOST_ONLY_PREFIXES` (§7.2, `39-45`), `PortService`
  and `BackendService` (§11.2, `52-76`), `DefaultPorts` and `ModuleSpec`'s fields (§7.1, §11.2, `78-93`), and
  `manifest()`/`register()` (`466-547`, runs of 31 and 6): §7.1's ports in order, §7.2's routes, UDP ports, the five
  admin actions and the `game_data` section. The local closure building a TCP `PortDefault` (`468-472`) is the
  obvious factoring of §7.1's three single-port entries. The rest are spec texts (`360-363`, `578`, `627`) and
  `GameModule` method heads, whose signatures `module-api` fixes. `start_inner`, the command and admin paths and
  `stop` match no further than the `Box::pin(async move {` heads and §7.3/§7.4's `started()`/`stopping()` loops.
- **Game data** (`game_data.rs`, 30): §8.1's three file lists (`16-42`, 24 lines) and §8.2's loader vector (BOM,
  `1.0`, `12345678901234567890`).
- **Seam signatures**: `shared.rs`'s `HomeApi` impl (`305-412`, the method heads of `module_api::network::HomeApi`,
  an MIT trait), `Shared::bare`'s first fields (§5.1's bare module), `handlers.rs`'s `Call` and `Handler` (§11.2,
  `19-29`), `settings.rs`'s `SharedSwitch` (§11.2) and `raw` over §6.1's three layers in their order (`175-191`),
  and `PatternError` (`pattern.rs:27-32`, below).
- **Wire text and test vectors**: §3.3's apex hosts and host shape (`pipeline.rs:35-39`, `51-55`) and its refused
  targets (`pipeline.rs:637-639`), §9.1's seed prefix (`replay.rs:41-46`), §8.4's `iso_utc` (`season.rs:48-52`),
  §13.3's sample path for an Express row with the Gauntlet prefix (`testing.rs:55-67`: split, `:name` to `x`, join),
  §3.6's 501 messages in `tests/pipeline.rs`, §6.3's `admin_api` vector (`settings.rs:307-312`), and the
  `sample_version` stubs (`testing.rs:363-398`) filling §5.2's fields.

No comment was copied: one comment line equals a restricted one (`/// The switch that unmasks it`, `version.rs:32`),
§5.2's own words. Of the crate's string literals of 12 characters or more that also occur in the 29 files, all but
one are in the core or auth spec; the one that is not is `PatternError`'s message. 140 of the crate's 296 function
names also occur in restricted; all but `strip_origin` (the plain name for §3.3's step) are the spec's (§11.2, §13.3),
`std` trait methods, or methods of the MIT seam traits the crate implements (`module_api`'s `GameModule` and
`HomeApi` with its `ring_*` calls, `rifts_seam`'s guard and realms hooks). No unusual structure is shared: the large
files (`pipeline.rs`, `shared.rs`, `testing.rs`, `body.rs`, the black-box tests) match at 3 to 17 %, and the
dispatcher, matcher, body reader, replay scope and module lifecycle share no logic beyond the lines above.

**The two spans that go beyond the spec.**

1. **`src/ids.rs` (whole file, 85.9 %).** §2.2 and §11.2 fix the API (`AccountId` over `GlobalId`, `new(node)`,
   `global()`, `home()`, `FromStr`, `TryFrom<GlobalId>`, the error text, `UserId::random()`), and a newtype that
   delegates to `module_api` has little room. But the file also agrees, item for item and in the same order, on what
   the spec leaves open: the derive lists, `AccountIdError` as a unit `thiserror` struct, `TryFrom` before `FromStr`
   with `from_str` as one parse-then-convert chain, `UserId` as a public tuple field, `UserId::as_str`, and both
   `Display` impls. Each is a stock choice; together they make the one file that reads as a copy.
2. **`src/pattern.rs:27-32` (`PatternError`).** A `path`/`reason` struct whose `#[error]` text is identical to a
   restricted string that no spec gives. Not a wire string (it reaches only a `MountError`'s text when the app is
   built), but it is the only shared literal outside the specs.

**Verdict: clean of structural or expressive copying, with two small rewrites asked for as a precaution.** Everything
else is spec-given wire strings, enum and record tables in the spec's order, spec- and `module-api`-fixed signatures,
pinned test vectors and boilerplate. The rewrites, from §2.2 and §4.1 only and without looking at the current text:
`src/ids.rs`'s choices the spec leaves open (for instance a private `UserId` field with a constructor, the impls in
§11.2's order, `FromStr` written out over the parsed id's kind), and `PatternError`'s message and fields in the
implementer's own words. `routes/mod.rs`, `version.rs` and `module.rs` need nothing: their matched lines are §11.2's
and §4.2/§5.2/§7's tables in the only idiomatic form.
