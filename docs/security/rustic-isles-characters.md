<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: Rustic Isles characters and loadouts (RX3)

The checklist for `crates/rustic-isles-characters`, the MIT characters-and-loadouts area of Rustic Isles (RD27,
`docs/port/spec-rustic-isles-characters.md` §16). It parses untrusted input (character data blobs, loadout slot and
`persistent` data, path params, body fields) behind core's pipeline, so it gets this page and a fuzz target.

How to use it: before a change to this crate merges, and before its cutover is signed (RD10), copy this list into the
change, check each item in the code (not from memory), link the code or test that shows it, and sign at the end with the
date and the commit reviewed.

## What it is and where the boundary is

- **In:** the eight `characters-loadouts` backend rows (`GET`/`PUT`/`POST /character`, `/loadout/...`), reached only
  through core's dispatcher after its budget and origin strip; the body through core's `read_body` (100 KiB).
- **Out:** the store's repositories (`IslesStore::read`, `write`, `account_write`), rifts' guard
  (`authorize_host_write`, `record_host_write`) and log lines.
- **Callers inside the module:** later units through `Characters` in its typed slot (`characters_for_user`,
  `ensure_starter_in`, `loadout_state`, ...).

Code: `crates/rustic-isles-characters/src/` (`routes.rs` the handlers, `controller.rs` the rules over the store,
`data.rs` and `starter.rs` the pure rules).

Fuzz target in `fuzz`: `rustic_isles_characters_data`: arbitrary text as character data, the stored data it is guarded
against, a trigger series, a loadout write with its index, journey nodes and a stored item list, with a synthetic
`CharacterData` and `StarterKit`. `normalize` never panics and, when it succeeds, answers an object that normalises to
itself; `guard_progression` never moves the step back or more than one forward when both texts are objects;
`read_form`, `process_triggers`, `dojo_unlocked` and `is_fresh` never panic; `validate_loadout_write` accepts only an
object text at `persistent` or `0`..`5`; `repair` of any item array keeps only objects and gives every `instanced`
catalog id a distinct id, a valid instance id wherever it chose one.

## Checklist

### Who may act

- [ ] Every route authenticates first: an `AuthData` already in the extensions, else auth's `game_auth`; a request
  with none of `authorization`, `x-rustic-isles-gameserver-apikey`, `x-rustic-isles-gameserver-key` answers the 501
  envelope (parity, spec §15 item 1) and reaches no store (`routes.rs` `authenticate`; test `entry`).
- [ ] A player acts only for its own token's userId: `POST /character` ignores `body.accountId` from a player, and
  the loadout routes ignore the `userId` param unless the caller is a game server (`Ctx::requestor`; tests
  `post_versions`, `reads`).
- [ ] Only a game server names another account (`accountId`, the loadout `userId` param) (test
  `post_from_a_gameserver`).
- [ ] A slot unlock needs a game server or a host runtime; a player or a caller with no authority gets 403
  `forbidden` and the guard is not asked (test `unlock_authority`).
- [ ] A host runtime's unlock goes through rifts' guard (`LoadoutUnlock`) for an account this node knows; an unknown
  target is 403 `P2P_HOST_WRITE_DENIED`; a runtime without the realms seam's `RuntimeAuth` handle is refused as the
  dedicated-only guard refuses; the guard's refusal is answered as it is (test `unlock_by_a_host_runtime`).
- [ ] The host-write tag carries counts only, never the character id (same test).

### Ownership

- [ ] Every character, loadout and inventory read or write is keyed on (character id, account): another account's
  character is "not owned", answered 400 (loadouts) or 409 (`POST /character`), and never read or written (tests
  `post_versions`, `reads`, `active_index`, `slot_and_persistent_writes`, `the_seam_other_units_call`).
- [ ] The in-transaction starter check refuses a character the account does not own before it can create records
  (`ensure_starter_in`; `EnsureError::NotOwned`).
- [ ] Writes go through `account_write`, so the store's home check applies; an account homed elsewhere is the 500
  page (spec §14 item 16).

### Entitlement

- [ ] The unlocked slot count comes from the target's own Slayer's Path rows (`journey_total`), never from the
  path's count, and never shrinks (tests `unlock_follows_the_journey`).

### Bounds

- [ ] Bodies are core's (100 KiB); loadout `data` must be a non-empty string that parses to an object, at
  `persistent` or the canonical `0`..`5` (`data::validate_loadout_write`; tests `loadout_writes`,
  `slot_and_persistent_writes`).
- [ ] Indexes are `Number()` whole values checked into 0..5 and slot totals into 1..6 before any store call
  (`routes.rs` `whole_in`).
- [ ] Revision retries stop after 10 attempts; the store's revision guards make concurrent writes safe.
- [ ] Instance ids are drawn by rejection (bytes at or above 252 skipped), so each of the 36 characters is equally
  likely, from the replay id source (`starter::generate_instance_id`).

### Failure mode

- [ ] Malformed stored or sent data is a data error (500 page) or a refusal, never a panic: no `unwrap`/`expect`
  outside tests, and the fuzz target above.
- [ ] A failure is logged as `{METHOD} {route}: {error} (request {id})` and answered with Express's 500 page.

### Secrets and logs

- [ ] No secret passes through the crate; `Debug` of `CharacterData`, `StarterKit` and `Characters` holds game values
  only. Auth's `RuntimeCaller` shows only whether it carries a handle.
- [ ] Client text in a log line (names, userIds, character ids, indexes) is written with Rust `Debug` escaping, so a
  newline cannot forge a line (spec §11, coordinator decision on item 6); no header, token or key is logged.

### Known weak spots (kept for parity, spec §14 and §16)

- Unlimited characters per account, each with a full manifest: storage grows bounded only by the request budget.
- A game server key can write any account's character and loadouts.
- Character names are not validated and are echoed to every client that reads them.
- Normalisation parses nested JSON strings within the body limit; `serde_json`'s depth limit turns a deep one into a
  data error (500).
- A starter row that was soft-deleted cannot be recreated (its key blocks the insert, store spec §16 item 3): the read
  that would repair it answers the 500 page until the retention purge removes the row.

## Sign-off

Reviewed by: ______ on ______ at commit ______.
