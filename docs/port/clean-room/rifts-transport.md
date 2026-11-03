<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-Room Provenance Record: `crates/rifts-transport` (RD27)

- **Component**: `crates/rifts-transport` (crate `rifts_transport`, `license = "MIT"`, `Copyright (c) 2026 Silent
  Spud`), a new library that replaces the C++ co-op transport in process (umbrella D1), and the fuzz targets
  `fuzz/fuzz_targets/transport_frame.rs`, `transport_files.rs` and `transport_pipe.rs`
- **Behaviour specification**: `docs/port/spec-rd27-rifts.md` (umbrella, with D1, D2 and the D3 rename table) and
  `docs/port/spec-rd27-rifts-transport.md` (whole), at dainty-lass `773a30c`, on top of the MIT `rifts-seam`,
  `rifts-relay`, `rifts-admission` and `rifts-guard` (`origin/feat/rd27-rifts-mit`, `b6ddc44`)
- **Clean-room worktree**: `R:\source\dauntless\dainty-lass\.claude\worktrees\agent-a8d9af813d5180b2e` (branch
  `feat/rd27-rifts-transport`, cut from `origin/feat/rd27-rifts-mit` at `b6ddc44`)
- **Implementer**: one Claude Code subagent session (model Claude Opus 5.5), 2026-11-02, briefed by the RD27
  coordinator

## 1. Isolation

### 1.1 Rules the implementer worked under

It never read, listed, grepped or `git show`ed anything under `dainty-restricted` (any worktree of it),
`modules/rustic-rifts`, `modules/rustic-isles`, Mystic Paradox's repository or libjuice's sources, and ran no
`git log -p` or `git show` on any commit. It searched nothing on the web. The session's environment listed a
dainty-restricted worktree as an additional working directory; no tool call read from it.

One slip, recorded for the reviewer: early on, looking for the cargo registry, it started a `find / -maxdepth 6 -name
rtc-ice-0.21.1` and stopped it after 60 s with no output read. It printed nothing and opened no file; the registry
path was then taken from `cargo metadata`.

### 1.2 Worktree

A full checkout of dainty-lass at `b6ddc44`. No transport source exists in dainty-lass at that revision.

## 2. Files read

- The brief (`impl-brief.md` in the coordinator's scratchpad)
- `docs/port/spec-rd27-rifts.md`, `docs/port/spec-rd27-rifts-transport.md` (whole); `spec-rd27-rifts-host.md` §3 (the
  `TransportAbi` trait the launcher wraps this crate in)
- `docs/port/clean-room/rifts-admission.md` (for this record's format), `docs/security/rifts-guard.md` (§ headings)
- `AGENTS.md`, `Cargo.toml`, `Cargo.lock`, `clippy.toml`, `deny.toml`, `fuzz/Cargo.toml`,
  `fuzz/fuzz_targets/relay_signal.rs` (a target's shape)
- `crates/rifts-admission`: `Cargo.toml`, `src/lib.rs`, `src/control.rs`, `src/ticket.rs`, `public_xy` in
  `src/keys.rs`; `crates/rifts-guard/Cargo.toml`
- How other MIT crates make outgoing requests and accept-any loopback certificates: grep hits in
  `crates/launcher-kit/src/net/http.rs` and `crates/gateway/tests/common/{mod,compat}.rs`
- Cargo registry, MIT/Apache dependencies: `rtc-ice-0.21.1` (`lib.rs`, `agent/mod.rs`, `agent/agent_config.rs`,
  `candidate/candidate_relay.rs`, the `candidate` and `state` enums, `examples/ping_pong.rs`), `rtc-turn-0.21.1`
  (`client/mod.rs`, `client/relay.rs`, `client/proto.rs`, `examples/turn_client_udp.rs`), `rtc-shared-0.21.1`
  (`lib.rs`, `transport.rs`, `ifaces/mod.rs`), `rtc-crypto-0.21.1` (`provider.rs`), `sansio-1.0.1` (licence),
  `turn-0.17.2` (`examples/turn_server_udp.rs`, `server/config.rs`, the server read loop, `relay_static.rs`),
  `webrtc-util-0.17.2` (`conn/mod.rs`, `conn/conn_udp.rs`), `aes-gcm-0.11.1` and `aead-0.6.1` (usage docs),
  `hyper-rustls-0.27.10` (builder signatures), `rustls-0.23.46` (`dangerous` builder signature)

## 3. Files written

| File | Spec |
| --- | --- |
| `crates/rifts-transport/Cargo.toml` | §6.2, with D2's crates |
| `src/lib.rs` | §2.1 as the `Transport` API (D1), §2.1.4 open rules, the registry |
| `src/session.rs` | §2.2 queues, ready line, roster view, result codes (§2.17) |
| `src/files.rs` | §2.3 |
| `src/url.rs`, `src/filter.rs` | §2.9.2, §2.9.4 |
| `src/link.rs` | §2.9, §2.10 over `rtc-ice` and `rtc-turn` |
| `src/seal.rs`, `src/fragment.rs` | §2.11, §2.10.1, §2.10.3 |
| `src/ticket.rs` | §2.13 on `rifts-admission`'s parser |
| `src/host.rs` | §2.7 |
| `src/join.rs` | §2.8 |
| `src/run.rs` | §2.5, §2.6 |
| `src/router.rs` | §2.15 and the in-memory test network |
| `src/bridge.rs` | §2.16 |
| `src/backend.rs` | §3.1, §3.2, the lease rule of §2.5.3 |
| `src/guard.rs` | §3.3 |
| `src/pipe.rs` | §3.4 |
| `src/text.rs` | §2.0 |
| `tests/common/{mod,backend,turn_server}.rs` | §5 helpers: signer, files, fake backend, TURN server, echo, client, carrier |
| `tests/unit.rs` | §5.1 crypto, parsing, tickets (with high-S), lease, lane, held signals and roster, late consume, local bridge; §5.3 "session" |
| `tests/publisher.rs` | §5.1 publisher |
| `tests/ice_link.rs` | §5.2 direct, relay-local, bursts; §5.4 relay file (`#[ignore]`) |
| `tests/module.rs` | §5.3 module sessions, config rules, eight sessions, the API checks of "abi" |
| `tests/hub.rs` | §5.3 hub, direct and relay-local |
| `tests/slow_backend.rs` | §5.3 slow backend |
| `fuzz/fuzz_targets/transport_{frame,files,pipe}.rs` | §5.6 (MPC1 and ticket parsing are `rifts-admission`'s, already fuzzed by `admission_control` and `admission_ticket`) |
| `docs/security/rifts-transport.md` | umbrella §4.5 |

## 4. Verification

Windows 11, toolchain from `rust-toolchain.toml`:

- `cargo fmt --all --check`: clean
- `cargo clippy --workspace --all-targets -- -D warnings`: clean (without `--locked`: the row adds dependencies;
  `Cargo.lock` and `fuzz/Cargo.lock` are committed)
- `cargo test -p rifts-transport --no-fail-fast`: 47 passed, 1 ignored (`relay_file`, needs a real relay): 16 unit
  tests in `src/`, 31 integration tests (`unit` 11, `publisher` 4, `ice_link` 4, `module` 4, `hub` 2,
  `slow_backend` 1, plus the ignored one)
- `cargo check --manifest-path fuzz/Cargo.toml --bin transport_frame --bin transport_files --bin transport_pipe`: clean
  (stable; the targets were not run)
- **D2 spike, done here:** two `rtc-ice` agents connected relay-only through a local TURN server on 127.0.0.2, both
  ICE roles (joiner controlling, host controlled), and carried 1..2000-byte messages, 300×600-byte and 100×1800-byte
  bursts with no frame rejected (`ice_link::relay_only_through_local_turn`, `bursts_relay_local`); the six-joiner hub
  ran relay-only end to end (`hub::hub_relay_local`, ≥ 12 `route=relay`, 0 `route=direct`). Cloudflare's relay was not
  tried (no credentials in the clean room): `relay_file` is the test for it.
- Nothing was run against the Linux build or a real game.

`cargo tree -e normal -p rifts-transport --depth 1`:

```text
rifts-transport v0.0.0
├── aes-gcm v0.11.1
├── base64 v0.23.1
├── bytes v1.12.1
├── http v1.5.0
├── http-body-util v0.1.5
├── hyper v1.12.0
├── hyper-rustls v0.27.10
├── hyper-util v0.1.21
├── p256 v0.14.0
├── rand v0.10.3
├── rifts-admission v0.0.0
├── rifts-seam v0.0.0
├── rtc-ice v0.21.1
├── rtc-shared v0.21.1
├── rtc-turn v0.21.1
├── rustls v0.23.46
├── sansio v1.0.1
└── tokio v1.54.1
```

The full normal tree (218 packages) holds no GPL crate: every licence is MIT, Apache-2.0, BSD, ISC, Zlib, Unicode
or (webpki-roots) CDLA-Permissive-2.0, all in `deny.toml`'s list. The dev-only `turn` pulls `ring` (ISC/MIT).

## 5. Seam changes (for the integration pass)

- `[workspace.dependencies]`: `rifts-transport` (path); `rtc-ice` and `rtc-turn` 0.21.1 with `crypto-aws-lc-rs` and
  no default features (no ring in the shipped tree, D10); `rtc-shared` 0.21.1 (`ifaces`) and `sansio` 1.0.1, the
  stack's own types the agent and client are driven through; dev-only `turn` 0.17.2 and `webrtc-util` 0.17
  (`conn`, `vnet`) for the test TURN server. All MIT OR Apache-2.0.
- The crate's own manifest turns on features of workspace dependencies only: `hyper-rustls` `webpki-roots`,
  `hyper-util` `client-legacy` and `http1`, `tokio` `rt-multi-thread`, `net`, `time`, `sync`, `io-util`.
- `fuzz/Cargo.toml`: the `rifts-transport` dependency and three `[[bin]]` entries; `fuzz/Cargo.lock` updated.
- Not added: `zeroize` (§5.3 below), `rustls-platform-verifier`, `p256` (already in the workspace).
- The migration guide's §2 line, for the integration pass to write: `crates/rifts-transport` — clean-room MIT
  rewrite of the co-op transport (`src/*.cpp` in dainty-restricted's `modules/rustic-rifts`), linked in process (D1).

## 6. Ambiguities and the choices taken

### 6.1 The API under D1

1. **ABI 1 and ABI 2 are not carried.** ABI 1 only ever refused; ABI 2 was a default handle over ABI 3, which a Rust
   caller keeps itself. `Transport` has ABI 3's calls: `initialize`, `shutdown`, `peer_id`, `networking_ready`,
   `host_open`/`join_open` (paths; an empty path is "missing"), `wait_ready` (`Ready::{Ready, TimedOut, Ended}` for
   1/0/-1), `session_running`, `stop_session`, `last_session_result` (-1 unknown), `read_signal`, `submit_signal`,
   `read_peer_event`, `read_diagnostic`, `add_peer`, `remove_peer`, plus `abi_version` (3) and `same_as`.
2. **"Copy to a buffer" has no counterpart**: reads return owned values, so `ReadSignalH`'s -1 (a buffer too small)
   cannot happen. Queue lines keep their truncation (1023 bytes for diagnostics, 255 for events).
3. **One process-wide instance** became a value: each `Transport` has its own registry, peer id and runtime (the
   tests run two "modules" as two values). Dropping the last clone stops and joins its sessions.
4. `Transport::with_timing` shortens the admission deadline and held-signal TTL for tests; the default is 60 s/20 s.
5. `UpdateRelayCredentialH` is not offered (§7 item 11), though `rtc-turn` has `update_credentials`.

### 6.2 ICE (D2)

6. Host candidates: one socket per IPv4 interface address, loopback included and link-local left out, so the agent
   knows which local address a datagram came in on. A peer may receive a loopback candidate it cannot use.
7. **A relay-only link offers the agent only relay candidates**: no host or reflexive candidate is gathered at all,
   so no check leaves from a host address. The spec allowed gathering them locally; this is stricter.
8. Relay candidates carry `raddr 0.0.0.0 rport 0`, so the line a relay-only peer sees names no local address.
9. The signal text is `rtc-ice`'s: `a=candidate:` + `Candidate::marshal()` (lowercase `udp`, CRC foundation); the
   description is `a=ice-ufrag`, `a=ice-pwd`, `a=ice-options:ice2,trickle`. D3 ends libjuice interop, so nothing was
   matched to its text.
10. Timers are `rtc-ice`'s defaults except `failed_timeout` 30 s (RFC 7675). Gathering is reported done when every
    STUN/TURN server has answered or failed, or after 10 s. The TURN realm comes from the server's 401.
11. "Connected" is reported only once the agent has a selected pair, so the route is known when `ICE_ROUTE` is written.
12. Server host names are resolved on the session thread when a link starts (blocking DNS).
12a. `initialize` does not turn the ICE library's logging off, as the C++ did: `rtc-ice` and `rtc-turn` log through
    the `log` facade, and silencing it here would be process-wide. The launcher's subscriber must filter the `rtc_*`
    targets out (they name addresses and, at debug, the remote ICE password); listed in the security checklist.

### 6.3 Behaviour details

13. Tickets go through `rifts-admission::ticket::parse_signed` and `check_lifetime`, which are stricter than the
    transport spec in places (canonical re-encoding, UUID version, times exact as doubles, party revision ≤ 2^53);
    all of those refuse as `malformed`, and a backend-minted ticket passes both. The 2048-byte payload limit is
    checked from the text length before decoding. High-S signatures pass (test `high_s_signatures_pass`).
14. A local batch check failure (§3.3.4) is written `GUARD_EVIDENCE_LOSS … reason=batch_refused status=0`.
15. `UDP_STATS` has nine bridge fields before `admitted=` (the spec's "first ten" counts one too many; its counter
    list has nine).
16. The lease sequence advances even when the lease lane refuses the job (it holds two, one in flight, so it never
    does in practice).
17. HTTP: webpki roots, no system proxy (spec §7 item 7); 5 s connect, 10 s to the response head, 5 s for the body;
    the WinHTTP setup text became `admission backend connection setup failed`.
18. A bridge bind failure ends the run as `SESSION_FAILURE code=exception`, result 3, without a panic.
19. The host's own consume succeeds on `ok` alone; a joiner's also needs a 64-character id (§2.7.4).
20. A joiner past its admission deadline is refused even while its consume is still with the backend (the late-consume
    scenario requires it); the late answer then undoes the backend admission with one disconnect.
21. Key wiping: no `zeroize`. The host session zeroes its pair keys on remove and drop and the API its decoded key;
    the joiner's key, the ticket and capability strings and the secrets text are dropped, not wiped. A seam entry if
    real wiping is wanted.

### 6.4 The Guard pipe without `unsafe`

22. **Security gap, for the maintainer.** `tokio::net::windows::named_pipe` creates the pipe inbound-only, message
    mode, first and only instance, remote clients refused, 8 KiB in-buffer, but its safe API takes no security
    descriptor and has no client process id. So the pipe gets Windows' default DACL instead of
    `D:P(A;;GA;;;SY)(A;;GA;;;OW)S:(ML;;NW;;;LW)`, and **`expected_server_pid` is checked for validity but not enforced**:
    any local process of the user may write Guard events, and a Low-integrity game server may be unable to. Fixing it
    needs `unsafe` (a module-level allowance like `gateway`'s allowlist) or a crate that wraps
    `GetNamedPipeClientProcessId` and the SDDL call; that is the maintainer's call (D1 says no `unsafe`).
23. The reader runs as a task on the transport's runtime and only queues; the session takes the queue each loop, so the
    16-messages-per-pump bound became the 256-event queue bound.
24. Other systems get a stand-in pipe that never receives.

### 6.5 Tests

25. The whole-run watchdog became per-scenario deadlines. Bursts are measured link to link; the module, hub and slow
    backend scenarios go through the real bridges and an echo server.
26. The test TURN server wraps its socket so Windows' connection-reset receive errors (an ICMP port unreachable from a
    closed client socket) do not end the server's read loop; without it the relay hub lost its relay mid-test.
27. ABI-only checks (`abi`: export names, ABI 2 default handle, buffer sizes) have no Rust counterpart and were dropped.

## 7. Similarity

For the reviewer outside the clean room: measure each `src/*.rs` against dainty-restricted `4efc8b5`'s
`modules/rustic-rifts/src/*.cpp` and the tests against `src/test/selftest_main.cpp`, by `similarity.md`'s measure.

## 8. Integration

- Merge after `feat/rd27-rifts-mit`; then `rifts-host` gains its in-process loader over `Transport` (D1).
- `docs/rifts-transport.md` (the spec's replacement for the README sections) is not written by this row.
- NOTICE: a clean-room MIT rewrite replacing the port of Mystic Paradox's co-op transport; the D2 crates' notices.
