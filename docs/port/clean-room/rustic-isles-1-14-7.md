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

1. **The starter manifest's lists are empty.** The maintainer ruled the 24 stacked ids and the quick-emoji wheel wire
   facts to commit (decision item 1), but their values are in no document this implementer may read. `TABLES` carries
   empty lists with a comment; the maintainer fills them. Until then a new character gets no starter stacked items
   and no quick-emoji wheel.
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

To be measured by the coordinator against restricted `e0e238b`.
