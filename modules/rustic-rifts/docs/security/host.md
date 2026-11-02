<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: realm host and transport loader (RF7)

The checklist for `crates/rifts-host` (DaintyLass plan RF7, DD12, RD9): loading `ParadoxTransport.dll` via `libloading`,
the per-session folder and `0600` secret files, the hosted server launch environment, and the boundary between realm
hosting and normal game launch.

How to use it: before the launcher enables the `realms` feature in release builds, copy this list into the cutover
notes, check each item in the code (not from memory), link the code or test that shows it, and sign at the end with the
date and the commit reviewed. An item that does not hold is a bug to fix before the cutover, not a box to leave open.

## What it is and where the boundary is

`rifts-host` runs on the player's machine inside the launcher when the optional `realms` Cargo feature is compiled in
(DD12). It loads the native C ABI transport (`ParadoxTransport.dll`), writes per-session configuration and short-lived
bearer credentials to a temporary session folder, spawns the local dedicated server bound to `127.0.0.1`, and bridges
ICE signaling and peer events between the transport and the realms backend.

- **In, launcher IPC:** `realm_host`, `realm_join`, `realm_stop`, `realm_get`. The launcher authenticates with its
  account key to obtain bootstrap material (`HostBootstrapSpec` / `JoinBootstrapSpec`), but the account key itself never
  crosses into `rifts-host`, `bridge.conf`, or the dedicated server's environment.
- **In, transport ABI:** diagnostic lines, peer event lines, signaling frames (`Signal`), and ready lines
  (`HOST_READY` / `JOIN_READY`) read across the C ABI boundary in `crates/rifts-host/src/ffi.rs`.
- **In, local disk:** `ParadoxTransport.dll`, `p2p-admission-public.xy`, `realm-host-limits.json`, and
  `host-settings.json`.
- **Secrets on disk (ephemeral):** `transport.secret`, `capability.secret`, and `ticket.secret` inside
  `<appLocalDir>/p2p-sessions/<prefix>-<session>-<timestamp>-<nonce>/`.
- **Out:** `ParadoxTransport.dll` C ABI calls, the dedicated server child process on `127.0.0.1`, and sanitized runtime
  telemetry (`sanitize_central_log_line`).

Code: `ffi.rs` (`NativeTransportLoader`, `LoadedTransport`, `sha256_file`), `transport.rs` (`TransportAbi`,
`TransportLoader`, `ABI_VERSION`, `EXPORTS`, `OPTIONAL_EXPORTS`), `files.rs` (`FileHostState`, `validate_hub_limits`),
`session.rs` (`LocalRealmHosting`, `write_session_file_0600`, `cleanup_directory`, `build_launch_environment`).

## Checklist

### Native transport verification and FFI boundary

- [ ] `NativeTransportLoader::load` computes the SHA-256 of `ParadoxTransport.dll` (`sha256_file`) and compares it
      against the caller's pinned 32-byte digest before calling `libloading::Library::new`; a mismatch returns
      `HostError::HashMismatch` without loading the image (`transport_loader_verifies_sha256_exports_abi_and_calls_cdylib`).
- [ ] Every required symbol in `EXPORTS` (14 functions) is resolved immediately after loading, and
      `ParadoxTransport_GetAbiVersion()` must equal `ABI_VERSION` (`3`); any missing export or wrong ABI version drops
      the library before any session call can run (`transport_loader_verifies_sha256_exports_abi_and_calls_cdylib`).
- [ ] `ParadoxTransport_UpdateRelayCredential` (`OPTIONAL_EXPORTS`) is resolved optionally; when absent on older builds
      of `ParadoxTransport.dll`, `update_relay_credential` succeeds as a no-op (`real_paradox_transport_dll_loads_when_built`).
- [ ] `crates/rifts-host/src/ffi.rs` is the only module in `rifts-host` with `#![allow(unsafe_code)]`; every `unsafe`
      block and function carries a `// SAFETY:` comment explaining pointer validity, lifetime, NUL termination, and
      buffer capacity invariants.
- [ ] All strings passed across the C ABI are checked for interior NUL bytes (`cstring`) and path/line discipline
      (`safe_config_value`, `safe_account`, `safe_credential`, `safe_key`, `safe_url`).

### Ephemeral session files and local state

- [ ] Session directories are created with a fresh cryptographic nonce under `<appLocalDir>/p2p-sessions/` and written
      with `write_session_file_0600` (`0600` permissions on Unix; restrictive single-file creation on Windows)
      (`realm_hosting_creates_0600_session_files_builds_launch_variables_and_cleans_up_on_stop`).
- [ ] `SESSION_FILES` (`bridge.conf`, `transport.secret`, `capability.secret`, `ticket.secret`) are removed together
      with their parent session directory via `cleanup_directory` whenever a session stops (`stop`, `stop_hub`), when
      `LocalRealmHosting` is dropped, or when `host_open` / `join_open` / `wait_ready` fails during startup
      (`realm_hosting_creates_0600_session_files_builds_launch_variables_and_cleans_up_on_stop`).
- [ ] `FileHostState` writes `realm-host-limits.json` and `host-settings.json` atomically via `.tmp` rename and sets
      aside corrupt JSON as `.bad` rather than silently overwriting or panicking
      (`host_state_persists_limits_and_cursors_and_preserves_corrupt_files_as_bad`).

### Dedicated server launch and RD9 separation

- [ ] `build_launch_environment` strips every variable in `CLEARED_VARIABLES` before populating `LAUNCH_VARIABLES`; the
      player's account key is never placed in the dedicated server environment or in any file under `p2p-sessions/`
      (`realm_hosting_creates_0600_session_files_builds_launch_variables_and_cleans_up_on_stop`).
- [ ] The hosted dedicated server always binds `127.0.0.1` (`MYSTICPARADOX_BIND_ADDRESS=127.0.0.1`,
      `host_server_arguments` with `-multihome=127.0.0.1`); external peers reach it only through the authenticated
      transport tunnel.
- [ ] Realm hosting and joining never inject a DLL into the game client or write `Archon.ini` redirects (RD9); the
      launcher's normal `PLAY` preflight continues to refuse leftover redirect `.ini` files and unknown DLLs in the
      game directory.
- [ ] Central log forwarding scrubs bearer tokens, `Secret` values, and long base64 strings via
      `sanitize_central_log_line`.
