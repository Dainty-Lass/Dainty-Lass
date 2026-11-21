<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Security checklist: Rustic Isles 1.14.7, the composed module (RX3)

Covers `crates/rustic-isles-1-14-7` (the version crate and its child executable) and
`crates/rustic-isles-1-14-7-plugin` (the same module as a library). Written from
`docs/port/spec-rustic-isles-1-14-7.md` §13 and §14.

## What it is and where the boundary is

The version crate opens no listener of its own. It decides what core's listeners serve (every area's rows, the rifts
routes) and what the module trusts (the stores, the version data, rifts' answers). Every service binds 127.0.0.1
(core); the gateway is the only listener off loopback and refuses the host-only prefixes core registers. The
composition adds no route and no host-only path of its own. The QoS pong binds loopback on the assigned `qos` port.

The library and the executable run only after the core checked their pinned SHA-256 and the module id
(`docs/security/plugin-host.md`). A construction failure is a failed load (a panic inside module-api's unwind guard),
never a half-built module.

## Checklist

### Secrets

- [x] The admission signing key (configured or generated), the transport key secret, the step-up TOTP secrets, the
  database password and every settings value reach no log and no `Debug`: `AccountsCore`, `RealmsAdapter`,
  `SharedCell`, `VersionComposer` and `VersionData` print names or counts only; core's `Settings` prints names.
- [x] A missing admission signing key is replaced by a fresh P-256 pair (key id `default-k1`) held in memory only; a
  missing `P2P_TRANSPORT_KEY_SECRET` by a fresh random 256-bit secret held as `Secret`. Neither is written anywhere.
- [x] The generated key reaches `Es256Tickets` through a forced settings overlay that lives only for the call.

### The capability header

- [x] Accepted under `x-rustic-host-capability` only (D3, spec §7.2): every reader inside the module is a rifts crate
  or core reading rifts-seam's constant. No alias.
- [x] Its value is never logged (rifts' census tags only its presence).

### Input files

- [x] `version_data.json` and `version_facts.json` are read only from the configured backend game data folder by
  their fixed names, through core's loader (lossy UTF-8, one BOM dropped, `JSON.parse` rules), and only when a
  database is wanted. They merge member by member before the checks below, so a split pair is checked as one document.
- [ ] The host's export (V-4) is data from the host's own game: `weapon_slots_1_14_7.jsonl` can change the starter
  kit's default parts and add them to `instanced`, never anything else; unparsable lines are skipped; the catalog
  check only reports.
- [x] Unknown top-level members are refused, so a typo never falls back silently; every reference the kit makes is
  checked against `instanced`; nine quick chats; finite amounts. A broken file fails the start (§14 item 18).
- [x] The values are identifiers handed to the store as data, never paths or SQL.
- [x] Fuzzed: `fuzz/fuzz_targets/rustic_isles_1_14_7_version_data.rs` (the document and the hook inputs).

### State

- [x] Every piece of state is per module (the late `Shared` cell, matchmaking, hub sessions, Reforged state, caches);
  nothing is a process global. The late cell holds a weak reference, so a stopped module's state is dropped.

### Replay headers

- [x] Core gates them; the composition never turns them on.

### Known weak spots (kept for parity, spec §14)

- An account rifts cannot find, or any account without a store, is eligible (item 12).
- With neither `P2P_ALLOW_ALL_ACCOUNTS` nor `P2P_CANARY_ACCOUNTS` and trust tier 0, every account may host (item 2).
- The capability resolver echoes an internal error's text as the refusal's message (item 9).
- A realms database that fails to open degrades silently to the stubs (item 11).
- In Observe mode, an artifact role with no setting is expected as 64 zeros (item 7).
- Rifts' node is `DAINTY_NODE_ID` (else `local`), not the module's node identity (item 3).
- Admissions are in memory and the relay provider is never reachable (items 4, 6).

## Sign-off

Implementer checklist complete; to be reviewed in the RX3 pass.
