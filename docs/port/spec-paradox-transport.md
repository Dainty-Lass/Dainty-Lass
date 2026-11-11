<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# `ParadoxTransport.dll`: what is left of the port (RD27, port plan item 10)

Written 2026-11-11 against `plan` at `d80cff53`, by lane F1's spec writer, after reading
`restricted/modules/rustic-rifts/` (`src/`, the `.vcxproj` files, the scripts, `tools/`, `docs/`, `oracle/`).

**Finding: the clean-room rewrite of this DLL is already done.** Port plan item 10 assumes an 8.7k-line C++ DLL
still waiting for a spec and an implementer. In fact:

| Step | Where it is |
| --- | --- |
| Spec | [`spec-rd27-rifts-transport.md`](spec-rd27-rifts-transport.md) (1,446 lines), under the umbrella [`spec-rd27-rifts.md`](spec-rd27-rifts.md) with the maintainer's D1–D4 answers (2026-11-02, `9cd52ad5`) |
| MIT implementation | `crates/rustic-rifts-transport` (6.8k lines, `rtc-ice` + `rtc-turn`, no libjuice), linked into the launcher through `rustic-rifts-host`'s default `in-process` feature |
| Clean-room review | [`clean-room/rifts-transport.md`](clean-room/rifts-transport.md), [`clean-room/rifts-host.md`](clean-room/rifts-host.md) |
| AGPL C++ | Still in `restricted/modules/rustic-rifts/src` and built by CI's `transport` job, but nothing ships or loads it |

The C++ has not changed since that spec was written: every commit touching `restricted/modules/rustic-rifts` after
the import (`772248c2`) changed only docs, the oracle crate or the deleted `transport.yml`; the last change to `src/`
is `ae3deaa` (RF4), which the spec's §1.1 already covers. So this page does not respecify the DLL. It answers the
brief's questions by pointing at the existing spec, and specifies the **cutover that was never finished**, which is
the real remaining work of item 10. Calls I made are marked **[call]**, ambiguities **[?]**.

---

## 1. The brief's questions, answered

### 1.1 How the DLL is loaded

**Not by the game, and not through `mystic_loader.ini`.** The brief's premise (RD9, the runtime loader) does not
apply. `ParadoxTransport.dll` was a launcher-side module: `rustic-rifts-host` checked its SHA-256 against the signed
Guard manifest's `TRANSPORT_MODULE` role and opened it with `libloading` in the launcher process (spec §2.1, umbrella
D1). It carries Unreal's UDP between the game's local socket and the peers. The game talks to it only over loopback
UDP (spec §2.16, the bridges). The runtime (`native/runtime`, item 11) handles the in-game side (`p2p/PlayerHost.cpp`).

Under D1 the transport is a Rust library inside the signed launcher. There is no file, no C ABI, no `cdylib`, no
`unsafe`. RD9 only touches realms through the leftover check (`ensure_realms_ready` refuses while a loader ini or an
unknown DLL sits in `Win64`), and that check is already MIT code in `dainty-launcher-kit`.

### 1.2 Hooks

**None.** The DLL installs no hooks: no MinHook, no detours, no patched addresses, no `DllMain` work beyond the CRT's.
The only "hooks" in `src/` are test hooks (a fake admission client, `transport_app.hpp`, `transport_module.hpp`).
Every game hook for co-op is in the runtime and belongs to item 11's spec.

### 1.3 Wire format to the `rustic-rifts-*` crates

Specified field by field in the existing spec: config and secrets files §2.3, lines §2.4, signalling and ICE text
§2.9, the link data path and fragments §2.10, the sealed frame (`MX`, AES-GCM, pair keys) §2.11, `MPC1` control
frames §2.12, `MPT2` tickets §2.13, admission over the link §2.14, the backend's HTTPS calls §3.1, Guard batches
§3.3, the `MGE1` pipe §3.4. Umbrella D3 renamed the branded identifiers (`spec-rd27-rifts.md`, the D3 table), so the
MIT wire is not the C++ wire byte for byte.

The `oracle/` vectors (`node-admission.json`, `node-relay.json`) are **Mystic Paradox's Node backend's** answers
(relay credentials, ICE policy, session rules, ticket signing), not the DLL's. They already run against the MIT
`rustic-rifts-admission` and `rustic-rifts-relay` (`rustic-rifts-oracle`, `spec-rd27-rifts.md` §4.3). No vector in
`oracle/` exercises the DLL's frames; the DLL's behaviour is fixed by the spec's §5 tests, which the MIT crate's
`tests/` implement.

### 1.4 libjuice

MPL-2.0 v1.7.4, fetched by `prepare-dependencies.ps1` from its tag archive with a pinned SHA-256, built as a separate
static library (`libjuice.vcxproj`). The C++ uses it for ICE agents, STUN and the TURN client (spec §2.9 describes the
ICE layer independently of it). **The MIT crate does not use libjuice** (umbrella D2: `rtc-ice` + `rtc-turn`), and
D3 ended libjuice interop on purpose (`clean-room/rifts-transport.md`, its departures list, item 9). libjuice leaves the repository when
the C++ does.

### 1.5 Threads, errors, logging

Spec §2.2 (queues and bounds), §2.5–§2.8 (runs and sessions), §2.17 (result codes), §3.2 (backend lanes), §6.1
(design rule for callback-based ICE crates, from the C++ README's 15 Oct lock-order bug). The MIT crate is sans-IO
over tokio, so the C++ thread layout is not a contract.

### 1.6 The self-test

`ParadoxTransportTest.exe` (`src/test/selftest_main.cpp`, 2,366 lines) with `all --relay-local`, and
`run-relay-selftest.ps1` (relay only, through a real relay, with a minted credentials file from
`tools/mint-relay-test-file.cjs`). Spec §5.1–§5.4 maps each self-test case to a test; the MIT crate has them in
`tests/unit.rs`, `publisher.rs`, `ice_link.rs` (direct, relay-local, bursts; `--relay-file` as an `#[ignore]` test),
`module.rs`, `hub.rs`, `slow_backend.rs`. Its fuzz targets are `fuzz/fuzz_targets/transport_*.rs`.

### 1.7 The build: Rust or C++

Rust, and it is already built that way. A `cdylib` is not needed either (D1). The one reason to keep a C ABI would be
loading a file the launcher did not ship; D1 chose not to.

---

## 2. The cutover still to do

`spec-rd27-rifts.md` §4.3 lists what restricted does after the port. Some of it landed (the AGPL rifts crates are gone,
the oracle crate exists, the launcher builds `in-process`). These parts did not. All are MIT code born in Dainty Lass
or restricted's own files, so none needs a clean-room spec: a normal row can do them.

### C1. The launcher still requires a transport file nobody ships (bug, blocks realms)

`crates/dainty-launcher-kit/src/controller/realms.rs`:

- `ensure_realms_ready` refuses with `transport_missing` unless `Win64\RusticTransport.dll` exists
  (`rustic_rifts_host::TRANSPORT_DLL`), then `expected_transport_sha256` hashes it or reads its digest from the runtime
  manifest, before `NativeTransportLoader::load`, which ignores both.
- `build_realm_view` reports `available: false` without that file.

Nothing produces `RusticTransport.dll` (no workflow, no runtime feed entry, no file in the repository).
`clean-room/rifts-host.md` §6.1 item 5 recorded this: "a file that nothing ships. At the cutover the launcher must stop
pinning a digest for it." On a real install, hosting or joining a realm should fail with `transport_missing` today.
**[?]** Not run against a real install; the tests inject a loader and digest through seams, so they pass.

Change: drop the file check, the digest and `HashMismatch` mapping from both paths; keep the admission key check and
the RD9 leftover check. `TRANSPORT_DLL` is in `rustic-rifts-host`'s §9.1 "used outside the crate" list; remove it from
both crates together, with the `TransportLoader::load` signature losing its path and digest **[call]**. The tests
that set `expected_sha256` in the seams change with it.

### C2. What the host reports as `transportModule`

The realms backend pins Guard roles (`crates/rustic-rifts-realms/src/host_policy.rs`: `TRANSPORT_MODULE`, and
`TRANSPORT_RUNTIME` falling back to it) and compares them with the host presence's `artifacts`. With the transport in
the launcher, the honest value is the launcher's own identity, or nothing. **[?]** Which one is a decision: (a) report
a zero hash and have the realms side stop requiring `TRANSPORT_MODULE` when it is unpublished, or (b) publish the
launcher's digest under that role. I recommend (a): the launcher is already pinned by its own signed update, and a
second pin on the same bytes adds nothing. This touches `rustic-rifts-realms` and the launcher's presence body;
umbrella §4.3 says "drop `ParadoxTransport.dll` ... from the Guard manifest's `TRANSPORT_MODULE` role".

### C3. Stale player text

`restricted/ui/packages/i18n/catalogs/{en,pt-BR}.json` (and the generated `gen/strings.json`) still name
`ParadoxTransport.dll`: `realm_err_transport_missing`, `realm_err_transport_hash_mismatch`, `rlm_transport_ready`,
`rlm_unavailable_hint`. After C1 the first two can no longer happen and the last two should not name a file. Reword in
both languages (DU2) or retire the keys with C1. This is item 6's catalog; coordinate with that lane.

### C4. Retire the C++ tree

Once C1 lands and the hands-on check (§3) passes **[call]**: delete from `restricted/modules/rustic-rifts/` the
`src/`, `ParadoxTransport.vcxproj`, `ParadoxTransportTest.vcxproj`, `libjuice.vcxproj`, `build.ps1`,
`prepare-dependencies.ps1`, `run-relay-selftest.ps1`, `tools/`, `.gitignore`'s build entries, and the transport parts
of `README.md` and `NOTICE.md` (libjuice's MPL notice goes with it). Remove the `transport` job and its `changes`
output from `.forgejo/workflows/ci.yml` (RR3's file; the integration pass). Update `docs/port/agpl-inventory.md`
and the migration guide's §2 rifts rows (integration pass).

**Why it can't stay as an oracle:** spec §5.5 planned a C++ ↔ Rust interop run, but D3 renamed the frame magics,
headers, routes and pipe names, so the two cannot admit each other by design (umbrella, "No mixed-version interop").
Nothing ships the C++ to players, so there is nothing to stay compatible with. Running both against each other would
take a shim that un-renames D3, which is a test of the shim, not of the port. **[call]**

### What stays in `restricted/` to test the rewrite

- `oracle/` (`rustic-rifts-oracle`): the Node vectors and `make-vectors.mts`. They test the MIT admission and relay
  crates and ship nothing. They stay until the Node reference goes at RB15; then they are frozen data and could move
  to the root as recorded goldens. **[?]** Whether recorded Node answers are copyable facts (like the Isles goldens) is
  for the reviewer.
- `docs/security/*.md`: the four AGPL checklists. Their MIT counterparts are `docs/security/rifts-*.md`; once those
  cover them, delete these.
- `ADDITIONAL_TERMS.md`, `LICENSE`, and `NOTICE.md`'s provenance for the deleted files.

---

## 3. The maintainer's hands-on check

Nothing here runs against the real 1.14.7 client in CI. Two PCs (or one PC plus a second Windows account on another
network for the relay case), both with Dauntless 1.14.7, the launcher built from `plan` with C1 applied, and a server
with realms on.

Prepare:

1. On the server: `dainty-host setup` with realms enabled, `dainty-host start`. Confirm the gateway answers and the
   realms service logs that it started.
2. On both PCs: install the launcher build, join the server with its invite, accept and install the server's runtime.
3. On both PCs: confirm there is **no** `ParadoxTransport.dll` or `RusticTransport.dll` in `Dauntless\Win64` (delete
   an old `ParadoxTransport.dll` from an earlier install, since RD9's leftover check may refuse an unknown DLL there).
   Open the launcher's Realms page: it should say the transport is ready (C3's new text), not "not ready".

Direct (hole punching):

1. PC A: host a realm. The launcher log shows the host's ready line (`HOST_READY ... transport=ice`, spec §2.4, with
   D3's names).
2. PC B: join it. The log shows a peer connected and admitted, `route=direct`.
3. Start a hunt from A with B in the party. Both load in, see each other move, and finish or abandon the hunt.
   Both return to Ramsgate.
4. PC B: leave. PC A's log shows the peer lost; A's realm stays up.

Relay only:

1. Make one side relay-only (the realm's route policy, or block UDP between the two networks so ICE must use TURN).
2. Repeat steps 1–4 above. The log shows `route=relay`. The server's relay accounting (realms or Guard admin page)
   shows egress for the session.

Failure paths (quick):

1. Stop the server mid-hunt: both launchers end the session with a lease failure, not a hang.
2. Put a dummy `mystic_loader.ini` next to the loader and try to host: refused with the loader-leftovers error. Remove it.

Record the launcher version, server commit, both routes' results and any log lines that looked wrong. If every step
passes, C4 can go ahead.

---

## 4. Open questions

1. **Item 10's premise.** The port plan's item 10 ("Rewrite it at the root", 8,725 lines) is done. It should become
   "cutover and delete" (C1–C4), roughly 150 lines changed plus deletions. Correct the port plan.
2. **C1 is unverified on a real install.** I read the code path, not a log. The maintainer may already have hit
   `transport_missing`, or may have a local `RusticTransport.dll` placeholder that hides it.
3. **C2's decision** (zero hash and an optional `TRANSPORT_MODULE`, or the launcher's digest).
4. **Spec §5.5 (interop oracle) is void** under D3. The clean-room review of the transport should note that it was
   never run and why; I did not edit that record.
5. **Item 11 crossover.** `p2p/PlayerHost.cpp` in the runtime is the game-side half of co-op. Its renames under D3
   (umbrella §4.3, "the D3 renames in ... the runtime's `PlayerHost.cpp`") belong to item 11's spec; check that the
   runtime as shipped today already speaks the renamed loopback contract, or realms fail in game even after C1.

## 5. Size

| Part | Size |
| --- | --- |
| C1 (launcher-kit, rustic-rifts-host, their tests) | ~100–150 lines changed |
| C2 (realms host policy, presence body) | ~30–60 lines, after the decision |
| C3 (four keys, two languages, regenerate) | ~10 lines |
| C4 | ~8.4k lines deleted, ~40 in CI and docs |
| Hands-on check | one session on two PCs, about an hour |
