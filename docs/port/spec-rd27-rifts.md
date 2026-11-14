<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# RD27 Clean-Room Specification: Rustic Rifts (`modules/rustic-rifts` in dainty-restricted)

> Since R21 (`spec-runtime-r21.md`) the AGPL runtime is gone; `native/runtime/p2p/PlayerHost.cpp` is deleted, and the
> in-game player host is the MIT runtime's R10 (`spec-runtime-r10.md`); no runtime patch or rebuild is involved.

This is the umbrella of the specification for relicensing Rustic Rifts, the realm-hosting and co-op transport module,
to MIT. Rifts is in dainty-restricted at `modules/rustic-rifts` under AGPL-3.0-only with Mystic Paradox's
additional terms. It holds six Rust crates and the C++ `ParadoxTransport.dll`. The maintainer chose to rewrite the
whole module (2026-11-02): spec first, then clean room. This file classifies the module, settles the decisions the
parts share, gives the target MIT design and the acceptance criteria, and indexes the part specs:

| Part | Spec | Source in dainty-restricted |
| --- | --- | --- |
| Transport (the DLL) | [`spec-rd27-rifts-transport.md`](spec-rd27-rifts-transport.md) | `src/`, `src/test/`, build scripts, `tools/` |
| Seam, admission, relay | [`spec-rd27-rifts-admission-relay.md`](spec-rd27-rifts-admission-relay.md) | `crates/rustic-rifts-seam`, `rustic-rifts-admission`, `rustic-rifts-relay` |
| Realms: rules and hosting | [`spec-rd27-rifts-realms.md`](spec-rd27-rifts-realms.md) | `crates/rustic-rifts-realms` (domain modules) |
| Realms: store, routes, service | [`spec-rd27-rifts-realms-store.md`](spec-rd27-rifts-realms-store.md) | `crates/rustic-rifts-realms` (persistence and wiring) |
| Guard | [`spec-rd27-rifts-guard.md`](spec-rd27-rifts-guard.md) | `crates/rustic-rifts-guard` |
| Host (launcher side) | [`spec-rd27-rifts-host.md`](spec-rd27-rifts-host.md) | `crates/rustic-rifts-host`, the module's top-level files |

Source: dainty-restricted `4efc8b5` (`plan`); dainty-lass `49197a8`. Measurement follows
[`clean-room/similarity.md`](clean-room/similarity.md).

> **Clean-room rule.** An implementer reads this file, the part spec for its crate, the other part specs where they
> name an API it calls, and dainty-lass's MIT code. Nothing else: not `modules/rustic-rifts`,
> `modules/rustic-isles` or any other tree of dainty-restricted, not Mystic Paradox's repository, not libjuice's
> sources, and not any commit of either repository from before the split that held them. Its worktree is a sparse
> checkout of dainty-lass with no dainty-restricted checkout reachable (§6). Nothing written from these documents
> names Mystic Paradox's files, TypeScript identifiers or commits in comments.

---

## 1. Provenance and classification

### 1.1 Evidence

- **NOTICE.** Rifts' `NOTICE.md` says the repository root is Mystic Paradox's `ParadoxTransport/` folder with its
  history (to `c1c5dca`, 2026-10-17), and that the six crates "port Mystic Paradox's `p2p/`, `realms/`, Guard and
  launcher realm-hosting modules (`21b9ff5`)". It also says every port and every change made here stays AGPL with
  the additional terms.
- **Headers.** Every C++ file carries Mystic Paradox's copyright header. Almost every Rust file's `//!` names the
  TypeScript module it ports (`p2p/security/admissionTickets.ts`, `realms/realmsService.ts`, ...) or the C++ file
  whose bytes it matches.
- **History.** The crates were written in Dainty Lass's rows RF1–RF7 (2026-10-26 to 2026-11-01), each as a port.
- **Callers.** `isles-core` and `isles-1-14-7` (rustic-isles, AGPL) take `rustic-rifts-seam` always and the four server
  crates behind `isles-1-14-7`'s `realms` feature. The launcher (`launcher/src-tauri`, feature `realms`) takes
  `rustic-rifts-host` and `rustic-rifts-seam`.

### 1.2 Categories

- **B**: derived from Mystic Paradox, by copy (the C++) or by port (the Rust). Rewritten clean room from the part
  specs. This is every source file, test, example, migration and fuzz target. The few files with no Paradox source
  of their own (for example `rustic-rifts-guard/src/digest.rs` or `rustic-rifts-seam/src/id.rs`) are rewritten too. They are small,
  and moving them would leave the reviewer to argue each one's provenance.
- **D**: data recorded from Mystic Paradox's programs: `rustic-rifts-admission/tests/vectors/node-admission.json`,
  `rustic-rifts-relay/tests/vectors/node-relay.json` and their generator `make-vectors.mts`. These stay in dainty-restricted
  as oracles. The vector tests (`tests/node_vectors.rs` in both crates) stay with them and run against the MIT
  crates by git dependency (§5).
- **K**: kept in dainty-restricted and not carried: `LICENSE`, `ADDITIONAL_TERMS.md`, `NOTICE.md`, `README.md`,
  `docs/inventory.*` (RF1's inventory of the Node module, prose derived from it), the `docs/security/*.md`
  checklists (rewritten fresh for the MIT crates, §4.5), the MSBuild projects, `build.ps1`,
  `prepare-dependencies.ps1`, `run-relay-selftest.ps1`, `tools/mint-relay-test-file.cjs`,
  `.forgejo/workflows/transport.yml` and the fuzz workspace's lock file. Once restricted switches, the C++ files and
  their scripts are deleted with the crates.

- **A**: written here for Dainty Lass with no Paradox counterpart, moved with MIT headers. The part specs class as A
  only the crate manifests (with `license = "MIT"`), the five fuzz targets (RF4; Mystic Paradox has no fuzzing, and
  the targets call only the public API), Guard's two migration sets and `tests/sql_store.rs` (DD15 and DF7 shapes on
  `store`). The realms store spec classes its migrations as B but gives the DDL exactly; either way the schema comes
  out byte for byte.

The part specs give each file's category and line count. Where a part spec classes a security checklist as A, this
umbrella overrides it: the checklists are rewritten fresh (§4.5), because restricted's `host.md` is wrong in places
(spec-rd27-rifts-host.md) and the others describe derived code.

| Part | A | B | D | Kept/retired |
| --- | ---: | ---: | ---: | ---: |
| Transport | 0 | 28 | 0 | 8 (projects, scripts, workflow, `tools/`) |
| Seam, admission, relay | 8 | 28 | 5 | 0 |
| Realms (both halves) | 0 | 47 | 0 | 0 |
| Guard | 6 | 20 | 0 | 0 |
| Host and top-level files | 3 | 6 | 2 (`docs/inventory.*`) | 4 (licence and notice files) |

### 1.3 Size

| Part | Files | Code lines (approx.) |
| --- | ---: | ---: |
| `ParadoxTransport.dll` (`src/`, with its self-test) | 30 | 8,700 |
| `rustic-rifts-seam` | 8 | 1,500 |
| `rustic-rifts-admission` | 13 | 4,100 (+ 5,900 lines of vectors, D) |
| `rustic-rifts-relay` | 11 | 3,800 (+ 600 lines of vectors, D) |
| `rustic-rifts-realms` | 46 | 27,700 |
| `rustic-rifts-guard` | 21 | 10,200 |
| `rustic-rifts-host` | 6 | 3,900 |
| `fuzz/` targets | 5 | 260 |
| **Total** | | **≈ 60,000** |

---

## 2. Decisions shared by the parts

**Settled by the maintainer on 2026-11-02:** D1 (in-process transport), D2 (`rtc-ice` + `rtc-turn` after the
spike) and D4 accepted as recommended. D3 was decided differently from the recommendation: the names are renamed to
Rustic ones.

### D1. The transport is a Rust library linked into the launcher, not a DLL (recommended)

`ParadoxTransport.dll` is not loaded into the game. The launcher loads it: `rustic-rifts-host`'s `ffi` module checks
the file's SHA-256 against the signed Guard manifest's `TRANSPORT_MODULE` entry, opens it with `libloading`,
resolves its 29 ABI 3 exports and wraps them in the `TransportAbi` trait. The game sees only a UDP socket on
127.0.0.1 (the bridge). The C ABI exists because the transport was C++ and shipped and signed apart from the
launcher.

Once the transport is Rust, the launcher can call it directly:

- **Recommended:** a library crate `rustic-rifts-transport` (MIT, in dainty-lass's workspace) that implements
  `rifts_host::transport::TransportAbi` in process: the same handles, lines, signals, roster commands and results as
  ABI 3, as Rust calls. It needs no `unsafe`, no `cdylib` and no exemption. `rustic-rifts-host` keeps the `TransportAbi`
  trait and gains an in-process loader next to the FFI one. The behaviour the launcher sees is ABI 3's, byte for
  byte in every line (spec-rd27-rifts-transport.md).
  - Cost: the transport now updates with the launcher, not through the runtime feed's `TRANSPORT_MODULE` role. The
    Guard manifest entry and the SHA-256 check go for the in-process path. That is a change to how the launcher is
    released, so it needs a decision row.
  - The MIT `rustic-rifts-host` then has no FFI loader and no `unsafe` at all. D3 rules out talking to the old DLL, so
    restricted drops its loader at the cutover too.
  - The transport part spec (its §6) was written for a `cdylib` that keeps the C ABI. Under this option its §2.1
    export table becomes the `TransportAbi` methods' contract, and everything else in it holds as written.
- **If the separate channel must stay:** a second crate, `rifts-transport-abi` (`cdylib`), exports the C ABI over
  `rustic-rifts-transport`. It is the only code that needs `unsafe`, either inside the workspace with a
  crate-level `#![allow(unsafe_code)]` as `plugin-host` has, or outside it as `native/runtime-loader` was. In the
  workspace is better: it gets the workspace's lints, CI and dependency pins. `rustic-rifts-host`'s FFI loader also needs
  `unsafe`; it already carries a module-level allowance in restricted, and the MIT rewrite needs the same exemption
  (one module, `ffi`).
- The first option is the one accepted, so the second is kept only as a record.

### D2. The ICE agent

The transport needs ICE with STUN and a TURN client over UDP: relay-only mode is the default for privacy, and
the backend strips every non-relay candidate when it is on. The candidates, chosen by the transport spec writer
after comparing them (spec-rd27-rifts-transport.md, "Rust ICE crate options"), are:

| Crate | Model | TURN client over UDP | Licence | Verdict |
| --- | --- | --- | --- | --- |
| `rtc-ice` + `rtc-turn` (webrtc-rs's sans-IO stack) | sans-IO | yes, a sans-IO TURN client meant to feed relay candidates to the agent | MIT OR Apache-2.0 | **Recommended** |
| `webrtc-ice` (+ `turn`) | async, tokio, callbacks | yes, the most proven Rust path | MIT OR Apache-2.0 | Fallback |
| `is` (str0m's agent alone) | sans-IO | none: we would write allocation refresh, permissions, channel binding and Send/Data ourselves | MIT OR Apache-2.0 | Too much to write |
| `librice` / `rice-proto` | sans-IO core | "initial" only; maturity and licence unverified | unverified | Not yet |

`rtc-ice` fits the transport's model: one session thread owns every socket and timer and calls into the agent, with
no lock held across a library call (the libjuice deadlock of 2026-10-15 cannot recur). Before an implementer starts,
the maintainer runs a **spike**: relay-only connect between two `rtc-ice` agents through a local TURN server on
127.0.0.2 and through Cloudflare's relay. If `rtc-ice`'s relay path or
its control of the ICE role falls short, `webrtc-ice` is the fallback. A TURN server for the tests (a
dev-dependency, for example `turn` from webrtc-rs) is a second seam entry.

The chosen crate becomes a `[workspace.dependencies]` entry (a seam change for the integration pass). Whatever is
chosen, the signalling text the transport writes stays the same: the `DESCRIPTION`, `CANDIDATE` and
`GATHERING_DONE` signals carry SDP attribute lines (`a=ice-ufrag`, `a=ice-pwd`, `a=candidate:`), which the
backend's relay-only filter reads.

### D3. Names on the wire: renamed to Rustic (maintainer decision 2026-11-02)

**Decided:** every identifier that carries Mystic Paradox's name is renamed to a Rustic one. The new code carries
none of the old names, not even as aliases. Where a part spec gives an old identifier as a contract, this table
overrides it, and the implementer writes the new one.

| Old (AGPL tree) | New (MIT crates) | Where |
| --- | --- | --- |
| `MysticTransport_*` (29 C exports), `ParadoxTransport.dll` | none: D1 removes the C ABI and the file | launcher ↔ transport |
| user agent `ParadoxTransport/1` | `RusticTransport/1` | transport → backend |
| `x-mystic-host-capability` / `X-Mystic-Host-Capability` | `x-rustic-host-capability` | hosted runtime and transport → backend |
| `X-MysticParadox-Request-Id` | `X-Rustic-Request-Id` | transport → backend |
| `/mystic/v1/hunts/:sessionId/...` | `/rustic/v1/hunts/:sessionId/...` | Guard's routes |
| ticket text prefix `mpt1.`, payload magic `MPT2` | `rpt1.`, `RPT2` | admission tickets |
| `MPC1` | `RPC1` | admission control frames |
| `MX` | `RX` | sealed frame header |
| `MGE1`, pipe prefix `MysticHuntGuard.events.` | `RGE1`, `RusticHuntGuard.events.` | game server → transport |
| hash labels `mysticparadox-<x>-v1` | `rustic-<x>-v1` | pair-key KDF and tags |
| `mysticparadox-ice-peer-v1
` | `rustic-ice-peer-v1
` | ICE peer ids |
| `paradox-realm-hub:v1:` | `rustic-realm-hub:v1:` | hub room ids |
| `MYSTICPARADOX_<NAME>` settings (host, Guard, `_P2P_MODULE`, `_REALMS_MODULE`) | `RUSTIC_RIFTS_<NAME>` | env files written by setup |
| route handler ids naming Node files (`antiCheat.ts:...`) | neutral ids (`guard.event_batches`) | 501 `Unimplemented` text |

The magic values keep their lengths and digits (`M`→`R`, `MX`→`RX`), so every byte layout in the part specs holds
with only the constant changed. Rust identifiers stay neutral (`HostCapability`, `TICKET_PREFIX`). No public Rust
item of the AGPL crates carries the old names, so the public API of each part spec is unchanged.

**What this costs, accepted with the decision:**

- **No mixed-version interop.** A C++ `ParadoxTransport.dll` peer and an MIT peer cannot admit each other, and the
  AGPL server and the MIT crates disagree on headers, routes and derived ids. Server, launcher and the hosted
  runtime cut over together. §5 item 7 becomes an end-to-end hosted hunt on MIT parts only.
- **Derived values change at the cutover.** Pair keys, ICE peer ids and hub room ids all hash the label, so
  everything derived from them is new. Hubs restart, tickets and pair keys in flight die, and any stored copy of a
  hub room id or peer id must be dropped or recomputed (the realms store spec names where they are kept). Guard's
  evidence chains of hunts running at the cutover end. Cut over with no hosted hunt running.
- **Restricted changes in the same cutover** (integration, not this row): `isles-core` (`areas/auth/middleware.rs`,
  `http.rs`, `routes/table.rs`, `areas/reforged_api/observe.rs` and their tests), the game server runtime's
  `native/runtime/p2p/PlayerHost.cpp` (the header, the pipe name and `MGE1`; a runtime patch, so a hands-on rebuild),
  and the launcher's `controller/realms.rs` and its tests.
- **Settings:** `RUSTIC_RIFTS_` must be a server prefix in hostkit's list (AGENTS.md "Configuration"), and setup
  moves each `MYSTICPARADOX_*` value to its new name once, as DX4 did for `RUSTIC_ISLES_*`
  (`hostkit::hostconfig::RENAMED_SECRET_KEYS`). That is a hostkit change, so a seam change reported for the
  integration pass. The MIT crates read only the new names.
- The D2 spike and risk 1 lose their C++-peer half: there is no libjuice peer to meet.

### D4. Crate names and placement in dainty-lass

Keep the package names, so restricted switches by changing only its `[workspace.dependencies]` paths to git
dependencies. Every crate goes in `crates/<package>`, MIT, `Copyright (c) 2026 Silent Spud`, with
`// SPDX-License-Identifier: MIT` in each source file:

| Package | Folder | Kind | Depends on (MIT) |
| --- | --- | --- | --- |
| `rustic-rifts-seam` | `crates/rustic-rifts-seam` | lib | — |
| `rustic-rifts-relay` | `crates/rustic-rifts-relay` | lib | `rustic-rifts-seam` |
| `rustic-rifts-admission` | `crates/rustic-rifts-admission` | lib | `rustic-rifts-seam`, `rustic-rifts-relay` |
| `rustic-rifts-guard` | `crates/rustic-rifts-guard` | lib | `rustic-rifts-seam`, `store` |
| `rustic-rifts-realms` | `crates/rustic-rifts-realms` | lib | `rustic-rifts-seam`, `-admission`, `-relay`, `-guard`, `store`, `module-api` |
| `rustic-rifts-transport` | `crates/rustic-rifts-transport` | lib (new) | `rustic-rifts-seam`, `rustic-rifts-admission` (ticket and control codecs) |
| `rustic-rifts-host` | `crates/rustic-rifts-host` | lib | `rustic-rifts-seam`, optionally `rustic-rifts-transport` (feature `in-process`) |
| `rifts-transport-abi` | `crates/rifts-transport-abi` | cdylib | only if D1's second option is taken |

`rustic-rifts-transport` reuses `rustic-rifts-admission`'s ticket verifier and MPC1 codec instead of having its own, as the C++
had to. The two sides of one format then share one implementation.

### D5. Crypto without new dependencies where possible

| Need | Today | MIT crate uses |
| --- | --- | --- |
| AES-256-GCM (pair frames) | Windows BCrypt (C++) | `aes-gcm` (in the workspace) |
| ES256 tickets: sign (backend), verify (host) | `p256` (Rust), BCrypt (C++) | `p256` 0.14 (`ecdsa`, `pem`, `pkcs8`, `std`): its types are in `rustic-rifts-admission`'s public API, so both repositories must share one version. **New workspace entry.** The transport may verify with `aws-lc-rs` instead. Both must accept high-S signatures |
| SHA-256, HMAC-SHA1 (turn-rest), HMAC-SHA256 | `sha2`, `sha1`, `hmac` | the same (all in the workspace) |
| Random | `getrandom`, BCryptGenRandom | `rand` or `aws-lc-rs::rand` (in the workspace); `getrandom` would be new |
| Dynamic loading (`rustic-rifts-host` FFI path) | `libloading 0.7` | `libloading` is pinned only in `plugin-host`'s own manifest; a workspace entry is a seam change |
| HTTPS from the transport to the backend | WinHTTP | `hyper` + `hyper-rustls` (in the workspace), on a small tokio runtime of the transport's own |
| Named pipe from the game server (Guard events) | Win32 | `tokio::net::windows::named_pipe` (safe API) |

Each crate's spec says where it differs. Seam changes for the integration pass:

- the ICE crate and a TURN server dev-dependency (D2);
- `p256`;
- the `rustic-rifts-*` entries in `[workspace.dependencies]`;
- `getrandom` only if `rand` cannot stand in;
- `libloading` only if D1 keeps an FFI path in an MIT crate;
- `zeroize` if key wiping is kept;
- `RUSTIC_RIFTS_` in hostkit's server prefixes and the `MYSTICPARADOX_*` moves in setup (D3).

**`serde_json` key order is a contract.** Restricted's workspace turns on `serde_json`'s `preserve_order`, and
dainty-lass's does not. Guard's batch hash covers the body's JSON text in document order, and the control payloads
and replies keep Node's insertion order. Each MIT crate that writes or hashes JSON turns `preserve_order` on in its
own manifest, as `service-kit` does. Guard's tests pin one known body to the hash the AGPL crate gives it.

**On boringtun / WireGuard** (asked 2026-11-02): not recommended. Today the backend derives one AES-256-GCM key per
host–joiner pair and hunt generation and hands it to both peers. WireGuard needs a static Curve25519 key pair per
peer instead, and boringtun only tunnels IP packets, so every game datagram and control frame would need a fake IP
header. The `MX` frame already has per-direction nonces, a random counter start and a 1024-frame replay window, and
its keys last one hunt generation. If forward secrecy is wanted later, a Noise handshake (`snow`) keyed with the
pair key as a PSK would give it without the IP constraint. That is a protocol change for its own row, not part of
this rewrite.

---

## 3. What does not change

- Every route, status code, JSON field and field order the launcher, the hosted runtime, the transport or the
  backend sees (plan section 6), except D3's renames. The part specs give them exactly.
- The two migration sets' schemas and names, so a server that has run the AGPL crates opens its database with the
  MIT ones (spec-rd27-rifts-realms-store.md, spec-rd27-rifts-guard.md).
- The ticket, control-frame, sealed-frame and fragment byte layouts and the signal texts, with only D3's magic values
  changed.
- The public Rust API that `isles-core`, `isles-1-14-7` and the launcher call, by name and signature (each part spec's
  public-API section). Restricted must switch by changing dependencies only.
- `bridge.conf` and `transport.secret` formats, and the ready, peer-event and diagnostic lines.

---

## 4. Target MIT design

### 4.1 Principle

All of rifts becomes MIT crates in dainty-lass (D4). dainty-restricted keeps no rifts code, only the D oracles
and the callers (rustic-isles, the launcher's realm adapter), which are AGPL for their own reasons.

### 4.2 Order

1. `rustic-rifts-seam` (every other crate and isles-core depend on it).
2. `rustic-rifts-relay`, then `rustic-rifts-admission` (vectors: admission needs relay's pair keys).
3. `rustic-rifts-guard`.
4. `rustic-rifts-realms` (both halves, one implementer or two working one after the other: the store half first).
5. `rustic-rifts-transport`, then `rustic-rifts-host` (its tests drive the transport in process).
6. The fuzz targets, each with the crate it covers.

Steps 2–3 and 5 can run in parallel once step 1 has landed.

### 4.3 What dainty-restricted does afterwards (integration, not this row)

- Delete `modules/rustic-rifts/crates/*`, `src/`, the build scripts and the transport workflow. Point
  `[workspace.dependencies]`' six `rustic-rifts-*` entries at dainty-lass by git rev.
- Keep `tests/vectors/` and the two `node_vectors.rs` tests in a small test-only crate (for example
  `modules/rustic-rifts/oracle`) that runs them against the MIT crates.
- Launcher: build `rustic-rifts-host` with `in-process`, drop `ParadoxTransport.dll` from the runtime feed and the Guard
  manifest's `TRANSPORT_MODULE` role, and delete the FFI loader.
- The D3 renames in isles-core, the runtime's `PlayerHost.cpp` and the launcher's realm adapter, in the same cutover.
- NOTICE: rifts moves to "Clean-room MIT rewrites" in dainty-lass's NOTICE, with a truthful line that it replaces a
  port of Mystic Paradox's co-op transport and realm modules. Restricted's NOTICE keeps the history and the
  additional terms for the deleted files and the D oracles. libjuice's MPL-2.0 notice goes when the C++ does.
- `agpl-inventory.md` and the migration guide's §2 rows for rifts.

### 4.4 The fuzz targets

The five targets (`admission_control`, `admission_ticket`, `relay_signal`, `guard_evidence`, `guard_grants`) are
rewritten from the part specs into dainty-lass's `fuzz/` (MIT), each with a `[[bin]]` entry.

### 4.5 Security checklists

RX3 requires a `docs/security/<component>.md` with each trust-boundary component. The MIT crates get fresh
checklists (`rifts-admission.md`, `rifts-relay.md`, `rifts-guard.md`, `rifts-host.md`, `rifts-transport.md`),
written from the part specs' threat notes in `relay.md`'s structure, not copied from restricted's.

---

## 5. Acceptance

1. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` and
   `cargo test --workspace --locked --no-fail-fast` pass in dainty-lass. CI's "Every package is MIT" check passes,
   and `cargo deny check` passes with the ICE crate added.
2. No `unsafe` outside the module or crate D1 exempts.
3. Each part spec's acceptance list passes; in particular the transport's rewritten self-test scenarios (crypto,
   parse, session, hub, hub relay-local, backend, ice, ice relay-local) run as `cargo test` on Windows and Linux,
   with a TURN server started in the test.
4. A reviewer outside the clean room runs the D oracles (`node_vectors.rs` for admission and relay) against the MIT
   crates unchanged, and they pass.
5. A reviewer outside the clean room measures every rewritten file against dainty-restricted `4efc8b5`'s rifts
   tree (the Rust against the crate it replaces, `rustic-rifts-transport` against `src/*.cpp`): under 30% per file by
   `similarity.md`'s measure, with every file at or above 25% explained (wire constants, DDL, public signatures).
6. Restricted builds and passes its suites (isles-core, isles-1-14-7, the launcher's `controller/tests/realms.rs`)
   with only its dependency lines changed.
7. End to end on MIT parts only (D3 removes mixed-version interop): a host and a joiner on the Rust transport, with
   restricted's server switched to the MIT crates and the runtime rebuilt with the renamed header and pipe, admit
   and carry datagrams through a relay. Hands-on, left to the maintainer.
8. Nothing is played against the real 1.14.7 client by any of the above. A real hosted hunt through a relay is the
   maintainer's canary.

---

## 6. The clean-room procedure

As `clean-room/chat.md` and `hostkit.md` record:

1. One implementer per crate (or per §4.2 step), each in its own dainty-lass worktree whose sparse checkout holds
   `crates/` (MIT only), `docs/port/spec-rd27-rifts*.md`, `docs/port/clean-room/`, `AGENTS.md`, `Cargo.toml`, `fuzz/`.
   There is no dainty-restricted checkout on the machine path it is given.
2. It writes the crate from its part spec, with the crate's tests and fuzz targets, and records in
   `docs/port/clean-room/<crate>.md` the session, the files it read and wrote, and its test, clippy and
   `cargo tree -e normal -p <crate>` output.
3. The workspace `members` glob takes the new folders; adding `rustic-rifts-*` to `[workspace.dependencies]` and the ICE
   crate are seam changes the implementer reports rather than makes on `plan`.
4. The reviewer (outside the clean room) measures similarity (§5.5) and adds the files to `similarity.md`.

---

## 7. Risks and ambiguities

Each part spec ends with its own list. These are the ones that cross parts or need the maintainer.

1. **The cutover is all at once** (D3). Server, launcher and hosted runtime switch together, with no hosted hunt
   running; derived ids and in-flight evidence do not survive it. libjuice interop is no longer a goal.
2. **JSON key order** (§2 D5). Without `preserve_order`, Guard's chain hash changes and hunts that run across the
   upgrade lose their evidence. Replies would also change order.
3. **Scheduled work is not wired at this revision.** Nothing outside tests calls realms' `sweep` or
   `sweep_lapsed_hosts`, the store's purge methods, or Guard's census flush. Leases, offers and deadlines never
   advance in production, and expired rows pile up. The rewrite keeps the API as it is. Wiring a scheduler in isles
   is a separate fix in restricted.
4. **Behaviour that looks like a bug, kept as found.** All of these are observable, so the rewrite keeps each one and
   lists it for a decision row:
   - emergency stop and the co-op-transport switch only echo the policy back;
   - Guard's rollout changes live in memory only;
   - typed `observe` always sends generation 1;
   - every non-hub hunt gets the same placeholder manifest;
   - five realms codes and `P2P_CAPABILITY_REQUIRED` fall to HTTP 500 in isles-core's status table;
   - the pipe accepts event types that the batch check refuses;
   - an idempotent consume re-runs the admit step outside a transaction;
   - `bridge.conf` and `transport.secret` are deleted when the session stops, not at readiness, and get no
     restricted ACL on Windows;
   - the launcher passes a no-op spawner, so no hosted game server is started.
5. **High-S ECDSA signatures** are accepted today (BCrypt and `p256`). Every MIT verifier must accept them, with a
   test.
6. **Substring matching of backend answers** in the transport (`"renewed":true`) is kept, so a backend reply that
   differs only in whitespace behaves as before.
7. **WinHTTP to rustls** drops the system proxy and the Windows certificate store. `rustls-platform-verifier` (a seam
   entry) restores the store; proxy support is a separate change.
8. **`HuntSession` is built as a struct literal** in an isles test. The MIT struct keeps its fields and their order,
   or restricted's test breaks on the switch.
9. **Route tests read `docs/inventory.json`**, which stays in restricted (D). The MIT tests compare against the
   route tables in the part specs.
10. **The README is stale in places.** It says consumes run on the session thread; since 2026-10-17 they go through
    a lane. Where the README and a part spec differ, the part spec holds.
11. **Mystic Paradox's terms.** The part specs name its wire strings and describe its behaviour. They copy no code,
    comments or prose, and the implementers never open the source, so the additional terms do not reach the MIT
    crates. Truthful attribution of what the rewrite replaces goes in dainty-lass's NOTICE through the integration
    pass, without implying endorsement.
