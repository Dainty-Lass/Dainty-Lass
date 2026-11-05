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

Measured by the RD27 reviewer (who may read the AGPL code) with `similarity.md` §1's method, on the crate as committed
at `14baf050`: every `.rs` file in `src/` and `tests/` against each of the 50 files of restricted `e0e238b`'s
`modules/rustic-isles/crates/isles-core/` that hold the three areas and the multipliers: `src/areas/inventory.rs` and
the 15 files of `areas/inventory/`, `src/areas/progression.rs` and the 15 of `areas/progression/` (with
`testkit/golden.rs`), `src/areas/bounties_store.rs` and the 16 of `areas/bounties_store/` (its `*_tests.rs` files
among them), `src/areas/matchmaking/multipliers.rs`, and the tests that drive them, `tests/inventory.rs` and
`tests/common/mod.rs`, the best single file named. "Any" counts a line matched by a 3+-line run in any of the 50.
Paths in the third column are under `isles-core/src/`; a file with no match names none.

| File | New lines | Best restricted file (old lines) | Matched | % | Any, % |
| --- | ---: | --- | ---: | ---: | ---: |
| `src/bounties/challenge_rewards.rs` | 171 | `areas/bounties_store/challenge_rewards.rs` (95) | 32 | **18.7** | 25.1 |
| `src/bounties/cooldowns.rs` | 395 | `areas/bounties_store/cooldown_state.rs` (323) | 35 | **8.9** | 13.2 |
| `src/bounties/economy_ledger.rs` | 675 | `areas/bounties_store/economy_ledger.rs` (829) | 82 | **12.1** | 15.3 |
| `src/bounties/entitlements.rs` | 70 | `areas/bounties_store/entitlements.rs` (75) | 36 | **51.4** | 54.3 |
| `src/bounties/mod.rs` | 82 | `areas/bounties_store/authn.rs` (337) | 13 | **15.9** | 15.9 |
| `src/bounties/platinum.rs` | 59 | `areas/bounties_store/platinum.rs` (231) | 27 | **45.8** | 47.5 |
| `src/bounties/routes.rs` | 1252 | `areas/bounties_store/handlers.rs` (1443) | 97 | **7.7** | 11.4 |
| `src/bounties/state.rs` | 892 | `areas/bounties_store/bounty_state.rs` (889) | 69 | **7.7** | 12.3 |
| `src/bounties/store.rs` | 894 | `areas/bounties_store/store.rs` (900) | 202 | **22.6** | 26.6 |
| `src/bounties/wallet.rs` | 258 | `areas/bounties_store/wallet.rs` (185) | 53 | **20.5** | 31.4 |
| `src/caches.rs` | 47 | none | 0 | **0.0** | 0.0 |
| `src/caller.rs` | 17 | none | 0 | **0.0** | 0.0 |
| `src/inventory/currency.rs` | 154 | `areas/inventory/ports.rs` (564) | 27 | **17.5** | 20.8 |
| `src/inventory/items.rs` | 681 | `areas/inventory/items.rs` (720) | 100 | **14.7** | 18.2 |
| `src/inventory/mod.rs` | 24 | none | 0 | **0.0** | 0.0 |
| `src/inventory/ports.rs` | 314 | `areas/inventory/ports.rs` (564) | 146 | **46.5** | 50.6 |
| `src/inventory/prestige.rs` | 111 | `areas/inventory/progression_config.rs` (113) | 12 | **10.8** | 17.1 |
| `src/inventory/reforge.rs` | 360 | `areas/inventory/reforge.rs` (335) | 22 | **6.1** | 15.3 |
| `src/inventory/routes.rs` | 922 | `areas/inventory/routes.rs` (1006) | 54 | **5.9** | 9.2 |
| `src/inventory/storage_policy.rs` | 452 | `areas/inventory/storage_policy.rs` (500) | 79 | **17.5** | 22.1 |
| `src/inventory/transaction.rs` | 627 | `areas/inventory/transaction.rs` (603) | 132 | **21.1** | 23.1 |
| `src/inventory/validation.rs` | 211 | `areas/inventory/validation.rs` (226) | 51 | **24.2** | 24.6 |
| `src/inventory/weapon_level.rs` | 84 | `areas/inventory/items.rs` (720) | 8 | **9.5** | 16.7 |
| `src/lib.rs` | 21 | none | 0 | **0.0** | 0.0 |
| `src/multipliers.rs` | 271 | `areas/matchmaking/multipliers.rs` (328) | 75 | **27.7** | 28.4 |
| `src/progression/escalation.rs` | 675 | `areas/progression/escalation.rs` (754) | 85 | **12.6** | 14.2 |
| `src/progression/journey.rs` | 376 | `areas/progression/journey.rs` (951) | 45 | **12.0** | 15.4 |
| `src/progression/mastery.rs` | 361 | `areas/progression/mastery.rs` (560) | 22 | **6.1** | 13.9 |
| `src/progression/mod.rs` | 91 | `areas/progression/ports.rs` (104) | 16 | **17.6** | 22.0 |
| `src/progression/reforge.rs` | 353 | `areas/progression/reforge.rs` (373) | 142 | **40.2** | 41.9 |
| `src/progression/routes.rs` | 2360 | `areas/progression/handlers.rs` (972) | 91 | **3.9** | 9.9 |
| `src/progression/supplies.rs` | 153 | `areas/progression/supplies.rs` (231) | 21 | **13.7** | 17.6 |
| `src/progression/tracks.rs` | 350 | `areas/progression/tracks.rs` (400) | 59 | **16.9** | 20.9 |
| `src/test_support.rs` | 64 | none | 0 | **0.0** | 0.0 |
| `src/tx.rs` | 34 | `areas/progression/escalation.rs` (754) | 3 | **8.8** | 8.8 |
| `src/util.rs` | 218 | `areas/bounties_store/wallet.rs` (185) | 11 | **5.0** | 7.8 |
| `tests/bounties.rs` | 360 | `areas/bounties_store/bounty_state_tests.rs` (1768) | 3 | 0.8 | 2.5 |
| `tests/common/mod.rs` | 692 | `areas/progression/testkit.rs` (460) | 39 | 5.6 | 10.8 |
| `tests/inventory.rs` | 732 | `areas/bounties_store/bounty_state_tests.rs` (1768) | 9 | 1.2 | 2.0 |
| `tests/progression.rs` | 679 | `areas/bounties_store/bounty_state_tests.rs` (1768) | 9 | 1.3 | 2.7 |
| `tests/routes.rs` | 23 | `areas/progression/supplies.rs` (231) | 3 | 13.0 | 13.0 |
| The 36 `src` files | 14079 | | 1847 | **13.1** | 17.3 |
| The 5 test files | 2486 | | 63 | **2.5** | 4.8 |
| All 41 | 16565 | | 1910 | **11.5** | 15.4 |

Five files reach 25 % against their best single file (`bounties/entitlements.rs` 51.4, `inventory/ports.rs` 46.5,
`bounties/platinum.rs` 45.8, `progression/reforge.rs` 40.2, `multipliers.rs` 27.7); `bounties/wallet.rs` (31.4),
`bounties/store.rs` (26.6) and `bounties/challenge_rewards.rs` (25.1) also do on the "any" column. The eight fuzz
targets (`fuzz/fuzz_targets/rustic_isles_economy_*.rs`) have no restricted counterpart and were not measured.

**What the matches are.** Every run of 5 lines or more against any of the 50 files was read: 237 of them over 169
distinct spans of the crate, 41 of which are, or end in, the `} } #[cfg(test)] mod tests { use super::*; #[test]`
boilerplate or a run of closing braces. The rest:

- **Ports and seam signatures** (`inventory/ports.rs`, 146 matched lines): `AuthRequest`'s five fields, the
  `Authentication` answer, `Authenticator`, `CurrencyChange`, `WalletError`'s three variants, the `Wallet` trait's
  three methods, `CharacterOwnership` with its display text, `StarterBootstrap::ensure` and
  `Characters::first_character_id` (`58-73`, `94-151`, `193-226`, `250-272`, runs up to 19). Inventory §11.3 gives
  every port, argument, return type and the error's text; the matched lines are those signatures and the impl heads
  that repeat them. The impl bodies (the extension authenticator, the bounties wallet, the characters adapters) match
  only in a 7-line `first().and_then(get("id"))...` chain, §11.3's "its first entry's `id` string".
- **Spec records and enums in the spec's order**: `TxRequest`, `TxOptions`, `Economy`, `TxError` and its wallet
  conversion (`inventory/transaction.rs:104-176`; inventory §12, bounties §6.2 I1), `inventory_record`'s fields
  (`41-58`, the store record), `ReforgeMode`, `ReforgePlan` with `kind()`'s four strings and the P2 to P5 signatures
  (`progression/reforge.rs:22-88`, `100-106`, `164-169`, `233-240`, `284-290`; progression §11 and inventory P1 to
  P5), `EconomySource` with its eight texts and the ledger scope (`bounties/economy_ledger.rs:23-81`, bounties §6.2),
  `PurchaseReason` with its codes and statuses, `SkuItem`, `SkuGrants` and `PurchasePlan` (`bounties/store.rs:37-117`,
  `644-689`; bounties §4.10's fields), `FundedClaim` and W5/W6's heads (`challenge_rewards.rs:67-89`, `137-143`),
  `WalletError::Insufficient` (`wallet.rs:37-50`, W2), the bounty and cooldown summaries (`state.rs:279-284`,
  `cooldowns.rs:122-129`; the spec's camelCase keys in snake case), `Storage` and `PolicyEntry`
  (`storage_policy.rs:18-41`), `Talent` and `SeasonTuning` (`escalation.rs:79-92`, progression §6's file shape) and
  `TestMultipliers` with `OFF` (`multipliers.rs:18-30`, inventory §11.2).
- **Spec tables**: progression's 23 default track ids (`progression/tracks.rs:37-61`, the longest run, 25), the 13
  Platinum buckets in spend order (`platinum.rs:15-28`), the starter wallet (`wallet.rs:18-24`, W1), the store's nine
  sections (`store.rs:18-26`, and again in `tests/common/mod.rs:97-104`), the currency-id map (`store.rs:633-642`),
  the null-valued SKU keys (`store.rs:593-604`, `915-919`), the draft keys (`state.rs:63-67`), the four item-list
  field names (`validation.rs:119-125`) and the multiplier's refusal reasons in §10.1's order (`multipliers.rs:72-89`).
- **Spec rules read in the spec's words**: the effective price (`platinum.rs:43-55`, §4.8.3's sentence), the talent
  build's spend check (`escalation.rs:243-293`: `earned = min(L, max)`, purchases sorted by gate, the spec's
  `{p}`/`{e}`/`{g}`/`{s}` placeholders as the locals), the level/XP check (`escalation.rs:207-213`), the reforge
  restore and refuse rows (`progression/reforge.rs:125-147`, §4.5's table), the stale repair's two increments and log
  line (`263-280`, §4.6), the entitlement grant loop (`entitlements.rs:67-82`, §4.7), the SKU id rule and its message
  (`store.rs:236-243`), the Tailscale address range (`multipliers.rs:94-107`), and logged texts
  (`bounties/routes.rs:752-756`, `1019-1023`, `1252-1265`).
- **Test vectors**: the stacked-apply rows of inventory's table (`inventory/items.rs:561-642`: `MAT`, `NEW` with
  `"source":"x"`, `CURRENCY_B` 5 then `CURRENCY_A` 7), the bounty state's key order (`state.rs:977-988`), the
  multiplier settings (`multipliers.rs:184-188`), and the dedicated-only seam stubs in `tests/common/mod.rs:276-315`
  (`rifts_seam`'s trait methods).

No comment was copied: no comment line of the crate equals a restricted one. Of the 1426 string literals of 12
characters or more that also occur in the 50 files, 1392 are in the economy, core, store, auth or characters specs;
the other 34 are templated spec messages written out (`addStackedItems entry has an invalid quantity` for
`<field> entry has an invalid quantity`), test dates and ids, or Rust fragments the scan caught (`.to_owned(), `).
242 of the crate's 626 function names and 92 of its 199 type and constant names also occur in restricted. Nearly all
are the specs' (each spec's module table and W/I/P call tables) or their Node names in snake case; about forty private
helpers are not written in any spec but are the plain name of a spec concept or route (`check_level_xp`,
`filter_unlocks`, `save_cooldowns`, `may_read_escalation`, `empty_breadcrumbs`, `is_huntpass_xp_track`,
`sha256_hex`), and their bodies match no further than the lines above. No unusual structure is shared: the large files
(`progression/routes.rs` 3.9 %, `bounties/routes.rs` 7.7, `inventory/routes.rs` 5.9, `bounties/state.rs` 7.7,
`economy_ledger.rs` 12.1, `escalation.rs` 12.6) and the black-box tests (0.8 to 5.6 %) share no dispatcher,
transaction, merge or ledger logic beyond the lines above.

**The two spans that go beyond the spec.**

1. **`src/multipliers.rs:32-37` (`TestMultipliers::off`).** A private constructor `off(reason)` built as
   `Self { reason: Some(..), ..Self::OFF }` and called once per refusal in `read_test_multipliers` (`72-83`). The
   spec names `OFF` and the reasons, not a helper; this one's name, signature and struct-update body coincide with
   restricted's, and it carries the 18-line run at `18-37` and the 8-line one at `82-89`. A stock shape, but the one
   private helper in the crate that coincides in name and body.
2. **`src/inventory/ports.rs:215` (`CharactersStarter`).** The default `StarterBootstrap` adapter's type name, which
   no spec gives, is identical; with §11.3's signature after it, it makes the 11-line run at `215-226`. The body
   differs. A name only.

**Verdict: clean of structural or expressive copying, with two small rewrites asked for as a precaution.** Everything
else is spec-given signatures, records and enums in the spec's order, spec tables and wire texts, rules transcribed
from the spec's own sentences, pinned test vectors and boilerplate. The rewrites, from inventory §10.1, §11.2 and
§11.3 only and without looking at the current text: `read_test_multipliers`' refusals without a private `off` helper
(for instance the reason decided first and the record built once), and `CharactersStarter` renamed in the
implementer's own words. `entitlements.rs`, `platinum.rs`, `ports.rs`' signatures, `progression/reforge.rs`,
`wallet.rs`, `store.rs` and `challenge_rewards.rs` need nothing: their matched lines are the specs' signatures, tables
and rules in the only idiomatic form.
