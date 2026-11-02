# Clean-Room Provenance Record: `crates/rifts-admission` (RD27)

- **Component**: `crates/rifts-admission` (crate `rifts_admission`, `license = "MIT"`, `Copyright (c) 2026 Silent
  Spud`), its two examples, and the fuzz targets `fuzz/fuzz_targets/admission_control.rs` and `admission_ticket.rs`
- **Behaviour specification**: `docs/port/spec-rd27-rifts.md` (umbrella, with its §2 D3 rename table) and §3, §5,
  §6.1-§6.3, §7 and §9 of `docs/port/spec-rd27-rifts-admission-relay.md` (with §4.2, §4.4.7 and §4.5 for the relay
  items admission calls), at dainty-lass `773a30c`, on top of `rifts-seam` (`4f9c09e`) and `rifts-relay` (`9d9eb02`)
- **Clean-room worktree**: `R:\source\dauntless\dainty-lass\.claude\worktrees\agent-a5a153cc989e8878e` (branch
  `feat/rd27-rifts-admission`, cut from `origin/feat/rd27-rifts-relay` at `9d9eb02`)
- **Implementer**: one Claude Code subagent session (model Claude Opus 5.5), 2026-11-02, briefed by the RD27
  coordinator with the rules below

## 1. Isolation

### 1.1 Rules the implementer worked under

It never read, listed, grepped or `git show`ed anything under `dainty-restricted` (any worktree of it),
`modules/rustic-rifts`, `modules/rustic-isles`, Mystic Paradox's repository or libjuice's sources, and ran no
`git log -p` or `git show` on any commit. It searched nothing on the web. The session's environment listed a
dainty-restricted worktree as an additional working directory; no tool call read from it. Outside the worktree it read
only the brief and, in the local cargo registry, the public API of its crypto dependencies (below).

### 1.2 Worktree

A full checkout of dainty-lass at `9d9eb02`. dainty-lass holds no admission source at that revision:
`crates/rifts-admission` did not exist until the implementer created it.

## 2. Files read

- The brief (`impl-brief.md` in the coordinator's scratchpad)
- `docs/port/spec-rd27-rifts.md` (whole)
- `docs/port/spec-rd27-rifts-admission-relay.md` §1, §3, §4 (whole, for the relay API admission calls), §5-§9
- `docs/port/clean-room/rifts-relay.md` (whole) and `chat.md` (§1-§2), for this record's format
- `AGENTS.md`, `Cargo.toml`, `Cargo.lock` (which `getrandom`, `rand_core` it holds), `rustfmt.toml`, `fuzz/Cargo.toml`
- `crates/rifts-seam`: `Cargo.toml`, `src/lib.rs`, `src/error.rs`, `src/store.rs`, `src/http.rs`, `src/id.rs`, and the
  `Settings` trait in `src/hooks.rs`
- `crates/rifts-relay`: `Cargo.toml`, `src/lib.rs`, `src/routes.rs` (its shared route rules), the `pub` lines of
  `src/{wire,rules,credentials,service}.rs` and the bodies of `JsonObject`, `random_uuid`, `HourlyCounter`, the rules
  types and `P2PGate`; `fuzz/fuzz_targets/relay_signal.rs` (a target's shape)
- Cargo registry, public API only: `p256-0.14.0` (`Cargo.toml` features, `src/lib.rs` exports), `ecdsa-0.17.0`
  (`signing.rs`, `verifying.rs` and `lib.rs` signatures; that P-256 does not normalise S), `elliptic-curve-0.14.3`
  (`lib.rs` exports, `secret_key.rs` SEC1 PEM signatures, `Cargo.toml` features, `NonZeroScalar`'s `Neg`),
  `pkcs8-0.11.0` and `spki-0.8.1` (exports, PEM trait signatures), `crypto-common-0.2.2` (`Generate`),
  `base64-0.23.1` (engine config), `rand-0.10.3` (exports)

## 3. Files written

| File | Spec |
| --- | --- |
| `crates/rifts-admission/Cargo.toml` | §7.1, with `rand` for `getrandom` (§5.1 below) and `tempfile` as a dev-dependency |
| `crates/rifts-admission/src/lib.rs` | §3.1.1: `AdmissionError`, `KeyId`, `PublicXy`, `GeneratedKeyPair`, the three traits, `derive_ice_peer_id`, `Stub`, re-exports |
| `crates/rifts-admission/src/ticket.rs` | §3.1.2, §3.2 |
| `crates/rifts-admission/src/ticket/tests.rs` | §6.1 (tickets) |
| `crates/rifts-admission/src/keys.rs` | §3.1.3, §3.3, with its tests |
| `crates/rifts-admission/src/control.rs` | §3.1.4, §3.4, with its tests |
| `crates/rifts-admission/src/store.rs` | §3.5, with its tests |
| `crates/rifts-admission/src/service.rs` | §3.6 |
| `crates/rifts-admission/src/service/tests.rs` | §6.1 (service) |
| `crates/rifts-admission/src/routes.rs` | §3.7, with its tests (the route table as a literal list) |
| `crates/rifts-admission/examples/generate-admission-key.rs` | §3.8 |
| `crates/rifts-admission/examples/mint-ticket.rs` | §3.8 |
| `fuzz/fuzz_targets/admission_control.rs`, `admission_ticket.rs` | §6.3, rewritten from the spec's statement of what each asserts |
| `Cargo.toml` | Seam changes: `rifts-admission` and `p256` in `[workspace.dependencies]` |
| `Cargo.lock` | `p256` 0.14.0 and its RustCrypto tree (20 packages, below) |
| `fuzz/Cargo.toml`, `fuzz/Cargo.lock` | Seam change: the `rifts-admission` dependency and two `[[bin]]` entries |
| `docs/port/clean-room/rifts-admission.md` | This record |

Every `.rs` file starts with `// SPDX-License-Identifier: MIT` and `// Copyright (c) 2026 Silent Spud`. No source file
names Mystic, Paradox or a Node source file.

## 4. Verification

- `cargo fmt --all --check`: clean; `rustfmt --edition 2024 --check` on the two fuzz targets: clean.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean (run after the lock update).
- `cargo test -p rifts-admission -p rifts-relay -p rifts-seam --locked --no-fail-fast`: **rifts-admission 51 passed**,
  rifts-relay 53 passed, rifts-seam 24 passed, 0 failed. Admission's are the unit tests in `keys`, `control`, `store`,
  `routes`, the crate root, `ticket/tests.rs` and `service/tests.rs`: §6.1's admission list in full, the route table
  as a literal list, and a high-S signature that verifies.
- `cargo +nightly fuzz run admission_control -- -max_total_time=60`: clean, 7,941,244 runs;
  `admission_ticket`: clean, 2,985,102 runs (Windows, with the ASan runtime on `PATH`).
- Examples, by hand: `generate-admission-key` with no arguments prints its usage and exits 2; with a new folder it
  writes the four files, prints the three lines and exits 0; again on the same folder it prints the `Exists` text and
  exits 1. `mint-ticket` on that key set's env file writes an `rpt1.` ticket and prints its jti digest and expiry
  (exit 0); without `--out` it prints its usage (exit 1); `--build x` prints `--build is not a number` (exit 1).
- Nothing here was run against the transport. Its acceptance of Rust-minted tickets is the maintainer's (spec §8.7).

`cargo tree -e normal -p rifts-admission`:

```text
rifts-admission v0.0.0 (crates\rifts-admission)
├── base64 v0.23.1
├── p256 v0.14.0
│   ├── ecdsa v0.17.0
│   │   ├── der v0.8.3
│   │   │   ├── const-oid v0.10.2
│   │   │   ├── pem-rfc7468 v1.0.0
│   │   │   │   └── base64ct v1.8.3
│   │   │   └── zeroize v1.9.1
│   │   ├── digest v0.11.3
│   │   │   ├── block-buffer v0.12.1
│   │   │   │   └── hybrid-array v0.4.15
│   │   │   │       ├── subtle v2.6.1
│   │   │   │       ├── typenum v1.20.1
│   │   │   │       └── zeroize v1.9.1
│   │   │   ├── const-oid v0.10.2
│   │   │   ├── crypto-common v0.2.2
│   │   │   │   ├── getrandom v0.4.3
│   │   │   │   │   ├── cfg-if v1.0.5
│   │   │   │   │   └── rand_core v0.10.1
│   │   │   │   ├── hybrid-array v0.4.15 (*)
│   │   │   │   └── rand_core v0.10.1
│   │   │   └── ctutils v0.4.8
│   │   │       ├── cmov v0.5.4
│   │   │       └── subtle v2.6.1
│   │   ├── elliptic-curve v0.14.3
│   │   │   ├── base16ct v1.0.0
│   │   │   ├── crypto-bigint v0.7.10
│   │   │   │   ├── cpubits v0.1.1
│   │   │   │   ├── ctutils v0.4.8 (*)
│   │   │   │   ├── getrandom v0.4.3 (*)
│   │   │   │   ├── hybrid-array v0.4.15 (*)
│   │   │   │   ├── num-traits v0.2.19
│   │   │   │   ├── rand_core v0.10.1
│   │   │   │   ├── subtle v2.6.1
│   │   │   │   └── zeroize v1.9.1
│   │   │   ├── crypto-common v0.2.2 (*)
│   │   │   ├── digest v0.11.3 (*)
│   │   │   ├── ff v0.14.0
│   │   │   │   ├── rand_core v0.10.1
│   │   │   │   └── subtle v2.6.1
│   │   │   ├── group v0.14.0
│   │   │   │   ├── ff v0.14.0 (*)
│   │   │   │   ├── rand_core v0.10.1
│   │   │   │   └── subtle v2.6.1
│   │   │   ├── hybrid-array v0.4.15 (*)
│   │   │   ├── pem-rfc7468 v1.0.0 (*)
│   │   │   ├── pkcs8 v0.11.0
│   │   │   │   ├── der v0.8.3 (*)
│   │   │   │   └── spki v0.8.1
│   │   │   │       └── der v0.8.3 (*)
│   │   │   ├── rand_core v0.10.1
│   │   │   ├── sec1 v0.8.1
│   │   │   │   ├── base16ct v1.0.0
│   │   │   │   ├── ctutils v0.4.8 (*)
│   │   │   │   ├── der v0.8.3 (*)
│   │   │   │   ├── hybrid-array v0.4.15 (*)
│   │   │   │   ├── subtle v2.6.1
│   │   │   │   └── zeroize v1.9.1
│   │   │   ├── subtle v2.6.1
│   │   │   └── zeroize v1.9.1
│   │   ├── rfc6979 v0.6.0
│   │   │   ├── crypto-bigint v0.7.10 (*)
│   │   │   └── hmac v0.13.0
│   │   │       └── digest v0.11.3 (*)
│   │   ├── signature v3.0.0
│   │   │   ├── digest v0.11.3 (*)
│   │   │   └── rand_core v0.10.1
│   │   ├── spki v0.8.1 (*)
│   │   └── zeroize v1.9.1
│   ├── elliptic-curve v0.14.3 (*)
│   ├── primefield v0.14.0
│   │   ├── crypto-bigint v0.7.10 (*)
│   │   ├── crypto-common v0.2.2 (*)
│   │   ├── ff v0.14.0 (*)
│   │   ├── rand_core v0.10.1
│   │   ├── subtle v2.6.1
│   │   └── zeroize v1.9.1
│   ├── primeorder v0.14.0
│   │   ├── elliptic-curve v0.14.3 (*)
│   │   ├── primefield v0.14.0 (*)
│   │   └── wnaf v0.14.1
│   │       ├── ff v0.14.0 (*)
│   │       ├── group v0.14.0 (*)
│   │       ├── hybrid-array v0.4.15 (*)
│   │       └── primefield v0.14.0 (*)
│   └── sha2 v0.11.1
│       ├── cfg-if v1.0.5
│       ├── cpufeatures v0.3.1
│       └── digest v0.11.3 (*)
├── rand v0.10.3
│   ├── chacha20 v0.10.2
│   │   ├── cfg-if v1.0.5
│   │   ├── cpufeatures v0.3.1
│   │   └── rand_core v0.10.1
│   ├── getrandom v0.4.3 (*)
│   └── rand_core v0.10.1
├── rifts-relay v0.0.0 (crates\rifts-relay)
│   ├── base64 v0.23.1
│   ├── hmac v0.13.0 (*)
│   ├── rand v0.10.3 (*)
│   ├── rifts-seam v0.0.0 (crates\rifts-seam)
│   │   └── thiserror v2.0.21
│   │       └── thiserror-impl v2.0.21 (proc-macro)
│   │           ├── proc-macro2 v1.0.107
│   │           │   └── unicode-ident v1.0.27
│   │           ├── quote v1.0.47
│   │           │   └── proc-macro2 v1.0.107 (*)
│   │           └── syn v3.0.9
│   │               ├── proc-macro2 v1.0.107 (*)
│   │               ├── quote v1.0.47 (*)
│   │               └── unicode-ident v1.0.27
│   ├── serde_json v1.0.154
│   │   ├── itoa v1.0.18
│   │   ├── memchr v2.8.3
│   │   ├── serde_core v1.0.229
│   │   └── zmij v1.0.23
│   ├── sha1 v0.11.0
│   │   ├── cfg-if v1.0.5
│   │   ├── cpufeatures v0.3.1
│   │   └── digest v0.11.3 (*)
│   ├── sha2 v0.11.1 (*)
│   └── thiserror v2.0.21 (*)
├── rifts-seam v0.0.0 (crates\rifts-seam) (*)
├── serde_json v1.0.154 (*)
├── sha2 v0.11.1 (*)
└── thiserror v2.0.21 (*)
```

No AGPL crate. Every crate `p256` brings in is RustCrypto's (MIT OR Apache-2.0), already allowed; `cargo deny` was not
run here.

## 5. Ambiguities and the choices taken

1. **`getrandom` without a new dependency.** As in `rifts-relay`: the crate draws OS randomness from
   `rand::rngs::SysRng` (getrandom's system source) for jtis, bridge instance ids and new keys
   (`SigningKey::try_generate_from_rng`). A random failure is `TicketError::Random` or `KeyError::Random`.
2. **D3 renames.** `TOKEN_PREFIX = "rpt1"`, `PAYLOAD_MAGIC = b"RPT2"`, `CONTROL_MAGIC = b"RPC1"`, the ICE peer label
   `rustic-ice-peer-v1`, the capability header `x-rustic-host-capability` (the seam's `HOST_CAPABILITY_HEADER`) and the
   host routes under `/rustic/v1/hunts/...`. The settings `P2P_ADMISSION_*`, the key-set file names and the error texts
   carry no Mystic name and are kept as the spec gives them; the `Steam` wire names stay (spec §9.2). The test of
   §3.4's example uses `rpt1.abc.def`, so its frame starts `52 50 43 31`.
3. **D3 handler ids.** The umbrella asks for neutral ids without listing admission's. Chosen, as the relay did:
   `admission.issue_self_ticket`, `admission.consume_ticket`, `admission.record_peer_disconnected`,
   `admission.create_join_bootstrap`, `admission.list_hub_peers`. They are `stub_routes()`'s operation names and appear
   in 501 answers; the route test pins them. The root `Stub`'s operation names are §3.1.1's (`generate`, `mint`, ...).
4. **`to_json()` returns `String`** on `IssuedTicket`, `JoinBootstrap`, `ConsumedTicket` and `HubPeers`, like the
   relay's `IceServer::to_json`; the routes wrap it in `Json`. The spec gives no return type.
5. **Signing** uses `p256`'s deterministic RFC 6979 signer (§3.2.6 allows it), through `try_sign`; its error, which
   P-256 cannot produce for a valid key, is reported as `BadSignature`. A PEM encoding failure while generating a key
   (also unreachable) is `KeyError::Io` with the encoder's text.
6. **High-S signatures verify**: `p256`'s verifier does not normalise S for P-256, and `Es256Tickets::verify` passes the
   signature through unchanged. `high_s_signatures_verify` builds an `(r, n - s)` signature and checks both forms.
7. **`IssuedTicket`'s `rendezvous.generation`** is the hunt snapshot's generation (equal to the ticket's).
8. **Roster lookup** in `issue_self_ticket` takes the account's first roster entry and does not look at
   `returned_to_hub`; §3.6.3 does not mention it.
9. **`write_key_set`**: "exists" is any directory entry at `output` (`symlink_metadata`, so a dangling link counts).
   If the temporary folder cannot be created (its name is taken), the error is `Io` and the folder is left alone: it
   is not this call's. An output with no parent at all reports `NoParent` with an empty path.
10. **Lenient base64** (§3.3.2) is a filter in front of a base64 engine that takes no padding and ignores trailing
    bits. A decode failure cannot happen after the filter, so `Unreadable` comes only from the PEM parse.
11. **`mint-ticket`** parses numbers with Rust's `FromStr` for each field's type, so a negative or out-of-range value
    is `--<name> is not a number`; `--now` defaults to the system clock in seconds and `--lifetime` is read only when
    given. **`generate-admission-key`** passes the system clock in Unix milliseconds.
12. **The consume answer's generation** is the ticket's (`u32`); the store's request and the active key use it widened
    to `u64`, which equals the capability's after step 4.
13. **`serde_json`'s `preserve_order`** is not turned on: every JSON text the crate writes is built with
    `rifts_relay::wire::JsonObject`, and the bodies it reads are read by key.
14. **The security checklist** `docs/security/rifts-admission.md` (umbrella §4.5) was not in this row's brief and is
    not written here.

## 6. The D oracle under D3

`rifts-admission/tests/node_vectors.rs` (dainty-restricted, §6.2) calls only items this crate has, with the same
names and signatures (`Es256Tickets::from_settings` on a `HashMap`, `.public_xy().0`, the inherent `.verify(&str,
u64)`, the `TicketClaims` fields, `ticket::{decode_claims, encode_claims, hash_jti}`, `TicketError`'s texts and
`NoncanonicalPayload`, `derive_ice_peer_id`). Its data was recorded from Node with the old names, so run unchanged:

| Vector input | Under D3 |
| --- | --- |
| The 5 issued tickets | **differ**: their text starts `mpt1.` and their payload `MPT2`, so they no longer parse (`MalformedToken`); re-encoding their claims gives the same bytes except the magic (`RPT2`) |
| The 349 verdicts | **differ** wherever the token or payload is otherwise well formed (a `mpt1` prefix is now `MalformedToken`, an `MPT2` payload `MalformedPayload`); verdicts decided before the prefix is read (oversize, wrong part count, empty parts) match. Every error text is unchanged |
| The 3 jti digests (`hash_jti`) | match (no label) |
| The 4 ICE peer ids | **differ** (label `rustic-ice-peer-v1`) |
| `from_settings` and `public_xy` | match |

The reviewer re-records the vectors with the Rustic names, or rewrites the four label- and magic-dependent inputs in
the harness, before running it as §5 item 4 expects; D3 makes an unchanged pass impossible.

## 7. Similarity

Not measured here; the reviewer outside the clean room measures each file against restricted's `rifts-admission` at
`4efc8b5` (umbrella §5 item 5).

## 8. Integration

Seam changes made on this branch, for the integration pass to review:

- `[workspace.dependencies]`: `rifts-admission = { path = "crates/rifts-admission" }` and
  `p256 = { version = "0.14", features = ["ecdsa", "pem", "pkcs8", "std"] }` (default features kept, as the brief
  gives it; spec §7.2 writes `default-features = false` with the same four features, which differs only by
  `precomputed-tables`). The `members` glob takes the folder.
- `Cargo.lock`: `p256` 0.14.0 and `base16ct`, `base64ct`, `cpubits`, `crypto-bigint` 0.7.10, `der` 0.8.3, `ecdsa` 0.17.0,
  `elliptic-curve` 0.14.3, `ff`, `group`, `pem-rfc7468`, `pkcs8` 0.11.0, `primefield`, `primeorder`, `rfc6979`,
  `sec1` 0.8.1, `serdect`, `signature` 3.0.0, `spki` 0.8.1, `wnaf`.
- `fuzz/Cargo.toml`: the `rifts-admission` path dependency and the `admission_control` and `admission_ticket`
  `[[bin]]` entries; `fuzz/Cargo.lock` updated.

Left for the integration pass or a later row: the security checklist, the migration guide's §2 line, the NOTICE line,
and dainty-restricted's switch of its `rifts-admission` dependency by git rev with the oracle package of §6.2.
