# Clean-Room Provenance Record: `crates/rifts-seam` (RD27)

- **Component**: `crates/rifts-seam` (crate `rifts_seam`, `license = "MIT"`, `Copyright (c) 2026 Silent Spud`)
- **Behaviour specification**: `docs/port/spec-rd27-rifts.md` (umbrella, with its §2 D3 rename table) and §2, §6.1,
  §7 and §9 of `docs/port/spec-rd27-rifts-admission-relay.md`, at dainty-lass `773a30c` (branch
  `docs/rd27-spec-rifts`)
- **Clean-room worktree**: `R:\source\dauntless\dainty-lass\.claude\worktrees\agent-a5c460cab6de4eab9` (branch
  `feat/rd27-rifts-seam`, cut from `origin/docs/rd27-spec-rifts` at `773a30c`)
- **Implementer**: one Claude Code subagent session (model Claude Opus 5.5), 2026-11-02, briefed by the RD27
  coordinator with the rules below

## 1. Isolation

### 1.1 Rules the implementer worked under

It never read, listed, grepped or `git show`ed anything under `dainty-restricted` (any worktree of it),
`modules/rustic-rifts`, `modules/rustic-isles`, Mystic Paradox's repository or libjuice's sources, and ran no
`git log -p` or `git show` on any commit. It searched nothing on the web. The session's environment listed a
dainty-restricted worktree as an additional working directory; no tool call read from it. Its only inputs were the
two specs, `AGENTS.md`, the workspace `Cargo.toml`, `rust-toolchain.toml`, `crates/protocol/Cargo.toml` (for the
manifest's shape) and this folder's `chat.md` and `protocol.md` (for this record's format).

### 1.2 Worktree

A full checkout of dainty-lass at `773a30c`. dainty-lass holds no rifts source at that revision: `crates/rifts-seam`
did not exist until the implementer created it.

## 2. Files read

- `docs/port/spec-rd27-rifts.md` (whole)
- `docs/port/spec-rd27-rifts-admission-relay.md` §1-§2 (whole), §5-§9 (whole); a grep of the other part specs for
  `rifts_seam::` to confirm every item they name is in §2.1
- `docs/port/clean-room/chat.md` (headings and §1-§2), `docs/port/clean-room/protocol.md`
- `AGENTS.md`, `Cargo.toml`, `rust-toolchain.toml`, `crates/protocol/Cargo.toml`
- The brief (`impl-brief.md` in the coordinator's scratchpad)

## 3. Files written

| File | What |
| --- | --- |
| `crates/rifts-seam/Cargo.toml` | Manifest, `license = "MIT"`, only `thiserror` |
| `crates/rifts-seam/src/lib.rs` | Re-exports (modules private), `stub`, `poll_once` |
| `crates/rifts-seam/src/error.rs` | `Component`, `Unimplemented`, `Refusal`, `SeamError` (§2.1.1) |
| `crates/rifts-seam/src/core.rs` | `Party`, `GuardMode`, `HostGrantCatalog`, `IslesCore`, `NoCore` (§2.1.3) |
| `crates/rifts-seam/src/hooks.rs` | `Settings`, `Lifecycle`, the hook data types, `RealmsHooks`, `RealmsEntry`, `GuardHooks` (§2.1.4) |
| `crates/rifts-seam/src/http.rs` | Routes as data, `Reply::not_implemented`, `RequestInfo::header`, `StubRoutes` (§2.1.5) |
| `crates/rifts-seam/src/id.rs` | `IdError`, `Ulid`, `NodeId`, `GlobalId`, the five node-tagged id newtypes, `AccountId`, `PeerId` (§2.1.6) |
| `crates/rifts-seam/src/store.rs` | `UnixMillis`, `Secret`, `Actor`, `TableSpec`, deletion steps, `StoreError` (§2.1.7) |
| `crates/rifts-seam/src/dedicated.rs` | `p2p_module_enabled`, `DedicatedOnly`, `RiftsParts`, `Rifts` (§2.1.8, §2.3) |
| `Cargo.toml` | Seam change: `rifts-seam = { path = "crates/rifts-seam" }` in `[workspace.dependencies]` |
| `Cargo.lock` | The new package |
| `docs/port/clean-room/rifts-seam.md` | This record |

Every `.rs` file starts with `// SPDX-License-Identifier: MIT` and `// Copyright (c) 2026 Silent Spud`.

## 4. Verification

- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean.
- `cargo test -p rifts-seam --no-fail-fast`: 24 passed, 0 failed (unit tests in each module; §6.1's seam list in
  full, plus the error texts, the escaper, `StubRoutes`, `stub`, `poll_once`, `Rifts`' route order and every
  `DedicatedOnly` answer of §2.3.2).

`cargo tree -e normal -p rifts-seam`:

```text
rifts-seam v0.0.0 (crates\rifts-seam)
└── thiserror v2.0.21
    └── thiserror-impl v2.0.21 (proc-macro)
        ├── proc-macro2 v1.0.107
        │   └── unicode-ident v1.0.27
        ├── quote v1.0.47
        │   └── proc-macro2 v1.0.107 (*)
        └── syn v3.0.9
            ├── proc-macro2 v1.0.107 (*)
            ├── quote v1.0.47 (*)
            └── unicode-ident v1.0.27
```

No AGPL crate.

## 5. Ambiguities and the assumptions taken

1. **D3 renames over the part spec.** `HOST_CAPABILITY_HEADER` is `x-rustic-host-capability` (the part spec's
   `x-mystic-host-capability`), and `p2p_module_enabled` reads `RUSTIC_RIFTS_P2P_MODULE` (the part spec's
   `MYSTICPARADOX_P2P_MODULE`), as the umbrella's D3 table orders. The §6.1 header test uses the new name.
2. **The setting's name is a private constant.** §2.1 lists no public constant for it, so none was added.
3. **`Ulid::from_parts`** masks `unix_millis` to 48 bits and `random` to 80 bits before placing them, so out-of-range
   bits are dropped rather than overlapping.
4. **`Ulid` FromStr** checks the first raw byte against `0`-`7` before uppercasing (digits have no case), then every
   byte uppercased against the alphabet. Lower-case `i`, `l`, `o`, `u` uppercase to refused letters, so they are
   refused too.
5. **`GlobalId`'s derived order** compares `ulid` first, then `node` (field order as listed in §2.1.6).
6. **`Reply::not_implemented`'s escaper** writes only characters below U+0020 as escapes; U+007F and above are
   written as is, as §2.1.5 says "everything else ... as is".
7. **`Json::default()`** is the empty string (a derived `Default` on a `String` newtype); only `empty_object()` is
   `{}`.
8. **`NoCore`** keeps the trait defaults for `host_grant_catalog`, `stacked_items` and `accepted_inventory_replay`, so
   their operation names are `stacked_items` and `accepted_inventory_replay` as the defaults give.
9. **`StubRoutes::handle`** answers through `stub`, so the future is ready on the first poll like every other seam
   stub.
10. **The `R1`/`R2` bodies** are built from one template with a fixed message; the messages contain nothing that
    needs escaping.

## 6. Similarity

Not measured here; the reviewer outside the clean room measures each file against restricted's `rifts-seam` at
`4efc8b5` (umbrella §5 item 5).

## 7. Integration

Seam changes made on this branch, for the integration pass to review:

- `[workspace.dependencies]`: `rifts-seam = { path = "crates/rifts-seam" }`.
- The workspace `members = ["crates/*"]` glob takes the folder with no edit.

Left for the integration pass: the migration guide's §2 line for the crate, the NOTICE line, and dainty-restricted's
switch of its `rifts-seam` dependency to this repository by git rev.

## 8. Restructure pass

2026-11-02, one Claude Code subagent session (model Claude Opus 5.5) under the isolation of section 1, on branch
`chore/rd27-rifts-restructure-seam-guard-host` cut from `origin/feat/rd27-rifts-mit`. No AGPL tree was read; the
only comparison run was of the crate against its own previous revision on this branch, as a proxy.

- `error.rs`, `store.rs`, `id.rs`: `Display`, `Error` and `From` written by hand instead of `thiserror`
  (same texts; `SeamError` and `StoreError` still show and source as the value they hold). `thiserror` is no
  longer a dependency of the crate (`Cargo.toml`, `Cargo.lock`).
- `id.rs`: the ULID decoder reads a 128-entry digit table built in a `const`; `from_parts` masks by shifting;
  every id's `Display` comes from one `shown_as!` macro, which `global_newtype!` reuses.
- `core.rs`: `NoCore`'s refusing methods generated by one `not_written!` macro from a list of names and types
  (operation names are the `stringify!`ed method names, as before); `GuardMode::from_setting` as one `match`.
- `dedicated.rs`: one `now` helper for ready answers; `Rifts` keeps its parts behind one `Arc<RiftsParts>`
  (private fields only); unused parameters unnamed.
- `http.rs`: `Method::as_str` reads a name table; the JSON escaper maps the short escapes through one table.
- `hooks.rs`: the two `Settings` impls from one macro; declarations grouped by component, trait methods by role.
- `lib.rs`: one nested re-export list.
- Every test module rewritten table-driven, every case kept.

Public API and behaviour: no public item's name, type, fields, field order, variants or signature changed; trait
methods are listed in a different order, which neither implementors nor callers see, and the crate still compiles
every dependent crate in the workspace. `cargo test -p rifts-seam --locked --no-fail-fast`: 24 passed, as before.

What cannot go lower: `hooks.rs`, `store.rs` and most of `http.rs` and `core.rs` are the public structs, enums
and trait signatures of spec §2.1, used by dainty-restricted; their fields, variants and signatures are the
contract, so their share stays dominated by declarations.

