<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-Room Provenance Record: `crates/rifts-host` (RD27)

- **Component**: `crates/rifts-host` (crate `rifts_host`, `license = "MIT"`, `Copyright (c) 2026 Silent Spud`), the
  launcher's side of realms, and the fuzz targets `fuzz/fuzz_targets/host_ready_line.rs`, `host_peer_line.rs` and
  `host_transport_config.rs`
- **Behaviour specification**: `docs/port/spec-rd27-rifts.md` (umbrella, with D1 accepted, D3's renames and D4) and
  `docs/port/spec-rd27-rifts-host.md` (whole), on top of the MIT `rifts-seam`, `rifts-relay`, `rifts-admission`,
  `rifts-guard` and `rifts-transport` (`origin/feat/rd27-rifts-transport`, `b81fa6e`)
- **Clean-room worktree**: `R:\source\dauntless\dainty-lass\.claude\worktrees\agent-a20a95504d8cbc789` (branch
  `feat/rd27-rifts-host`, cut from `origin/feat/rd27-rifts-transport` at `b81fa6e`)
- **Implementer**: one Claude Code subagent session (model Claude Opus 5.5), 2026-11-02, briefed by the RD27
  coordinator

## 1. Isolation

It never read, listed, grepped or `git show`ed anything under `dainty-restricted` (any worktree of it),
`modules/rustic-rifts`, `modules/rustic-isles`, Mystic Paradox's repository or libjuice's sources, and ran no
`git log -p` or `git show` on any commit. It searched nothing on the web and read no crate source from the cargo
registry. The session's environment listed a dainty-restricted worktree as an additional working directory; no tool
call read from it. The worktree is a full checkout of dainty-lass at `b81fa6e`, which holds no `rifts-host` source.

## 2. Files read

- The brief (`impl-brief.md` in the coordinator's scratchpad)
- `docs/port/spec-rd27-rifts.md`, `docs/port/spec-rd27-rifts-host.md` (whole)
- `docs/port/clean-room/rifts-transport.md` (this record's format), `docs/security/rifts-transport.md` (its structure)
- `AGENTS.md`, `Cargo.toml`, `fuzz/Cargo.toml`, `fuzz/fuzz_targets/transport_files.rs` (a target's shape)
- `crates/rifts-seam`: `Cargo.toml`, `src/lib.rs`, `src/id.rs`, `src/error.rs`, `Secret` in `src/store.rs`, `HubKind`
  in `src/hooks.rs`, `BoxFuture` in `src/http.rs`
- `crates/rifts-transport` (the API this crate drives): `Cargo.toml`, `src/lib.rs`, `src/files.rs` (first 200 lines),
  `src/run.rs` (the host run and the join ready line), `src/text.rs`, `src/pipe.rs` (the config check and the
  non-Windows stand-in), `BackendClient::new` in `src/backend.rs`, `tests/common/{mod,backend}.rs`, `tests/module.rs`
- `crates/rifts-admission`: `Cargo.toml`, the ticket types in `src/ticket.rs` and `public_xy` in `src/keys.rs` (to
  mint test tickets); `crates/rifts-guard/Cargo.toml`

## 3. Files written

| File | Spec |
| --- | --- |
| `crates/rifts-host/Cargo.toml` | §7.1, with D1's `in-process` feature (default on) |
| `src/lib.rs` | §2.1 `HostError`, §9 re-exports, §9.2 `Stub` |
| `src/transport.rs` | §3 (constants, types, `TransportAbi`, `TransportLoader`) |
| `src/loader.rs` | §4.1 `sha256_file`; `NativeTransportLoader` and `InProcessTransport` under D1 (§6.1 below) |
| `src/files.rs` | §5 |
| `src/session/mod.rs` | §6.1, §6.2 types, `CommonBootstrapConfig::to_config_lines` |
| `src/session/rules.rs` | §6.2.1 |
| `src/session/backend.rs` | §6.3 |
| `src/session/folder.rs` | §6.4 |
| `src/session/hosting.rs` | §6.5, §6.6 |
| `tests/common/mod.rs` | a scripted `TransportAbi` (the fake library's behaviour as the flows see it), a recording spawner, a backend wrapping `DefaultHostBackend` |
| `tests/unit.rs` | §8.2 items 2, 3, 6, 8 and the additions (`ice_files`, `parse_ice_ready`, `windows_command_line`, `trials_week`, `relay_refresh_due`, `build_launch_environment`, config lines) |
| `tests/hosting.rs` | §8.2 item 7 and the additions (`bridge.conf` bytes for hub, local hunt, away hunt and join; failure cleanup; the join's sliced wait; the roster pump; ended sessions; drop) |
| `tests/in_process.rs` | §8.2 item 4 rewritten for D1, and a hub hosted and joined end to end through the real in-process transport |
| `fuzz/fuzz_targets/host_{ready_line,peer_line,transport_config}.rs` | §8.4 |
| `docs/security/rifts-host.md` | umbrella §4.5, spec §10 item 6 |

## 4. Verification

Windows 11, toolchain from `rust-toolchain.toml`:

- `cargo fmt --all --check`: clean
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean (`Cargo.lock` gains only the `rifts-host`
  package; no new external dependency). `cargo clippy -p rifts-host --no-default-features --all-targets -- -D
  warnings`: clean
- `cargo test -p rifts-host --no-fail-fast`: 29 passed (`unit` 16, `hosting` 10, `in_process` 3). The end-to-end test
  hosts a Ramsgate hub on one in-process transport and joins it from a second, through a fake HTTP backend on
  127.0.0.1 and a signal relay, with real ES256 tickets; ICE runs over loopback host candidates (no STUN or TURN)
- `cargo check --manifest-path fuzz/Cargo.toml --bin host_ready_line --bin host_peer_line --bin host_transport_config`:
  clean (stable; the targets were not run)
- Not run: Linux, restricted's two launcher tests (§8.1), a real hosted server, a real 1.14.7 client

`cargo tree -e normal -p rifts-host --depth 1`:

```text
rifts-host v0.0.0
├── rand v0.10.3
├── rifts-seam v0.0.0
├── rifts-transport v0.0.0
├── serde v1.0.229
├── serde_json v1.0.154
├── sha2 v0.11.1
└── thiserror v2.0.21
```

Every crate below is MIT or Apache-2.0 (rifts-transport's own tree is listed in its record). No AGPL crate.

## 5. Seam changes (for the integration pass)

- `[workspace.dependencies]`: `rifts-host = { path = "crates/rifts-host" }`.
- `fuzz/Cargo.toml`: the `rifts-host` dependency (no default features) and three `[[bin]]` entries; `fuzz/Cargo.lock`
  updated.
- Not needed: `libloading`, `getrandom` (the workspace's `rand` `SysRng` draws the folder and pipe randomness), any
  `unsafe` exemption. plugin-host's "only crate allowed unsafe code" sentence stays true (spec R2).
- The migration guide's §2 line, for the integration pass to write: `crates/rifts-host` — clean-room MIT rewrite of
  the launcher's realm hosting (`crates/rifts-host` in dainty-restricted's `modules/rustic-rifts`), with the
  transport in process (D1).

## 6. Ambiguities and the choices taken

### 6.1 The transport under D1

1. **No FFI module.** §4's loader, the C bindings and the `unsafe` sites are not written. `transport::EXPORTS` and
   `OPTIONAL_EXPORTS` are dropped: the launcher does not use them (§9.1), and their values are the old export names
   D3 removes. The test that compared them to the header goes with them.
2. **`NativeTransportLoader`** keeps its name and unit shape (§9.1) and, with `in-process` on, returns a new
   `InProcessTransport` from `load`, ignoring the path and the digest: there is no file, so the Guard manifest's
   `TRANSPORT_MODULE` digest check has nothing to check (D1's accepted cost). With the feature off, `load` answers
   `Transport("this build has no co-op transport (feature in-process is off)")`.
3. **`sha256_file`** is kept exactly as §4.1 (the launcher calls it). `HostError::HashMismatch`, `MissingExport` and
   `UnsupportedAbi` are kept for the launcher's error mapping; nothing in the crate produces them now.
4. **`InProcessTransport`** (new public item, re-exported at the root under `in-process`): `TransportAbi` over
   `rifts_transport::Transport`, with `new`, `from_transport` and `transport`. It keeps §4.3's error texts and the
   interior-NUL refusals with their labels (checked before the call, `update_relay_credential` included), trims text
   as §4.3 does except signal data, and maps `stop_session`'s `false` to `NotFound("unknown session handle {n}")`.
   Dropped as meaningless in process: the path UTF-8 checks (the transport takes `Path`) and "signal buffer too
   small". `update_relay_credential` is `Ok` after its NUL checks: the transport has no credential update, which is
   ABI 3's "optional export absent".
5. **`TRANSPORT_DLL`** must exist (§9.1) but may not carry the old name (D3); it is `RusticTransport.dll`, a file
   that nothing ships. At the cutover the launcher must stop pinning a digest for it, because `sha256_file` on a
   missing file fails before `load` is reached (restricted's launcher change, umbrella §4.3).

### 6.2 D3 renames applied here

6. Environment: every `MYSTICPARADOX_<NAME>` launch and cleared variable is `RUSTIC_RIFTS_<NAME>`.
7. Pipes: `RusticInternalServer.ready.<X>`, `RusticInternalServer.control.<X>` and `RusticHuntGuard.events.<X>`. The
   D3 table names only the Guard pipe; the internal-server pipes are renamed under the umbrella's "none of the old
   names" rule. The runtime's `ReadLaunch` must read the new names (a runtime patch in the cutover).
8. The flag file's second folder is `<app_local_data>/RusticRifts/` (was the old product's folder name).
9. `DefaultHostBackend`'s tickets start `rpt1.` (`rpt1.ticket.signature`, `rpt1.joiner_ticket.signature`,
   `rpt1.hub_host_ticket.signature`).

### 6.3 Behaviour details

10. **Hub limits file layouts.** The map layout is parsed with unknown keys refused, so the older single-object
    layout (`{"maxPlayers",...}`) is not read as an empty map and reaches step 3. A map whose entry has an
    out-of-range number fails the whole map parse and is damaged.
11. A JSON array as `p2p-host-settings.json` parses as empty settings (serde reads a struct from a sequence), so it
    is not damaged; any text that is not JSON is.
12. The single-value secret files (`capability.secret`, `ticket.secret`) hold the bare value with no newline; the
    two `key=value` files end every line in `\n`.
13. A hub's `bridge.conf` always carries `host_local_client=1` (the §6.5 template), whatever the bootstrap's
    `local_client`.
14. A pump thread that cannot be spawned fails the start with `Io("could not start the session pump: {err}")`,
    after stopping the transport session and the server.
15. R9: non-UTF-8 inherited variables are skipped. R10 is kept as found (check and insert under separate locks).
    R11 is kept: `sessions()` cleans an ended session's folder and leaves it listed until `stop`.
16. `sanitize_central_log_line` stops once the line reaches 2048 bytes and cuts it to its first 2048 characters, so
    a line of multi-byte text can stop short of 2048 characters (as the spec words it).
17. The join's sliced wait clamps the remaining time to 1–50 ms and loops while `TimedOut` comes back before the
    deadline; a backend without signalling gets one full wait.

### 6.4 Found while testing, for the transport row and the integration pass

18. **`DefaultHostBackend` values do not pass the real transport.** The transport's `hunt_definition_id` charset
    (`[A-Za-z0-9._:/-]`) refuses `Hunt_Default` and `ShatteredIsles_ReturnToRamsgate`, and its account check
    (`[A-Za-z0-9-]`) refuses `_` and `:` that `safe_account` here allows. Today this only matters where the launcher
    falls back to the local backend with a real transport (no server configured), where such a session ends as
    `BAD_INPUT`. The end-to-end test uses ids both sides accept. One of the two rules should move; not changed here.
19. The tests replace §8.2's fake `cdylib` with a scripted `TransportAbi` in Rust (`tests/common`) for the flows and
    with the real in-process transport for item 4 and the end-to-end run. Item 5 (a real built DLL) has no
    counterpart and is dropped.
