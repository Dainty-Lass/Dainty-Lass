# Clean-Room Provenance Record: `crates/rifts-relay` (RD27)

- **Component**: `crates/rifts-relay` (crate `rifts_relay`, `license = "MIT"`, `Copyright (c) 2026 Silent Spud`), and
  the fuzz target `fuzz/fuzz_targets/relay_signal.rs`
- **Behaviour specification**: `docs/port/spec-rd27-rifts.md` (umbrella, with its §2 D3 rename table) and §4, §5,
  §6.1, §6.2, §6.3, §7 and §9 of `docs/port/spec-rd27-rifts-admission-relay.md` (with §3.7's shared route rules), at
  dainty-lass `773a30c`, on top of `rifts-seam` at `4f9c09e`
- **Clean-room worktree**: `R:\source\dauntless\dainty-lass\.claude\worktrees\agent-a6d311f69ffd77981` (branch
  `feat/rd27-rifts-relay`, cut from `origin/feat/rd27-rifts-seam` at `4f9c09e`)
- **Implementer**: one Claude Code subagent session (model Claude Opus 5.5), 2026-11-02, briefed by the RD27
  coordinator with the rules below

## 1. Isolation

### 1.1 Rules the implementer worked under

It never read, listed, grepped or `git show`ed anything under `dainty-restricted` (any worktree of it),
`modules/rustic-rifts`, `modules/rustic-isles`, Mystic Paradox's repository or libjuice's sources, and ran no
`git log -p` or `git show` on any commit. It searched nothing on the web. The session's environment listed a
dainty-restricted worktree as an additional working directory; no tool call read from it. Outside the worktree it read
only the brief and, in the local cargo registry, the public API of three dependencies (below).

### 1.2 Worktree

A full checkout of dainty-lass at `4f9c09e`. dainty-lass holds no relay source at that revision: `crates/rifts-relay`
did not exist until the implementer created it.

## 2. Files read

- The brief (`impl-brief.md` in the coordinator's scratchpad)
- `docs/port/spec-rd27-rifts.md` (whole)
- `docs/port/spec-rd27-rifts-admission-relay.md` §1, §2, §3.7, §4-§9 (whole)
- `docs/port/clean-room/rifts-seam.md` and `chat.md` (§1-§3, for this record's format)
- `AGENTS.md`, `Cargo.toml`, `Cargo.lock` (which `getrandom` and `rand` it holds), `rustfmt.toml`, `fuzz/Cargo.toml`
- `crates/rifts-seam/Cargo.toml` and `src/lib.rs`; the `pub` signatures of `src/{error,store,http,id,hooks}.rs`
- From other MIT crates, only how they call their crypto crates: grep lines of `rand::`, `Hmac`, `base64::` in
  `crates/`, the `preserve_order` lines of the manifests, and `fuzz/fuzz_targets/{remote_wire,network_ticket}.rs`
  (head, for a target's shape)
- Cargo registry, public API only: `rand-0.10.3/src/rngs/mod.rs` (that `rand::rngs::SysError` is
  `getrandom::Error`), `rand-0.10.3/Cargo.toml` (features), `getrandom-0.4.3/src/{lib,sys_rng}.rs` (signatures),
  `hmac-0.13.0/src/lib.rs` (exports)

## 3. Files written

| File | Spec |
| --- | --- |
| `crates/rifts-relay/Cargo.toml` | §7.1, with `rand` for `getrandom` (§5.1 below) |
| `crates/rifts-relay/src/lib.rs` | §4.1, §4.1.1: `RelayError`, `RelayCredential`, the three traits, `TransportKeys`, `Stub`, re-exports |
| `crates/rifts-relay/src/wire.rs` | §4.2 |
| `crates/rifts-relay/src/config.rs` | §4.3 |
| `crates/rifts-relay/src/credentials.rs` | §4.4 |
| `crates/rifts-relay/src/rules.rs` | §4.5 |
| `crates/rifts-relay/src/service.rs` | §4.6 |
| `crates/rifts-relay/src/service/tests.rs` | §6.1 (service) |
| `crates/rifts-relay/src/routes.rs` | §4.7, §3.7's shared rules |
| `crates/rifts-relay/examples/mint-relay-file.rs` | §4.8 |
| `fuzz/fuzz_targets/relay_signal.rs` | §6.3, rewritten from the spec's statement of what it asserts |
| `Cargo.toml` | Seam change: `rifts-relay = { path = "crates/rifts-relay" }` in `[workspace.dependencies]` |
| `Cargo.lock` | The new package; `hmac 0.13.0` and its `ctutils`/`cmov` enter the lock (the workspace pinned `hmac` but no crate used it yet) |
| `fuzz/Cargo.toml`, `fuzz/Cargo.lock` | Seam change: the `rifts-relay` dependency and the `relay_signal` `[[bin]]` |
| `docs/port/clean-room/rifts-relay.md` | This record |

Every `.rs` file starts with `// SPDX-License-Identifier: MIT` and `// Copyright (c) 2026 Silent Spud`. No source file names
Mystic, Paradox or a Node source file.

## 4. Verification

- `cargo fmt --all --check`: clean; `rustfmt --edition 2024 --check fuzz/fuzz_targets/relay_signal.rs`: clean.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean (run after the lock update).
- `cargo test -p rifts-relay --locked --no-fail-fast`: **53 passed, 0 failed** (unit tests in `wire`, `config`,
  `credentials`, `rules`, `routes`, the crate root and `service/tests.rs`; §6.1's relay list in full, the route table
  as a literal list).
- `cargo +nightly fuzz run relay_signal -- -max_total_time=60`: clean, 1,131,276 runs (Windows, with the ASan runtime
  on `PATH`).
- `mint-relay-file` with no arguments prints the usage line and exits 2. Not run against a real turn-rs relay (§9.15
  of the spec: hands-on).

`cargo tree -e normal -p rifts-relay`:

```text
rifts-relay v0.0.0 (crates\rifts-relay)
├── base64 v0.23.1
├── hmac v0.13.0
│   └── digest v0.11.3
│       ├── block-buffer v0.12.1
│       │   └── hybrid-array v0.4.15
│       │       └── typenum v1.20.1
│       ├── const-oid v0.10.2
│       ├── crypto-common v0.2.2
│       │   └── hybrid-array v0.4.15 (*)
│       └── ctutils v0.4.8
│           └── cmov v0.5.4
├── rand v0.10.3
│   ├── chacha20 v0.10.2
│   │   ├── cfg-if v1.0.5
│   │   ├── cpufeatures v0.3.1
│   │   └── rand_core v0.10.1
│   ├── getrandom v0.4.3
│   │   ├── cfg-if v1.0.5
│   │   └── rand_core v0.10.1
│   └── rand_core v0.10.1
├── rifts-seam v0.0.0 (crates\rifts-seam)
│   └── thiserror v2.0.21
│       └── thiserror-impl v2.0.21 (proc-macro)
│           ├── proc-macro2 v1.0.107
│           │   └── unicode-ident v1.0.27
│           ├── quote v1.0.47
│           │   └── proc-macro2 v1.0.107 (*)
│           └── syn v3.0.9
│               ├── proc-macro2 v1.0.107 (*)
│               ├── quote v1.0.47 (*)
│               └── unicode-ident v1.0.27
├── serde_json v1.0.154
│   ├── itoa v1.0.18
│   ├── memchr v2.8.3
│   ├── serde_core v1.0.229
│   └── zmij v1.0.23
├── sha1 v0.11.0
│   ├── cfg-if v1.0.5
│   ├── cpufeatures v0.3.1
│   └── digest v0.11.3 (*)
├── sha2 v0.11.1
│   ├── cfg-if v1.0.5
│   ├── cpufeatures v0.3.1
│   └── digest v0.11.3 (*)
└── thiserror v2.0.21 (*)
```

No AGPL crate. `ctutils` and `cmov` are RustCrypto crates (MIT OR Apache-2.0), new to the lock; `cargo deny` was not
run here.

## 5. Ambiguities and the choices taken

1. **`getrandom` without a new dependency.** The spec lists `getrandom` (§7.1) and gives `random_uuid()` the error
   type `getrandom::Error`. The brief forbids new dependencies beyond its list, and umbrella D5 prefers `rand`. The
   crate depends on `rand` (in the workspace) and returns `rand::rngs::SysError`, which `rand` 0.10 re-exports from
   `getrandom` 0.4: the same type, so the signature is unchanged for callers. The bytes come from
   `rand::rngs::SysRng`, which is getrandom's system source.
2. **D3 labels.** `relay_account_tag` hashes `rustic-relay-tag-v1`; `derive_pair_key` hashes `rustic-ice-pair-v1`.
   Each label is a private constant (the spec names no public one).
3. **D3 handler ids.** The umbrella replaces route handler ids that name Node files with neutral ids but lists none
   for the relay. Chosen: `relay.update_coop_transport`, `relay.create_ice_transport_config`,
   `relay.submit_ice_signal`. They appear in 501 answers and in `stub_routes()`'s operation names; the route test
   pins them.
4. **Self-test account** of `mint-relay-file` is `rustic-selftest` (§4.8 leaves it free). A read error on the env file
   (unspecified) prints `<env file>: <error>` and exits 2, like the other refusals.
5. **`JsonObject`'s builder methods** take and return `self` (`JsonObject::new().string(..).finish()`); the spec says
   "chained" without the receiver. Keys are escaped with `json_string`. The inner `String` is private; `Default` stays
   derived and yields the brace-less builder (§9.14).
6. **`RelayUsageCache::get`/`set`** take `&AccountId`, `&str` for the month, `f64` bytes and `UnixMillis`;
   `HourlyCounter::allow` takes `&str`. The spec gives the parameters without types. Both hold their maps behind a
   `Mutex`; a poisoned lock is recovered rather than unwrapped.
7. **`refusal(code, message)`** takes two `&str`.
8. **`js_number` radix literals** accumulate digits in `f64`, so a literal beyond 2^53 rounds per digit rather than
   once. `Number("0x")` and a bad digit are NaN.
9. **`iso_time`** writes years with at least four digits and does not switch to JavaScript's `+YYYYYY` form outside
   0-9999 (the spec's range).
10. **The coop-transport reason cut** at 500 UTF-16 units never splits a character: a surrogate pair that would
    straddle the limit is dropped whole (JavaScript would keep half of it). `expectedVersion` must be a key of the
    body; a non-object body fails validation.
11. **Generation mismatch** compares the hunt's `u64` and the request's `i64` exactly, so a negative request is fenced.
12. **`ProviderHttp` posts** in `mint_cloudflare` use the key id as given; the config guarantees it is alphanumeric.
13. **`summary_json`'s `iceRelayQuotaGb`** is written with Rust's `f64` `Display` (`10`, `0.5`), which matches
    `JSON.stringify` for every value `from_settings` can produce.
14. **`TurnRelayPorts`** has no behaviour in the spec beyond its `Default`; it is a plain data type.
15. **`serde_json`'s `preserve_order`** is not turned on: the crate writes every JSON text with `JsonObject`, so the
    key order of a parsed `Value` never reaches the wire.
16. **Fuzz `serde_json`**: the fuzz manifest's own `serde_json = "1"` line was left alone; `float_roundtrip` (§7.2.3)
    reaches the target through `rifts-relay`'s workspace dependency.

## 6. The D oracle under D3

`rifts-relay/tests/node_vectors.rs` (dainty-restricted, §6.2) was recorded from Node with the old labels. Run unchanged
against this crate, the vectors that hash a label will differ by design; the rest must match:

| Vector input | Under D3 |
| --- | --- |
| `relay_account_tag` | **differs** (label `rustic-relay-tag-v1`) |
| `mint_turn_rest` `username` and `credential.expose()` | **differ** (the username carries the tag, the credential is its HMAC); `urls` match |
| `derive_pair_key` | **differs** (label `rustic-ice-pair-v1`) |
| `read_monthly_relay_egress_bytes` request `.body` | **differs** in the `customIdentifier` tag only; `.url` and the rest of the query match |
| `normalize_provider_ice_servers` | matches |
| `RelayConfig::from_settings` fields and `summary_json()` | matches |
| `rules::parse_ice_signal`, `filter_relay_only` | matches |

The reviewer either re-records the four label-dependent vectors with the Rustic labels or checks them against a
re-labelled Node run; the umbrella §5 item 4 expects the oracles to pass unchanged, which D3 makes impossible for
these.

## 7. Similarity

Not measured here; the reviewer outside the clean room measures each file against restricted's `rifts-relay` at
`4efc8b5` (umbrella §5 item 5).

## 8. Integration

Seam changes made on this branch, for the integration pass to review:

- `[workspace.dependencies]`: `rifts-relay = { path = "crates/rifts-relay" }`. The `members` glob takes the folder.
- `fuzz/Cargo.toml`: the `rifts-relay` path dependency and the `relay_signal` `[[bin]]`; `fuzz/Cargo.lock` updated.
- `Cargo.lock`: `hmac` 0.13.0, `ctutils` 0.4.8, `cmov` 0.5.4 newly resolved (already allowed by the `hmac` pin).

Left for the integration pass or a later row: the security checklist `docs/security/rifts-relay.md` (umbrella §4.5;
not in this row's brief), the migration guide's §2 line, the NOTICE line, and dainty-restricted's switch of its
`rifts-relay` dependency by git rev with the oracle package of §6.2.

## Restructure pass

- **Date and session**: 2026-11-02, one Claude Code subagent session (model Claude Opus 5.5), on branch
  `chore/rd27-rifts-restructure-admission-relay` cut from `origin/feat/rd27-rifts-mit` at `2e06bda`, after the
  admission pass. Same isolation rules as §1: nothing under dainty-restricted, `modules/` or any pre-split history was
  opened; the outside reviewer's per-file shares were the only input about the replaced code.
- **Files changed**: `src/lib.rs`, `src/service.rs`, `src/rules.rs`, `src/credentials.rs`, `src/config.rs`,
  `src/wire.rs`, `src/routes.rs` (its production half; the tests, with the route table as a literal list, are as
  they were), `examples/mint-relay-file.rs`; in `src/service/tests.rs` only the `BoxFuture` import the old
  `use super::*` used to bring in.
- **Techniques**:
  - One `wire_enum!` table (in `lib.rs`, ahead of the modules) declares `CoopTransport`, `PresenceState`, `IceRole`,
    `SignalKind` and `RelayDenied` with their wire words and generates `as_str` and, where the API has one, `parse`.
  - Errors: `RelayError`, `ProviderFailure` and `ProviderUnreachable` keep their variants and texts, with hand-written
    `Display`/`Error` and a `From` macro instead of `thiserror` attributes.
  - Service: every refusal a named `(code, text)` constant checked through one `ensure`; the gate, the live-hunt
    checks and the hub presence folded into one lookup returning the hunt with its presence; the admin body read
    into a small struct; the trait impls through the crate's private `Answer` alias.
  - Rules: role, joiner and peer lookups through shared `seated`/`live_rows` iterators; routing as a match on the
    pair of roles; the relay-only filter as one loop with named line predicates; the pair key and TURN REST
    credential through `Mac::chain_update`.
  - Credentials: the quota check split out of `grant`; Cloudflare's tagged and untagged requests from one closure;
    the hourly counter on the map's entry; tests table-driven with a `granted` helper.
  - Config: the four bounded numbers as one table of (setting, fallback, range) shared by `Default` and
    `from_settings`; a small settings reader for trimmed text, URL lists, ids with tokens and secrets.
  - Wire: escapes from a table, the object builder through one `field` method, the civil date by whole 400/100/4/1
    year spans instead of the shifted-era formula (checked equal to the old one for every day from about year -220
    to 10,000 by a temporary test, then removed), the decimal-literal check by splitting rather than a cursor.
  - Routes: the table through a `const fn route`, the launcher's three checks in one `launcher_call`.
  - Derive lists sorted alphabetically.
- **Unchanged**: every public item's name, type, fields and field order, variants and signatures (parameter names in
  trait declarations changed, which is not part of the API); wire bytes, JSON field order, every error code and text,
  the provider request bodies and the analytics query text, the example's output and exit codes. Dependents
  (`rifts-admission`, `rifts-realms`, `rifts-transport`, `rifts-host`) build and pass their tests.
- **Tests**: `cargo test -p rifts-relay --locked --no-fail-fast`: 53 passed, 0 failed (every case kept).
  `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean.
- **What cannot go much lower**: `rules.rs` (the `HuntState`, `RosterEntry`, `PresenceRow`, `HuntSnapshot`, `IcePeer`
  and `IceSignal` declarations and the pinned function signatures), `credentials.rs` (eleven public structs and
  enums plus the provider functions' signatures and the analytics query text) and `config.rs` (`RelayConfig`'s
  eleven fields, the provider structs and `SECRET_SETTINGS`) are bounded by what the spec pins. `thiserror` stays
  in `Cargo.toml` (now unused here) to leave `Cargo.lock` untouched.
