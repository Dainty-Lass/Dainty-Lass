<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# Clean-Room Behaviour Specification: `rifts-host` (RD27)

> **Scope (RD27, part of [`spec-rd27-rifts.md`](spec-rd27-rifts.md)):** the launcher's side of realms. It is
> dainty-restricted's `modules/rustic-rifts/crates/rifts-host` (AGPL, a Rust port of Mystic Paradox's launcher
> `p2p/` modules), plus the module's top-level files, `docs/inventory.*`, `docs/security/host.md` and the module's
> fuzz workspace files, which this part classifies. The transport DLL itself (`src/`, the C++ ABI's other side) is
> [`spec-rd27-rifts-transport.md`](spec-rd27-rifts-transport.md); the shared id and secret types are
> `rifts-seam`'s ([`spec-rd27-rifts-admission-relay.md`](spec-rd27-rifts-admission-relay.md)).
>
> Everything below is stated from observable contracts: the C export names and signatures, the buffer sizes passed
> across them, the readiness and peer-event line formats, the files written and their exact key order, the hosted
> server's environment and arguments, error texts, the public Rust API the launcher calls, and the tests. No
> implementation code is reproduced and no private helper is named.
>
> Source revision: dainty-restricted `4efc8b5` (worktree `rd27-spec-rifts-c45124`). Callers read:
> `launcher/src-tauri/src/controller/realms.rs` (1,214 lines) and `controller/tests/realms.rs` (642 lines).

---

## 1. Provenance and classification

### 1.1 Evidence

| Source | What it says |
| --- | --- |
| `modules/rustic-rifts/NOTICE.md` | The Rust workspace, `crates/rifts-host` included, "ports Mystic Paradox's `p2p/`, `realms/`, Guard and launcher realm-hosting modules (`21b9ff5`)" |
| `docs/inventory.md` §3 | The launcher code RF7 ports into `rifts-host`: Mystic Paradox's `ParadoxLauncher/src-tauri/src/p2p/{mod,transport,session,control,realm}.rs`, `p2p/session/{host,hub,ice}.rs`, `servers/realm_*.rs` |
| File headers | `transport.rs` and `ffi.rs` name `src/paradox_transport_api.hpp`; `files.rs` names `p2p/{realm,control,mod}.rs`; `session.rs` names `ParadoxLauncher/src-tauri/src/p2p/` and cites a Mystic Paradox function by name on most pure rules |
| History | `b3f9cd3` RF2 (seam stubs, 2026-10-26), `d02a680` RF7 (loader and hosting, 2026-10-27), `fc5289f`, `ae3deaa` RF4 (signal and roster pumps, 2026-10-31) |

Every Rust file is therefore treated as derived. None was measured against Mystic Paradox's Rust here; the headers
alone settle it.

### 1.2 Categories

- **A**: born in DaintyLass, free of Mystic Paradox derivation.
- **B**: derived; rewritten clean-room from §2–§6 of this document.
- **D**: reference data recorded from Mystic Paradox's tree; stays in dainty-restricted.
- **G**: generated.
- **L**: licence and provenance text; stays in dainty-restricted as it is (legal records, not code).

### 1.3 Files

| Path (under `modules/rustic-rifts/`) | Lines | Cat. | What it is | Fate |
| --- | ---: | --- | --- | --- |
| `crates/rifts-host/Cargo.toml` | 24 | A | Manifest (RF2) | Rewritten trivially for dainty-lass (§7.1): `license = "MIT"`, workspace deps |
| `crates/rifts-host/src/lib.rs` | 926 | B | `HostError`, `Stub`, re-exports; ~175 lines of code, ~750 of unit tests (§8.2) | Rewrite |
| `crates/rifts-host/src/transport.rs` | 144 | B | `TransportAbi`, `TransportLoader`, ABI types and the export list | Rewrite (§3) |
| `crates/rifts-host/src/ffi.rs` | 432 | B | `NativeTransportLoader`, `sha256_file`, the C bindings; the only `unsafe` | Rewrite (§4) |
| `crates/rifts-host/src/files.rs` | 283 | B | `HostState`, `FileHostState`, file name constants, `validate_hub_limits` | Rewrite (§5) |
| `crates/rifts-host/src/session.rs` | 2,075 | B | `RealmHosting`, `LocalRealmHosting`, `HostBackend`, the pure rules, launch environment, session folder | Rewrite (§6) |
| `docs/security/host.md` | 79 | B | RF7's checklist. Born here, but it describes the AGPL code and is wrong in several places (§10, R6) | Replaced by a fresh `docs/security/rifts-host.md` (umbrella §4.5) |
| `docs/inventory.md` | 534 | D | RF1's inventory of Mystic Paradox's realms: routes, settings, runtime hooks, the ABI table, the host PC's files, backend tables, decisions | Stays. This spec restates as contracts what `rifts-host` needs from its §3, §4, §6 |
| `docs/inventory.json` | 983 | D | The 68 routes of the inventory, machine-readable | Stays |
| `fuzz/Cargo.toml` | 64 | A | The rifts fuzz workspace (RF4); five targets, none for `rifts-host` | Stays until the umbrella's §4.4 moves the targets; `rifts-host` adds its own in dainty-lass's `fuzz/` (§8.4) |
| `fuzz/Cargo.lock` | 1,568 | G | Lock file of that workspace | Follows its manifest |
| `fuzz/.gitignore` | 4 | A | `target/ corpus/ artifacts/ coverage/` | Stays |
| `LICENSE` | 661 | L | The AGPL-3.0 text | Stays |
| `ADDITIONAL_TERMS.md` | 47 | L | Mystic Paradox's section 7 terms, copied unchanged | Stays |
| `NOTICE.md` | 42 | L | Provenance of the module (Mystic Paradox, libjuice) | Stays; the integration pass edits it (umbrella §4.3) |
| `README.md` | 226 | L | Mystic Paradox's transport README (build, session walkthrough, ABI 2 and 3 notes) as adapted | Stays; the transport spec covers its content |
| `.gitignore` | 3 | A | `build/ third_party/ /target` | Stays |

Counts: **A 4** (24 + 64 + 4 + 3 lines), **B 6** (3,939 lines), **D 2** (1,517), **G 1** (1,568), **L 4** (976).

Tests (§8): the six unit tests in `lib.rs` are B and are rewritten from §8.2. The launcher's two tests in
`controller/tests/realms.rs` are restricted's (AGPL, outside this crate) and are the black-box oracle: they must pass
against the MIT crate with only a dependency change.

---

## 2. Overview and conventions

`rifts-host` is a library linked into the launcher behind its `realms` feature. It:

1. loads the realm transport (a native library with a C ABI, today `ParadoxTransport.dll`) after checking its SHA-256,
   and wraps it as a Rust trait (§3, §4);
2. keeps two small JSON files in the launcher's config directory (§5);
3. hosts and joins realm sessions: it asks a `HostBackend` (implemented by the launcher over HTTP) for bootstrap
   material, writes a per-session folder of config and secret files, starts the hosted game server, opens a
   transport session, waits for readiness, reports the route, and pumps ICE signals and the peer roster between
   the transport and the backend (§6).

It opens no listener. The account key never reaches it: the launcher's `HostBackend` holds it.

Conventions:

- All file contents written are UTF-8, LF line endings, every line ending in `\n` (including the last).
- "Wire session id" is `wire_session_id` (§6.2.1).
- `HostError` (§2.1) is the only error type. Texts in quotes are exact; `{x}` is a substitution; `{path}` is the
  path's `Display`; `{err}` is the `std::io::Error`'s `Display`.
- Every mutex in the crate recovers from poisoning (takes the inner value); none panics.
- No `unwrap`/`expect` outside tests (workspace lints).

### 2.1 `HostError`

`#[non_exhaustive]`, derives `Debug, Clone, PartialEq, Eq` and `thiserror::Error`. Variants and `Display`:

| Variant | `Display` |
| --- | --- |
| `Unimplemented(rifts_seam::Unimplemented)` (`#[from]`) | transparent (the seam error's text) |
| `Io(String)` | the string |
| `HashMismatch { expected: [u8; 32], actual: [u8; 32] }` | `transport SHA-256 mismatch` |
| `MissingExport(&'static str)` | `transport is missing export {0}` |
| `UnsupportedAbi(u32)` | `transport speaks ABI version {0}; expected 3` |
| `InvalidInput(String)` | the string |
| `Transport(String)` | the string |
| `NotFound(String)` | the string |
| `Busy(String)` | the string |

The launcher matches `HashMismatch { .. }`, `MissingExport(sym)`, `UnsupportedAbi(ver)`, `NotFound(msg)`,
`InvalidInput(msg)`, `Transport(msg)`, `Io(msg)`, `Unimplemented(_)` and a wildcard (which today catches `Busy`), and
turns them into its own codes (`transport_hash_mismatch`, `missing_export:{sym}`, `unsupported_abi:{ver}`, ...). So
variant names, payload types and the `&'static str` of `MissingExport` are API.

---

## 3. The transport ABI as a trait (`transport` module)

### 3.1 Constants

- `ABI_VERSION: u32 = 3`.
- `EXPORTS: [&str; 29]`, in this order (the header's order; also the order the loader checks them, §4.2):
  `MysticTransport_AbiVersion`, `_Initialize`, `_Shutdown`, `_PeerId`, `_NetworkingReady`, `_StartHost`,
  `_StartJoin`, `_StartHostV2`, `_StartJoinV2`, `_WaitReady`, `_SessionRunning`, `_LastSessionResult`,
  `_StopSession`, `_ReadDiagnostic`, `_ReadSignal`, `_SubmitSignal`, `_ReadPeerEvent`, `_HostOpen`, `_JoinOpen`,
  `_WaitReadyH`, `_SessionRunningH`, `_StopSessionH`, `_LastSessionResultH`, `_ReadSignalH`, `_SubmitSignalH`,
  `_ReadPeerEventH`, `_ReadDiagnosticH`, `_AddPeerH`, `_RemovePeerH` (each prefixed `MysticTransport`; the array
  holds the full names).
- `OPTIONAL_EXPORTS: [&str; 1] = ["MysticTransport_UpdateRelayCredentialH"]`.

The twelve version 1 and five version 2 exports must be present though the trait never calls them (version 1's start
functions always fail; version 2's are wrappers over a default handle).

### 3.2 Types

| Type | Shape | Derives |
| --- | --- | --- |
| `Handle` | `pub struct Handle(pub i32)`; valid handles are > 0, up to 8 per process | `Debug, Clone, Copy, PartialEq, Eq, Hash` |
| `HostOpen` | `pub public_key: PathBuf, pub config: PathBuf, pub capability: PathBuf, pub host_ticket: Option<PathBuf>, pub secrets: PathBuf` | `Debug, Clone, PartialEq, Eq` |
| `JoinOpen` | `pub public_key, pub config, pub ticket, pub secrets: PathBuf` | `Debug, Clone, PartialEq, Eq` |
| `ReadyState` | `Ready(String)`, `TimedOut`, `Ended` | `Debug, Clone, PartialEq, Eq` |
| `SessionResult` | `Clean`, `BadInput`, `Setup`, `RefusedAdmission`, `LeaseFailed`, `UnknownHandle`, `Other(i32)` | `Debug, Clone, Copy, PartialEq, Eq` |
| `Signal` | `pub to_account: AccountId, pub kind: String, pub data: String` | `Debug, Clone, PartialEq, Eq` |

`SessionResult::from_code(i32) -> Self` is `pub const fn`: 0 → `Clean`, 2 → `BadInput`, 3 → `Setup`, 4 →
`RefusedAdmission`, 5 → `LeaseFailed`, -1 → `UnknownHandle`, anything else → `Other(code)` (1 included).

### 3.3 `TransportAbi`

`pub trait TransportAbi: Send + Sync`, synchronous (callers keep it off async executors). Every method returns
`Result<_, HostError>`:

| Method | Signature (after `&self`) | Returns |
| --- | --- | --- |
| `abi_version` | `()` | `u32` |
| `initialize` | `()` | `()` |
| `shutdown` | `()` | `()` |
| `peer_id` | `()` | `rifts_seam::PeerId` |
| `networking_ready` | `()` | `bool` |
| `host_open` | `(open: &HostOpen)` | `Handle` |
| `join_open` | `(open: &JoinOpen)` | `Handle` |
| `wait_ready` | `(handle: Handle, timeout_ms: u32)` | `ReadyState` |
| `session_running` | `(handle: Handle)` | `bool` |
| `stop_session` | `(handle: Handle)` | `()` |
| `last_session_result` | `(handle: Handle)` | `SessionResult` |
| `read_signal` | `(handle: Handle)` | `Option<Signal>` |
| `submit_signal` | `(handle: Handle, from: &AccountId, kind: &str, data: &str)` | `()` |
| `read_peer_event` | `(handle: Handle)` | `Option<String>` |
| `read_diagnostic` | `(handle: Handle)` | `Option<String>` |
| `add_peer` | `(handle: Handle, account: &AccountId, peer: PeerId, pair_key: &Secret)` | `()` |
| `remove_peer` | `(handle: Handle, account: &AccountId, reason: &str)` | `()` |
| `update_relay_credential` | `(handle: Handle, username: &str, credential: &Secret)` | `()` |

Semantics the transport guarantees (from the header; documented on the trait): `submit_signal` for an account not
yet on the roster is held by the transport for 20 s; `read_peer_event` lines are `PEER_CONNECTED`, `PEER_ADMITTED`,
`PEER_LOST`, `PEER_REMOVED`, `PEER_ADD_REJECTED` as written by the transport; a transport without the optional export
keeps the relay credential it started with.

### 3.4 `TransportLoader`

`pub trait TransportLoader: Send + Sync { fn load(&self, dll: &Path, expected_sha256: [u8; 32]) ->
Result<Box<dyn TransportAbi>, HostError>; }`. The launcher passes the Guard manifest's digest for the file.

---

## 4. Dynamic loading (`ffi` module, private; re-exports `NativeTransportLoader`, `sha256_file`)

### 4.1 `sha256_file(path: &Path) -> Result<[u8; 32], HostError>`

1. Read the path's metadata **without following links**. Failure → `Io("transport artifact missing ({path}): {err}")`.
2. If it is not a regular file, is a symlink, or has length 0 → `InvalidInput("transport artifact is not a regular
   non-empty file: {path}")`.
3. Read the whole file. Failure → `Io("could not read {path}: {err}")`.
4. Return its SHA-256.

The launcher calls it directly to pin a digest when the manifest gives none.

### 4.2 `NativeTransportLoader`

`#[derive(Debug, Clone, Copy, Default)] pub struct NativeTransportLoader;` implementing `TransportLoader`. `load`:

1. `sha256_file(dll)?`; if different from `expected_sha256` → `HashMismatch { expected, actual }`. Nothing is loaded.
2. Open the library at exactly `dll` (no name mangling, no search path; the caller chooses the file, today
   `<game Win64>/ParadoxTransport.dll` on every OS). Failure → `Transport("failed to load {path}: {err}")` where
   `{err}` is the loader library's error text.
3. For each name in `EXPORTS`, in order, look the symbol up (without calling it). The first missing one →
   `MissingExport(name)`; the library is dropped.
4. Bind the 17 exports the trait calls (§4.3) and look up the optional one; absence of the optional one is not an
   error.
5. Call `AbiVersion`. If the version is not 3 → `UnsupportedAbi(version)` (a negative C value maps to
   `UnsupportedAbi(0)`).
6. Return the bound transport. It owns the library for as long as any function pointer lives (shared ownership); it
   is `Send + Sync`.

There is no accepted range: exactly 3. The library is never unloaded explicitly; it unloads when the last owner drops.

### 4.3 The C signatures and the calls

All `extern "C"` (`__cdecl` on Windows x64, the platform default). `int` is `i32`; strings in are NUL-terminated
UTF-8; buffers out are caller-allocated, the callee writes a NUL-terminated string of at most `size` bytes.

| Export | C signature | Trait method; buffer sizes; result mapping |
| --- | --- | --- |
| `AbiVersion` | `int ()` | `abi_version`: negative → `UnsupportedAbi(0)`, else the value |
| `Initialize` | `int (char* error, uint32_t error_size)` | error buffer 512 bytes. 1 → `Ok`; else `Transport(text)` or, when the text is empty, `Transport("transport initialization failed")` |
| `Shutdown` | `void ()` | always `Ok` |
| `PeerId` | `uint64_t ()` | 0 → `Transport("transport peer id is 0 (not initialized)")`; else `PeerId(v)` |
| `NetworkingReady` | `int ()` | non-zero → `true` |
| `HostOpen` | `int (const char* public_key_path, const char* config_path, const char* capability_path, const char* host_ticket_path /* may be NULL */, const char* secrets_path, char* error, uint32_t error_size)` | error buffer 1024. Result > 0 → `Handle`; else `Transport(text)` or `Transport("transport host_open failed")`. `host_ticket: None` passes NULL |
| `JoinOpen` | `int (const char* public_key_path, const char* config_path, const char* ticket_path, const char* secrets_path, char* error, uint32_t error_size)` | error buffer 1024; > 0 → `Handle`; else `Transport(text)` or `Transport("transport join_open failed")` |
| `WaitReadyH` | `int (int handle, uint32_t timeout_ms, char* ready, uint32_t ready_size)` | ready buffer 1024. 1 → `Ready(text)`; 0 → `TimedOut`; any other value → `Ended`. Never an error |
| `SessionRunningH` | `int (int handle)` | non-zero → `true` |
| `StopSessionH` | `int (int handle)` | 1 → `Ok`; else `NotFound("unknown session handle {handle}")` |
| `LastSessionResultH` | `int (int handle)` | `SessionResult::from_code` |
| `ReadSignalH` | `int (int handle, char* to_account, uint32_t, char* kind, uint32_t, char* data, uint32_t)` | buffers 128, 32 and 4,097 bytes (the largest signal, 4,096, plus NUL). 1 → `Some(Signal)`; 0 → `None`; anything else → `Transport("transport signal buffer too small")` |
| `SubmitSignalH` | `int (int handle, const char* from_account_id, const char* kind, const char* data)` | 1 → `Ok`; else `Transport("transport refused incoming signal")` |
| `ReadPeerEventH` | `int (int handle, char* line, uint32_t line_size)` | buffer 512. See the line rule below |
| `ReadDiagnosticH` | same | buffer 1024. Same rule |
| `AddPeerH` | `int (int handle, const char* account_id, uint64_t peer_id, const char* pair_key_base64url)` | 1 → `Ok`; else `Transport("transport refused peer addition")` |
| `RemovePeerH` | `int (int handle, const char* account_id, const char* reason)` | 1 → `Ok`; else `Transport("transport refused peer removal")` |
| `UpdateRelayCredentialH` (optional) | `int (int handle, const char* username, const char* credential)` | absent → `Ok` (no call). Present: 1 → `Ok`; else `Transport("transport refused relay credential update")` |

Text read from a buffer: take the bytes up to the first NUL (or the whole buffer when none), decode as UTF-8
replacing invalid sequences, then **trim leading and trailing whitespace**, except `Signal.data`, which is not trimmed
(ICE descriptions end in `\r\n`). This applies to error texts, the ready line, `to_account` and `kind`.

Line rule (`read_peer_event`, `read_diagnostic`): result 1 and a non-empty trimmed text → `Ok(Some(text))`; any other
result, or an empty text, → `Ok(None)`. Neither method ever returns an error.

Strings passed in: each argument is converted to a C string first. An interior NUL →
`InvalidInput("{label} contains an interior NUL byte")`, with label `from_account`, `kind`, `data` (submit),
`account`, `pair_key` (add), `account`, `reason` (remove), `username`, `credential` (update; checked **before** the
optional export's presence, so an interior NUL fails even when the export is absent). Paths must be valid UTF-8,
else `InvalidInput("{label} path is not valid UTF-8: {path}")`, labels `public_key`, `config`, `capability`,
`host_ticket`, `secrets`, `ticket`; then the interior NUL rule with the same label. All C strings live until the
synchronous call returns.

### 4.4 The `unsafe` boundary

`unsafe` is needed only here, at five kinds of site: opening the library; looking up a symbol (twice: the presence
check over `EXPORTS` and the typed binding); and each call through a bound function pointer. Each site's justification,
for the rewrite's own `// SAFETY:` lines:

- open: the file's digest has just matched the signed manifest's;
- lookup: the name is NUL-terminated and the typed signature matches the header (§4.3);
- calls: the function pointers outlive no library (shared ownership, §4.2 step 6); input strings are valid C strings
  for the call's duration; each output buffer is exclusively owned, its true length is the size passed, and the
  callee writes at most that many bytes including the NUL.

dainty-lass's workspace denies `unsafe_code` (`[workspace.lints.rust] unsafe_code = "deny"`, root `Cargo.toml`). The
existing exemptions: `crates/plugin-host/src/lib.rs` opens with a crate-level `#![allow(unsafe_code)]` and its module
doc says it is "the only crate in dainty-lass allowed unsafe code"; `crates/service-plugin` puts an item-level
`#[allow(unsafe_code)]` on the `#[unsafe(no_mangle)]` entry point its macro generates. The MIT `rifts-host` puts a
**module-level** `#![allow(unsafe_code)]` at the top of `ffi.rs` and nowhere else; the rest of the crate keeps the
deny. Under umbrella D1's recommended option `ffi` is still built (for older runtime feeds) but may sit behind a
default-on feature (§7.1). plugin-host's "only crate" sentence then needs the integration pass's edit (R2).

---

## 5. Files the host PC keeps (`files` module)

### 5.1 Constants

| Constant | Value | Where |
| --- | --- | --- |
| `SESSION_FILES` | `["bridge.conf", "transport.secret", "capability.secret", "ticket.secret"]` (this order) | the session folder (§6.4) |
| `ADMISSION_PUBLIC_KEY_FILE` | `p2p-admission-public.xy` | the game folder (Guard role `ADMISSION_KEY`) |
| `TRANSPORT_DLL` | `ParadoxTransport.dll` | the game folder (Guard roles `TRANSPORT_MODULE`, `TRANSPORT_RUNTIME`) |
| `HOST_SETTINGS_FILE` | `p2p-host-settings.json` | the launcher's config dir |
| `HUB_LIMITS_FILE` | `realm-host-limits.json` | the launcher's config dir |
| `DISABLE_COOP_FLAG_FILE` | `DISABLE_COOP_TRANSPORT.flag` | see §6.5 |

The launcher passes its `user_data` directory as both the config dir and the app-local-data dir.

### 5.2 `validate_hub_limits(&HubLimits) -> Result<(), HostError>`

Checked in this order, each an `InvalidInput`:

1. `max_players` not in 1..=64 → `the player limit must be from 1 to 64`;
2. `max_servers` not in 1..=8 → `the server limit must be from 1 to 8`;
3. `afk_seconds` > 7200 → `the AFK limit must be from 0 to 7200 seconds`.

### 5.3 `HostState`

`pub trait HostState: Send + Sync` with `hub_limits(&self) -> Result<Option<HubLimits>, HostError>`,
`save_hub_limits(&self, &HubLimits) -> Result<(), HostError>`, `control_cursor(&self, &AccountId) ->
Result<Option<u64>, HostError>`, `set_control_cursor(&self, &AccountId, u64) -> Result<(), HostError>`.

### 5.4 `FileHostState`

`#[derive(Debug)]`. Constructors `new(config_dir: impl Into<PathBuf>)` (server id `default`) and
`for_server(config_dir, server_id: impl Into<String>)`; accessors `config_dir(&self) -> &Path`,
`server_id(&self) -> &str`. One internal lock serialises every read-modify-write.

**`realm-host-limits.json`** (keys camelCase):

```json
{
  "version": 1,
  "servers": {
    "<server id>": { "maxPlayers": 16, "maxServers": 2, "afkSeconds": 600 }
  }
}
```

Reading:

1. File absent → no limits, not damaged. Other read errors → `Io("could not read {path}: {err}")`.
2. If it parses as the object above (`version` and `servers` both optional, `afkSeconds` optional defaulting to 0;
   numbers out of `u8`/`u32` range fail the parse): keep each entry whose id is 1–64 bytes and whose limits pass
   §5.2. It is **damaged** only when at least one entry was dropped and none kept.
3. Else, if it parses as a single `{ "maxPlayers", "maxServers", "afkSeconds"? }` object (an older layout) that passes
   §5.2: it is this instance's server's entry; not damaged.
4. Else: no limits, damaged.

`hub_limits` returns this server id's entry, ignoring damage. `save_hub_limits`:

1. §5.2 on the input (its error as is); then the server id must be 1–64 bytes, else
   `InvalidInput("invalid server id")`.
2. Read as above. If not damaged and the stored entry already equals the input, write nothing.
3. Else set the entry, and write the whole map with `version: 1`, pretty-printed (serde_json's pretty format, two-space
   indent, no trailing newline), servers sorted by id, fields in the order `maxPlayers`, `maxServers`, `afkSeconds`.
   Encoding failure → `Io("could not encode hub limits: {err}")`.

**`p2p-host-settings.json`**: `{ "controlCursors": { "<account id>": <u64>, ... } }`, pretty, sorted. Reading also
accepts the key `control_cursors`. Absent → empty; unparsable → empty and damaged.

- `control_cursor(account)`: empty account id → `InvalidInput("account id must not be empty")`; else the stored value.
- `set_control_cursor(account, seq)`: account id empty or over 128 bytes → `InvalidInput("invalid account id")`. The
  stored value only moves forward: new = max(stored, seq). If not damaged and new equals stored, nothing is written.
  Encoding failure → `Io("could not encode host settings: {err}")`.

**Atomic write** (both files):

1. Create the parent directory (and ancestors). Failure → `Io("could not create {dir}: {err}")`.
2. If the read found the file damaged and it exists, rename it to `<name>.json.bad` (replacing an older `.bad`).
   Failure → `Io("could not set aside damaged {path}: {err}")`.
3. Remove any stale `<name>.json.tmp` (ignore errors), create it new (exclusive), mode `0600` on Unix, write, flush to
   disk. Failures → `Io("could not create {tmp}: {err}")` / `Io("could not write {tmp}: {err}")`.
4. Rename the `.tmp` over the file. Failure → `Io("could not commit {path}: {err}")`.

The `.bad` and `.tmp` names replace the final `.json` extension: `realm-host-limits.json.bad`,
`p2p-host-settings.json.tmp`.

---

## 6. The session surface (`session` module)

### 6.1 Constants

| Constant | Value |
| --- | --- |
| `LAUNCH_VARIABLES: [&str; 17]` | `MYSTICPARADOX_AUTH_MODE`, `_BIND_ADDRESS`, `_HOST_RUNTIME_CAPABILITY`, `_HOSTED_ROLE`, `_SESSION_ID`, `_SESSION_GENERATION`, `_GAMESERVER_LAUNCH_ID`, `_READY_PIPE`, `_CONTROL_PIPE`, `_GUARD_PIPE`, `_HUNT_GUARD_EVENTS`, `_HUNT_GUARD_COMPLETION_MODE`, `_RUNTIME_PROFILE`, `_LAUNCHER_PID`, `_MAX_PLAYERS`, `_HUB_AFK_SECONDS`, `_TRIALS_WEEK` (each prefixed `MYSTICPARADOX`; this order) |
| `CLEARED_VARIABLES: [&str; 2]` | `MYSTICPARADOX_ALLOW_DAMAGE_DEBUG`, `MYSTICPARADOX_DEBUG_DAMAGE_MULTIPLIER` |
| `DIRECTOR_SERVER_SWITCHES: [&str; 3]` | `-server`, `-log`, `-nullrhi` |
| `MAX_HUB_PEERS: usize` | 64 |
| `MAX_HUNT_PEERS: usize` | 4 |
| `HOST_COMMIT_FLOOR_MIB: u64` | 4096 |
| `HOST_MEMORY_FLOOR_MIB: u64` | 3072 |
| `GAME_LEFT_IDLE: Duration` | 6 s |
| `HUNT_JOIN_LEAVE_MAX: Duration` | 120 s (in `session`, not re-exported at the root) |
| `RELAY_REFRESH_MARGIN_SECONDS: i64` | 1200 (not re-exported) |
| `RELAY_EARLY_REFRESH_SECONDS: i64` | 43200 (not re-exported) |
| `RELAY_EARLY_REFRESH_SPACING: Duration` | 1800 s (not re-exported) |

### 6.2 Types

| Type | Shape | Derives |
| --- | --- | --- |
| `HostedRole` | `Hunt`, `Hub(HubKind)`; `pub const fn wire_role(self) -> &'static str`: `HUNT` / `HUB` | `Debug, Clone, Copy, PartialEq, Eq` |
| `LocalState` | `Starting`, `Ready`, `Running`, `Ended(SessionResult)` | `Debug, Clone, Copy, PartialEq, Eq` |
| `LocalSession` | `pub session: SessionId, pub generation: u64, pub role: HostedRole, pub hosting: bool, pub state: LocalState` | `Debug, Clone, PartialEq, Eq` |
| `HubLimits` | `pub max_players: u8, pub max_servers: u8, pub afk_seconds: u32`; `Default` = 16, 2, 0 | `Debug, Clone, Copy, PartialEq, Eq` |
| `IceServerEntry` | `pub urls: Vec<String>, pub username: Option<String>, pub credential: Option<String>` | `Debug, Clone, PartialEq, Eq, Serialize, Deserialize`, camelCase, both options default `None` |
| `IcePeerEntry` | `pub account_id, pub peer_id, pub transport_key: String` | same, camelCase |
| `RelayGrant` | `pub allowed: bool` (default false), `pub expires_at: Option<String>` (default `None`) | same plus `Default`, camelCase |
| `TransportConfig` | `pub generation: u64, pub local_account_id, pub local_peer_id, pub host_account_id, pub host_peer_id: String, pub ice_servers: Vec<IceServerEntry>, pub peers: Vec<IcePeerEntry>, pub relay: Option<RelayGrant>` (only `relay` defaults) | `Debug, Clone, PartialEq, Eq, Serialize, Deserialize`, camelCase |
| `FilesFor` | `Joiner`, `HuntHost`, `AwayHuntHost`, `HubHost` | `Clone, Copy, Debug, PartialEq, Eq` |
| `IceFiles` | `pub config: String, pub secrets: Secret` | `Debug, Clone, PartialEq, Eq` |
| `PeerEvent` | `Connected { account: String, route: Option<String> }`, `Admitted { account, route }`, `Lost { account, reason: Option<String> }`, `Removed { account, reason, admitted: bool }`, `AddRejected { account, reason }` | `Debug, Clone, PartialEq, Eq` |
| `PeerLive` | `pub route: Option<String>, pub since: String` | `Clone, Debug, PartialEq, Eq` |
| `SessionLive` | `pub hunt_id: String, pub started_at: Option<String>, pub peers: BTreeMap<String, PeerLive>, pub relay_expires_at: Option<String>, pub local_since: Option<String>` | `Clone, Debug, Default, PartialEq, Eq` |
| `PeerListEntry` | `pub account_id, pub peer_id, pub transport_key: String` | `Debug, Clone, PartialEq, Eq, Serialize, Deserialize`, camelCase |
| `WantedPeer` | `pub account_id: String, pub peer_id: u64, pub transport_key: String` | `Clone, Debug, PartialEq, Eq` |
| `PeerDiff` | `pub add: Vec<WantedPeer>, pub remove: Vec<String>` | `Debug, Default, PartialEq, Eq` |
| `CapacityDecision` | `Accept`, `Refuse { reason: &'static str }` | `Debug, Clone, Copy, PartialEq, Eq` |
| `GameTrafficWatch` | opaque; `new(now: Instant)`, `observe(&mut self, line: &str, now: Instant)`, `restart(&mut self, now)`, `idle(&self, now) -> Duration` | `Debug, Clone` |
| `LaunchEnvironmentSpec` | `pub capability: Secret, pub role: HostedRole, pub session_id: SessionId, pub generation: u64, pub launch_id, pub ready_pipe, pub control_pipe, pub guard_pipe: String, pub hunt_evidence: bool, pub guard_completion_mode, pub runtime_profile: String, pub launcher_pid: u32, pub max_players: Option<u8>, pub hub_afk_seconds: Option<u32>, pub trials_week: Option<String>` | `Debug, Clone, PartialEq, Eq` |
| `CommonBootstrapConfig` | `pub key_id: String, pub session_id: SessionId, pub party_id: String, pub party_revision: u64, pub hunt_definition_id: String, pub generation: u64, pub build: u32, pub bridge_protocol: u8, pub virtual_port: u16, pub host_peer_id, pub bridge_instance_id, pub metadata_nonce: String, pub force_relay: bool, pub guard_completion_mode: String` | `Debug, Clone, PartialEq, Eq` |
| `HostBootstrapSpec` | `pub common: CommonBootstrapConfig, pub role: HostedRole, pub capability: Secret, pub host_ticket: Option<Secret>, pub backend_url: String, pub server_udp_port: u16, pub maximum_peers: u8, pub local_client: bool, pub hunt_evidence: bool, pub matchmaker_hunt_id, pub map_path, pub behemoth_path, pub expected_players, pub runtime_profile: String, pub max_players: Option<u8>, pub hub_afk_seconds: Option<u32>` | `Debug, Clone, PartialEq, Eq` |
| `JoinBootstrapSpec` | `pub common: CommonBootstrapConfig, pub role: HostedRole, pub ticket: Secret` | `Debug, Clone, PartialEq, Eq` |

`TransportConfig` methods:

- `relay_expires_at(&self) -> Option<String>`: `relay.expires_at` when `relay` is present and `allowed`, else `None`.
- `relay_turn_credential(&self) -> Option<(String, Secret)>`: the first ICE server that has both a username and a
  credential; `Some` only if both pass `safe_credential`, else `None` (it does not look further).

The launcher deserialises `TransportConfig` straight from the backend's `transport-config` reply and
`Vec<PeerListEntry>` from a reply's `peers`; peer ids are JSON strings.

### 6.2.1 Pure rules (all `pub fn`)

**Identifiers and validation**

- `wire_session_id(&SessionId) -> String`: the session's ULID as its 128-bit integer, 32 lower-case hex digits,
  grouped 8-4-4-4-12 with `-` (36 characters). The node part is ignored.
- `safe_config_value(value: &str, maximum: usize) -> Result<&str, HostError>`: returns the value if it is non-empty,
  at most `maximum` bytes and holds no CR, LF or NUL; else `InvalidInput("the backend returned unsafe player-host
  bootstrap data")`.
- `safe_account(&str) -> bool`: 1–64 bytes of ASCII letters, digits, `-`, `_`, `:`.
- `safe_peer_id(&str) -> bool`: 1–20 ASCII digits, no leading `0`, fits `u64`.
- `safe_key(&str) -> bool`: exactly 43 bytes of ASCII letters, digits, `-`, `_` (unpadded base64url of 32 bytes).
- `safe_url(&str) -> bool`: at most 256 bytes, starts with `stun:`, `turn:` or `turns:`, every byte in 0x21–0x7E and
  none a comma.
- `safe_credential(&str) -> bool`: 1–512 bytes, every byte 0x21–0x7E.

**`ice_files(config: &TransportConfig, files_for: FilesFor, host_peer_id: &str) -> Result<IceFiles, HostError>`**

Every refusal is `InvalidInput("the backend returned an unsafe co-op transport configuration")`. "Host" means any
`FilesFor` but `Joiner`; "hub" means `HubHost`.

1. Refuse unless `local_account_id` and `host_account_id` pass `safe_account`, `local_peer_id` and `host_peer_id` pass
   `safe_peer_id`, and `config.host_peer_id == host_peer_id` (the bootstrap's).
2. Walk `ice_servers` in order. Refuse if any URL of any server fails `safe_url`. A server with **both** username and
   credential: refuse if either fails `safe_credential`; the first such server is the relay credential, and its URLs
   that start with `turn` (so `turn:` and `turns:`) are the TURN list; later credentialed servers are validated and
   otherwise ignored. A server lacking either (or both): its URLs that start with `stun:` join the STUN list; others
   are ignored.
3. Peers. Hub: ignored entirely (neither checked nor written). Otherwise every peer must pass `safe_account`,
   `safe_peer_id` and `safe_key`, and: a host may have at most 3 peers (`HuntHost`) or 4 (`AwayHuntHost`), none whose
   account is the local account; a joiner must have exactly 1, whose account is `host_account_id` and peer id is
   `host_peer_id`. Else refuse.
4. `config` text, in order, each line only when stated:
   `ice_stun_urls=<STUN URLs joined by ,>` (if any), `ice_turn_urls=<TURN URLs joined by ,>` (if any),
   `local_account_id=<local account>` (always), then for a joiner only `local_peer_id=<local peer id>` and
   `host_account_id=<host account>`.
5. `secrets` text: `turn_username=<user>` and `turn_credential=<credential>` when a credentialed server was found and
   the TURN list is non-empty; then, unless hub, one `peer=<accountId>,<peerId>,<transportKey>` line per peer in the
   config's order.

**`parse_ice_ready(line, expected_state, force_relay, local_client) -> Result<u16, HostError>`** (all errors
`Transport`, checked in this order):

1. Split on whitespace. The first token must equal `expected_state` (`HOST_READY` or `ROUTE_READY`), else `the co-op
   transport module returned an unexpected readiness state`.
2. Every other token must be `key=value` (split at the first `=`) with both sides non-empty and no key repeated, else
   `the co-op transport module returned a malformed local route`.
3. `localHost` must be `127.0.0.1` and `transport` must be `ice`, else `the co-op transport module did not return a
   verified loopback route`.
4. `routePolicy` must be `RELAY_ONLY` when `force_relay`, else `AUTOMATIC`; and with `force_relay` and `ROUTE_READY`,
   `route` must be `relay`. Else `the co-op transport module returned a route outside the requested transport
   policy`.
5. `localPort` must parse as `u16`, and be non-zero exactly when `local_client`. Else `the co-op transport module
   returned an invalid local route`. Returns the port.

Examples (the fake transport's): `HOST_READY localHost=127.0.0.1 localPort=8791 transport=ice routePolicy=AUTOMATIC`
and `ROUTE_READY localHost=127.0.0.1 localPort=29100 transport=ice routePolicy=AUTOMATIC route=direct`.

**Peer events**

- `parse_peer_event(&str) -> Option<PeerEvent>`: first whitespace token is the kind; the remaining tokens that contain
  `=` are fields (split at the first `=`; a repeated key keeps the last value; tokens without `=` are ignored).
  `account` is required (`None` without it). Kinds: `PEER_CONNECTED` → `Connected { route }`, `PEER_ADMITTED` →
  `Admitted { route }`, `PEER_LOST` → `Lost { reason }`, `PEER_REMOVED` → `Removed { reason, admitted }` where
  `admitted` is true only for the field value `1`, `PEER_ADD_REJECTED` → `AddRejected { reason }`. Anything else →
  `None`.
- `is_local_admission(&str) -> bool`: first token `ADMISSION_ACCEPTED`, and the remaining tokens include the exact
  tokens `role=host` and `source=local`.
- `apply_peer_event(live: &mut SessionLive, event: &PeerEvent, now_iso: &str)`: `Admitted` inserts or replaces
  `peers[account] = { route, since: now_iso }`; `Lost` and `Removed` remove the account; the others change nothing.
- `apply_peer_line(live, line, now_iso)`: a local admission sets `local_since = Some(now_iso)` and stops; otherwise a
  parsed event is applied; an unparsable line is ignored.

**Roster**

- `wanted_peers(entries: &[PeerListEntry], host_account_id: Option<&str>) -> (Vec<WantedPeer>, usize)`: consider only
  the first `MAX_HUB_PEERS` (64) entries. An entry is wanted if its peer id passes `safe_peer_id`, its account
  `safe_account`, its key `safe_key`, and its account differs from `host_account_id`; otherwise it counts as refused.
  A wanted entry whose account was already taken earlier is skipped silently (neither added nor counted). Order kept.
- `peer_diff(linked: &HashSet<String>, wanted: &[WantedPeer]) -> PeerDiff`: `add` = wanted peers not in `linked`, in
  wanted order; `remove` = linked accounts not wanted, sorted ascending.

**Hosted server launch**

- `trials_week(matchmaker_hunt_id: &str) -> Option<&str>`: for the id `Arena_MatchmakerHunt_<Tier>_<week>` with tier
  `Easy`, `Hard` or `Elite`, the `<week>` when it is exactly three ASCII digits; else `None`.
- `host_server_arguments(server_udp_port, map_path, behemoth_path, matchmaker_hunt_id, expected_players) ->
  Vec<String>`: exactly `-`, the port in decimal, map path, behemoth path, matchmaker hunt id, expected players,
  `127.0.0.1:<port>`, `-server`, `-log`, `-nullrhi`, `-MULTIHOME=127.0.0.1` (11 items, this order). The runtime's
  `ReadLaunch` checks the leading `-` and `-MULTIHOME=127.0.0.1`.
- `windows_command_line(program: &Path, arguments: &[String]) -> String`: the program (lossy UTF-8) always in double
  quotes, then each argument after one space, quoted only when empty or containing a space or tab; inside an
  argument, a `"` is preceded by twice-plus-one the run of backslashes before it (each such backslash doubled plus
  one escaping backslash), and in a quoted argument a trailing run of backslashes is doubled before the closing
  quote. This is the Microsoft C runtime's `CommandLineToArgvW` inverse.
- `build_launch_environment(spec: &LaunchEnvironmentSpec, inherited: impl IntoIterator<Item = (String, String)>) ->
  BTreeMap<String, String>`: start from `inherited` minus every key equal, ignoring ASCII case, to a name in
  `CLEARED_VARIABLES` or `LAUNCH_VARIABLES`; then set:

  | Key | Value |
  | --- | --- |
  | `MYSTICPARADOX_AUTH_MODE` | `HUNT_CAPABILITY` |
  | `MYSTICPARADOX_BIND_ADDRESS` | `127.0.0.1` |
  | `MYSTICPARADOX_HOST_RUNTIME_CAPABILITY` | the capability |
  | `MYSTICPARADOX_HOSTED_ROLE` | `wire_role` |
  | `MYSTICPARADOX_SESSION_ID` | `wire_session_id` |
  | `MYSTICPARADOX_SESSION_GENERATION` | decimal |
  | `MYSTICPARADOX_GAMESERVER_LAUNCH_ID` | `launch_id` |
  | `MYSTICPARADOX_READY_PIPE`, `_CONTROL_PIPE`, `_GUARD_PIPE` | the three pipe fields |
  | `MYSTICPARADOX_HUNT_GUARD_EVENTS` | `1` or `0` |
  | `MYSTICPARADOX_HUNT_GUARD_COMPLETION_MODE` | `guard_completion_mode` |
  | `MYSTICPARADOX_RUNTIME_PROFILE` | `runtime_profile` |
  | `MYSTICPARADOX_LAUNCHER_PID` | decimal |
  | `MYSTICPARADOX_MAX_PLAYERS` | hub role and `Some` only |
  | `MYSTICPARADOX_HUB_AFK_SECONDS` | hub role and `Some` only |
  | `MYSTICPARADOX_TRIALS_WEEK` | when `Some`, any role |

- `sanitize_central_log_line(&str) -> String`: re-join the whitespace-split tokens with single spaces; a token whose
  lower-cased text contains any of `ticket`, `capability`, `token`, `challenge`, `secret`, `signature`, `password`,
  `auth_password` becomes `<text before its first =>=[redacted]`, or `[redacted]` when it has no `=`. After each token,
  once the result is 2,048 bytes or more, cut it to its first 2,048 characters and stop. Example:
  `p2p ticket=secret123 capability=cap456 ok=1` → `p2p ticket=[redacted] capability=[redacted] ok=1`.

**Timers and capacity** (used by the launcher's hosting panel and future hub supervision; pure)

- `udp_stats_local_rx(&str) -> Option<u64>`: for a line containing `UDP_STATS`, the value of the first token starting
  `localRx=` parsed as `u64`.
- `GameTrafficWatch`: `new(now)` starts with no reading and last change `now`. `observe(line, now)`: when the line
  has a `localRx` reading that is the first or strictly greater than the previous, last change becomes `now`; the
  reading is stored either way. `restart(now)` sets last change. `idle(now)` = time since last change (saturating).
- `hunt_join_leave_outcome(asked_at: Instant, traffic: &GameTrafficWatch, now: Instant) -> Option<&'static str>`:
  `game_left` when idle ≥ 6 s; else `timeout` when ≥ 120 s since `asked_at`; else `None`.
- `relay_refresh_due(seconds_left: Option<i64>, since_last: Option<Duration>) -> bool`: false without a value; true
  when ≤ 1200; true when ≤ 43200 and either never refreshed or ≥ 1800 s since the last; else false.
- `relay_restart_due(Option<i64>) -> bool`: a value ≤ 300.
- `default_max_servers(physical_gb: u64) -> u8`: 1 below 4 GB; else `2 * (gb - 4) / 3` (integer) clamped to 1..=6.
- `capacity_decision(commit_mib: u64, active_servers: u8, max_servers: u8) -> CapacityDecision`: `Refuse { reason:
  "HOST_BUSY" }` when commit < 4096 or active ≥ max; else `Accept`.
- `restart_backoff(attempts_in_window: usize) -> Duration`: 0 → 5 s, 1 → 15 s, more → 60 s.
- `attempts_in_window(attempts: &[Instant], now) -> usize`: attempts less than 600 s old.
- `restart_allowed(attempts, now) -> bool`: fewer than 5 in the window.
- `next_idle_since(previous: Option<Instant>, present: usize, now) -> Option<Instant>`: `None` when anyone is present;
  else the previous value, or `now`.
- `training_should_stop(idle_since: Option<Instant>, now) -> bool`: idle for ≥ 300 s.
- `scheduled_restart_due(uptime: Duration, present: bool) -> bool`: nobody present and uptime > 72,000 s.

### 6.3 The backend and spawner ports

**`HostBackend: Send + Sync`** (synchronous; the launcher's implementation blocks on its own runtime):

| Method | Signature (after `&self`) | Default |
| --- | --- | --- |
| `host_bootstrap` | `(&AccountId, &SessionId) -> Result<(HostBootstrapSpec, TransportConfig), HostError>` | required |
| `join_bootstrap` | `(&AccountId, &SessionId) -> Result<(JoinBootstrapSpec, TransportConfig), HostError>` | required |
| `start_hub` | `(&AccountId, HubKind, &HubLimits) -> Result<(HostBootstrapSpec, TransportConfig), HostError>` | required |
| `stop_hub` | `(&AccountId, HubKind) -> Result<Option<SessionId>, HostError>` | required |
| `route_ready` | `(&AccountId, &SessionId, generation: u64, local_port: u16) -> Result<(), HostError>` | required |
| `supports_signalling` | `() -> bool` | `false` |
| `poll_peers` | `(&AccountId, &SessionId, generation: u64, role: HostedRole) -> Result<Vec<PeerListEntry>, HostError>` | `Ok(vec![])` |
| `send_signal` | `(&AccountId, &SessionId, generation: u64, &Signal) -> Result<(), HostError>` | `Ok(())` |
| `poll_signals` | `(&AccountId, &SessionId, generation: u64) -> Result<Vec<(AccountId, Signal)>, HostError>` | `Ok(vec![])` |

`rifts-host` itself makes **no HTTP call**. For the record (it is the launcher's code, specified by the launcher
spec): the launcher's `ServerHostBackend` maps these to `POST /launcher/v1/p2p/hunts/{wire}/host-bootstrap` then
`POST /mystic/v1/hunts/{wire}/host-ready` (`{"lobbyId":"ICE","virtualPort"}`, headers `x-mystic-host-capability`
and `x-rustic-isles-host-capability`), `.../tickets/consume` when there is a host ticket, and
`POST /launcher/v1/p2p/hunts/{wire}/transport-config` (`{"generation"}`); `join-bootstrap` then `transport-config`;
`/launcher/v1/realm-hosting/hubs/start` and `/hubs/stop` (`{"hubKind":"RAMSGATE"|"TRAINING", ...}`);
`.../route-ready` (`{"generation","localBridgePort","bridgeInstanceId"}`); `.../peers` (hub) or `.../transport-config`
(hunt) for `poll_peers`; `.../signal` (`{"generation","toAccountId","kind","data"}`); and
`GET /launcher/v1/p2p/control?afterSequence=N&waitMs=0` with `POST /launcher/v1/p2p/control/{id}/ack` for
`poll_signals` (`ICE_SIGNAL` messages, `payload.fromAccountId/kind/data`). It returns `Signal`s with an empty
`to_account`; `rifts-host` ignores that field on incoming signals.

**`DefaultHostBackend`** (`#[derive(Debug, Default)]`, `new()`, `recorded_route(&self, &SessionId) -> Option<u16>`): a
deterministic local backend. The launcher uses it as its fallback whenever no server is configured, and its tests
use it, so its values are observable:

- Common config for session `s`, generation 1: `key_id` `local-key-1`; `party_id` `party-<wire>`; `party_revision` 1;
  `build` 647472; `bridge_protocol` 1; `virtual_port` 1; `host_peer_id` `100001`; `bridge_instance_id`
  `bridge-<wire>`; `metadata_nonce` `0123456789abcdef0123456789abcdef`; `force_relay` false.
- Transport config for account `a`: `generation` 1; `localAccountId` `a`; `localPeerId` `200002` (joiner) or `100001`;
  `hostAccountId` `host-account-1` (joiner) or `a`; `hostPeerId` `100001`; `iceServers` = first
  `{urls:["stun:stun.cloudflare.com:3478"]}`, then `{urls:["turn:127.0.0.1:3478?transport=udp"], username:
  "1700000000:local", credential:"local-turn-secret"}`; `peers` = for a joiner one
  `{host-account-1, 100001, "A" × 43}`, else none; `relay` `{allowed: true, expiresAt: "2026-10-28T00:00:00Z"}`.
- `host_bootstrap`: hunt definition `Hunt_Default`, completion mode `FIRST_DEFEAT`; role `Hunt`; capability `cap_`
  followed by 39 `A`; host ticket `mpt1.ticket.signature`; backend URL `https://127.0.0.1:8443`; server UDP port 8791;
  maximum peers 4; local client true; hunt evidence false; matchmaker id `Arena_MatchmakerHunt_Easy_001`; map
  `/Game/Maps/HuntMap`; behemoth `Behemoth_Gnasher`; expected players `1`; profile `production`; no hub limits.
- `join_bootstrap`: same common values; role `Hunt`; ticket `mpt1.joiner_ticket.signature`.
- `start_hub(a, kind, limits)`: `validate_hub_limits` first (its error as is). The session id is deterministic:
  a ULID with timestamp 1,792,000,000,000 ms and 80-bit random part `(k << 64) | h`, where `k` is 1 for `Ramsgate`,
  2 for `Training`, and `h` is the account's bytes folded from 0 by `h = h × 131 + byte` (wrapping `u64`); node
  `local`. It is remembered under `(a, kind)`. Hunt definition `ShatteredIsles_ReturnToRamsgate`, completion
  `NO_COMPLETION`; role `Hub(kind)`; capability `cap_HUB_` followed by 35 `A`; host ticket
  `mpt1.hub_host_ticket.signature`; backend URL as above; port 8792; maximum peers = `max_players`; local client true;
  hunt evidence false; matchmaker id `ShatteredIsles_ReturnToRamsgate`; map `/Game/Maps/Ramsgate`; behemoth `None`;
  expected players = `max_players` in decimal; profile `production`; `max_players` and `hub_afk_seconds` `Some`.
- `stop_hub(a, kind)`: removes and returns the remembered session (or `None`).
- `route_ready`: records the port per session (`recorded_route`). Signalling: the trait defaults.

**`ServerSpawner: Send + Sync`**: `spawn(&self, program: &Path, arguments: &[String], working_dir: &Path, environment:
&BTreeMap<String, String>) -> Result<u32, HostError>` (the pid) and `stop(&self, pid: u32)`. RD9: it starts a process
with that environment and nothing else (no injection).

**`NoopServerSpawner`** (`Debug, Default`, `new()`): `spawn` records the arguments and environment and returns 4242;
`stop` does nothing; `last_environment(&self) -> Option<BTreeMap<String, String>>`, `last_arguments(&self) ->
Option<Vec<String>>`. It is what the launcher ships today (the hosted server is not yet started for real; R8).

### 6.4 The session folder

- `create_session_directory(app_local_data: &Path, session_id: &SessionId, generation: u64) -> Result<PathBuf,
  HostError>`: the wire id must pass `safe_config_value(_, 64)`; create `<app_local_data>/p2p-sessions/` with
  ancestors (`Io("could not create {path}: {err}")`); draw 8 random bytes from the OS
  (`Io("could not generate session randomness")` on failure); create
  `p2p-sessions/<wire>-<generation>-<16 lower-case hex>` as a **new** directory (failing if it exists), mode `0700` on
  Unix, default inherited ACL on Windows.
- `write_session_file_0600(path: &Path, contents: &[u8]) -> Result<(), HostError>`: create exclusively (fail if it
  exists), mode `0600` on Unix, write all, flush to disk. Errors `Io("could not create {path}: {err}")`,
  `Io("could not write {path}: {err}")`. On Windows no ACL is set (R5).
- `cleanup_directory(path: &Path)`: remove each of `SESSION_FILES` in the folder, then the folder itself if empty;
  every error ignored. Any other file left in the folder keeps the folder.

Every start flow creates the folder and arranges that any failure after that point removes it (as by
`cleanup_directory`); success keeps it until the session is stopped or seen ended.

### 6.5 `RealmHosting` and `LocalRealmHosting`

`pub trait RealmHosting: Send + Sync`, every method returning `rifts_seam::BoxFuture<'_, Result<_, HostError>>`:
`coop_available() -> bool`, `host_hunt(AccountId, SessionId) -> LocalSession`, `join_hunt(AccountId, SessionId) ->
LocalSession`, `start_hub(AccountId, HubKind, HubLimits) -> LocalSession`, `stop_hub(AccountId, HubKind) -> ()`,
`stop(SessionId) -> ()`, `sessions() -> Vec<LocalSession>`. `LocalRealmHosting` does all its work synchronously when
the method is called and returns an already-completed future (the launcher awaits it; `poll_once` resolves it).

**Construction.** `LocalRealmHosting::new(app_local_data, game_dir, transport: Arc<dyn TransportAbi>)` uses a
`DefaultHostBackend` and a `NoopServerSpawner`; `with_backend_and_spawner(app_local_data, game_dir, transport,
backend: Arc<dyn HostBackend>, spawner: Arc<dyn ServerSpawner>)`; both take `impl Into<PathBuf>` paths, and set the
readiness wait to 5,000 ms. `with_wait_ready_timeout_ms(self, u32) -> Self` changes it (the launcher uses 15,000).
`game_dir` is the game's `Win64` folder. Accessors by session: `session_directory -> Option<PathBuf>`,
`session_launch_environment -> Option<BTreeMap<String, String>>` (empty for a joined session),
`session_local_port -> Option<u16>`. Sessions are kept in a map ordered by `SessionId`.

**`coop_available`** (never an error): `false` if `DISABLE_COOP_TRANSPORT.flag` exists in `app_local_data`, in
`app_local_data/MysticParadox/`, or in `game_dir`; `false` if `abi_version` errs or is not 3; else
`networking_ready`, with an error counting as `false`. Start flows require it, else
`Transport("co-op transport is disabled or not ready")`.

**Host flow** (`host_hunt`: `backend.host_bootstrap`; `start_hub`: `backend.start_hub`; a backend error is returned as
is). Then, in order:

1. Co-op check. If the session id is already in the map → `Busy("session {session} is already running")` (`{session}`
   is `SessionId`'s `Display`, `<ULID>@<node>`).
2. `FilesFor`: `HubHost` for a hub role, else `HuntHost` when `local_client`, else `AwayHuntHost`. `ice_files` with
   the bootstrap's `host_peer_id`.
3. Create the session folder (generation from the bootstrap).
4. Draw 16 random bytes → 32 lower-case hex `X`. Ready pipe `\\.\pipe\MysticInternalServer.ready.X`, control pipe
   `\\.\pipe\MysticInternalServer.control.X`, guard pipe token `MysticHuntGuard.events.X` and pipe
   `\\.\pipe\MysticHuntGuard.events.X`, launch id `host-X`.
5. Launch environment (§6.2.1) from the process's own environment, with the bootstrap's capability, role, session,
   generation, `hunt_evidence`, completion mode, runtime profile, this process's pid, the hub limits, and
   `trials_week(matchmaker_hunt_id)`.
6. Spawn `<game_dir>/Dauntless.exe` with `host_server_arguments(server_udp_port, map_path, behemoth_path,
   matchmaker_hunt_id, expected_players)`, working dir `game_dir`. A spawn error is returned.
7. Build `bridge.conf` (template below); then write `bridge.conf`, `transport.secret` (the ICE secrets),
   `capability.secret` (the capability), `ticket.secret` (the host ticket, or empty) with `write_session_file_0600`,
   in that order. A config-value or write error stops the server (`spawner.stop(pid)`) and is returned.
8. `host_open` with the public key `<game_dir>/p2p-admission-public.xy`, the three paths, and `host_ticket` =
   `ticket.secret`'s path only when `local_client`. An error stops the server and is returned.
9. `wait_ready(handle, timeout)`:
   - `Ready(line)`: when the line starts with `HOST_READY` plus a space, `parse_ice_ready(line, "HOST_READY",
     force_relay, local_client)` gives the port (an error stops the transport session and the server and is returned);
     any other ready text gives `server_udp_port` when `local_client`, else 0.
   - `TimedOut` → `Transport("transport timed out waiting for HOST_READY")`; `Ended` →
     `Transport("transport session ended before HOST_READY")`; `Err` → that error. Each first stops the transport
     session (ignoring its result) and the server.
10. When `local_client`: `backend.route_ready(account, session, generation, port)`; an error stops session and server
    and is returned.
11. If `backend.supports_signalling()`, start the host pump thread (§6.6) with: the roster role, the host account =
    the transport config's `local_account_id`, and the initial linked set = empty for a hub, else the accounts of the
    transport config's `peers` (those already written to `transport.secret`).
12. Record and return `LocalSession { session, generation, role, hosting: true, state: Running }`; the folder is kept.

Host `bridge.conf`, exactly (one `key=value` per line; booleans as `true`/`false` except where `1`/`0` is shown):

```text
key_id=<key_id>
session_id=<wire session id>
party_id=<party_id>
party_revision=<u64>
hunt_definition_id=<hunt_definition_id>
generation=<u64>
build=<u32>
bridge_protocol=<u8>
virtual_port=<u16>
host_peer_id=<host_peer_id>
bridge_instance_id=<bridge_instance_id>
metadata_nonce=<metadata_nonce>
backend_url=<backend_url>
server_udp_port=<u16>
maximum_peers=<u8>
force_relay=<true|false>
guard_pipe_name=MysticHuntGuard.events.<X>
hunt_evidence=<true|false>
expected_server_pid=<pid>
session_role=HUB                 (hub only)
host_local_client=1              (hub only)
host_local_client=0              (hunt without local client only)
<ice_files config lines>
```

The first twelve lines are `CommonBootstrapConfig::to_config_lines(&self) -> Result<String, HostError>`, which first
checks with `safe_config_value`, in this order: `key_id` ≤ 32, wire session id ≤ 64, `party_id` ≤ 192,
`hunt_definition_id` ≤ 128, `guard_completion_mode` ≤ 32 (checked though not written), `host_peer_id` ≤ 20,
`bridge_instance_id` ≤ 96, `metadata_nonce` ≤ 64. `backend_url` is checked ≤ 512.

**Join flow** (`join_hunt`: `backend.join_bootstrap`, its error as is):

1. Co-op check; `Busy` as above.
2. `ice_files(config, Joiner, host_peer_id)`; create the folder.
3. `bridge.conf` = the twelve common lines, then `local_udp_port=0`, `force_relay=<true|false>`, then the ICE config
   lines. Write `bridge.conf`, `transport.secret`, `capability.secret` (empty), `ticket.secret` (the ticket), in that
   order (no server, so an error is just returned).
4. `join_open` with the public key, `bridge.conf`, `ticket.secret`, `transport.secret`. An error is returned.
5. Readiness. Without signalling: one `wait_ready` with the full timeout. With signalling: until the deadline (now +
   timeout), pump signals once (§6.6), then `wait_ready` for the remaining time clamped to 1–50 ms; a `TimedOut`
   before the deadline repeats, anything else ends the loop.
6. `Ready(line)` starting `ROUTE_READY` plus a space → `parse_ice_ready(line, "ROUTE_READY", force_relay, true)` (an
   error stops the transport session and is returned); any other ready text → port 29001. `TimedOut` →
   `Transport("transport timed out waiting for ROUTE_READY")`; `Ended` → `Transport("transport session ended before
   ROUTE_READY")`; `Err` → it; each after stopping the transport session.
7. `backend.route_ready(...)` always for a joiner; an error stops the session and is returned.
8. With signalling, start the join pump thread. Record and return `LocalSession { hosting: false, state: Running,
   role: <bootstrap role>, ... }`. No server pid; empty launch environment.

**Stopping** a recorded session: set its pump's stop flag; `stop_session(handle)` (keep its result); join the pump
thread; `spawner.stop(pid)` when hosted; `cleanup_directory`; return the `stop_session` result (so stopping a
session the transport already forgot gives `NotFound("unknown session handle {n}")`, after the cleanup).

- `stop(session)`: remove it from the map, else `NotFound("session {session} is not running")`; then stop it.
- `stop_hub(account, kind)`: `backend.stop_hub(account, kind)?`. The session to stop is the backend's id when it is in
  the map, else the first recorded session (map order) whose account is `account` and role `Hub(kind)`; none →
  `NotFound("no running hub session for that kind")`; gone by the time it is removed →
  `NotFound("hub session already stopped")`.
- Dropping `LocalRealmHosting` stops every recorded session, ignoring results.

**`sessions()`** (the snapshot the launcher shows): for each recorded session in map order, if its state is
`Starting`, `Ready` or `Running` and `session_running` is not `Ok(true)`, set it to `Ended(last_session_result, or
Clean on error)` and run `cleanup_directory` on its folder. Ended sessions stay in the map (until `stop`). Returns
clones. The start flows only ever record `Running`; `Starting` and `Ready` exist for the launcher's view.

The launcher shows each `LocalSession` as `{ session: <Display>, kind: Hub|Hunt, hosting, generation (u32,
saturating), state, local_port: session_local_port when > 0, result }` where `result` for `Ended` is `clean`,
`bad_input`, `setup`, `refused_admission`, `lease_failed`, `unknown_handle`, or `ended` for `Other`. `SessionLive` and
the peer-event functions are not wired into `LocalRealmHosting` (R7).

### 6.6 Signal and roster pumping

One **signal pass** for `(handle, account, session, generation)`:

1. Read signals from the transport until `None` or an error; send each with `backend.send_signal` (errors ignored).
2. `backend.poll_signals`; on `Ok`, submit each `(from, signal)` with `submit_signal(handle, from, signal.kind,
   signal.data)` (errors ignored). On error, skip.
3. Read and discard peer events until `None`/error, then diagnostics likewise (they are drained so the transport's
   queues do not fill).

**Host pump thread**: until its stop flag is set: if `session_running` is not `Ok(true)`, exit. `poll_peers(account,
session, generation, role)`; on `Ok(entries)`: `wanted_peers(entries, Some(host account))`, `peer_diff(linked,
wanted)`; for each to add, `add_peer(handle, account, PeerId(peer_id), Secret(transport_key))`, and on `Ok` add it to
`linked`; for each to remove, `remove_peer(handle, account, "roster")` and drop it from `linked` whatever the result.
On a poll error, skip the roster this round. Then one signal pass, then sleep 50 ms.

**Join pump thread**: the same loop without the roster step.

---

## 7. Target MIT crate

### 7.1 Shape

`crates/rifts-host` in dainty-lass, `license = "MIT"`, `[lints] workspace = true`, modules `transport` (pub), `ffi`
(private, the only `#![allow(unsafe_code)]`), `files` (pub), `session` (pub), with the root re-exports of §9.

Dependencies: `rifts-seam` (MIT rewrite, umbrella D4), `serde`, `serde_json`, `sha2`, `thiserror` (all in dainty-lass's
`[workspace.dependencies]`); dev `tempfile` (in it). **Seam changes** for the integration pass:

- `libloading = "0.7"` as a workspace entry (today pinned only in `crates/plugin-host/Cargo.toml`); plugin-host would
  switch to `workspace = true`.
- OS randomness: restricted uses `getrandom` 0.4, which dainty-lass does not list. Use the workspace's `rand` (0.10)
  OS-backed generator instead and avoid the seam change; or add `getrandom`.
- Umbrella D1: an optional `rifts-transport` dependency behind a feature `in-process` providing a second
  `TransportLoader`; the FFI path could sit behind a default feature `ffi` so an in-process-only build has no `unsafe`.

### 7.2 Clean-room notes

The pure rules are short and constrained by their contracts; write them from §6.2.1 in your own structure. The
places where this spec deliberately fixes order (error check order, file write order, `bridge.conf` line order,
`EXPORTS` order, environment keys) are contracts, not structure to imitate.

---

## 8. Tests that define the behaviour

### 8.1 Black-box oracle (stays in restricted)

`launcher/src-tauri/src/controller/tests/realms.rs`:
`realm_commands_host_join_stop_keep_rd9_and_never_expose_account_key` (hub host, hunt join, `realm_get`, the four
files present with the account key absent from them and from the environment, `realm_stop` leaves no session
folder, RD9 loader leftovers refused) and `server_host_backend_drives_realm_hub_and_join_across_two_launchers` (two
launchers against a live server: hub, roster, signals). They compile a fake transport `cdylib` with `rustc` at test
time. Both must pass unchanged against the MIT crate.

### 8.2 Unit tests to rewrite (from `lib.rs`)

1. **Exports match the header.** In dainty-lass there is no header; instead assert `EXPORTS` equals the list in §3.1
   (29 distinct names) and `OPTIONAL_EXPORTS` is disjoint from it.
2. **Session result codes**: 0, 2, 3, 4, 5, -1, 9 as §3.2.
3. **Stub refuses everything** (§9.2): `load`, `abi_version`, `update_relay_credential`, `coop_available` (via
   `poll_once`), `hub_limits` each return the seam's `Unimplemented` for `Component::Host` and the operation named
   in §9.2.
4. **Loader against a fake `cdylib`** (built at test time with `rustc --crate-type=cdylib`, extension `dll`, `dylib`
   or `so` by OS; the generated source is test data and may contain `unsafe` since it is not compiled into the
   crate): missing `RemovePeerH` → `MissingExport("MysticTransport_RemovePeerH")`; ABI 2 → `UnsupportedAbi(2)`; wrong
   digest → `HashMismatch` with both digests; a correct load exercises every method: version 3, initialize, peer id
   `0xDEAD_BEEF_CAFE_1234`, networking ready, `host_open` handle > 0, the `HOST_READY` line, a signal
   (`peer-acct-1`, `DESCRIPTION`, `v=0\r\na=ice-ufrag:test\r\n`, untrimmed) then `None`, submit, the peer event
   `PEER_ADMITTED account=peer-acct-1 route=direct`, the diagnostic `ADMISSION_ACCEPTED role=host peer=100001
   source=local`, add and remove peer, the optional export (`Ok`, and an error from the fake's handle 99), result
   `Clean`, stop, not running, shutdown.
5. **Real DLL when built**: skip unless `../../build/Release/ParadoxTransport.dll` exists (restricted only; in
   dainty-lass drop it or point it at an env variable such as `RIFTS_TEST_TRANSPORT_DLL`).
6. **Host state**: limits round trip for server `srv-1`; out-of-range refused; a corrupt file reads as none and the
   next save leaves `realm-host-limits.json.bad`; cursors are per account and never move back.
7. **Hosting end to end** with the fake `cdylib`, `DefaultHostBackend`, `NoopServerSpawner`: co-op on, off with the
   flag in `app_local_data`, on again; hub start → `Hub(Ramsgate)`, hosting, `Running`; the folder under
   `p2p-sessions/` with the four files (`0600` on Unix); the environment's auth mode, bind address, role `HUB`, max
   players 16, AFK 300, cleared variables absent; host a hunt (`01ARZ3NDEKTSV4RRFFQ69G5FAV@local`) with trials week
   `001` and every launch variable but the two hub ones; join (`...FAW@local`) with local port 29100; three sessions;
   `stop_hub`, `stop`, `stop` each delete their folder; `sessions()` empty.
8. **Pure rules**: the peer-line sequence, `wanted_peers` (1 wanted, 2 refused), `peer_diff`, the traffic watch
   (`None` at 3 s, `game_left` at 8 s), `default_max_servers(16) = 6`, `(2) = 1`, `capacity_decision`, the
   sanitiser example.

Add (new, from this spec): `ice_files` for each `FilesFor` (exact `config`/`secrets` texts, each refusal),
`parse_ice_ready` (each error in order, relay policy), `bridge.conf` exact bytes for hub, local-client hunt, away hunt
and join, `windows_command_line` (quotes, backslashes before quotes and at the end), `trials_week`,
`relay_refresh_due`, `build_launch_environment` case-insensitive clearing, failure cleanup (a `host_open` error leaves
no folder and stops the server), and the join pump's sliced wait with a signalling backend fake.

### 8.3 What is not tested

The real transport, a real hosted server and a real 1.14.7 client. Say so when only these tests pass.

### 8.4 Fuzz targets (new)

`rifts-host` parses transport output and backend JSON but has no target today. Add in dainty-lass's `fuzz/`:
`host_ready_line` (`parse_ice_ready` with all flag combinations), `host_peer_line` (`apply_peer_line` on a
`SessionLive`) and `host_transport_config` (`TransportConfig` JSON into `ice_files` for each `FilesFor`; the output
must never contain CR or NUL inside a value). Seam change: `[[bin]]` entries in `fuzz/Cargo.toml`.

---

## 9. Public API

### 9.1 Used outside the crate (must keep name and signature)

From `controller/realms.rs` and `controller/tests/realms.rs` (the only callers; isles-core and isles-1-14-7 do not use
`rifts-host`):

| Item | How it is used |
| --- | --- |
| `ADMISSION_PUBLIC_KEY_FILE`, `TRANSPORT_DLL`, `DISABLE_COOP_FLAG_FILE`, `SESSION_FILES` | joined to the Win64 / user-data paths; tests iterate `SESSION_FILES` |
| `sha256_file` | `rifts_host::sha256_file(&Path)` to pin the DLL's digest |
| `TransportLoader` (`load`), `NativeTransportLoader` (unit value) | the launcher's loader seam |
| `TransportAbi` (`initialize`, `shutdown`; as `Arc<dyn TransportAbi>`) | after load, on launcher shutdown |
| `HostError` (variants of §2.1, `Display`) | matched and constructed (`Io`, `InvalidInput`, `Transport`, `NotFound`) |
| `LocalRealmHosting::with_backend_and_spawner`, `with_wait_ready_timeout_ms`, `session_local_port` | building and viewing |
| `RealmHosting` (all seven methods, awaited) | `realm_host`, `realm_join`, `realm_stop`, `realm_get`, shutdown |
| `LocalSession` (`session`, `role`, `hosting`, `generation`, `state`), `LocalState` (4 variants), `SessionResult` (7 variants), `HostedRole` (`Hunt`, `Hub(HubKind)`) | the realm view |
| `HostBackend` (all nine methods implemented by the launcher), `DefaultHostBackend::new` (fallback and tests) | the launcher's server backend |
| `HostBootstrapSpec` (all 16 fields), `JoinBootstrapSpec` (3), `CommonBootstrapConfig` (all 14) | built from the backend's JSON |
| `TransportConfig`, `PeerListEntry` (`Deserialize`, camelCase) | parsed from replies |
| `Signal` (`to_account`, `kind`, `data`; constructed and read) | the relay |
| `HubLimits` (3 fields, `Copy`), `FileHostState::for_server`, `HostState::hub_limits` | the saved hub limits |
| `ServerSpawner`, `NoopServerSpawner::new`, `NoopServerSpawner::last_environment` | the spawner seam; tests read the environment |
| `default_max_servers`, `wire_session_id` | the default limits; route paths |

### 9.2 The rest of the public surface

Re-exported at the root besides the above: `ffi::NativeTransportLoader`; `files::{FileHostState, HostState,
HOST_SETTINGS_FILE, HUB_LIMITS_FILE, validate_hub_limits}`; every `session` item of §6.1–§6.4 except the four
constants marked "not re-exported"; `transport::{ABI_VERSION, EXPORTS, Handle, HostOpen, JoinOpen, OPTIONAL_EXPORTS,
ReadyState, SessionResult, Signal}`. The modules `files`, `session` and `transport` are themselves public. Keep all of
it: the umbrella's acceptance (§5.6) is that restricted switches by changing dependency lines only.

`Stub` (`#[derive(Debug, Clone, Copy, Default)] pub struct Stub;`) implements `TransportLoader`, `TransportAbi`,
`RealmHosting` and `HostState` by refusing: each method returns `HostError::Unimplemented` for component
`rifts_seam::Component::Host` and the operation `transport.load`, `transport.<method name>` (all 18), the bare
`RealmHosting` method name (`coop_available`, `host_hunt`, `join_hunt`, `start_hub`, `stop_hub`, `stop`, `sessions`)
or `state.<method name>` (4). No caller uses it; keep it for the seam's convention.

Size: about 120 public items (49 functions and methods, 25 types and traits, 18 constants, plus variants and fields).

---

## 10. Acceptance

1. dainty-lass's fmt, clippy (`-D warnings`) and `cargo test --workspace` pass; CI's MIT check passes.
2. `unsafe` appears only in `ffi.rs`, under its module-level allowance, each block with a `// SAFETY:` line.
3. §8.2's tests (rewritten) and §8.2's additions pass on Windows and Linux.
4. Restricted's two launcher tests (§8.1) pass with only `Cargo.toml` changed.
5. Similarity under 30% per file against restricted `4efc8b5`'s `crates/rifts-host/src/*.rs`
   (`clean-room/similarity.md`).
6. A fresh `docs/security/rifts-host.md` in `relay.md`'s structure, correcting the old checklist's errors: 29 exports
   (not 14), `MysticTransport_AbiVersion` (not `ParadoxTransport_GetAbiVersion`), `ROUTE_READY` (not `JOIN_READY`),
   `p2p-host-settings.json` (not `host-settings.json`), `-MULTIHOME=` (upper case), the session folder name
   `<wire>-<generation>-<16 hex>`, no Windows ACL, and `sanitize_central_log_line`'s real rule (no base64 scrub).

---

## 11. Risks and ambiguities

- **R1. Mystic Paradox names on the wire** (umbrella D3): the 30 `MysticTransport_*` export names, the 19
  `MYSTICPARADOX_*` environment names, the pipe names `MysticInternalServer.*` and `MysticHuntGuard.events.*`, the
  folder `MysticParadox` in the flag-file lookup, the file name `ParadoxTransport.dll`, and the `x-mystic-*` header
  the launcher sends. All are contracts with AGPL peers (the C++ transport, the rustic-isles runtime's `ReadLaunch`,
  the backend). Keep them verbatim unless the umbrella decides otherwise; renaming any is a change to both sides.
- **R2. `unsafe` exemption.** plugin-host's module doc claims the only `unsafe` crate. The MIT `rifts-host` needs a
  second, module-level exemption unless D1's in-process path replaces FFI entirely. A maintainer decision and a
  doc edit for the integration pass.
- **R3. Library names per OS.** There are none: the loader takes a path and the launcher always passes
  `ParadoxTransport.dll`. On Linux this cannot `dlopen` a PE file, so realms fail with `failed to load ...` there;
  the restricted module doc's ".dll / .so" is aspirational. D1's in-process transport removes the issue.
- **R4. No version range.** Only ABI 3 is accepted although v1/v2 exports must exist. A future ABI 4 transport is
  refused outright.
- **R5. Secrets on Windows.** The session folder and files get Unix `0700`/`0600`, but on Windows no restricted ACL
  is applied: they inherit `%LOCALAPPDATA%`'s (or the launcher's user-data) ACL. Nor are `bridge.conf` and
  `transport.secret` deleted after readiness; all four files live until stop or until `sessions()` sees the session
  end. The brief that commissioned this spec expected both; neither is in the source. Adding them is a behaviour
  change that needs the transport's confirmation that it reads the files only at open (spec-rd27-rifts-transport.md).
- **R6. `docs/security/host.md` is inaccurate** (§10 item 6), so it must not be used as a description of the code.
- **R7. Dead surface.** `SessionLive`, `apply_peer_*`, `GameTrafficWatch`, the relay/restart/training timers,
  `capacity_decision`, `TransportConfig::relay_*`, `update_relay_credential` and `HostState`'s cursors are public but
  unused by `LocalRealmHosting` and the launcher (peer events and diagnostics are drained and dropped). Keep them for
  API parity; their rules are specified here so the rewrite is faithful, not because they are exercised.
- **R8. The hosted server is not started.** The launcher passes `NoopServerSpawner`, so host flows write the files
  and open the transport against a server that does not exist (expected pid 4242). Out of scope; noted so nobody
  reads a passing host test as gameplay.
- **R9. Process environment.** The launch environment is built from the launcher's whole environment; restricted
  reads it in a way that panics on a non-UTF-8 variable. The rewrite should skip such variables rather than panic
  (a deliberate, harmless difference; record it).
- **R10. Races.** The "already running" check and the final insert are not under one lock, so two concurrent starts
  of one session can both proceed until the second insert replaces the first. The launcher serialises realm
  commands, so this is latent; a rewrite may hold a reservation instead.
- **R11. `sessions()` cleans folders as a side effect** of a status read, and an ended session stays listed until
  `stop`, whose `stop_session` then answers `NotFound`. The launcher depends on `realm_stop` succeeding overall;
  keep the behaviour.
- **R12. `rifts-seam` types** (`AccountId`, `PeerId`, `Secret`, `SessionId`, `GlobalId`, `Ulid::from_parts`,
  `NodeId::new`, `HubKind`, `BoxFuture`, `Unimplemented`, `Component::Host`, `stub`, `poll_once`) come from the seam
  part's rewrite; `rifts-host` cannot start before it lands (umbrella §4.2).
