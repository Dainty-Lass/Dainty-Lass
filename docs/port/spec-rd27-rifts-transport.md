<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->
# Clean-Room Behaviour Specification: the co-op transport DLL (`ParadoxTransport.dll`, RD27)

> **Scope and purpose (RD27):** this is the clean-room behaviour specification for relicensing Rustic Rifts' C++ co-op
> transport, `ParadoxTransport.dll`, as MIT Rust. The DLL lives in dainty-restricted (AGPL) at `modules/rustic-rifts`
> (`src/`, `src/test/selftest_main.cpp`, the `.vcxproj` files, the PowerShell scripts, `tools/`, the Forgejo
> workflow). It is Mystic Paradox's `ParadoxTransport/` folder taken with its history, so every source file is
> rewritten from this document alone by an implementer who never sees the AGPL code.
>
> Everything below is stated from observable contracts: the C exports and their return codes, the files the DLL
> reads, the lines it emits, the bytes it puts on the wire (sealed frames, control frames, tickets), the HTTPS calls
> it makes and the UDP sockets it opens. No implementation code is reproduced and no private helper is named.
> The ICE layer is described independently of libjuice, so a Rust ICE crate can reproduce it (§2.9).
>
> Spec written against dainty-restricted `4efc8b5` (worktree `rd27-spec-rifts-c45124`).

---

## 1. Provenance and licensing classification

### 1.1 Evidence used

| Source | What it says |
| --- | --- |
| `modules/rustic-rifts/NOTICE.md` | The repository root is Mystic Paradox's `ParadoxTransport/` folder with its history (to `c1c5dca`, 17 Oct 2026): `src/`, `src/test/`, the build and dependency scripts and `tools/`; files keep Mystic Paradox's headers |
| File headers | Every `src/**` file and `tools/mint-relay-test-file.cjs` carries "Copyright (C) 2026 MysticFox / Pranav Karande", AGPL-3.0-only |
| `git log` | `src/` authored by pranav158 (5935cde, fbecfe1, 56596fe, a8e49c1, 62f64dd); one DaintyLass change, ae3deaa (RF4, 31 Oct: plain HTTP and lax TLS for a loopback backend in the admission client); `.forgejo/workflows/transport.yml` born in 68a5126 (RF3, Silent Spud); README licence section 5ea3a5d (RF3) |
| `README.md` "Provider-neutral files copied from `SteamBridge/src`" | `admission_ticket.*`, `admission_client.*`, `guard_event_pipe.*`, `message_transport.*`, `udp_bridge.*` came from Mystic Paradox's private Steam bridge |
| Callers | `crates/rifts-host/src/{transport.rs,ffi.rs,session.rs}` load the DLL with `libloading`, write its files and parse its lines; rustic-isles' `release-runtime-extras.yml` builds and ships it |

### 1.2 Categories

- **A**: born in DaintyLass, free of Mystic Paradox derivation; moves as is (or is retired).
- **B**: derived from Mystic Paradox; rewritten clean-room from §2–§4. A B file with born-here parts says so.
- **D**: data recorded from Mystic Paradox's programs (oracle). None in this part.
- **G**: generated. None in this part.
- **Retire**: B files with no MIT counterpart because the Rust build replaces them (MSBuild projects, PowerShell
  build scripts). They stay in dainty-restricted until the C++ DLL is dropped.

### 1.3 Files

| Path (under `modules/rustic-rifts/`) | Lines | Cat. | Evidence | Target |
| --- | ---: | --- | --- | --- |
| `src/paradox_transport_api.hpp` | 120 | B | MP header; the ABI | `ffi` module (§2.1) |
| `src/paradox_transport_api.cpp` | 184 | B | MP | `ffi` module |
| `src/transport_module.hpp` / `.cpp` | 134 / 458 | B | MP | `registry` module (§2.2) |
| `src/transport_app.hpp` / `.cpp` | 86 / 745 | B | MP | `host` / `join` runs, file parsing (§2.3–§2.6) |
| `src/session_types.hpp` | 73 | B | MP | limits and stats types |
| `src/ice_session.hpp` / `.cpp` | 232 / 688 | B | MP | host and joiner session logic (§2.7, §2.8) |
| `src/ice_link.hpp` / `.cpp` | 168 / 470 | B | MP; libjuice wrapper | `link` over the chosen ICE crate (§2.9, §2.10) |
| `src/ice_transport.hpp` / `.cpp` | 46 / 62 | B | MP | peer-to-link router (§2.15) |
| `src/message_transport.hpp` / `.cpp` | 77 / 73 | B | MP, from Steam bridge | transport trait + in-memory test network |
| `src/pair_crypto.hpp` / `.cpp` | 72 / 254 | B | MP | `seal` module (§2.11) |
| `src/control_codec.hpp` / `.cpp` | 30 / 138 | B | MP | `control` module (§2.12) |
| `src/admission_ticket.hpp` / `.cpp` | 81 / 309 | B | MP, from Steam bridge | `ticket` module (§2.13) |
| `src/admission_client.hpp` / `.cpp` | 201 / 366 | B | MP, from Steam bridge; born-here part: loopback HTTP / lax TLS (ae3deaa) | `backend` client (§3.1) |
| `src/backend_lane.hpp` / `.cpp` | 64 / 72 | B | MP | `lane` (§3.2) |
| `src/guard_publisher.hpp` / `.cpp` | 127 / 263 | B | MP | `guard` publisher (§3.3) |
| `src/guard_event_pipe.hpp` / `.cpp` | 45 / 147 | B | MP, from Steam bridge | `pipe` (Windows only, §3.4) |
| `src/udp_bridge.hpp` / `.cpp` | 152 / 422 | B | MP, from Steam bridge | `bridge` (§2.16) |
| `src/test/selftest_main.cpp` | 2366 | B | MP | Rust integration tests rewritten from §5 |
| `ParadoxTransport.vcxproj` | 97 | Retire | MP build project | `Cargo.toml` of the new crate |
| `ParadoxTransportTest.vcxproj` | 84 | Retire | MP | `cargo test` |
| `libjuice.vcxproj` | 78 | Retire | MP; builds MPL-2.0 libjuice | none (no C dependency) |
| `build.ps1` | 61 | Retire | MP | `cargo build -p <crate> --release` |
| `prepare-dependencies.ps1` | 54 | Retire | MP; pins libjuice v1.7.4 (SHA-256 `02CCA696…78FD`) | none |
| `run-relay-selftest.ps1` | 26 | Retire | MP | an `#[ignore]` test reading a relay file (§5.4) |
| `tools/mint-relay-test-file.cjs` | 102 | B, retire | MP header | already covered by `crates/rifts-relay/examples/mint-relay-file.rs` (another spec) |
| `.forgejo/workflows/transport.yml` | 52 | A | RF3 (68a5126) | retire; the Rust family of dainty-lass CI (RR3 owns it) |
| `README.md` transport sections | 226 (file) | B | MP text with RF3 licence section (A) | a new page `docs/rifts-transport.md` in dainty-lass, written from this spec |

Totals: A 1 file (52 lines); B 28 source/test files (7,567 lines) plus README sections; Retire 6 (400 lines);
D 0; G 0.

**Tests.** All of `selftest_main.cpp` is B. The implementer rewrites every scenario of §5 as Rust tests. Nothing is
D-dependent, but the C++ DLL and `ParadoxTransportTest.exe` stay in dainty-restricted as an **interop oracle**:
the reviewer runs a C++ joiner against the Rust host and a Rust joiner against the C++ host (§5.5). That is the only
way to show mixed-version compatibility.

---

## 2. Behaviour

### 2.0 Conventions

- "Safe token (n, extra)" means: 1..n bytes, each `A-Z`, `a-z`, `0-9` or one of the listed extra characters.
- "Account" means safe token (64, `-`).
- "Copy to a buffer" means: if the pointer is null or the capacity is 0, write nothing; otherwise copy
  `min(len, capacity-1)` bytes and a NUL. Truncation is silent unless a rule says otherwise.
- Lines the DLL emits are ASCII, fields separated by one space, `key=value` with no quoting. They never contain a
  ticket, capability, key, credential, packet content or IP address.
- Times are monotonic (steady clock) except ticket validity, which uses wall-clock Unix seconds.
- A "diagnostic" goes to the session's diagnostic queue (§2.2), a "peer event" to its event queue, a "failure" is a
  diagnostic `SESSION_FAILURE code=<code>`.

### 2.1 C exports (the ABI)

The DLL is named `ParadoxTransport.dll`, x64, every export `extern "C"` (`__cdecl`, the x64 default), undecorated
names. ABI version 3. Every export may be called from any thread at any time, including concurrently. No export
may unwind a panic into the caller.

`int` is `i32`, `uint32_t` is `u32`, `uint64_t` is `u64`, strings are NUL-terminated UTF-8 `const char*`.

#### 2.1.1 Module-level

| Export | C signature | Behaviour |
| --- | --- | --- |
| `MysticTransport_AbiVersion` | `int (void)` | Returns `3` |
| `MysticTransport_Initialize` | `int (char* error, uint32_t error_size)` | Turns ICE-library logging off (it can name addresses). On the first successful call draws a random non-zero u64 from the OS CSPRNG as the module's peer id; later calls keep it. `1` on success; `0` with `The co-op transport could not create its identity.` copied to `error` when the RNG fails |
| `MysticTransport_Shutdown` | `void (void)` | Sets the stop flag of every session in the registry, then joins every session thread. Sessions stay in the registry (readable) |
| `MysticTransport_PeerId` | `uint64_t (void)` | The peer id, `0` before `Initialize` |
| `MysticTransport_NetworkingReady` | `int (void)` | `1` when the peer id is non-zero, else `0` |

The module is one process-wide instance created on first use and never destroyed (no static teardown may run while
a session thread lives). Nothing else requires `Initialize`: sessions open without it.

#### 2.1.2 ABI 1 (refused)

| Export | C signature | Behaviour |
| --- | --- | --- |
| `MysticTransport_StartHost` | `int (const char* public_key, const char* config, const char* capability, const char* host_ticket, char* error, uint32_t error_size)` | Always `0`, error `This co-op transport needs a launcher with transport ABI 2 (ICE).` |
| `MysticTransport_StartJoin` | `int (const char* public_key, const char* config, const char* ticket, char* error, uint32_t error_size)` | The same |

#### 2.1.3 ABI 2 (the default handle)

ABI 2 calls act on one **default handle** (initially 0 = none). They are wrappers over ABI 3.

| Export | C signature | Behaviour |
| --- | --- | --- |
| `MysticTransport_StartHostV2` | `int (const char* public_key, const char* config, const char* capability, const char* host_ticket, const char* secrets, char* error, uint32_t error_size)` | Null or empty `host_ticket` → `0`, `A co-op host input path is missing.`. Then, serialised against other V2 starts: if the default handle names a registry session that is running → `0`, `A co-op session is already running.`; if it names one that ended → join its thread and remove it from the registry (its unread lines are discarded). Then `HostOpen` with the same arguments; its `0` is returned as is (its message already copied); otherwise the new handle becomes the default and the call returns **`1`** (not the handle) |
| `MysticTransport_StartJoinV2` | `int (const char* public_key, const char* config, const char* ticket, const char* secrets, char* error, uint32_t error_size)` | Same replace rule, then `JoinOpen`; `1` on success |
| `MysticTransport_WaitReady` | `int (uint32_t timeout_ms, char* ready, uint32_t ready_size)` | Default handle 0 → `0`. Else `WaitReadyH(default, …)` (so `-1` if the default session left the registry) |
| `MysticTransport_SessionRunning` | `int (void)` | `SessionRunningH(default)` |
| `MysticTransport_LastSessionResult` | `int (void)` | Default 0 → `0`; else `max(LastSessionResultH(default), 0)` |
| `MysticTransport_StopSession` | `void (void)` | `StopSessionH(default)`, result ignored |
| `MysticTransport_ReadDiagnostic` | `int (char* buffer, uint32_t size)` | `ReadDiagnosticH(default, …)` |
| `MysticTransport_ReadSignal` | `int (char* to, uint32_t to_size, char* kind, uint32_t kind_size, char* data, uint32_t data_size)` | `ReadSignalH(default, …)` |
| `MysticTransport_SubmitSignal` | `int (const char* from_account, const char* kind, const char* data)` | `SubmitSignalH(default, …)` |
| `MysticTransport_ReadPeerEvent` | `int (char* buffer, uint32_t size)` | `ReadPeerEventH(default, …)` |

#### 2.1.4 ABI 3 (handles)

| Export | C signature | Returns |
| --- | --- | --- |
| `MysticTransport_HostOpen` | `int (const char* public_key, const char* config, const char* capability, const char* host_ticket, const char* secrets, char* error, uint32_t error_size)` | Handle > 0, or `0` + message. `host_ticket` may be null |
| `MysticTransport_JoinOpen` | `int (const char* public_key, const char* config, const char* ticket, const char* secrets, char* error, uint32_t error_size)` | Handle > 0, or `0` + message |
| `MysticTransport_WaitReadyH` | `int (int h, uint32_t timeout_ms, char* ready, uint32_t ready_size)` | `1` ready (line copied), `0` timeout, `-1` ended before readiness or unknown handle |
| `MysticTransport_SessionRunningH` | `int (int h)` | `1` while the session thread runs, else `0` (unknown too) |
| `MysticTransport_StopSessionH` | `int (int h)` | Sets the stop flag, joins the thread (blocks through the session's shutdown, §2.5.4), `1`; `0` for an unknown handle |
| `MysticTransport_LastSessionResultH` | `int (int h)` | Result code (§2.17; `0` while running), `-1` unknown |
| `MysticTransport_ReadSignalH` | `int (int h, char* to, uint32_t to_size, char* kind, uint32_t kind_size, char* data, uint32_t data_size)` | `1` copied, `0` none (or unknown handle), `-1` a buffer is null or too small; the signal stays queued |
| `MysticTransport_SubmitSignalH` | `int (int h, const char* from_account, const char* kind, const char* data)` | `1` queued, `0` refused |
| `MysticTransport_ReadPeerEventH` | `int (int h, char* buffer, uint32_t size)` | `1` one line copied, `0` none or unknown |
| `MysticTransport_ReadDiagnosticH` | `int (int h, char* buffer, uint32_t size)` | `1` one line copied, `0` none or unknown |
| `MysticTransport_AddPeerH` | `int (int h, const char* account, uint64_t peer_id, const char* pair_key_b64url)` | `1` queued, `0` refused |
| `MysticTransport_RemovePeerH` | `int (int h, const char* account, const char* reason)` | `1` queued, `0` refused |

Not exported: `MysticTransport_UpdateRelayCredentialH`. The caller (`rifts-host`) looks it up as optional with the
signature `int (int h, const char* user, const char* credential)`; the MIT DLL must not export it unless a decision
row adds it (§7).

**Open rules.** Each path argument is converted from UTF-8 strictly (invalid UTF-8 counts as empty).
`HostOpen`: an empty key, config, capability or secrets path → `0`, `A co-op host input path is missing.`.
`JoinOpen`: an empty key, config, ticket or secrets path → `0`, `A co-op join input path is missing.`. Files are not
opened here: a bad file is reported on the session thread (`WaitReadyH` → `-1`, diagnostics, result 2 or 3).

Under the registry lock, before counting:

1. Every registry session that is not running has its thread joined. A non-running session whose diagnostic,
   outgoing-signal and event queues are all empty is removed. Of the non-running sessions with unread lines, only
   the 8 newest (highest handles) are kept; older ones are removed.
2. If 8 or more sessions are running → `0`, `At most 8 co-op sessions can run at once.`.
3. The session gets handle `next` (starting at 1, incremented only on success, never reused), is marked running and
   its thread starts. A thread that cannot start → `0`, `The co-op session thread could not start.`.

The session thread runs the host run (§2.5) or the join run (§2.6). Its result defaults to 3; a panic/exception
inside the run pushes the diagnostic `SESSION_FAILURE code=exception` and leaves the result 3 (this is how a UDP
bind failure or a socket-layer start failure surfaces). On exit the thread records the result, marks
"ended before ready" if it never reported ready, clears running and wakes `WaitReadyH` waiters.

**`WaitReadyH`** waits up to `timeout_ms` for "ready" or "ended before ready". A ready line, once reported, is kept:
later calls return `1` with it immediately, even after the session ended.

**`SubmitSignalH`** validates before looking up the handle: all three pointers non-null; `from_account` an account;
`kind` exactly `DESCRIPTION`, `CANDIDATE` or `GATHERING_DONE`; `data` at most 4096 bytes, each byte `0x20..=0x7E`,
CR or LF (empty data is allowed). Then the handle must exist and be running, and the incoming queue must hold fewer
than 2048 signals. Otherwise `0`.

**`AddPeerH`**: `0` when `account` or the key pointer is null, `peer_id` is 0, the account is not an account, or the
key does not decode (§2.11.1). The handle must name a running **host** session. Against the API layer's roster view
(below), `0` for a duplicate account or a duplicate peer id, a roster at its limit, or 256 commands already queued.
Else the add is recorded in the view and queued as a command; `1`. The decoded key is wiped after use. The host's
own peer id is **not** checked here: the session refuses it later (`PEER_ADD_REJECTED … reason=invalid_peer`).

**`RemovePeerH`**: `0` for a null or non-account `account`, a handle that is not a running host, an account absent
from the view, or 256 commands queued. Else removes it from the view and queues a remove with `reason` truncated to
48 bytes (null → empty); `1`.

**Roster view.** Starts empty with limit 64. Once the host session accepts its configuration it reports
`maximum_peers` and its starting peers (the secrets file's `peer=` lines), which set the limit and are inserted into
the view (overwriting). When the session refuses a queued add for any reason other than `duplicate_account`, the
account is removed from the view.

### 2.2 Session queues and their bounds

| Queue | Bound | When full |
| --- | ---: | --- |
| Diagnostics | 256 lines, each truncated to 1023 bytes | Oldest dropped |
| Peer events | 256 lines, each truncated to 255 bytes | Oldest dropped |
| Outgoing signals (to the launcher) | 2048 | The new signal is dropped; on the 1st, 2nd, 4th, 8th … drop (count a power of two) the diagnostic `SIGNAL_DROPPED reason=outbox_full` |
| Incoming signals (from the launcher) | 2048 | `SubmitSignalH` returns 0 |
| Roster commands | 256 | `AddPeerH`/`RemovePeerH` return 0 |

A failure code is truncated to 96 bytes in its `SESSION_FAILURE code=` line. Each read pops the oldest entry.

### 2.3 The config and secrets files

Both are `key=value` text files read on the session thread.

**Reading.** The path must be a regular file and not a symlink (`input must be a regular non-symlink file`); its size
must be 1..=16384 bytes (`input file is empty or exceeds its size limit`), except that a **host's** secrets file may
be empty (a solo host without relay). A short read → `input file could not be read completely`. Trailing CR and LF
bytes are stripped. The capability file uses the same reader with a limit of 128 bytes; ticket files 4096 bytes. The
secrets text is wiped from memory once parsed.

**Parsing** (each file separately):

1. Split on LF; strip one trailing CR from each line.
2. Skip empty lines and lines whose first byte is `#`.
3. A line needs `=` at index ≥ 1 with at least one byte after it, else `configuration contains a malformed line`.
4. Key: safe token (48, `_-`); value: at most 1024 bytes, no NUL; a key other than `peer` may not repeat in one
   file. Violations → `configuration contains an unsafe or duplicate field`.
5. Values are not trimmed. Unknown keys are accepted and ignored.

**Field readers.**

- *Required text*: the key present and its value a safe token of the stated size and extras, else
  `missing or invalid configuration field: <key>`.
- *Number*: decimal digits only (no sign, no space), within the stated range; absent →
  `missing numeric configuration field: <key>`, bad → `invalid numeric configuration field: <key>`.
- *Boolean* (`force_relay`, `hunt_evidence`): true only for the exact value `true`.

All fields of a group are evaluated; the error reported is the **last** failing one in the order the tables list.

**Binding fields** (config, host and joiner):

| Key | Rule |
| --- | --- |
| `key_id` | text 32, `._-` |
| `session_id` | text 36, `-` (must be the lowercase 8-4-4-4-12 form to match tickets, §2.13) |
| `party_id` | text 192, `._:+/=-` |
| `hunt_definition_id` | text 128, `._:/-` |
| `bridge_instance_id` | text 96, `._:-` |
| `party_revision` | u64, 1..=u64::MAX |
| `host_peer_id` | u64, 1..=u64::MAX |
| `generation` | u32, 1..=u32::MAX |
| `build` | u32, 1..=u32::MAX |
| `bridge_protocol` | u16, 1..=65535 |
| `virtual_port` | u16, 1..=999 |

**Host-only config fields:**

| Key | Rule | Default |
| --- | --- | --- |
| `backend_url` | text 512, `:/._-` | required |
| `server_udp_port` | u16, 1..=65535 | required |
| `maximum_peers` | 1..=4 for HUNT, 1..=64 for HUB | required |
| `session_role` | exactly `HUNT` or `HUB` | `HUNT` |
| `host_local_client` | exactly `0` or `1` | `1` |
| `guard_pipe_name` | text 96, `._-` | required for HUNT; for HUB only when this key or `expected_server_pid` is present |
| `expected_server_pid` | u32, 1..=u32::MAX | as `guard_pipe_name` |
| `hunt_evidence` | boolean | HUB: false unless `true`; HUNT: true when absent or `true`, false for any other value |
| `force_relay` | boolean | false |

**Joiner-only config fields:** `local_udp_port` (u16, 0..=65535; 0 = any free port), `local_peer_id` (u64 ≥ 1),
`local_account_id` and `host_account_id` (text 64, `-`), `force_relay`.

**ICE fields** (config): `ice_stun_urls` and `ice_turn_urls`, comma lists. List splitting: split on `,`, trim spaces
at both ends of each item, drop empty items.

- STUN: the first item that parses as a STUN URL (§2.9.2) is the only STUN server; the others are ignored.
- TURN: used only when `ice_turn_urls`, `turn_username` and `turn_credential` are all present. The username and
  credential must each be 1..=512 bytes of `0x21..=0x7E`, else `invalid relay credential`. Each item that parses as
  a TURN-over-UDP URL becomes a relay server with those credentials; items that do not parse are skipped; duplicates
  of (host, port) are skipped; at most 4 relay servers.

**Secrets file:** `turn_username`, `turn_credential`, and any number of `peer=<account>,<peer id>,<pair key>` lines.
A `peer` value is split by the list rule and must give exactly 3 items: an account, a u64 ≥ 1 and a pair key that
decodes (§2.11.1), else `invalid peer entry in the transport secrets`. A second line with the same account or the
same peer id → `duplicate peer entry in the transport secrets`.

### 2.4 Lines the DLL emits

#### 2.4.1 Ready lines (the `WaitReady` buffer)

Host (one line, fields in this order):

```text
HOST_READY session=<session_id> generation=<generation> transport=ice localHost=127.0.0.1 localPort=<port> routePolicy=<RELAY_ONLY|AUTOMATIC> relayReady=<0|1> peers=<n>
```

`localPort` is the local-client bridge's port, `0` when `host_local_client=0`. `routePolicy` is `RELAY_ONLY` when
`force_relay`. `relayReady` is `1` when at least one relay server is configured. `peers` counts the secrets file's
`peer=` lines.

Joiner:

```text
ROUTE_READY localHost=127.0.0.1 localPort=<port> session=<session_id> generation=<generation> transport=ice routePolicy=<RELAY_ONLY|AUTOMATIC> route=<relay|direct>
```

`rifts-host` parses these as whitespace tokens of `key=value` with unique keys and checks `localHost`, `transport`,
`routePolicy`, `route` (joiner, relay-only) and `localPort` (non-zero exactly when the host plays a local client).

#### 2.4.2 Peer events

| Line | When |
| --- | --- |
| `PEER_CONNECTED account=<a> route=<relay\|direct>` | A link first reaches Connected (host: per joiner; joiner: account is the host's) |
| `PEER_ADMITTED account=<a> route=<relay\|direct>` | Admission completed |
| `PEER_LOST account=<a> reason=<r>` | An admitted peer's link closed; `r` ≤ 48 bytes: `link_lost`, `restarted`, `host_stopped`, a remove reason token |
| `PEER_REMOVED account=<a> reason=<token> admitted=<0\|1>` | Host, the answer to `RemovePeerH` (after `PEER_LOST` when admitted) |
| `PEER_ADD_REJECTED account=<a> reason=<code>` | Host, a queued add refused: `invalid_peer`, `duplicate_account`, `duplicate_peer`, `full` |

#### 2.4.3 Diagnostics (every kind)

Configuration and setup (`<error>` is one of the texts of §2.3, §2.13.4, §3.1.5 or §3.4):

- `HOST CONFIG FAIL: <error>`, `HOST CONFIG FAIL: invalid session_role`, `HOST CONFIG FAIL: invalid host_local_client`,
  `HOST CONFIG FAIL: more transport peers than maximum_peers`
- `HOST KEY FAIL: <error>`, `HOST BACKEND FAIL: <error>`, `HOST SESSION FAIL: invalid session configuration`,
  `HOST GUARD PIPE FAIL: <error>`
- `JOIN CONFIG FAIL: <error>`, `JOIN CONFIG FAIL: the transport secrets must name exactly the host`,
  `JOIN KEY FAIL: <error>`, `JOIN SESSION FAIL: invalid session configuration`, `JOIN SESSION FAIL: <error>`
- `SESSION_FAILURE code=<code>` (§2.17)
- `ICE_READY role=<host|join> stun=<0|1> turn=<n relay servers> relayOnly=<0|1> peers=<n>`

Links, signals and roster:

- `ICE_ROUTE role=<host|join> peer=<peer id> route=<relay|direct>`
- `ICE_ROUTE_REFUSED role=host peer=<id> local=<type> remote=<type>` (types: `host`, `srflx`, `prflx`, `relay`,
  `unknown`)
- `ICE_LINK_FAILED role=host peer=<id> reason=<link start error>`
- `ICE_SIGNAL_REJECTED role=host peer=<id> reason=<error>`, `ICE_SIGNAL_REJECTED role=join reason=<error>`
- `ICE_SIGNAL_HELD role=host reason=unknown_peer`, `ICE_SIGNAL_REJECTED role=host reason=unknown_peer`,
  `ICE_SIGNAL_REJECTED role=host reason=held_signals_full`,
  `ICE_SIGNAL_EXPIRED role=host reason=unknown_peer count=<n>`, `ICE_SIGNAL_REPLAYED role=host peer=<id> count=<n>`
- `SIGNAL_DROPPED reason=outbox_full`
- `ROSTER_ADD peer=<id> roster=<n>`, `ROSTER_REMOVE peer=<id> roster=<n>`, `ROSTER_ADD_REJECTED reason=<code>`,
  `ROSTER_REMOVE_IGNORED reason=unknown_account`

Admission:

- `ADMISSION_ACCEPTED role=host peer=<host peer id> source=local` (the host's own client; `rifts-host` looks for
  the tokens `role=host` and `source=local`)
- `ADMISSION_ACCEPTED role=<host|join> peer=<id> source=ice route=<relay|direct>`
- `ADMISSION_REJECTED role=host peer=<id> code=<code ≤ 64>`, `ADMISSION_REJECTED role=join peer=<host id> code=<code>`

Lease and Guard (§3): `HOST LEASE FAIL: …`, `HOST LEASE WARN: …`, `HUNT GUARD OBSERVE WARN: …`,
`HUNT GUARD OBSERVE: …`, `HUNT GUARD COMPLETION WARN: …`, `GUARD_EVIDENCE_LOSS action=completion_fenced reason=…`.

UDP:

```text
UDP_STATS role=<host|join> path=<local|remote> localRx=<n> localTx=<n> transportRx=<n> transportTx=<n> oversized=<n> foreign=<n> unknownPeers=<n> backpressure=<n> socketErrors=<n> admitted=<n> rejected=<n> disconnected=<n> signalsRejected=<n> guardPublishFailures=<n> guardEvidenceLoss=<n>
```

(the first ten numbers are the bridge's counters, §2.16; the rest the session's, §2.7.7; `rifts-host` reads
`localRx` to detect an idle game), `LOCAL_CLIENT_REPINNED role=host path=local`,
`HOST_UDP_SOCKET_ERROR path=local op=<client_recvfrom|client_send|server_recv|server_sendto> wsa=<os error code>`.

### 2.5 The host run

#### 2.5.1 Start (in order; the first failure ends the run)

| Step | Failure → diagnostic / failure code, result |
| --- | --- |
| 1. Read config (1..16384) and secrets (0..16384); parse both; parse ICE material | `HOST CONFIG FAIL: <error>`, 2 |
| 2. `session_role` | `HOST CONFIG FAIL: invalid session_role`, 2 |
| 3. `host_local_client` | `HOST CONFIG FAIL: invalid host_local_client`, 2 |
| 4. Bindings, `backend_url`, `server_udp_port`, `maximum_peers`, guard pipe keys (when required), capability file (≤ 128 bytes), host ticket file (≤ 4096; when `host_local_client=1`; a missing path gives `a host ticket is required unless host_local_client=0`). A ticket path given to a host with `host_local_client=0` is ignored | `HOST CONFIG FAIL: <last error>`, 2 |
| 5. More `peer=` lines than `maximum_peers` | `HOST CONFIG FAIL: more transport peers than maximum_peers`, 2 |
| 6. `force_relay` and at least one `peer=` line and no relay server | failure `host:relay_unavailable`, 3 |
| 7. Load the admission public key (§2.13.1) | `HOST KEY FAIL: <error>`, 3 |
| 8. Create the backend client (§3.1) with `backend_url` and the capability text | `HOST BACKEND FAIL: <error>`, 3 |
| 9. Socket layer start, two backend lanes (§3.2: lease lane capacity 2, work lane capacity 256), the Guard publisher when `hunt_evidence` (§3.3) | panic → `SESSION_FAILURE code=exception`, 3 |
| 10. Build the host session: `host_peer_id ≠ 0`, `virtual_port ≠ 0`, peers ≤ `maximum_peers`, each peer valid (§2.7.1) | `HOST SESSION FAIL: invalid session configuration`, 3 |
| 11. Create the guard pipe when required (§3.4) | `HOST GUARD PIPE FAIL: <error>`, 3 |
| 12. Create the remote UDP bridge (§2.16.2) with limits 2048 bytes / 64 per pump / `maximum_peers` | panic → 3 |
| 13. Host-ready call (§3.1.2), request id `ready-<generation>`, made in place | `HOST BACKEND FAIL: <error>`, 3 |
| 14. When `host_local_client=1`: verify the host ticket (§2.13) expecting joiner = host = `host_peer_id` | failure `host:<ticket code>`, 4 |
| 15. … then consume it in place (§3.1.1): connection handle `localhost`, request id `host-local-<generation>`, authenticated id `host_peer_id`. The ticket text is wiped. The jti is **not** added to the local consumed set | failure `host:<consume code>` (`central_consume_failed` when empty), 4 |
| 16. … diagnostic `ADMISSION_ACCEPTED role=host peer=<host_peer_id> source=local`; the host account is the ticket's account (empty without a local client) | — |
| 17. Backend calls of the session now go through the work lane. Guard events `SESSION_STARTED` `{"source":"ice_transport"}` and, if that was taken, `WORLD_READY` `{"source":"launcher_ready_pipe"}`, both with the host account as actor | not taken → `HUNT GUARD OBSERVE WARN: <error>` (§3.3.6), continues |
| 18. When `host_local_client=1`: the local bridge (§2.16.3), client port 0 (any), limits 2048 / 64 / 1 | panic → 3 |
| 19. Report the roster limit and starting peers to the API layer (§2.1.4) | — |
| 20. Diagnostic `ICE_READY role=host …`; ready line `HOST_READY …` | — |

Without hunt evidence, step 17 sends nothing and reports success.

#### 2.5.2 The loop

Until the stop flag is set, each iteration, in this order:

1. Apply queued roster commands in order (§2.7.2). A remove of an absent account → `ROSTER_REMOVE_IGNORED
   reason=unknown_account`. A refused add → `ROSTER_ADD_REJECTED reason=<code>` and the peer event
   `PEER_ADD_REJECTED account=<a> reason=<code>`, and the API view drops the account unless the code is
   `duplicate_account`. Commands come before signals so a description queued with its add finds the peer.
2. Apply incoming signals in order (§2.7.3).
3. Pump the session (§2.7.4).
4. Pump the guard pipe (§3.4). If it dropped events since the last take, queue the Guard event `EVIDENCE_LOSS` with
   no actor and payload `{"droppedEvents":<n>,"reason":"bridge_queue_overflow"}`. Then queue every pipe event. If
   any of these was not taken, the first time only: `HUNT GUARD OBSERVE WARN: <error> (completion fenced until
   this hunt ends)`.
5. For each peer the session disconnected, free its remote bridge slot.
6. Pump the remote bridge, then the local bridge.
7. Every 2 s (first time at once): with the local bridge, `UDP_STATS role=host path=local …`; always
   `UDP_STATS role=host path=remote …`. The Guard counters come from the publisher (0 without one).
8. Lease (§2.5.3).
9. Sleep 1 ms.

#### 2.5.3 The lease

A tracker starts "renewed" at loop start. Rule: `Renewed` resets the refusal count and the renewal time; `Rejected`
fails the lease on the 2nd refusal in a row; `Unavailable` resets the refusal count and fails the lease once 30 s
have passed since the last renewal (`≥ 30 s`).

Each iteration, if an answer has come back from the lease lane: clear "in flight", re-arm the "waiting" warning, and
record it. Recorded failure → diagnostic `HOST LEASE FAIL: the backend refused the lease (<error>)` for a refusal or
`HOST LEASE FAIL: central authority unavailable for <s> s (<error>)` otherwise (`<s>` = whole seconds since the
last renewal), failure `host:lease_failed`, leave the loop with "lease failed". A recorded non-renewal that does not
fail: `HOST LEASE WARN: <error>`, followed by ` (waiting; last renewal <s> s ago)` for `Unavailable`.

Every 2 s (first beat at once): if a renewal is still in flight, record `Unavailable` (fail →
`HOST LEASE FAIL: central authority unavailable for <s> s (no answer to the renewal)`, `host:lease_failed`, leave),
and on the first such beat since the last answer `HOST LEASE WARN: the last renewal has not been answered yet (last
renewal <s> s ago)`. Otherwise post a renewal to the lease lane: sequence `n` (1, 2, …), request id
`lease-<generation>-<n>`, admitted peer count, queue pressure 0, bridge alive true, server ready true. A lane that
refuses the job leaves nothing in flight.

#### 2.5.4 Shutdown

1. Stop the session (§2.7.6): every link closed with reason `host_stopped`.
2. With a publisher whose completion was not accepted: queue `HUNT_ABORTED` with the host account and payload
   `{"reason":"LEASE_FAILED"}` (lease failed) or `{"reason":"HOST_STOPPED"}`; not taken →
   `HUNT GUARD OBSERVE WARN: <error>`. It is queued after the `PLAYER_LOST` events, so it is last in the chain.
3. Publisher: flush and stop, waiting up to 3000 ms. Work lane: drain and stop, up to 3000 ms. Lease lane: stop
   at once.
4. Settle late consumes in place and send the remaining disconnect notices in place (§2.7.5).
5. Result 5 when the lease failed, else 0.

### 2.6 The join run

| Step | Failure → diagnostic / failure code, result |
| --- | --- |
| 1. Read config and secrets (both 1..16384), parse, ICE material | `JOIN CONFIG FAIL: <error>`, 2 |
| 2. Bindings, `local_udp_port`, `local_peer_id`, `local_account_id`, `host_account_id`, ticket file (≤ 4096) | `JOIN CONFIG FAIL: <last error>`, 2 |
| 3. Exactly one `peer=` line, naming `host_account_id` with peer id `host_peer_id` | `JOIN CONFIG FAIL: the transport secrets must name exactly the host`, 2 |
| 4. `force_relay` with no relay server | failure `join:relay_unavailable`, 3 |
| 5. Load the public key | `JOIN KEY FAIL: <error>`, 3 |
| 6. Build the joiner session: host account non-empty, host peer id = `host_peer_id` ≠ 0, local peer id ≠ 0 and ≠ host's, local account and ticket non-empty, `virtual_port ≠ 0` | `JOIN SESSION FAIL: invalid session configuration`, 3 |
| 7. Start (§2.8.1) | `JOIN SESSION FAIL: <error>` and failure `join:transport_start_failed`, 3 |
| 8. Diagnostic `ICE_READY role=join …`; the joiner bridge (§2.16.1) on 127.0.0.1:`local_udp_port` | bind failure → panic → 3 |
| 9. Loop until stopped, admitted or failed: apply signals, pump (§2.8.2), sleep 2 ms | — |
| 10. Not admitted: failed → failure `join:<code>`, 4; stopped → 0 | — |
| 11. Ready line `ROUTE_READY …` (`route` from the selected pair at this moment) | — |
| 12. Loop until stopped or failed: apply signals, pump, pump the bridge, every 2 s (first at once) `UDP_STATS role=join path=remote …`, sleep 1 ms | — |
| 13. Failed → failure `join:<code>`, 5; stopped → 0 | — |

On exit the link is stopped and the peer detached. A joiner sends no backend calls.

### 2.7 The host session

#### 2.7.1 Peers and links

A peer is (account, peer id, 32-byte pair key) plus at most one link. Valid: account 1..64 bytes, peer id ≠ 0 and
≠ `host_peer_id`; no other peer with the same account (`duplicate_account`) or peer id (`duplicate_peer`); otherwise
`invalid_peer`. Each link opened gets a **link index** from a per-session counter starting at 1 that never repeats
(it counts links, not accounts, so rejoins get new indexes).

The host creates a peer's link only when that peer's `DESCRIPTION` arrives, giving it the description first (the
host is the controlled side, §2.9.1). Opening counts a connection attempt; a start error →
`ICE_LINK_FAILED role=host peer=<id> reason=<error>` and the signal counts as rejected.

#### 2.7.2 Roster changes

**Add**: validate (codes above); a roster at `maximum_peers` → `full`. Else add, diagnostic
`ROSTER_ADD peer=<id> roster=<count>`, then replay held signals (§2.7.3).

**Remove**: drop the account's held signals. Absent → false. Else sanitise the reason: its first 48 bytes, every
byte outside `A-Za-z0-9_-.` replaced by `_`; empty → `removed`. Close the link telling the backend (§2.7.5), peer
event `PEER_REMOVED account=<a> reason=<token> admitted=<0|1>`, wipe the key, remove the peer, diagnostic
`ROSTER_REMOVE peer=<id> roster=<count>`. The same account may be added again later.

#### 2.7.3 Signals in

From an account **not on the roster**: hold it (the joiner may have started before the host's launcher added it).

- At most 64 accounts may have held signals. A signal from a 65th account → rejected count +1,
  `ICE_SIGNAL_REJECTED role=host reason=unknown_peer`.
- At most 16 per account. Beyond → rejected +1 and, once per account entry, `ICE_SIGNAL_REJECTED role=host
  reason=held_signals_full`.
- The first signal held for an account (its list was empty) → `ICE_SIGNAL_HELD role=host reason=unknown_peer`.
- Each pump drops held signals 20 s old or older (`≥ TTL`): per account with any expired,
  `ICE_SIGNAL_EXPIRED role=host reason=unknown_peer count=<n>`, rejected += n; empty entries are removed.
- On add, the account's held signals are replayed in arrival order through the normal path; ones already expired
  are counted rejected silently; if any were replayed: `ICE_SIGNAL_REPLAYED role=host peer=<id> count=<n>`.

From a peer on the roster:

- `DESCRIPTION`: if the peer has a link, close it telling the backend with reason `restarted` (the joiner started
  over), then open a new link with this description.
- Any other kind with no link: rejected +1, no diagnostic.
- Otherwise apply it to the link (§2.10.2); an error → rejected +1,
  `ICE_SIGNAL_REJECTED role=host peer=<id> reason=<error>`.

#### 2.7.4 Pump

1. Settle consumes that came back (§2.7.5).
2. Expire held signals.
3. For each peer with a link:
   - forward the link's outgoing signals with `to` = the peer's account;
   - a link that is closing: when its close time has come, close it **without** telling the backend, reason
     `rejected`; skip the rest;
   - first time Connected: admission deadline = now + 60 s, `ICE_ROUTE role=host peer=<id> route=<r>`,
     `PEER_CONNECTED account=<a> route=<r>`;
   - Failed: close telling the backend, reason `link_lost` if it had connected else `connect_failed`; skip;
   - not yet connected: skip;
   - handle up to 64 received frames: Gameplay from an admitted peer goes to the router (§2.15), else counts
     `pre_admission_gameplay_dropped`; Control that does not decode as an admission request counts
     `malformed_control`; from an admitted peer, resend the stored admission result and never consume again; while
     closing or while this link's consume is with the backend, ignore; otherwise admit (below);
   - not admitted and past the deadline: `authentication_timeouts` +1, reject `admission_request_timeout`.
4. Flush disconnect notices.

**Reject(code)**: send an admission result (admitted 0, code, empty id) when it encodes; `admission_rejected` +1;
`ADMISSION_REJECTED role=host peer=<id> code=<code, first 64 bytes>`; mark closing, close in 1 s (the result gets a
moment to leave).

**Admit(ticket)**, in order:

1. `force_relay` and the selected pair is not relayed → `ICE_ROUTE_REFUSED role=host peer=<id> local=<t> remote=<t>`,
   reject `transport_not_relayed`.
2. Verify the ticket (§2.13) expecting joiner = this peer's id, host = `host_peer_id`; failure → reject `<code>`.
3. The ticket's account must equal the peer's account (the pair key proves who is on the link) → else reject
   `wrong_account`. Nothing was consumed.
4. Consume (§3.1.1): connection handle `ice<index>`, request id `adm-<generation>-ice<index>`, authenticated id =
   the peer's id. With the work lane: post it; a full lane → reject `central_consume_busy`; else remember the
   pending consume (account, peer id, link index, jti) under a token and wait. Without a lane (unit tests): in place.

**Complete(consume answer)**: not admitted, or an admission id that is not 64 characters → reject `<code>` or
`central_consume_failed`. The jti already in the local consumed set → queue a disconnect notice with request id
`disc-replay-<generation>-ice<index>` and reject `local_replay`. Otherwise: admitted; store the id and the encoded
result (admitted 1, `ok`, id); attach the link to the router; send the result; `admitted` +1;
`ADMISSION_ACCEPTED role=host peer=<id> source=ice route=<r>`; `PEER_ADMITTED account=<a> route=<r>`; Guard event
`PLAYER_ADMITTED` actor = account, payload `{"transport":"ice"}`.

#### 2.7.5 Late consumes and disconnect notices

Settling: for each answer whose token is pending, if the peer still has that link (same index, same token) and is
not closing, complete it. Otherwise (the link closed, was replaced or was refused meanwhile): if the backend
admitted it with a 64-character id, add the jti to the consumed set and queue a disconnect notice with the default
request id. The joiner is not admitted.

**Close(link, tell backend, reason)**: if admitted, detach from the router, report the peer id to the bridge (its
slot is freed), queue a disconnect notice when telling the backend and an admission id exists, `disconnected` +1,
Guard `PLAYER_LOST` actor = account, payload `{"transport":"ice"}`, peer event `PEER_LOST account=<a>
reason=<reason, ≤ 48 bytes>`. Then stop and drop the link and clear its admission state; a consume still out is
settled later as above.

**Flush**: each queued notice (in order) becomes a disconnect call (§3.1.6) with request id
`disc-<generation>-<peer id>-<link index>` unless it carries its own. Posted to the work lane; a full lane keeps the
rest for the next pump. Without a lane, in place.

#### 2.7.6 Stop

Close every peer that has a link or is admitted, telling the backend, reason `host_stopped`; clear held signals;
flush.

#### 2.7.7 Session counters

`connection_attempts`, `admitted`, `admission_rejected`, `authentication_timeouts`, `malformed_control`,
`pre_admission_gameplay_dropped`, `disconnected`, `signals_rejected` (plus the publisher's `guard_publish_failures`,
`guard_evidence_loss` in the stats line).

### 2.8 The joiner session

#### 2.8.1 Start

Verify the joiner's own ticket expecting joiner = `local_peer_id`, host = `host_peer_id` → failure
`join ticket verification failed: <code>`; the ticket's account must be `local_account_id` →
`join ticket belongs to another account`. Create the link (controlling side, no remote description); a start error is
returned as is. Count a connection attempt. Deadline = now + 60 s. The link's `DESCRIPTION` is the first signal out.

#### 2.8.2 Pump

1. Forward the link's outgoing signals to the host account (even after failing).
2. Failed already → stop here.
3. Link Failed: if admitted → detach, failed with `connection_closed`, `disconnected` +1, peer event
   `PEER_LOST account=<host> reason=link_lost`; else fail `connection_closed_before_admission` (had connected) or
   `transport_connect_failed`.
4. First time Connected: route from the selected pair; `force_relay` and direct → fail `transport_not_relayed`;
   else `ICE_ROUTE role=join peer=<host id> route=<r>`, `PEER_CONNECTED account=<host> route=<r>`.
5. Up to 64 frames: Gameplay → router when admitted, else `pre_admission_gameplay_dropped`; Control after admission
   ignored; not an admission result → `malformed_control`; admitted 0 → fail `<code>` (or `admission_rejected` when
   empty); admitted 1 → attach, wipe the ticket, admitted, `admitted` +1,
   `ADMISSION_ACCEPTED role=join peer=<host id> source=ice route=<r>`, `PEER_ADMITTED account=<host> route=<r>`.
6. Admitted → done. Past the deadline → `authentication_timeouts` +1, fail `admission_response_timeout` (had
   connected) or `transport_connect_timeout`.
7. Connected and the resend time has come (first at once) → send the admission request; next in 750 ms. An
   unencodable request → fail `admission_send_failed`.

**Fail(code)** (once): failed, `admission_rejected` +1, `ADMISSION_REJECTED role=join peer=<host id> code=<code>`.

#### 2.8.3 Signals in

From an account other than the host's, or with no link: rejected +1, silent. A `DESCRIPTION` when the link already
has the host's description: the host replaced its link; if not admitted, fail `transport_restarted`; ignored
otherwise. Else apply; error → rejected +1, `ICE_SIGNAL_REJECTED role=join reason=<error>`.

### 2.9 Signalling and the ICE link, independent of libjuice

#### 2.9.1 Roles and ordering

- Signal kinds: `DESCRIPTION` (ICE credentials), `CANDIDATE` (one local candidate), `GATHERING_DONE` (data `-`).
- The **joiner speaks first**: it creates its agent as the **controlling** side, emits its `DESCRIPTION` at once
  (before any candidate), then one `CANDIDATE` per gathered candidate (trickle) and `GATHERING_DONE`.
- The host creates its agent only when the joiner's description arrives and applies it **before** producing its own,
  so its agent is the **controlled** side and no role conflict arises. Its `DESCRIPTION` and candidates follow.
- Full ICE (RFC 8445), trickle (RFC 8838), one component, UDP only. Nomination is the controlling side's.
- The launcher carries signals (`POST /launcher/v1/p2p/hunts/:sessionId/signal`, the `ICE_SIGNAL` control message);
  nothing in the DLL talks to the signalling server.

#### 2.9.2 Server URLs

- STUN: `stun:<host>[:<port>]`. TURN: `turn:<host>[:<port>]` optionally followed by exactly `?transport=udp`; any
  other query, `turns:`, TCP → refused.
- `<host>` 1..253 bytes of `A-Za-z0-9.-` (no IPv6 literal, no space); `<port>` decimal 1..65535 after the **last**
  colon; absent → 3478.
- One STUN server at most, up to 4 TURN servers sharing one username/credential (long-term credentials, RFC 8656
  over UDP only).

#### 2.9.3 Description and candidate text (interop with the C++ DLL)

The text must stay compatible with libjuice v1.7.4, because a Rust peer will meet C++ peers during roll-out (§5.5).

- A description is CRLF-terminated SDP attribute lines: `a=ice-ufrag:<ufrag>`, `a=ice-pwd:<pwd>` and
  `a=ice-options:<options>`. The C++ side emits it before gathering, so it holds no candidates
  (unverified detail: libjuice writes `a=ice-options:ice2,trickle`; check against the oracle).
- A candidate is one line `a=candidate:<foundation> 1 UDP <priority> <address> <port> typ <host|srflx|prflx|relay>
  [raddr <address> rport <port>]`. The receiver accepts the `a=candidate:` and the bare `candidate:` prefix.
- `GATHERING_DONE` carries `-`; its data is ignored on receipt.
- A received description must give the agent the remote ufrag and pwd; a received description may also carry
  candidate lines and `a=end-of-candidates`, which must be honoured.

#### 2.9.4 Relay-only filtering (`force_relay=true`)

Applied to every outgoing description and candidate, and to every incoming description and candidate before it
reaches the agent:

1. Split on LF; trim trailing CR and spaces and leading spaces; skip empty lines.
2. A candidate line (prefix `a=candidate:` or `candidate:`) is kept only when it contains ` typ relay` followed by
   end of line or a space.
3. In a description, also keep `a=ice-ufrag:`, `a=ice-pwd:`, `a=ice-options:` lines and `a=end-of-candidates`;
   drop everything else.
4. `CANDIDATE` result: the kept line if exactly one was kept, else empty. `DESCRIPTION` result: empty if no
   `a=ice-ufrag:` line was kept, else every kept line followed by CRLF. `GATHERING_DONE`: its data, or `-` if empty.

An empty outgoing candidate is not sent. An incoming candidate filtered to empty is accepted and ignored (not an
error). An incoming description filtered to empty is refused with an **empty** error text (see §7).

Relay-only also requires at least one TURN server at link start (`relay-only needs a relay server`). Host and
server-reflexive candidates may still be gathered locally, but they never leave the DLL, and a remote non-relay
candidate never reaches the agent. "Relayed" means the selected pair's local **or** remote candidate is of type
`relay`; the host still checks it at admission and the joiner at connection.

#### 2.9.5 Link start and errors

| Condition | Error text |
| --- | --- |
| Pair key not 32 bytes | `pair key must be 32 bytes` |
| Cipher could not be set up | `AES provider unavailable`, `AES-GCM unavailable`, `pair key could not be loaded`, `random number generator unavailable` |
| Relay-only without TURN | `relay-only needs a relay server` |
| Agent creation | `ICE agent creation failed` |
| Host: given description filters to empty, exceeds 4096 bytes or is refused | `remote description rejected` |
| Local description not available | `ICE local description unavailable` |
| Local description filters to empty | `ICE local description is empty` |
| Gathering does not start | `ICE candidate gathering failed to start` |

The cipher texts are the C++ ones and appear only in `ICE_LINK_FAILED` / `JOIN SESSION FAIL` lines; a Rust
implementation using a pure-Rust AEAD will practically never produce them.

#### 2.9.6 Link state

States New → Gathering (after start) → Connecting → Connected → Failed, and Closed after stop. The agent's
"connected" and "completed" both map to Connected; its "failed" to Failed; its initial "disconnected" is ignored.
A connected link that loses consent must eventually report Failed (the C++ DLL relies on libjuice's internal
timers; it sets none itself — the exact connectivity-check and consent timeouts are libjuice v1.7.4 defaults,
**unverified** here; RFC 7675's 30 s consent loss is an acceptable choice). The only DLL-level timeouts are the 60 s
admission deadlines (§2.7.4, §2.8.2).

#### 2.9.7 Outgoing signal queue of a link

At most 64 signals wait in a link; more are dropped silently.

### 2.10 Link data path

#### 2.10.1 Send

Refused (counted `send_failures`) when the link is not Connected, the type is Fragment or the payload is empty.
Payloads up to 1100 bytes go as one sealed frame of their type. Larger ones are split into `ceil(n/1100)` pieces of
1100 bytes (the last shorter); more than 4 pieces → refused. Each piece is sealed as a Fragment frame whose
plaintext is:

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 1 | Inner type (1 Control or 2 Gameplay) |
| 1 | 2 | Message id, big-endian; a per-link u16 counter starting at 0, wrapping |
| 3 | 1 | Piece index, 0-based |
| 4 | 1 | Piece count, 2..=4 |
| 5 | n | Piece bytes |

1100-byte pieces keep each datagram under 1280 bytes with the 28-byte seal, TURN and IP/UDP overhead (a relay run
lost every 1400- and 2000-byte datagram). The send reports failure if any piece failed.

#### 2.10.2 Remote signals into a link

| Condition | Result |
| --- | --- |
| Link closed | error `link is closed` |
| Data over 4096 bytes | error `signal too large` |
| Relay-only, not `GATHERING_DONE` | filter first (§2.9.4) |
| `DESCRIPTION` when one is already set | error `remote description already set` |
| `DESCRIPTION` refused by the agent | error `remote description rejected` |
| `DESCRIPTION` accepted | then apply the early candidates in order (their errors ignored) and an early gathering-done |
| `CANDIDATE` before the description | kept as early, up to 32; more → `too many early candidates` |
| `CANDIDATE` refused by the agent | `remote candidate rejected` |
| `GATHERING_DONE` | applied, or remembered until the description |
| Other kind | `unknown signal kind` |

#### 2.10.3 Receive

Every datagram from the agent is opened (§2.11); a failure counts `frames_rejected` and is dropped. A Fragment is
reassembled:

- Plaintext must be longer than 5 bytes; inner type 1 or 2, count 2..=4, index < count, else `frames_rejected`.
- Partial messages older than 1 s (strictly more) are discarded on each piece's arrival.
- At most 32 partial messages; a new one evicts the oldest (`queue_drops` +1).
- A piece whose message id exists with a different count or inner type restarts that message.
- A repeated piece index is ignored. When all pieces have arrived, they are concatenated in index order and
  delivered as one message of the inner type.

Delivered messages wait in a queue of at most 512 (more → `queue_drops` +1, dropped). A lost piece loses the
message, as IP fragmentation would.

### 2.11 The sealed frame

#### 2.11.1 Pair key

Exactly 43 base64url characters (`A-Za-z0-9-_`, no padding) decoding to 32 bytes; the two unused low bits of the
last character must be zero.

#### 2.11.2 Layout

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 2 | Magic `MX` (0x4D 0x58) |
| 2 | 1 | Version 1 |
| 3 | 1 | Type: 1 Control, 2 Gameplay, 3 Fragment |
| 4 | 8 | Counter, big-endian |
| 12 | n | AES-256-GCM ciphertext, 1..=65536 bytes |
| 12+n | 16 | GCM tag |

- AAD: the 12-byte header.
- Nonce (12 bytes): a 4-byte direction prefix then the counter big-endian. Host→joiner prefix `48 4A 00 01`
  (`HJ\0\x01`), joiner→host `4A 48 00 01` (`JH\0\x01`). A host seals with HJ and opens with JH; a joiner the reverse.
- Overhead 28 bytes.

#### 2.11.3 Counter and replay window

- Each cipher picks a random start `s` in 1..=2^63−1 (uniform 64-bit draw with the top bit cleared, redrawn while 0).
  The first frame sealed carries `s+1`, then `s+2` …. Sealing fails at u64::MAX, for an empty payload or one over
  65536 bytes. (A rejoin reuses the pair key, so a fixed start would reuse nonces.)
- Opening refuses: size ≤ 28 or > 28+65536, wrong magic, version or type, counter 0, a counter `highest − counter ≥
  1024` (checked before decrypting), a failed tag, a counter already seen in the window.
- Window: the highest accepted counter `H` (starts 0, so any first counter is taken) and a 1024-bit bitmap of
  `H, H−1, … H−1023`. A larger counter shifts the bitmap (≥ 1024 clears it) and becomes `H`; a smaller one inside
  the window is taken once. The window is updated only after the tag verifies.
- Sealing may be called from the session thread while opening runs on the network side; each needs its own lock.

### 2.12 MPC1 control frames

Inside sealed Control frames.

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 4 | `MPC1` |
| 4 | 1 | Version 1 |
| 5 | 1 | Type: 1 admission request, 2 admission result |
| 6 | 2 | Reserved, 0 |
| 8 | 4 | Payload length, big-endian, equal to the remaining bytes, > 0 |
| 12 | n | Payload |

Whole frame ≤ 4608 bytes. Decoding requires every header field exactly (type matching the expected one).

- Request payload: u16 big-endian ticket length (1..=4096) then the ticket bytes; payload length must be exactly
  2 + length (and ≥ 3).
- Result payload: `admitted` (0 or 1), code length (1..=64), admission id length (0..=64), reserved 0, then the code
  and the id. Payload length exactly 4 + both lengths (≥ 5). `admitted=1` requires a 64-byte id. Encoding refuses an
  empty or over-64-byte code or an over-64-byte id.

### 2.13 MPT2 admission tickets (verification)

#### 2.13.1 Public key file

A regular, non-symlink file of exactly 64 bytes: the P-256 public point X‖Y (32 bytes each, big-endian), no
prefix. Errors: `admission public key must be a regular 64-byte raw X||Y file`, `could not read admission public
key`. The launcher names it `p2p-admission-public.xy`.

#### 2.13.2 Token text

`mpt1.<payload>.<signature>`, at most 4096 characters, exactly two dots. Each part is unpadded base64url: only
`A-Za-z0-9-_`, length mod 4 ≠ 1, unused trailing bits zero. The payload decodes to at most 2048 bytes; the signature
to exactly 64 bytes (r‖s). Anything else → `malformed`. The signature is ECDSA P-256 over SHA-256 of the decoded
payload bytes; failure → `bad_signature` (checked before the payload is parsed).

#### 2.13.3 Payload (big-endian; 106 fixed bytes)

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 4 | `MPT2` |
| 4 | 1 | Version, must be 2 |
| 5 | 1 | Key id length |
| 6 | 2 | Account id length |
| 8 | 2 | Hunt definition id length |
| 10 | 2 | Party id length |
| 12 | 2 | Bridge protocol (≠ 0) |
| 14 | 2 | Virtual port (1..=999) |
| 16 | 2 | Reserved, 0 |
| 18 | 4 | Generation (≠ 0) |
| 22 | 4 | Build (≠ 0) |
| 26 | 8 | Party revision (≠ 0) |
| 34 | 8 | Issued at, Unix s |
| 42 | 8 | Not before |
| 50 | 8 | Expires at |
| 58 | 8 | Joiner peer id (≠ 0) |
| 66 | 8 | Host peer id (≠ 0) |
| 74 | 16 | Session id (UUID bytes, not all zero) |
| 90 | 16 | jti (not all zero) |
| 106 | … | Key id, account id, hunt definition id, party id (the lengths above), then nothing |

Text rules: key id safe token (32, `._-`), account safe token (96, `._:-`), hunt definition id (128, `._:/-`), party
id (192, `._:+/=-`). The session id is rendered as lowercase hex `8-4-4-4-12`. Any violation → `malformed`.

#### 2.13.4 Checks, in order (first failure wins)

`malformed` → `bad_signature` → `malformed` (payload) → `bad_lifetime` (expires < issued, not-before < issued, or
expires − issued > 180) → `not_yet_valid` (issued or not-before > now + 30) → `expired` (now > expires + 30) →
`wrong_key` → `wrong_session` → `wrong_generation` → `wrong_party` (party id or revision) → `wrong_hunt` →
`wrong_joiner` → `wrong_host` → `wrong_build` → `wrong_protocol` (bridge protocol or virtual port) → `replay` (jti in
this session's consumed set). The expected values come from the bindings (§2.3).

The consumed set is per session, in memory; a jti enters it only when a joiner's admission completes (or a late
consume is settled, §2.7.5).

### 2.14 Admission over the link

Joiner: the MPC1 request every 750 ms from connection until a result arrives or the 60 s deadline. Host: one consume
per link; repeated requests while the consume is out are ignored, after admission answered with the stored result.
Gameplay before admission is dropped on both sides. Codes a joiner can receive from a host are listed in §2.17.

### 2.15 The router between links and bridges

Per session: peer id → link, for admitted peers only. Delivering a gameplay message from a peer that is not attached,
or an empty one, counts as dropped. The inbox holds at most 256 × (attached + 1) messages and
1 MiB × (attached + 1) bytes; beyond → dropped. Detaching a peer also removes its queued messages. Sending to an
unattached peer or an empty payload fails. Reads drain up to the requested count in arrival order.

The in-memory test network (from `message_transport`) is a test double with per-peer limits of 256 messages and
1 MiB, refusing peer id 0, duplicate endpoints and empty payloads.

### 2.16 UDP bridges

All sockets are IPv4 UDP, non-blocking, bound to 127.0.0.1. Receive buffers are `max datagram + 1` (2049) bytes so an
oversize datagram is detected; zero-length datagrams are skipped but use up the per-pump budget. A send whose byte
count differs counts `socket_errors`. A receive error other than "would block" counts `socket_errors` and ends that
receive loop (on Windows an ICMP port-unreachable shows as a reset error here).

Counters (§2.4.3 order): `local_datagrams_received`, `local_datagrams_sent`, `transport_messages_received`,
`transport_messages_sent`, `oversized_datagrams_dropped`, `unexpected_local_sources_dropped`,
`unknown_peers_dropped`, `transport_backpressure_drops`, `socket_errors`.

#### 2.16.1 Joiner bridge (limits 2048 / 64 / 1)

Bound to 127.0.0.1:`local_udp_port` (0 = any; the bound port is the ready line's `localPort`). Per pump:

1. Up to 64 receives from the game client. Count received; oversize → dropped. The first sender must come from
   address 127.0.0.1 exactly and is pinned (address and port) for the session; any other sender → `foreign`. Send
   to the host peer through the router: ok → `transportTx`, refused → `backpressure`.
2. Up to 64 messages from the router: count; a source other than the host peer → `unknownPeers`; empty or > 2048 →
   oversized; no pinned client yet → `foreign`; else send to the pinned client.

#### 2.16.2 Host remote bridge (limits 2048 / 64 / `maximum_peers`)

One socket per joiner, made on the first message from that peer: bound 127.0.0.1:0 and connected to
127.0.0.1:`server_udp_port`, so the game server sees one source port per joiner. At most `maximum_peers` sockets; a
peer beyond that, peer id 0 or a socket that cannot be made → `unknownPeers` (a failed make also counts a socket
error). A disconnected peer's socket is closed and its slot freed. Per pump:

1. Up to 64 messages from the router: empty or > 2048 → oversized; else send on that peer's socket.
2. Up to 64 receives in total across the peer sockets (in map order): oversize → dropped; else send to that peer
   through the router.

#### 2.16.3 Host local bridge (the host's own client; limits 2048 / 64 / 1)

Two sockets: the client socket bound 127.0.0.1:0 (its port is `HOST_READY`'s `localPort`) and a server socket bound
127.0.0.1:0 and connected to 127.0.0.1:`server_udp_port`. Per pump:

1. Up to 64 receives on the client socket: oversize → dropped. The first sender must be 127.0.0.1 and is pinned. A
   different sender is taken over only when it is from 127.0.0.1, the pinned client has been silent for at least
   2 s, and a fresh server socket (new source port) could be made; then `LOCAL_CLIENT_REPINNED role=host
   path=local`. Otherwise → `foreign`. Forward to the server socket.
2. Up to 64 receives on the server socket: nothing pinned yet → skipped; oversize → dropped; else send to the pinned
   client.

Socket errors here also produce `HOST_UDP_SOCKET_ERROR path=local op=<op> wsa=<code>` at most 8 times per bridge,
skipping one identical (same op and code) to the previous reported one.

### 2.17 Result and failure codes

| Result | Meaning |
| ---: | --- |
| 0 | Clean stop (host stopped by the caller, joiner stopped before or after admission) |
| 2 | Bad input (config, secrets, tickets files, field rules) |
| 3 | Setup failure (key, backend client, host-ready, session build, guard pipe, relay unavailable, transport start, panic) |
| 4 | Refused admission (host's own ticket or consume; joiner refused or timed out before admission) |
| 5 | Host: lease failed. Joiner: the link failed after admission (`connection_closed`) |
| -1 | (API) unknown handle |

Failure codes (`SESSION_FAILURE code=`): `host:relay_unavailable`, `host:<ticket code>`, `host:<consume code>`,
`host:central_consume_failed`, `host:lease_failed`, `join:relay_unavailable`, `join:transport_start_failed`,
`join:<joiner failure code>`, `exception`.

Codes a host sends in an admission result: the ticket codes of §2.13.4, `wrong_account`, `transport_not_relayed`,
`central_consume_busy`, `central_consume_failed`, the consume codes of §3.1.1 other than `ok`, `local_replay`,
`admission_request_timeout`. Joiner failure codes: any of those, `admission_rejected`, `transport_restarted`,
`connection_closed`, `connection_closed_before_admission`, `transport_connect_failed`, `transport_not_relayed`,
`admission_response_timeout`, `transport_connect_timeout`, `admission_send_failed`.

---

## 3. The backend side (host only)

### 3.1 The admission client

#### 3.1.1 Transport

- Base URL: scheme, host, optional port and path; no user, password, query or fragment. `https` required, except
  `http` when the host is exactly `127.0.0.1` or `localhost` (ae3deaa). For `https` to those two hosts, an unknown
  CA, a name mismatch and an expired certificate are accepted. Trailing `/` of the base path is dropped; every route
  is appended to it.
- Every call is `POST` with headers `Content-Type: application/json`, `X-Mystic-Host-Capability: <capability>` and
  `X-MysticParadox-Request-Id: <request id>`; user agent `ParadoxTransport/1`; the system's automatic proxy setting;
  redirects not followed; 5 s timeouts for resolve, connect, send and receive; a fresh connection per call.
- The capability (from the capability file) is exactly 43 characters of `A-Za-z0-9_-`; it is wiped when the client
  is dropped.
- Before sending: route path safe token (512, `._:/-`), request id safe token (96, `._:-`), body ≤ 8192 bytes (57344
  for event batches), else `invalid admission HTTP request`.
- The response body is read up to 16 KiB; more → `admission backend response exceeded limit`.
- **No retries** in the client; responses are matched by substring on the raw body, never parsed as JSON. The
  Rust port must match the same substrings (no spaces after `:`).

#### 3.1.2 Routes

`{s}` is the session id (safe token 64, `-`).

| Route | Body (exact field order) | Success when |
| --- | --- | --- |
| `/mystic/v1/hunts/{s}/tickets/consume` | `{"ticket":"…","authenticatedSteamId":"<u64 decimal>","bridgeInstanceId":"…","connectionHandle":"…"}` | 200, body contains `"admitted":true` and matches `"admissionId":"<64 lowercase hex>"`; `"idempotent":true` noted |
| `/mystic/v1/hunts/{s}/host-ready` | `{"transport":"ICE","virtualPort":<n>,"bridgeInstanceId":"…"}` | 200 and `"ready":true` |
| `/mystic/v1/hunts/{s}/lease` | `{"leaseSequence":<n>,"bridgeAlive":true,"serverReady":true,"admittedPeerCount":<n>,"queuePressure":<n>}` | §3.1.4 |
| `/mystic/v1/hunts/{s}/event-batches` | §3.3.4 | 200, `"accepted":true` and `"batchHash":"<64 lowercase hex>"` |
| `/mystic/v1/hunts/{s}/completion` | `{"finalSequence":<n>,"finalHash":"<64 hex>"}` | 200, `"accepted":true` and `"progressionApplied":false` |
| `/mystic/v1/hunts/{s}/peers/{admissionId}/disconnect` | `{}` | answer ignored |

`authenticatedSteamId` is the 64-bit transport peer id (the name is the Steam bridge's).

**Consume** request rules: ticket safe token (4096, `._-`), bridge instance id (96, `._:-`), connection handle
(32, no extras), peer id ≠ 0, else code `invalid_request` without a call. Codes: no answer → `backend_unavailable`;
status ≠ 200 → `backend_http_<status>`; 200 without the two matches → `malformed_backend_response`; success → `ok`.

**Host-ready**: bridge instance id (96, `._:-`), virtual port ≠ 0, else `invalid host-ready request`. Failure texts:
the transport error, or `host-ready backend rejected the active generation (HTTP <status>)`.

#### 3.1.3 Request ids

| Call | Request id |
| --- | --- |
| Host-ready | `ready-<generation>` |
| Host's own consume | `host-local-<generation>` |
| Joiner consume | `adm-<generation>-ice<link index>` |
| Lease | `lease-<generation>-<sequence>` |
| Disconnect | `disc-<generation>-<peer id>-<link index>`; replay case `disc-replay-<generation>-ice<link index>` |
| Event batch | `guard-<generation>-<first sequence>` |
| Completion | `complete-<generation>-<final sequence>` |

#### 3.1.4 Lease answers

Invalid request (sequence 0, admitted count > 64, queue pressure > 100) → `Rejected`, `invalid host heartbeat`, no
call. Classification: no answer → `Unavailable`; 200 with `"renewed":true` → `Renewed`; 200 otherwise →
`Rejected`; 503 containing `P2P_EMERGENCY_STOP` → `Rejected`; 500, 502, 503, 504, 408, 429 → `Unavailable`; any
other status → `Rejected`. Error texts: the transport error; `host heartbeat: the backend is unavailable (HTTP <n>)`;
`host heartbeat rejected active generation (HTTP <n>)`.

#### 3.1.5 Client errors

`host capability must be a 32-byte base64url secret`, `invalid admission backend URL`,
`admission backend must be an HTTPS origin without credentials, query, or fragment`,
`WinHTTP session initialization failed` (the Rust port may keep this text for an HTTP client setup failure or pick
a neutral one; it appears only in `HOST BACKEND FAIL:`), `admission backend connection setup failed`,
`admission backend request failed`, `admission backend returned no status`,
`admission backend response read failed`, `admission backend response exceeded limit`.

#### 3.1.6 Disconnect

Session id safe token 64 `-`, admission id exactly 64 alphanumerics; otherwise nothing is sent. Fire and forget.

### 3.2 Backend lanes

A lane is one worker thread running posted jobs one at a time in post order, each job given the shared client. `post`
refuses (returns false, job not run) when the lane is stopping or holds `capacity` jobs counting the running one.
`drain_and_stop(limit)`: wait up to `limit` for the queue to empty and the running job to finish, then stop: queued
jobs not started are dropped, a running job is waited for (bounded by the client's timeouts). A job that panics
reports nothing and the next runs. Results reach the session through whatever the job captured (mailboxes), never
through session state.

Host lanes: **lease lane**, capacity 2, one renewal in flight at most; **work lane**, capacity 256, carrying joiner
consumes and disconnect notices in the order the session posts them. Host-ready and the host's own consume run in
place before the first packet. The session thread never waits for the network.

### 3.3 The Guard publisher

#### 3.3.1 Timing defaults

| Setting | Value |
| --- | --- |
| Batch window (oldest event's wait) | 2000 ms |
| First retry / ceiling | 250 ms doubling to 4000 ms |
| Events per batch | 96 (never above 128) |
| Body estimate per batch | 56 KiB = 57344 bytes |
| Queue | 4096 events |
| Completion attempts | 6, waits 1, 2, 4, 8, 16 s between |

The tests shorten the window, retries and queue (§5); the implementation must take them as parameters.

#### 3.3.2 Enqueue (never waits for the network)

Refused (false) when stopping; after a refusal of the evidence (counts `evidence_loss` +1); when the queue holds 4096
(counts `evidence_loss` +1, marks the evidence incomplete, and on the first overflow of a run
`GUARD_EVIDENCE_LOSS action=completion_fenced reason=publisher_queue_full`). An accepted event is stamped with
`monotonicMs` = ms since the publisher was created. If events overflowed since the last accepted one, an
`EVIDENCE_LOSS` event with no actor and payload `{"droppedEvents":<n>,"reason":"publisher_queue_full"}` is queued
just before it. `COMPLETION_REQUESTED` and `HUNT_ABORTED` are **terminal**.

#### 3.3.3 Building a batch

A batch is due when the queue is non-empty and: flushing, a terminal event is queued, 96 events are queued, or the
oldest has waited the window. Building takes events in order up to 96, stopping before one whose estimated size
(112 + type + actor + payload bytes) would push the running total (starting at 256) over 57344 (the first event is
always taken), and stopping right after a terminal event. Sequences continue the chain (first ever = 1). Batch id: a
random UUID v4, lowercase hyphenated; if the RNG fails the evidence ends (`GUARD_EVIDENCE_LOSS
action=completion_fenced reason=batch_id_generation`, queued events counted lost). `previousHash` is the chain head,
initially 64 `0`s. After building, a pending overflow gap is queued as its `EVIDENCE_LOSS` event if there is room.

#### 3.3.4 Batch body

```text
{"schema":1,"batchId":"<uuid>","firstSequence":<n>,"lastSequence":<n>,"previousHash":"<64 hex>","events":[{"sequence":<n>,"monotonicMs":<n>,"type":"<type>","actorAccountId":"<actor>","payload":<payload JSON as given>}, …]}
```

`actorAccountId` is omitted when the actor is empty. Before sending, the client validates: session id (64, `-`),
batch id (64, `-`), previous hash 64 alphanumerics, 1..=128 events, first sequence ≥ 1, consecutive sequences,
non-decreasing `monotonicMs`, type safe token (64, `_`), actor empty or safe token (96, `._:-`), payload 1..=4096
bytes starting `{` and ending `}`; and the body ≤ 57344 bytes. A failure is reported as an **answered** refusal
(`invalid Guard event batch` / `Guard event batch exceeds its size limit`), which ends the evidence (§7: pipe event
types may contain `.:-`, which this check refuses).

#### 3.3.5 Results

- Accepted with a 64-character hash: the head becomes that hash, the next sequence follows the batch; if failures
  preceded, `HUNT GUARD OBSERVE: the batch was taken after <n> failed send(s)`; retry delay resets; a batch that held
  `COMPLETION_REQUESTED` makes the completion due now.
- Not taken and retryable — no answer, 408, 429, 500..=599: `publish_failures` +1; on the first failure in a row
  `HUNT GUARD OBSERVE WARN: <error> (sending the same batch again)`; the **same** batch (same id, previous hash,
  events) is sent again after the retry delay, which then doubles up to the ceiling.
- Any other answer: `publish_failures` +1; the evidence ends: refused, incomplete, every event of the batch and the
  queue counted lost, completion cancelled; `GUARD_EVIDENCE_LOSS action=completion_fenced reason=batch_refused
  status=<status>` then `HUNT GUARD OBSERVE WARN: <error> (no more evidence is sent for this hunt)`.

Error text when none was given: `Guard event batch was rejected`. Other client texts: `Guard event backend rejected
active generation (HTTP <n>)`, `malformed Guard event response`.

#### 3.3.6 Completion

Only when no batch is out. If the evidence is incomplete: cancel, `HUNT GUARD COMPLETION WARN: Guard evidence is
incomplete; completion is not requested`. Else request it with the last sequence and the head hash. Accepted →
completed. Otherwise after the 6th attempt give up with `HUNT GUARD COMPLETION WARN: <error> (gave up after <n>
attempts)`; after the first, `HUNT GUARD COMPLETION WARN: <error> (trying again)`; waits 1 s × 2^(attempt−1).
Client failure texts: `invalid Guard completion request`, `Guard completion backend rejected active generation (HTTP
<n>)`.

The session's own Guard calls (`publish_guard_event`): without hunt evidence → success, nothing sent; no publisher →
error `no Guard event publisher`; not taken → error `the Guard event queue did not take the event`.

Event types the host itself produces: `SESSION_STARTED`, `WORLD_READY`, `PLAYER_ADMITTED`, `PLAYER_LOST`,
`EVIDENCE_LOSS`, `HUNT_ABORTED`; others (including `COMPLETION_REQUESTED`) come from the game server through the
pipe.

#### 3.3.7 Flush and stop

Flushing makes every queued event due at once; `flush_and_stop(limit)` waits up to `limit` until idle (refused, or
nothing queued, no batch out and no completion due), then stops the thread. Dropping a publisher stops it without
flushing.

### 3.4 The guard event pipe (Windows)

- Name `\\.\pipe\<guard_pipe_name>` (the launcher uses `MysticHuntGuard.events.<suffix>`); name safe token
  (96, `_-.:`) and `expected_server_pid ≠ 0`, else `invalid local Guard pipe configuration`.
- Security descriptor SDDL `D:P(A;;GA;;;SY)(A;;GA;;;OW)S:(ML;;NW;;;LW)` (SYSTEM and owner full access, Low
  integrity no-write-up label, so a Low-integrity game server can write). Failure →
  `local Guard pipe security could not be prepared`.
- Inbound only, first instance only, message type and message read mode, non-blocking, remote clients rejected,
  one instance, in-buffer 8192, out-buffer 0. Failure → `local Guard pipe creation failed`.
- Each pump: if not connected, try a non-blocking connect; when a client is (or already was) connected, its process id
  must equal `expected_server_pid`, else disconnect it. Then up to 16 messages: peek (error → disconnect); none
  available → done; more than 8192 bytes → drop count +1 and disconnect; read (failure, 0 bytes → drop +1 and
  disconnect).
- Message: `MGE1\t<type>\t<actor>\t<payload>` (split at the first three tabs; the payload is the rest). Type safe token
  (64, `_-.:`), actor empty or safe token (96, `_-.:`), payload 1..=4096 bytes beginning `{` and ending `}` (not
  otherwise validated). Bad → drop +1. At most 256 events waiting; more → drop +1.
- The session takes the drop count (resetting it) each loop (§2.5.2 step 4).

---

## 4. Contracts kept by callers outside this part

| Contract | Used by | Rule |
| --- | --- | --- |
| DLL file name `ParadoxTransport.dll` | `rifts-host` loader, Guard manifest role `TRANSPORT_MODULE`, `release-runtime-extras.yml` | Keep the name (§7) |
| The 29 export names and signatures (§2.1) | `rifts-host/src/transport.rs` `EXPORTS`, `ffi.rs` | All resolved before first call; `AbiVersion` must be 3 |
| Optional `MysticTransport_UpdateRelayCredentialH` | `rifts-host` | Absent |
| Config keys of §2.3 | `rifts-host/src/session.rs` writes `bridge.conf`, `transport.secret`, `capability.secret`, `ticket.secret` | Same keys, same rules |
| Ready lines §2.4.1, peer events §2.4.2, `ADMISSION_ACCEPTED … role=host … source=local`, `UDP_STATS … localRx=` | `rifts-host` `parse_ice_ready`, `parse_peer_event`, `is_local_admission`, `udp_stats_local_rx` | Token names and values exactly |
| Signal kinds and texts §2.9 | the launcher's relay, the backend's relay-only filter, C++ peers | Byte-compatible |
| Sealed frame, MPC1, fragments | C++ peers (mixed versions) | Byte-compatible |
| MPT2 ticket, raw X‖Y key file | the backend issuer (rustic-isles / rifts-admission) | As §2.13 |
| HTTPS routes, headers, bodies | the backend's P2P router | As §3.1 |
| `MGE1` pipe messages | the game server's Hunt Guard producer (rustic-isles native) | As §3.4 |

---

## 5. Tests that define the behaviour

Every scenario of the C++ self-test, restated as acceptance tests. Bind test sockets to `127.0.0.1:0`. Shared
helpers the tests need: an ES256 ticket signer writing the raw X‖Y key and minting MPT2 tickets (now, now, now+120);
a fake backend recording consumes, disconnects, host-ready, heartbeats (admitted count per session), Guard batches
(hash `bbbb…<n>`), with an optional per-call delay except host-ready; a scripted Guard backend (answers 0 = no
answer, 200 = taken, else the status); a UDP echo server recording source ports; a game client doing round trips;
a signal carrier moving `ReadSignalH` output into the other side's `SubmitSignalH` (a host's outbox addressed by the
`to` account); a whole-run watchdog (default 300 s, env override, 0 = off) failing with the section name.

### 5.1 Unit level

**Crypto.** Decodes the 43-character all-`A` key; refuses 42 characters and a `+`. Host and joiner ciphers with one
key: a 1200-byte gameplay frame grows by 28 and opens host→joiner; the same frame again is refused; a frame sealed
by the host is refused by the host's own opener; a flipped ciphertext byte and a changed type byte are refused; a
different key refuses. 1100 sealed frames: the newest opens, #500 opens, #10 is refused (outside the window), #500
again is refused. Eight new ciphers with the same key give eight distinct non-zero first counters; a fresh pair's first
counter is > 1 and opens.

**Parsing.** `stun:stun.cloudflare.com:3478` → host and 3478; `turn:…:3478?transport=udp` → 3478;
`turn:…:443` → 443; `?transport=tcp`, `turns:` and `stun:bad host:1` refused. Relay-only filtering of a description
holding a host and a relay candidate keeps the relay line and `a=ice-ufrag:abcd`, drops the host line; a lone srflx
`CANDIDATE` filters to empty. ICE material: a config with three TURN URLs (UDP, `turns` TCP, UDP 443) and
credentials plus two `peer=` lines gives 2 relay servers, 2 peers, STUN port 3478; a short key and a duplicate account
are refused.

**Lease.** Classification: 200 `{"renewed":true,"state":"RUNNING"}` renewed; 200 `{"renewed":false}` rejected; no
answer, and 500/502/503/504/408/429 with an HTML body, unavailable; 503 with `P2P_EMERGENCY_STOP` rejected;
400/401/403/404/409/410 rejected. Tracker: unavailable at +2, +4, +6 s then renewed at +8 never fails; then
unavailable at renewal+29 s holds and at renewal+30 s fails; two refusals in a row fail, one does not; refused,
renewed, refused does not fail.

**Held signals and roster** (host session alone, TTL shortened to 400 ms, `maximum_peers` 8, a real joiner link's
description): a description and two `GATHERING_DONE` from an unknown account are held (`ICE_SIGNAL_HELD` once,
nothing rejected, nothing sent); adding the account replays 3 (`ICE_SIGNAL_REPLAYED role=host peer=3001 count=3`)
and the host answers with its own `DESCRIPTION` after a pump. 20 candidates from another account: 4 rejected,
`held_signals_full` once; after 500 ms and a pump `ICE_SIGNAL_EXPIRED … count=16`, rejected 20; adding that
account replays nothing. 70 accounts: rejected 26 in total (64 held). Remove of an unknown account fails;
`duplicate_account`, `duplicate_peer`, the host's own id → `invalid_peer`; six more adds fill 8; one more → `full`;
removing `member-0` emits exactly `PEER_REMOVED account=member-0 reason=left admitted=0`; it can be added again.

**Lane.** Three 150 ms jobs on a capacity-3 lane post in under 50 ms; a fourth is refused; they ran in order 1,2,3
before the stop; a stopped lane refuses.

**Publisher** (window 200 ms, retries 20→80 ms):
- 250 events enqueued against a 300 ms backend in under 50 ms; with `COMPLETION_REQUESTED` and `PLAYER_LOST` and a
  20 s flush, all 252 arrive in one valid chain (sequences 1..n, each batch naming the previous hash starting at 64
  `0`s, times non-decreasing, ≤ 128 per batch, terminal only last) in 3..=8 batches; one completion with the head
  (sequence ≥ 251); completed, 0 failures, 0 lost.
- Script 429, no answer, 503: 4 attempts all with the same id, previous hash, size and first sequence; then a valid
  chain of 5, 3 failures, nothing lost; diagnostics contain `sending the same batch again` and
  `was taken after 3 failed send(s)`.
- Script 409 with 3 events and `COMPLETION_REQUESTED`: the evidence becomes incomplete; a further enqueue is
  refused; 1 attempt, nothing accepted, no completion, 5 lost;
  `GUARD_EVIDENCE_LOSS action=completion_fenced reason=batch_refused status=409`.
- Queue 8, window 10 s: 12 enqueues → 8 taken, 4 lost, incomplete; after flush the chain holds 9 events including
  `EVIDENCE_LOSS` with `"droppedEvents":4`.

**Late consume** (host session with a work lane of capacity 8, admission deadline 1 s, backend delay 1500 ms, so
the link is refused at 1 s, closed at 2 s and the consume answers at 1.5 s while it lingers): within 12 s exactly
one disconnect reaches the backend; the joiner is not admitted, the host has 0 admitted, one consume was made.

**Local bridge** (server and two clients): the first client reaches the server; a second local source is dropped
and counted `foreign`=1 while the first is active; after 2 s of silence the second takes over through a different
server-side source port, with `LOCAL_CLIENT_REPINNED role=host path=local`; the server's reply reaches the second
client and not the first.

### 5.2 ICE and data path

**`ice`** (direct; `--relay-local` with an in-process TURN server; `--relay-file` with real credentials): a joiner
link and a host link created from the joiner's description connect within 30 s. With the local relay at least one
side is relayed; with a real relay both are. Through real bridges and an echo server: datagrams of 1, 64, 512, 1200,
1400 and 2000 bytes return intact within 3 s; a 300-datagram burst of 600 bytes at ~3 ms spacing loses ≤ 3; 100
datagrams of 1800 bytes (two pieces) lose ≤ 3; no frame fails authentication on either side.

### 5.3 Sessions

**`session`** (sessions without the module, fake backend, no lanes): joiner admitted, host admitted count 1, exactly
one consume despite 1.2 s of repeated requests, a 900-byte datagram echoes, the consume names the joiner's peer id.
A joiner whose valid ticket belongs to `other-account` is never admitted and nothing is consumed (`wrong_account`).

**Module sessions** (two module instances, carrier between them):
- One module hosts a HUB (`host_local_client=0`, peer added at run time) and a HUNT (local client, peer in the secrets
  file) at once: distinct handles; the hub's ready line has `localPort=0 `, the hunt's a non-zero port; both
  published host-ready. `AddPeerH` on the hub is 1; on the hunt (account already in the roster) 0; with `bad
  account!` 0.
- A second module joins both at once; both reach `ROUTE_READY` with their session ids; 700- and 900-byte round trips
  through each; each echo server saw exactly one source; each host reported `PEER_ADMITTED account=join-account` once;
  the hub consumed once, the hunt twice, one of them `localhost` with the host's peer id; the hub's heartbeat reports
  1 admitted. `AddPeerH` on a joiner handle, `RemovePeerH` of an unknown account or handle → 0.
- Eight sessions run; a ninth → 0 with `At most 8`; stopping one leaves its diagnostics readable and frees a place.
- Config rules: HUB 64 ready, 65 `HOST CONFIG FAIL`; HUNT 4 ready, 5 refused with or without local client; role
  `BOGUS` refused; `host_local_client=2` refused; HUNT with local client and no ticket refused; HUB given a ticket
  ready (ignored); no `session_role` line behaves as HUNT (4 ok, 5 refused).
- ABI 2 on the default handle: `StartHostV2` 1, a second refused (`already running`), after a stop a new start
  replaces the default.
- Teardown: stopping the joins then both hosts → results 0; `PEER_LOST account=join-account reason=host_stopped`
  readable after the stop on both; exactly 2 disconnects.

**`abi`** (loading the built DLL): all 17 ABI 2 and 12 ABI 3 names resolve; version 3; initialize gives a non-zero
peer id; v1 `StartHost` refused with `ABI 2` in the text; `SubmitSignal` without a session 0; handle 999 refused
everywhere (`WaitReadyH` -1, `SessionRunningH` 0, `StopSessionH` 0, `LastSessionResultH` -1, reads 0, submit 0,
`AddPeerH` 0, `RemovePeerH` 0); ABI 2 reads with no session quiet (0, 0, 0, 0). `HostOpen` without a key path and
`JoinOpen` without a ticket path → 0 with `missing`. `HostOpen` on missing files → a handle that ends before
readiness (`-1` within 10 s), shows `HOST CONFIG FAIL`, result 2, then `StopSessionH` 1. `StartJoinV2` starts and
`ReadSignal` returns `DESCRIPTION` addressed to `host-account`; a second start refused; `JoinOpen` beside it runs;
`ReadSignalH` gives its description; `SubmitSignalH` takes `GATHERING_DONE` `-` and refuses `bad account` and kind
`BOGUS`; `AddPeerH` on a joiner 0; `StopSession` stops only the default; `StopSessionH` stops its own, result 0; a new
`StartJoinV2` replaces the finished default; `Shutdown` stops it; `StartHostV2` with a null ticket path 0.

**`hub`** (direct, and `--relay-local` with the relay on 127.0.0.2 so only relay pairs exist; `maximum_peers` 6,
HUB, no local client):
0. An add of the host's own peer id is queued (1) and comes back as `PEER_ADD_REJECTED`; that account is then not on
   the roster (remove → 0); add then remove in one burst leaves the roster as it was.
1. Six joiners join one after another (each added, then started): each `PEER_CONNECTED` and `PEER_ADMITTED` once;
   relay mode: ≥ 12 `route=relay`, 0 `route=direct`; each echoes through its own bridge slot (6 sources); 6 consumes,
   none for the host.
2. A seventh add → 0 (full); duplicate account or peer id → 0.
3. `RemovePeerH member-3 kicked` → 1; `PEER_REMOVED account=member-3` follows; the backend gets one more disconnect;
   removing again → 0.
4. A seventh joiner started before its add: its description is held (`ICE_SIGNAL_HELD` for it, not ready); after the
   add it connects within 30 s, `ICE_SIGNAL_REPLAYED role=host peer=<its id>` once; its traffic flows (7 sources:
   the removed slot was freed).
5. `member-2` stopped and removed, `PEER_REMOVED` seen, added again, rejoins over a fresh link: two
   `PEER_ADMITTED account=member-2`; traffic flows.
6. `member-5` restarts without a remove: `PEER_LOST account=member-5 reason=restarted` once and a second
   `PEER_ADMITTED`; traffic flows.
7. Across all links: 9 consumes with unique request ids and unique connection handles, `member-2` consumed twice;
   disconnect request ids unique (≥ 2); the heartbeat reports 6 admitted.
8. Stop: result 0; 6 `reason=host_stopped` events; disconnects grow by ≥ 7 in total since step 3.

**Slow backend** (every call but host-ready takes 400 ms; a hunt with local client and one joiner): ready within
20 s; the joiner admitted although its consume takes 400 ms; 5 s of traffic with ≥ 50 round trips, worst < 200 ms,
≥ 2 lease renewals; the heartbeat reports 1 admitted; one `PEER_ADMITTED`. After stopping: Guard batches form a valid
chain; the first batch starts `SESSION_STARTED`, `WORLD_READY`; the last event is `HUNT_ABORTED`.

### 5.4 Opt-in

`ice --relay-file`: an `#[ignore]` test reading `ice_stun_urls`, `ice_turn_urls`, `turn_username`, `turn_credential`
from a file named by an env variable (minted by `rifts-relay`'s example), deleting nothing it did not create, printing
no secret.

### 5.5 Interop oracle (reviewer, in dainty-restricted)

Run the C++ `ParadoxTransportTest.exe`-built DLL against the MIT DLL: C++ joiner → Rust host, Rust joiner → C++ host,
direct and relay-only, each with a 2000-byte datagram (fragments both ways) and an admission. This is the acceptance
test for §2.9.3, §2.10–§2.12.

### 5.6 Fuzz targets (RX3)

The DLL is a trust boundary (it parses remote frames and local files). Targets: sealed-frame open with a fixed key;
fragment reassembly; MPC1 request and result decoders; MPT2 parse (unverified path); the key=value parser and ICE
material; the relay-only filter; the `MGE1` message parser. A checklist `docs/security/rifts-transport.md` comes with
the crate.

---

## 6. Target MIT crate

### 6.1 Shape

A crate in dainty-lass (`crates/<name>`, MIT, `crate-type = ["cdylib", "rlib"]`; name chosen by the umbrella
spec), producing `ParadoxTransport.dll` (cargo cannot name the artefact differently from the lib name; the umbrella
decides between a lib name and a packaging rename, §7). Modules: `ffi` (exports only), `registry`, `files`, `host`,
`join`, `link` (ICE adapter), `seal`, `control`, `ticket`, `backend` (client + lanes), `guard` (publisher), `pipe`
(Windows), `bridge`, `router`. Every export wraps its body in `catch_unwind` and returns the "refused" value on panic.

Threads: one OS thread per session (as now) plus two lanes and one publisher per host; the session thread owns the
ICE agent (sans-IO, §6.3) and its UDP socket(s), so no lock is held across ICE calls — the libjuice deadlock of
15 Oct 2026 (a lock taken by the library's callbacks held across a library call that takes the library's global
lock) cannot arise with a sans-IO agent. With an async/callback ICE crate the same rule applies: never hold a lock
that the crate's callbacks take while calling into the crate.

### 6.2 Dependencies (dainty-lass `[workspace.dependencies]`)

| Need | Choice | Status |
| --- | --- | --- |
| AES-256-GCM | `aes-gcm` 0.11 or `aws-lc-rs` (`aead::AES_256_GCM`) | present |
| ECDSA P-256 verify of raw r‖s over SHA-256 | `aws-lc-rs` `signature::ECDSA_P256_SHA256_FIXED` with the key as `0x04‖X‖Y`; or `p256` 0.14 (`ecdsa`) | `aws-lc-rs` present; **`p256` absent** (restricted has it) — seam if chosen |
| SHA-256 | `sha2` 0.11 (only if not using aws-lc-rs's hashing verify) | present |
| CSPRNG (peer id, counter start, UUID) | `rand` 0.10 or `aws-lc-rs::rand` | present |
| base64url strict | `base64` 0.23 (`URL_SAFE_NO_PAD`, trailing bits refused by default) | present |
| HTTPS client | `hyper` 1 + `hyper-util` + `hyper-rustls` 0.27 + `rustls` 0.23 on a current-thread `tokio` runtime per lane | present; `tokio` needs `net`/`time` features in the crate (feature flags only) |
| Loopback TLS with any certificate | a custom `rustls` `ServerCertVerifier` used only for `127.0.0.1`/`localhost` | code |
| System proxy (WinHTTP automatic proxy) | none in the workspace; dropping it is a behaviour change | §7 |
| Windows trust store (WinHTTP uses it) | `hyper-rustls` `webpki-tokio` uses webpki roots; `rustls-platform-verifier` would be a **seam** | §7 |
| Named pipe with SDDL, client pid | `windows` 0.62.2 (`Win32_Security`, `Win32_Security_Authorization`, `Win32_System_Pipes`, `Win32_Storage_FileSystem`, `Win32_Foundation`) | present; needs `unsafe` |
| UDP sockets | `std::net::UdpSocket` (non-blocking, `connect`) | std, no `unsafe` |
| Key and ticket wiping | `zeroize` | **absent, seam** (or accept no wipe) |
| ICE agent + TURN client | §6.3 | **seam** |
| Test-only TURN server | `turn` (webrtc-rs) server or `rtc-turn`'s server, dev-dependency | **seam** |
| Test-only ES256 signer | `aws-lc-rs` `EcdsaKeyPair` (`ECDSA_P256_SHA256_FIXED_SIGNING`) | present |

**`unsafe`.** The workspace denies `unsafe_code`; the crate needs scoped `#[allow(unsafe_code)]` as
`crates/gateway/src/allowlist/windows.rs` does, in exactly two places: `ffi` (`#[unsafe(no_mangle)] extern "C"`
exports in edition 2024, reading `const char*` with `CStr::from_ptr`, writing caller buffers) and `pipe` (Win32
calls: SDDL conversion, `CreateNamedPipeW`, `ConnectNamedPipe`, `PeekNamedPipe`, `ReadFile`,
`GetNamedPipeClientProcessId`, `DisconnectNamedPipe`, `LocalFree`). BCrypt (AES, ECDSA, SHA-256, RNG), WinHTTP,
Winsock and `CryptStringToBinaryA` are all replaced by safe crates.

### 6.3 Rust ICE crate options

Requirements from §2.9: full ICE with both roles, trickle, UDP, **TURN client over UDP producing relay candidates and
carrying checks and data through the allocation** (relay-only is essential), the ability to run with only relay
candidates, access to the selected pair's candidate types, credentials (ufrag/pwd) and candidate lines that
interoperate with libjuice's text, MIT/Apache licence, and preferably sans-IO so the session thread owns everything.

| Crate | I/O model | TURN client (UDP) | ICE | Maintenance | Licence | Signalling text |
| --- | --- | --- | --- | --- | --- | --- |
| `is` 0.11.1 (str0m's ICE agent, standalone; str0m itself uses it) | sans-IO (`handle_packet` / `poll_transmit` / `poll_timeout` / `poll_event`) | **none**: relay candidates must be added by the caller, who must also wrap and unwrap TURN Send/ChannelData for the relay candidate's traffic | full, both roles (str0m's agent; ice-lite also); trickle only | active (0.11.1, 25 Sep 2026) | MIT OR Apache-2.0 | candidates and credentials are values; `a=candidate:` text must be produced and parsed by us (str0m has the SDP code, unverified whether `is` exports it) |
| `webrtc-ice` 0.17.2 (webrtc-rs) | async, tokio-bound, callbacks | yes, through the `turn` crate, UDP | full, controlling/controlled (`dial`/`accept`), trickle | maintained (0.17.2, 20 Jul 2026) but the project's effort moved to the sans-IO `rtc` stack (unverified that it is formally superseded) | MIT OR Apache-2.0 (known, page did not state it) | `Candidate::marshal()` gives the `candidate:…` attribute body; local ufrag/pwd from the agent |
| `rtc-ice` 0.21.1 + `rtc-turn` (webrtc-rs "rtc" sans-IO stack) | sans-IO (`Agent`, `Event`, no sockets, no clock) | `rtc-turn` is a sans-IO TURN client (RFC 5766/6156); its docs say `rtc-ice` gathers relay candidates through it (unverified how much wiring the caller does) | full (unverified whether both roles are exposed; webrtc-rs's ICE has both), trickle | active (0.21.1, 11 Oct 2026) | MIT OR Apache-2.0 | `candidate` module parses `a=candidate`; `Credentials` type |
| `librice` 0.4.3 / `rice-proto` (sans-IO core) | sans-IO core + async wrapper | "initial TURN support"; RFC 8656 listed; maturity unverified | full RFC 8445, trickle listed | active (0.4.3, 11 May 2026); author's own note: "more work is needed on the ICE layer"; docs.rs builds failed for several versions | **unverified** (believed MIT OR Apache-2.0) | own candidate types; text form unverified |

**Recommendation: `rtc-ice` with `rtc-turn`.** It is the only option that is sans-IO **and** ships its own TURN
client meant to feed relay candidates into the agent, under MIT/Apache, recently released. Sans-IO fits the DLL's
model (one session thread owning sockets and timers; no tokio inside ICE; no lock across library calls). Fall back to
`webrtc-ice` if the spike shows `rtc-ice`'s relay path or role control incomplete: it is the most proven TURN path
in Rust but drags a tokio runtime and callbacks into each session. `is` is the cleanest agent but would make us
write the TURN client side (allocation refresh, permissions, channel binding, Send/Data indications) ourselves.
`librice` is promising but its TURN maturity and licence must be verified first.

Before committing, a **spike** (hands-on, maintainer): relay-only connect between two agents through a local TURN
server on 127.0.0.2 and through Cloudflare, plus a joiner on the C++ DLL against the Rust host (§5.5). The crate
choice and its addition to `[workspace.dependencies]` are a seam change for the integration pass.

Sources consulted for this table (2 Nov 2026): docs.rs pages for `is`, `librice`, `rtc-ice`, `rtc-turn`,
`webrtc-ice`; github.com/ystreet/librice; github.com/algesten/str0m issue 405. Items marked unverified were not
confirmed from a primary source.

### 6.4 What dainty-restricted needs afterwards

`rifts-host` keeps loading `ParadoxTransport.dll` by path with `libloading`; nothing changes on its side if the export
table, ready lines and file formats are kept. The C++ tree, its MSBuild projects, scripts and `transport.yml` stay in
restricted as the interop oracle until the maintainer retires them; rustic-isles' `release-runtime-extras.yml`
switches to the cargo-built DLL (the release workflow belongs to RH6/RA14; not this row).

---

## 7. Risks and ambiguities

1. **Branded wire identifiers** (contracts; the umbrella decides keep or rename): export prefix `MysticTransport_`,
   file name `ParadoxTransport.dll`, frame magic `MX`, `MPC1`, `MPT2`/`mpt1.`, `MGE1`, headers
   `X-Mystic-Host-Capability` and `X-MysticParadox-Request-Id`, routes `/mystic/v1/hunts/...`, user agent
   `ParadoxTransport/1`, pipe prefix `MysticHuntGuard.events.`, body field `authenticatedSteamId`. Renaming any of
   them breaks C++ peers, the backend or the launcher; keep them until every side moves together.
2. **ICE interop with libjuice** is the main technical risk: a Rust agent must accept libjuice's description and
   candidate text and produce text libjuice accepts (ufrag/pwd lengths and characters, `ice-options`, candidate
   priority and foundation formats, `raddr`/`rport` for relay candidates). libjuice's exact output and its timeouts
   are unverified here; measure them with the oracle (§5.5).
3. **Relay-only semantics**: "relayed" is either side's selected candidate being `relay`. On one PC with a relay on
   the same address a direct-to-relay pair can be chosen; the tests use 127.0.0.2 for that reason. A Rust agent that
   gathers host candidates but only advertises relay ones must still route checks through the allocation.
4. **Empty error text**: a relay-only incoming `DESCRIPTION` that filters to empty is refused with an empty error,
   producing `ICE_SIGNAL_REJECTED role=join reason=` (trailing nothing). Keep it (it is observable) or fix it under
   a decision row.
5. **Guard type character sets differ**: the pipe accepts event types and actors with `.`, `:` and `-`; the batch
   validator accepts types of `A-Za-z0-9_` only. Such an event makes its batch an answered refusal and ends the
   hunt's evidence. Kept as is (observable); flag for a decision.
6. **Raw-substring response matching** (`"renewed":true`, `"admitted":true`, …): a backend that pretty-prints JSON
   would break the C++ DLL; the Rust port must keep substring matching to behave identically, or parse JSON under a
   decision row.
7. **WinHTTP replacement**: the C++ client uses the system proxy (automatic) and the Windows certificate store.
   `hyper-rustls` with webpki roots loses both; corporate TLS interception or a proxy-only network would stop
   working. `rustls-platform-verifier` (seam) restores the store; proxy support needs code or a crate.
8. **ECDSA low-S**: BCrypt accepts high-S signatures. `aws-lc-rs` and RustCrypto `ecdsa` verification also accept
   them (unverified for the exact versions); a verifier that normalises or rejects high-S would refuse tickets the
   backend signs. Test with a high-S signature.
9. **Lock-order bug of 15 Oct 2026** (README "Locking"): describes libjuice only; irrelevant for a sans-IO crate, and
   stated as a design rule in §6.1 for any callback-based crate.
10. **Known limit in the README is stale**: it says consumes are synchronous on the session thread; since 17 Oct they
   go through the work lane (§3.2). Follow this spec.
11. **`UpdateRelayCredentialH`**: `rifts-host` probes it as optional (RF1 decision 6); the C++ DLL does not export
   it. Adding it is a new feature, not part of the port.
12. **Joiner result 5** reuses "lease failed" for a link lost after admission; `rifts-host` may read it as a lease
   failure. Kept (observable).
13. **ABI 2 `WaitReady` with no default handle returns 0, not -1**, and `LastSessionResult` clamps -1 to 0; keep.
14. **Session id case**: the ticket's session id is compared as lowercase hex; an uppercase `session_id` in the config
   always yields `wrong_session`. Keep.
15. **DLL artefact name**: cargo names a cdylib after the lib name; `ParadoxTransport` as a lib name triggers a
   `non_snake_case` lint and carries the brand into code. A post-build rename in packaging avoids both; the umbrella
   decides.
16. **Interop window**: launchers and DLLs ship through the runtime feed and the Guard manifest's SHA-256 pin; until
   all players have the Rust DLL, a host and joiner may run different implementations. §5.5 must pass before the
   first release.
