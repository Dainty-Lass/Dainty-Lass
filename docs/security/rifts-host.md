<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: Rustic Rifts' realm hosting on the launcher (RD27)

The checklist for `crates/rustic-rifts-host`, the launcher's side of realms: the co-op transport behind the `TransportAbi`
trait, the two JSON files the host PC keeps, and the host and join flows that write a session folder, start the hosted
game server, open a transport session and pump signals and the roster. Written with the clean-room MIT rewrite from
`docs/port/spec-rd27-rifts-host.md` (and the umbrella's D1 and D3). It replaces nothing in this repository; the old
checklist for the AGPL crate (`restricted/modules/rustic-rifts/docs/security/host.md`, retired with it) describes it and is wrong in places, so it was not used.
`SECURITY_REVIEW.md` indexes it.

How to use it: before the launcher switches to this crate, copy this list into the cutover commit, check each item
in the code (not from memory), link the code or test that shows it, and sign at the end with the date and the
commit reviewed.

## What it is and where the boundary is

- **In, from the backend (through the launcher's `HostBackend`, untrusted values):** bootstrap specs (ids, nonces,
  paths, the capability and tickets), `transport-config` replies (ICE servers, credentials, peers and pair keys),
  roster lists and incoming ICE signals.
- **In, from the transport (in process, trusted code, but text that carries peer-controlled parts):** ready lines,
  peer event lines, diagnostics and outgoing signals.
- **In, from disk:** `realm-host-limits.json` and `p2p-host-settings.json` in the launcher's config folder, and the
  disable flag `DISABLE_COOP_TRANSPORT.flag`.
- **Out:** the session folder (`bridge.conf`, `transport.secret`, `capability.secret`, `ticket.secret`), the hosted
  server's command line and environment, and calls into the backend port and the transport.

There is no listener, no HTTP client and no account key here: the launcher's backend holds the key and makes every
call. There is no dynamic loading and no `unsafe` (D1): the transport is the linked `rustic-rifts-transport` crate, so the
old checks on a loaded file (its SHA-256 against the Guard manifest, the export table, the ABI handshake) have nothing
to check. `NativeTransportLoader::load` ignores the path and digest it is given.

Fuzz targets in `fuzz`: `host_ready_line` (`parse_ice_ready`, every state and flag), `host_peer_line`
(`apply_peer_line` on a live view), `host_transport_config` (`TransportConfig` JSON into `ice_files` for each role:
nothing written carries CR or NUL or a line break inside a value).

## Checklist

### Backend values that reach files and the command line

- [ ] Every bootstrap value written to `bridge.conf` passes `safe_config_value` (non-empty, bounded, no CR, LF or NUL)
  before anything is written: `key_id` 32, session 64, `party_id` 192, `hunt_definition_id` 128, completion mode 32,
  `host_peer_id` 20, `bridge_instance_id` 96, `metadata_nonce` 64, `backend_url` 512.
- [ ] `ice_files` refuses the whole config on any unsafe account, peer id, key, URL or credential: accounts 1–64 of
  `[A-Za-z0-9_:-]`, peer ids decimal `u64` without a leading zero, keys 43 base64url characters, URLs
  `stun:`/`turn:`/`turns:` printable with no comma, credentials 1–512 printable bytes (tests `ice_files_for_each_role`).
- [ ] A joiner accepts exactly one peer, the host by account and peer id; a hunt host at most 3 (local client) or 4
  peers and never itself; a hub host writes no peers at all.
- [ ] The bootstrap's host peer id must equal the transport config's, so the backend cannot point a joiner at another
  peer in one reply and bind its ticket to a third.
- [ ] The session id written and used for paths is the 36-character hex form, never the node-tagged text.

### Secrets on disk

- [ ] The session folder is `p2p-sessions/<wire>-<generation>-<16 random hex>`, created new (never reused), `0700` on
  Unix; each file is created exclusively, `0600` on Unix, and flushed (`create_session_directory`,
  `write_session_file_0600`).
- [ ] **Known gap (R5):** on Windows no restricted ACL is set; the folder and files inherit the launcher's data folder
  ACL. Decide before shipping whether the per-user profile ACL is enough.
- [ ] **Known gap (R5):** `bridge.conf` and `transport.secret` stay until the session stops or a snapshot sees it end,
  not only until readiness. The transport reads them on its session thread at open; deleting them at readiness
  needs that confirmed first.
- [ ] Every failure after the folder exists removes it, a stop removes it, and a snapshot that sees a session end
  removes it; any file the crate did not write keeps the folder (test `failures_after_the_folder_leave_nothing_behind`).
- [ ] The account key never appears in any file, argument or environment value (the launcher's tests check the four
  files and the environment).
- [ ] `Secret` values print as `Secret(<redacted>)` in `Debug`; `IceFiles` holds its secrets text as a `Secret`.
- [ ] The two JSON files are written atomically (`.tmp` created new and `0600`, flushed, renamed); a damaged file is
  kept as `.bad`, never parsed into partial state; cursors only move forward.

### The hosted server (RD9)

- [ ] The server is started only through `ServerSpawner` with the environment built by `build_launch_environment` and
  nothing else: the inherited environment minus every launch and debug variable, compared without case.
- [ ] Non-UTF-8 inherited variables are dropped rather than panicking (R9).
- [ ] The command line binds the server to loopback (`127.0.0.1:<port>`, `-MULTIHOME=127.0.0.1`); `windows_command_line`
  quotes as the C runtime splits, so no argument can add another.
- [ ] Pipe names and the launch id carry 16 fresh random bytes per start.
- [ ] **Known gap (R8):** the launcher passes `NoopServerSpawner`, so no server is started; a passing host test is not
  gameplay.

### Transport output

- [ ] A ready line is accepted only as a loopback (`localHost=127.0.0.1`) ICE route with the requested route policy,
  `route=relay` for a relay-only joiner, unique keys and a port that matches the local client flag; anything else
  stops the session.
- [ ] Peer events and diagnostics are drained and dropped by the pumps, so the transport's queues cannot fill; nothing
  from them reaches a file or a command line.
- [ ] `sanitize_central_log_line` redacts any token whose text names a ticket, capability, token, challenge, secret,
  signature or password, and cuts a line at 2048 characters. It does not look for base64 or keys by shape.

### Signals and the roster

- [ ] Incoming signals go to the transport with the sender the backend names; the transport refuses bad accounts,
  kinds and data. The `to_account` of an incoming signal is never read.
- [ ] The roster takes at most 64 entries, refuses unsafe ones and the host's own account, adds each account once and
  removes accounts no longer listed (test `the_host_pump_keeps_the_roster`).
- [ ] A pump stops when its flag is set or the transport session ends, and every stop joins it before the folder goes.

### Failures and concurrency

- [ ] Every mutex recovers from poisoning; no `unwrap` or `expect` outside tests.
- [ ] **Known gap (R10):** the "already running" check and the final insert are not one critical section; the launcher
  serialises realm commands, so two starts of one session cannot race today.
- [ ] Dropping `LocalRealmHosting` stops every session, its server and its pump, and removes every folder (test
  `dropping_the_hosting_stops_every_session`).

## Sign-off

| Date | Commit reviewed | Reviewer | Notes |
| --- | --- | --- | --- |
| | | | |
