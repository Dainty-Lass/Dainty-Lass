<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: Rustic Isles economy (RX3)

The checklist for `crates/rustic-isles-economy`, the MIT crate holding Rustic Isles' inventory, progression, bounties
and store areas (RD27; `docs/port/spec-rustic-isles-economy-inventory.md`, `-progression.md`, `-bounties.md`). One
page for the crate (coordinator decision), a section per part.

How to use it: before a change to this crate merges, and before its cutover is signed (RD10), copy this list into the
change, check each item in the code (not from memory), link the code or test that shows it, and sign at the end with the
date and the commit reviewed. An item that does not hold is a bug to fix first, not a box to leave open.

## What it is and where the boundary is

The crate answers 50 backend rows (8 `inventory-reforge`, 25 `progression`, 17 `bounties-store`) inside the Rustic
Isles backend (port 61000), which listens on loopback behind the gateway; the gateway is the only public port and
refuses the host-only headers (`x-rustic-isles-gameserver-apikey`) from the internet.

- **In:** core's handler calls (`rustic_isles::Call`); authority is the auth crate's game auth (a game server key, a
  player token, or a player-host runtime the realms seam vouches for). Bodies are read with core's reader (100 KiB).
- **Out:** the store (`IslesStore`, every write an `account_write` with the home check except the economy rollups and
  the session supplies), rifts' guard (`authorize_host_write`, `record_host_write`, `authorize_inventory`,
  `host_policy_refuses`, `note_would_refuse`), the home API's `ring_forward_hunt_rewards`, auth's store-token signer,
  and the slots `BountiesSeams`, `RingInstancesSlot`, `InventoryActivitySlot`, `EconomyCaches`.
- **Files:** only the module's backend game-data folder, by fixed names (`inventory_storage_policy.json`,
  `progression_config.json`, `slayers_path.json`, `slayers_path_definitions.json`, `mastery_objectives.json`,
  `escalation_tuning.json`, the four store files) and `REFORGED_SESSION_SUPPLIES_FILE`. No path is built from request
  text.

Code: `crates/rustic-isles-economy/src/` (`inventory/`, `progression/`, `bounties/`, `multipliers.rs`, `caller.rs`,
`tx.rs`, `caches.rs`, `util.rs`).

Fuzz targets in `fuzz`: `rustic_isles_economy_inventory`, `rustic_isles_economy_progression_report`,
`rustic_isles_economy_journey`, `rustic_isles_economy_escalation`, `rustic_isles_economy_bounties`,
`rustic_isles_economy_cooldowns`, `rustic_isles_economy_store_data`, `rustic_isles_economy_text` (each file's header
says what it feeds and what it asserts).

## Shared

- [ ] A host runtime's authority is the realms seam's handle downcast to `rifts_seam::RuntimeAuth`
      (`caller::capability`); a missing or mistyped handle never writes (inventory and progression: 403 `forbidden`;
      bounties: empty 403).
- [ ] Every domain refusal inside a transaction rolls the whole write back (`tx::guarded`: the closure's error is held
      and the store sees an `Err`).
- [ ] Logs carry ids, never a token, key or setting value; client text in log lines goes through `log_token` (plain
      only when 1 to 128 bytes of `[A-Za-z0-9_.:-]`, else a JSON string), so CR/LF cannot forge a line (a departure
      from restricted's bare `userId`, coordinator decision).
- [ ] Game-data caches are per module (`EconomyCaches` slot), never process-wide; without the slot every read loads
      afresh.

## Inventory

- [ ] Grants and spends need a game server or a fenced runtime; a player may only save owned instances
      (`inventory/routes.rs`, 403 `Authoritative inventory mutations require gameserver authority`; test `saves`).
- [ ] Reforge routes refuse players outright (403 `Forbidden`); a runtime is asked `authorize_host_write(Reforge)`.
- [ ] Validation bounds ids (256 UTF-16 units), lists (4096), quantities and versions (1e9) and item data (262 144
      units) before anything is stored (`validation.rs`).
- [ ] Every transaction is bound to its body by SHA-256 over a canonical stringify; a reused id with another body is
      409, a pending duplicate is 409, a failure leaves no ledger row (`transaction.rs`; test `grants_replays_and_refusals`).
- [ ] Ledger, wallet, prestige claims, challenge credit, the fence's grant hook and the revision-guarded inventory
      write commit together or not at all.
- [ ] Currency is the wallet's (no `CURRENCY_` row stays in a blob); a debit never goes below zero (one guarded
      `increment_balance`); a prestige-gated reward needs banked levels; a season coin needs a claimed challenge
      (`currency.rs`; test `currency_rules`).
- [ ] Wallet keys are strict identifiers (`is_valid_balance_catalog_id`) before any wallet row is created.
- [ ] Known weak spots: game auth runs only on wire credentials here (spec §14 item 1, parity); reads write (starter
      records, storage repair, starter wallet); no `progression_config.json` ungates prestige rewards (§14 item 15,
      parity, a candidate decision row); one body may expand to about two million instances through normalization
      (§14 item 18, gameserver or fenced authority only; a candidate decision row for a total cap).

## Progression

- [ ] Only a game server key or an accepted host runtime grants XP, confirms ranks or saves the journey; a player reads
      only its own state and reports only diagnostics (`progression/routes.rs`; tests `grants`, `reports`).
- [ ] XP grants are keyed by a validated grant id (`^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$`) with a hash over user,
      track and amount; a reused id with another request is 409; the reforge's +1 and banked restore are marked inside
      the grant's transaction by compare-and-set (`reforge.rs`; test `confirm_and_reforge`).
- [ ] Host ceilings (§4.4): weapon and player XP 1000 a grant, prestige exactly 1, mastery never, Hunt Pass 200, the
      player track's total cap; observe mode only notes `amount_over_ceiling` and `track_full` (test `host_limits`).
- [ ] Mastery credit comes from stored objective state, bounded by the report's claim, a host's 64 new completions
      and its track caps (`mastery.rs`; fuzz `rustic_isles_economy_progression_report`).
- [ ] A host's journey save unlocks only nodes the definitions allow; a stored unlock is never lost
      (`journey.rs`; fuzz `rustic_isles_economy_journey`).
- [ ] A host's escalation write never lowers the level or the XP at an unchanged level; the season table bounds level,
      XP and the talent build (`escalation.rs`; fuzz `rustic_isles_economy_escalation`).
- [ ] Integers are checked before conversion (no `as` on an unchecked float); report size and counts, escalation
      entries (512), amounts (100 000) are capped.
- [ ] Known weak spots: the grant's 15-minute reforge window is on the system clock (§15 item 3); a missing
      `progression_config.json` is "no curves" until restart (§15 item 8); a federation forward is not idempotent
      (§15 item 10); the session supplies refill uses plain system writes without the home check (§15 item 14).

## Bounties and the store

- [ ] Bounty and cooldown writes come only from a game server or, after rifts' host-write check, a host runtime;
      reads only for the caller's own `userId`, a game server or a runtime's roster (`bounties/routes.rs`; tests
      `bounty_routes`, `cooldown_routes`).
- [ ] Host limits: weekly stamps only on the schedule and forward only, 110 per window, claimed instances protected
      from omission and the cap; cooldowns only later and at most 48 h ahead (season ids: the window end), never
      deleted; progress clamped at 100 000 000. Each limit refuses one entry, never the save (`state.rs`,
      `cooldowns.rs`; fuzz `rustic_isles_economy_bounties`, `rustic_isles_economy_cooldowns`).
- [ ] Bounded input: 512 bounties and 256 cooldowns a save, ids 200 UTF-16 units, dates 64; 1000 stored bounties, 1024
      cooldowns.
- [ ] Money path: the price is the server's SKU's, never the request's; a purchase needs a signed token whose `userId`
      is the caller's; prices must be finite numbers at least 0; Platinum is all buckets or nothing; one transaction
      for the debit, the items, the entitlements and the progression; one-time and daily ids are derived so a replay
      never pays twice; the daily claim fails closed (`store.rs`; test `store_routes`, `platinum_buckets`).
- [ ] The fountain offer is fail-closed (stacked, uncapped, not prestige-gated, free, id-safe); an id clash disables
      the section (fuzz `rustic_isles_economy_store_data`).
- [ ] Entitlement names and balance ids are checked against the store's safe-identifier rules before any write.
- [ ] A purchase token never reaches a log or a `Debug`.
- [ ] Known weak spots: a delete clears the daily reward day (§14 item 3); a one-time SKU is one-time per character
      (§14 item 4); a bearer-less game server can buy for the configured account (§14 item 11); a repeatable
      purchase's id is system random (§14 item 5); an instanced store item is capped at 1000 instances per SKU entry
      here (an implementation bound, not in the spec).

## Sign-off

Reviewed by: ________ on ________ at commit ________
