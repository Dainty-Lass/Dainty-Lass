# Clean-Room Provenance Record: `crates/rustic-isles-economy` (RD27)

- **Component**: `crates/rustic-isles-economy` (crate `rustic-isles-economy`, `license = "MIT"`, `Copyright (c) 2026
  Silent Spud`)
- **Behaviour specifications**: `docs/port/spec-rustic-isles-economy-inventory.md`, `-progression.md` and
  `-bounties.md` at dainty-lass `3ac75074` (branch `feat/rustic-isles-clean-room`, which holds `c155b29e`), each with
  its closing coordinator decisions
- **Clean-room worktree**: `R:\source\dauntless\dainty-lass\.claude\worktrees\agent-a8535dd76c16a4ad6` (branch
  `feat/rustic-isles-economy`, reset to `feat/rustic-isles-clean-room` before the work)
- **Implementer**: one Claude Code subagent session, briefed by the RD27 coordinator with the rules below

## 1. Isolation

### 1.1 Rules the implementer worked under

It never read, listed, grepped, globbed or opened anything under `R:\source\dauntless\dainty-restricted` (any
worktree of it, including the session's additional working directory `splashloader-integration-90130d`),
`legacy-revived`, any `rustic-isles` or `rustic-rifts` tree outside dainty-lass, or any other repository. It ran no
`git show` or `git log -p` of another branch. It searched nothing on the web. It did not look for the goldens the specs
keep in dainty-restricted (`tests/golden/inventory/*`, `tests/golden/progression/*`, `tests/golden/bounties_store/*`);
the tests here are written from the specs' tables with synthetic data.

### 1.2 Other sources

None outside this repository: no crate sources from the cargo registry were read.

## 2. Files read

- The three economy specs (whole), `AGENTS.md`, the brief; from `spec-rustic-isles-characters.md` its closing
  decisions; from `spec-rustic-isles-core.md` its §17 decisions (by grep). The store, auth and game-data specs were not
  read: their contracts were taken from the crates' public items below.
- `Cargo.toml` (workspace dependencies and lints), `rust-toolchain.toml`, `fuzz/Cargo.toml`, the header of
  `fuzz/fuzz_targets/rustic_isles_characters_data.rs` and `rustic_isles_chat_names.rs`
- `crates/rustic-isles`: `lib.rs`, `version.rs`, `game_data.rs` (first 150 lines), `season.rs` (first 60 lines),
  `replay.rs` (lines 100-170), `shared.rs` (lines 95-215 and its public items), `handlers.rs` (first 60 lines),
  `ids.rs` (first 60 lines), `testing.rs` (lines 330-451), `pipeline.rs`' `not_implemented`, the
  `routes/table.rs` rows of the three groups (by grep), the public items of `body.rs`, `settings.rs`, `module.rs`
- `crates/rustic-isles-store`: `Cargo.toml`, `lib.rs` (first 230 lines), `repo/mod.rs` (first 200 lines),
  `repo/inventory.rs` (most), `repo/bounties.rs` (records, traits, `save_if_version`, `create_if_missing`,
  `increment_balance`, the rollup document), `repo/progression.rs` (records, traits, `increment`, `begin_reforge`,
  `update_reforge`), the trait lines of `repo/auth.rs` and `repo/characters.rs`
- `crates/rustic-isles-auth`: `seams.rs` (`RuntimeCaller`, `GameLogin`, `Seams`), `lib.rs`' `AuthData` and store-token
  methods, `store.rs`' `GameAccount`, `tokens.rs`' `StorePurchase`, `game.rs` (`game_auth` and its inner function),
  `util.rs`' `hex`, the public items of `test_support.rs`, `Cargo.toml`'s features
- `crates/rustic-isles-characters`: `Cargo.toml`, `lib.rs`, `routes.rs` (first 250 lines), `controller.rs`
  (`Characters`, `characters_for_user`, `ensure_starter_in`, `ensure_starter`, `loadout_state`), `tests/common/mod.rs`
  (whole; the test harness here follows its shape)
- `crates/rifts-seam`: `hooks.rs` (lines 170-300), `http.rs` (`Json`, `Reply`, `RuntimeAuth`), `error.rs`,
  `dedicated.rs` (the dedicated-only guard), the `Fut` aliases and public items (by grep)
- `crates/module-api/src/network.rs` (`IdKind`, `GlobalId`, `HuntRewards`, `HomeApi`), the `ModuleId` lines of
  `ids.rs`
- `crates/service-kit/src/js.rs` (first 140 lines), `jsjson.rs` (first 230 lines), the public items of `json.rs`,
  `collate.rs`, `body.rs`
- `crates/rustic-isles-gamedata/src/progression_config.rs` (first 80 lines), the public items of its `lib.rs`
- `docs/security/rustic-isles-auth.md` (first 80 lines) for the checklist's format;
  `docs/port/clean-room/rustic-isles-auth.md` (first 120 lines) for this record's

## 3. Files written

### 3.1 The crate, from the specs

| File | Spec |
| --- | --- |
| `src/lib.rs`, `src/util.rs`, `src/caller.rs`, `src/tx.rs`, `src/caches.rs` | inventory §2 and decisions (caches, the runtime handle); progression §2.3 (`Number(text)`) |
| `src/multipliers.rs` | inventory §10 (with the decision: JavaScript `Number`), progression §4.7 |
| `src/inventory/{items,storage_policy,prestige,validation,weapon_level}.rs` | inventory §4.6, §7, §8, §9.3 |
| `src/inventory/{ports,transaction,currency}.rs` | inventory §6, §9.1, §9.2, §11 |
| `src/inventory/{reforge,routes}.rs` | inventory §3 to §5 |
| `src/progression/{tracks,reforge}.rs` | progression §4.1 to §4.6 (P1 to P6) |
| `src/progression/{mastery,escalation,journey,supplies}.rs` | progression §4.8 to §4.14 |
| `src/progression/{mod,routes}.rs` | progression §3, §5, §9, §10 |
| `src/bounties/{wallet,platinum,challenge_rewards,economy_ledger}.rs` | bounties §4.5, §4.8, §4.14, §6.2 (W1 to W9) |
| `src/bounties/{state,cooldowns,entitlements,store}.rs` | bounties §4.1 to §4.13 |
| `src/bounties/{mod,routes}.rs` | bounties §3, §6.1, §6.4, §10 |
| `src/test_support.rs` | bounties §12.3 (synthetic game data only) |
| `tests/{common/mod.rs,inventory.rs,progression.rs,bounties.rs,routes.rs}` | inventory §13, progression §13, bounties §12 |

### 3.2 Elsewhere

- `Cargo.toml`: `rustic-isles-economy` in `[workspace.dependencies]` (a seam change for the integration pass, its own
  commit); `Cargo.lock` with the crate's commit
- `fuzz/fuzz_targets/rustic_isles_economy_*.rs` (eight), their `[[bin]]` entries and the dependency in
  `fuzz/Cargo.toml`, `fuzz/Cargo.lock`
- `docs/security/rustic-isles-economy.md`, this record

No other crate was changed.

## 4. Verification

- `cargo test -p rustic-isles-economy --locked --no-fail-fast`: 63 unit tests and 35 black-box tests (`inventory` 17,
  `progression` 11, `bounties` 6, `routes` 1), 0 failed
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`: see the report
- `cargo check` of the eight fuzz binaries in `fuzz/`: clean (not fuzzed; the targets need nightly)
- The worked examples the specs give were checked by tests: §4.1's curve example, §7.4 and §7.5's tables, §8.4's
  repair rows, §4.7's multiplier table, §4.9's mastery rows, §4.14's supplies rows, the daily window at
  16:59:59.999 and 17:00
- Not run against the real 1.14.7 client or game servers, and not against the goldens, which replay in
  dainty-restricted once the composer switches to this crate

## 5. Deviations and ambiguities, and how they were resolved

1. **The starter wallet reaches the wallet primitives as a parameter.** W1 to W3 (`get_wallet`, `add_currency`,
   `spend_platinum`) and `read_wallet` take the starter balances, which `wallet::starter_balances(shared)` reads from
   the characters area's `starter_wallet()` (characters decision item 4), else the six balances the spec lists.
2. **Per-module caches need a slot.** The storage policy, prestige sources, curves, journey definitions and the store
   catalog are kept in an `EconomyCaches` the composer puts in the module's slots; without it every read loads afresh.
3. **A runtime without rifts' handle.** Inventory answers 403 `{"code":"forbidden",...}` (`This runtime cannot mutate
   inventory.` on `POST /inventory`, `This runtime cannot write player state.` elsewhere); progression the 403 of its
   §5.1; bounties an empty 403 (its decision).
4. **A runtime's user.** Auth's game auth gives a runtime no `user_id`, so a fenced `POST /inventory` and a runtime's
   item save run only when the realms seam or composer supplies `AuthData` with one (tests do so in the extensions).
5. **The economy ledger's `commit` and `commit_to_store`** take the monitor settings as well as the time.
6. **`store::Section`** is not a type here: a section is its tag text.
7. **An instanced store item** builds at most 1000 instances per SKU entry (a bound the spec does not state; a file
   with a larger quantity would otherwise loop without limit).
8. **Escalation summary key order** with a season table: `levelXpRejected` after `levelXpRegressionIgnored`,
   `talentPoints`, `talentsReset` and `talentsRejected` after `talents`, `version` last.
9. **Over-limit mastery objectives** appear in the report answer only when a stored row exists ("keeps the stored
   state if there is one").
10. **The host ceiling on the multiplied amount** is checked just before the grant's transaction rather than inside
    it; both are pure checks ahead of the ledger read, so the order the spec gives is kept.
11. **The reads' authoritative check** (stale repair, login reconciliation) asks the guard with `parity: true` and no
    probe; a refusal means "not authoritative", a guard error is the empty 500.
12. **A mastery persistence guard error** reads as refused (diagnostics only), like a refusal.
13. **Encountered content**: each category's `content` is the list of the stored entries' values.
14. **Log escaping** is applied crate-wide to client text (the decision names inventory and bounties; progression and
    the reforge lines follow the same `log_token`).
15. **`Number(text)`** of path params uses `js_number_text`, which adds `0o` and `0b` to `service_kit::js::number`.
16. **The 15-minute duplicate window** is judged on the system clock (progression §15 item 3); the test shows a retry
    after a replay-pinned reset reads as an ordinary grant.
17. **Without the characters area composed**, the starter bootstrap only checks ownership and needs an existing
    inventory, and the first-character lookup only reads (it creates nothing).

## 6. Similarity

To be measured by the coordinator against restricted `e0e238b`.
