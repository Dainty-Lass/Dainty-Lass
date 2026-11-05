# Clean-Room Provenance Record: `crates/rustic-isles-1-14-7` and `crates/rustic-isles-1-14-7-plugin` (RD27)

- **Components**: `crates/rustic-isles-1-14-7` (lib and bin `rustic-isles-1-14-7`) and
  `crates/rustic-isles-1-14-7-plugin` (cdylib `rustic_isles_1_14_7_plugin`), both `license = "MIT"`,
  `Copyright (c) 2026 Silent Spud`
- **Behaviour specification**: `docs/port/spec-rustic-isles-1-14-7.md` (its closing "Coordinator decisions" override
  the rest), with the seam sections of the area specs for the contracts it composes
- **Clean-room worktree**: `R:\source\dauntless\dainty-lass\.claude\worktrees\agent-a0a69cc8918526172` (branch
  `feat/rustic-isles-1-14-7`, reset to `feat/rustic-isles-clean-room` at `54b380ec`, later merged with it at
  `6d6a4dd6`)
- **Implementer**: one Claude Code subagent session, briefed by the RD27 coordinator

## 1. Isolation

### 1.1 Rules the implementer worked under

It never read, listed, grepped, globbed or opened anything under `R:\source\dauntless\dainty-restricted` (any worktree,
including the session's additional working directory `splashloader-integration-90130d`), `legacy-revived`, any
`rustic-isles` or `rustic-rifts` tree outside dainty-lass, or any other repository. It ran no `git show` or
`git log -p` of another branch. It searched nothing on the web and looked for no golden file or capture.

### 1.2 Other sources

None beyond this repository.

## 2. Files read

- The coordinator's brief and its follow-up message; `AGENTS.md`; `docs/port/spec-rustic-isles-1-14-7.md` (whole).
- `docs/port/spec-rustic-isles-economy-inventory.md` (§3.1, §11), `spec-rustic-isles-matchmaking.md` (the QoS
  start-up section and the decision list), `spec-rustic-isles-characters.md` (the starter manifest section and its
  decisions), `spec-rustic-isles-economy-bounties.md` (the `ForeignAccounts` rows), by grep for the seams named.
- The MIT crates composed, for their public APIs: `crates/rustic-isles` (lib, composer, version, module, shared,
  handlers, routes and table, settings, game_data, dedicated, observe, testing, tests/module.rs),
  `rustic-isles-auth` (seams, store, policy, limiter, errors, test_support), `rustic-isles-store` (lib, adapter, the
  auth repository signatures), `rustic-isles-gamedata` (lib, generators, reward_cache), `rustic-isles-director`
  (lib, config), `rustic-isles-characters` (lib), `rustic-isles-chat` (lib, rules, source, home, hubs),
  `rustic-isles-economy` (lib, caches, caller, inventory ports, progression and bounties modules, multipliers),
  `rustic-isles-matchmaking` (lib, state, startup, hooks, rifts_core), `rustic-isles-reforged` (lib, seams, state,
  jobs, admin, tester_max, activity, operations), `rustic-isles-db` (`LegacyStarter`), `rifts-seam`, `rifts-realms`
  (lib, service, leases, ports, capabilities, host_policy, tiers, directory, routes, db, ids, selection,
  tests/routes_dispatch.rs), `rifts-admission` (lib, keys, ticket, service, routes), `rifts-relay` (lib, config,
  credentials, service, routes), `rifts-guard` (lib, ports, routes, sql, store), `module-api` (plugin, child, host,
  manifest), `plugin-host` (tests/library.rs), `store` (engine, database), `fuzz/Cargo.toml` and one fuzz target for
  the declaration format, `docs/security/rustic-isles-auth.md` and `docs/port/clean-room/rustic-isles-matchmaking.md`
  for the page formats.

## 3. Files written

- `crates/rustic-isles-1-14-7/`: `Cargo.toml`, `src/{lib,hooks,data,composer,accounts,realms,rifts,test_support,main}.rs`,
  `tests/{version,module,rifts}.rs`.
- `crates/rustic-isles-1-14-7-plugin/`: `Cargo.toml`, `src/lib.rs`, `tests/load.rs`.
- `crates/rustic-isles/src/{composer,module}.rs`: `ComposeInputs.ports` (own commit).
- Root `Cargo.toml` (`[workspace.dependencies]`), `Cargo.lock`, `fuzz/Cargo.toml`, `fuzz/Cargo.lock`,
  `fuzz/fuzz_targets/rustic_isles_1_14_7_version_data.rs`, `docs/security/rustic-isles-1-14-7.md`, this record.

## 4. Verification

`cargo test -p rustic-isles-1-14-7` (default features and `--no-default-features`), `cargo test -p
rustic-isles-1-14-7-plugin`, `cargo clippy ... -D warnings` and `cargo fmt --all --check`. Nothing was played against
the real 1.14.7 client.

## 5. Deviations and ambiguities, and how they were resolved

1. **The starter manifest's lists (resolved).** The maintainer ruled the 24 stacked ids and the quick-emoji wheel
   wire facts to commit (decision item 1), but at first their values were in no document this implementer could
   read, so `TABLES` carried empty lists. The spec's closing subsection "starter manifest wire facts" (merged from
   `feat/rustic-isles-clean-room` at `5304f463`) then gave them; `TABLES` now holds the bootstrap version, the 24
   stacked ids and the nine wheel slots exactly as given, pinned by `starter_manifest_wire_facts` (counts, order,
   the wheel never empty, every wheel id a stacked id). No characters test assumed empty lists: the characters
   crate's tests use their own tables.
2. **`starterManifest` in `version_data.json`** existed only under option B; option A was ruled, so the member is
   refused as unknown.
3. **The QoS port.** Core's `ComposeInputs` carried no ports, so the composer could not bind the pong on the assigned
   `qos` port; core gained `ComposeInputs.ports` (a seam change, own commit).
4. **The Reforged admin state** is built at compose (`ReforgedAdmin::new(discord, EconomyFunding)`, the API the
   Reforged unit shipped), not at start-up 1 as §5.8 had it, and is in the slots with or without a store. Its chat
   half needs the module's `Shared`, which does not exist at compose, so the `chat.*` home operations are answered by
   the composer's hook through chat's `home_operation` with the chat source start-up 1 builds (the coordinator's
   "adapt core's arguments into a HomeInvocation").
5. **The admin authenticator** is installed by the Reforged admin start-up (`admin_startups`), since it needs
   `Shared`; `Composition.admin_authenticator` is none.
6. **Rifts' realms database**: `RealmsDb::open` already applies Guard's schema first, so §6 step 3's two opens are one
   call. With a database but no usable ticket key (neither configured nor generated), the stubs answer, as without a
   database.
7. **Guard's hunt sessions** share the host capabilities' presence cache (`HostCapabilities::hunt_sessions`), as
   rifts-realms documents, rather than a fresh `RealmsHuntSessions`.
8. **The generated transport secret** is rifts-realms' 256-bit capability secret generator (at least 128 bits, §14
   item 5).
9. **`LegacyStarter`** is rustic-isles-db's type, so the crate depends on `rustic-isles-db` (not in §10.1's list).
10. **The header test** (§12.2 "Header") is not repeated here: core's own pipeline tests pin the
    `x-rustic-host-capability` bucket, and every reader in the composition is rifts-seam's constant.

## 6. Similarity

Measured by the RD27 reviewer (who may read the AGPL code) with `similarity.md` §1's method, on the crates as
committed at `f0504520`: every `.rs` file in both crates' `src/` and `tests/` against each of the 13 `.rs` files of
restricted `e0e238b`'s `modules/rustic-isles/crates/isles-1-14-7/` (`src/{lib,main}.rs` and its 8 test files),
`isles-1-14-7-plugin/` (`src/lib.rs`, `tests/load.rs`) and `isles-core/src/module.rs` (the composition), the best
single file named. "Any" counts a line matched by a 3+-line run in any of the 13. Paths in the first column are under
`crates/`, in the third under `modules/rustic-isles/crates/`; a file with no match names none.

| File | New lines | Best restricted file (old lines) | Matched | % | Any, % |
| --- | ---: | --- | ---: | ---: | ---: |
| `rustic-isles-1-14-7/src/accounts.rs` | 147 | `isles-1-14-7/tests/module.rs` (594) | 13 | **8.8** | 11.6 |
| `rustic-isles-1-14-7/src/composer.rs` | 391 | `isles-core/src/module.rs` (1349) | 26 | **6.6** | 7.2 |
| `rustic-isles-1-14-7/src/data.rs` | 301 | `isles-core/src/module.rs` (1349) | 6 | **2.0** | 2.0 |
| `rustic-isles-1-14-7/src/hooks.rs` | 161 | `isles-1-14-7/src/lib.rs` (628) | 36 | **22.4** | 22.4 |
| `rustic-isles-1-14-7/src/lib.rs` | 143 | `isles-1-14-7/src/lib.rs` (628) | 61 | **42.7** | 42.7 |
| `rustic-isles-1-14-7/src/main.rs` | 10 | `isles-1-14-7/src/lib.rs` (628) | 3 | **30.0** | 30.0 |
| `rustic-isles-1-14-7/src/realms.rs` | 109 | `isles-1-14-7/src/lib.rs` (628) | 6 | **5.5** | 8.3 |
| `rustic-isles-1-14-7/src/rifts.rs` | 341 | `isles-1-14-7/src/lib.rs` (628) | 32 | **9.4** | 10.0 |
| `rustic-isles-1-14-7/src/test_support.rs` | 45 | none | 0 | **0.0** | 0.0 |
| `rustic-isles-1-14-7-plugin/src/lib.rs` | 7 | none | 0 | **0.0** | 0.0 |
| `rustic-isles-1-14-7/tests/module.rs` | 291 | `isles-1-14-7/tests/module.rs` (594) | 14 | 4.8 | 4.8 |
| `rustic-isles-1-14-7/tests/rifts.rs` | 169 | `isles-1-14-7/tests/module.rs` (594) | 7 | 4.1 | 5.9 |
| `rustic-isles-1-14-7/tests/version.rs` | 256 | `isles-1-14-7/tests/module.rs` (594) | 4 | 1.6 | 2.7 |
| `rustic-isles-1-14-7-plugin/tests/load.rs` | 44 | `isles-1-14-7-plugin/tests/load.rs` (47) | 10 | 22.7 | 25.0 |
| The 10 `src` files | 1655 | | 183 | **11.1** | 11.7 |
| The 4 test files | 760 | | 35 | **4.6** | 5.5 |
| All 14 | 2415 | | 218 | **9.0** | 9.8 |

Two files reach 25 % against their best single file (`lib.rs` 42.7, `main.rs` 30.0, which is 3 of 10 lines); the
plugin's `tests/load.rs` reaches it on the "any" column (25.0). The fuzz target
(`fuzz/fuzz_targets/rustic_isles_1_14_7_version_data.rs`) has no restricted counterpart and was not measured.

**What the matches are.** Every run of 5 lines or more against any of the 13 files was read: 9 of them, over 9
distinct spans, and the files at 25 % or more were read whole with every matched line marked. The matches:

- **The version's tables** (`lib.rs`, 61 matched lines): `TABLES` (`lib.rs:57-102`, runs of 23, 7 and 5) is §3.2's
  values in §3.2's order, in the field order of core's `VersionTables`, `BountyItemGrant`, `SeasonalCoin`,
  `MaskedJourneyNode` and `StarterManifest` (MIT types); `DEFAULT_PORTS`' game range (`46-49`, §3.1);
  `DIRECTOR`'s six switches (`125-131`, §3.4, in order); `capabilities()` (`139-145`, `module_api::Capabilities`'
  fields with §3.1's values); and `version()` (`147-153`, core's `IslesVersion` fields). `switch_defaults`,
  `session_payload`, the `DIRECTOR` constructors and `spec_over`/`spec`/`spec_with`/`module` do not match.
- **The hooks** (`hooks.rs`, 36): the `VersionHooks` method heads (core §5.2, a trait of MIT `rustic-isles`), each
  with §3.3's one-line answer (`22-44`, `57-66`: `ConfiguredAccount`, `true`, `true`, §3.3 row 6's
  `json!({ "huntpass_id": id, "season": id })`, `RollingHuntpassWindows`, row 9's `StubReply`), and the
  `!settings.raw("KEEP_WEAPON_ITEM_LEVELS")` head of row 3 (`29-30`). `objective_row`, `entitlements_payload` and
  `candidate_destination`'s bodies, the `number` helper and the tests do not match.
- **Seam signatures and MIT struct literals**: `accounts.rs:124-136` and `tests/rifts.rs:39-45` are
  `rifts_seam::IslesCore` method heads delegating to `rifts_seam::NoCore`, as §5.6.1's table says; `rifts.rs`'s
  matches are its `rifts_realms` `use` lines (`38-44`) and the fields of MIT `HostingRuntime`/`DirectoryParts`,
  `GuardParts`, `RealmsServiceParts`, `RealmsRoutesParts` and `RiftsParts` (`274-295`, `348-384`), with
  `RandomIds`, `StderrLog`, `RealmLimits::from_settings` and `GuardConfig::from_settings` from those crates.
- **Wire text and test vectors**: §5.8's start-up multipliers line, warn when on and info otherwise
  (`composer.rs:359-362`); the manifest asserts in `tests/version.rs:20-23` (§3.1's id, name and game version).
- **Boilerplate**: the remaining matched lines in `composer.rs`, `realms.rs`, `data.rs`, `accounts.rs` and `main.rs`
  are closing-brace runs (`}) } }`); `tests/module.rs:62-87` and the plugin's `tests/load.rs:37-40` are
  `module_api::ExternalModule`'s fields and `ModuleDriver::load`'s four arguments; `tests/load.rs:16-23`'s
  `library()` helper is the one in `crates/chat-plugin/tests/load.rs` and `crates/content-plugin/tests/parity.rs`,
  this repository's own idiom for finding a built cdylib.

No comment was copied: one comment line equals a restricted one (`lib.rs:63`, `// The 1.14.7 runtime collapses the
line`), §3.2's own parenthesis. Every string literal of 8 characters or more that also occurs in the 13 files is in
the specs (core, 1.14.7 or an area spec) or is a format fragment the MIT crates already use (`multi_thread`,
`: null, `, the 64-zero hash). Of the crates' 130 function names, 44 also occur in restricted: the `VersionHooks` and
`IslesCore` methods, the `ModuleHooks` and `Composer` methods, the MIT port traits' (`is_limited`, `post`), §10.2's
public functions (`version`, `spec`, `module`, `rifts_with_core`), `main`, and five plain helper names (`text`,
`env_file`, `host`, `tables`, `library`). One of 18 type names is shared (`Hooks`, §3.3's name), and the five shared
constant names (`ID`, `DISPLAY_NAME`, `TABLES`, `DIRECTOR`, `RAMSGATE_HUNT`) are named by §10.2 and the matchmaking
spec. No unusual structure is shared: the composer, the version-data reader, the accounts adapter, the realms adapter
and the rifts composition match at 2 to 10 %, and share no logic beyond the lines above.

**Verdict: clean of structural or expressive copying; nothing to rewrite.** The two files over 25 % are §3.2's and
§3.4's tables and §3.3's one-line hook answers in the only idiomatic form, and three closing braces; the rest is
spec-given wire text, MIT seam signatures and struct fields, and boilerplate. `TABLES.starter_manifest`'s two lists
were empty when measured; filling them from the spec's amended decisions (2026-11-05) adds 33 lines of wire facts
that will match restricted by construction.
