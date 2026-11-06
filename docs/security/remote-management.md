<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: remote management (`crates/remote`, RX3, DD27, RM1–RM4)

The checklist for remote management across a controller (`DaintyLass Manager` or `dainty-host` with
`[remote] role = "controller"`) and its enrolled managed server installs (`crates/remote`,
`docs/remote-management.md`, DD27, RM1–RM4). `SECURITY_REVIEW.md` indexes it.

How to use it: before enabling or modifying remote management, check each item in the code (not from memory), link the
code or test that shows it, and sign at the end with the date and the commit reviewed. An item that does not hold is a
bug to fix before merging, not a box to leave open.

## What it is and where the boundary is

One controller install manages enrolled server installs across machines, containers, and regions. Managed installs
pull shared `host.toml` revisions, presets, invites, and runtime/gamedata digests from the controller, report status,
doctor summaries, migration state, and history summaries back, and execute authenticated operator commands (`start`,
`stop`, `setup`, `ban`, `module` load/unload, `history` query/revert, `backup`, `update`, `db`).

Because a compromised controller request or forged enrollment could alter a server's configuration or run operator
actions, and a compromised or curious controller must never obtain a managed install's secrets or player account keys,
the boundary enforces mutual authentication, single-use pinned enrollment, strict secret non-export, local identity
preservation, and `DH6` two-person guarded approval.

- **In, enrollment (`POST /__node/enroll`):** HTTPS on the controller's existing gateway listener (`61000` / `443`),
  rate-limited at the gateway's registration bucket, carrying a one-time 32-byte token (`dainty-lass-remote://enroll`)
  and the managed install's `SubjectPublicKeyInfo` PEM (`-----BEGIN PUBLIC KEY-----`).
- **In / Out, remote management (`POST /__node/remote`, `Service::Remote`):** mutual TLS 1.3 (`dainty-lass-node/1` ALPN)
  over `crates/network` node links (`network::tls::TlsPeerLink` and `network::endpoint::NodeEndpoint`), authenticated
  by `ClusterCa` client and server certificates and checked for `<= 30s` clock skew (`x-node-time`) and `<= 1 MiB`
  body size (`MAX_REMOTE_WIRE`).
- **No new public listener:** "The gateway is the only listener off loopback" (`AGENTS.md`, DD3, DD16) holds without
  exception; neither the controller nor a managed install opens a separate TCP or UDP port for remote management.
- **No player client surface:** `dainty-launcher` has no remote-management commands or listeners (`docs/remote-management.md` §10).

Code: `crates/remote/src/enroll.rs` (enrollment URIs, token validation, SHA-256 fingerprint pinning),
`crates/remote/src/wire.rs` (wire envelopes, `MAX_REMOTE_WIRE`, secret-leak detector, `DH2` role and `DH6` guarded
checks), `crates/remote/src/merge.rs` (`host.toml` merger preserving per-host identity and `local_overrides`),
`crates/remote/src/state.rs` (`RemoteControllerState`, `RemoteManagedAgent`, `sign_enrolled_node_cert`).

Fuzz target: `remote_wire` in `fuzz/fuzz_targets/remote_wire.rs` (enrollment URIs, `EnrollRequest`, `EnrollAnswer`,
`RemoteRequest`, `RemoteResponse`, secret-leak detection, and `merge_managed_host_toml`). It checks that no malformed or
oversized frame panics, that any payload carrying a secret key (`RRK_`, `RRA_`, `secrets.env` variables, PEM private
keys, password hashes, or SQLite database headers) is refused, and that every accepted message round-trips identically.

## Checklist

### Who can use it and how installs enroll

- [x] Enrollment URIs (`dainty-lass-remote://enroll?v=1&...`) require `v=1`, an `https://` controller URL (or loopback
      in local tests) with no userinfo, query, or fragment, a 64-character lowercase hex SHA-256 controller gateway
      certificate fingerprint (`fp`), valid `NodeId` and `Region`, and a 43-character base64url 32-byte non-zero token
      (`enroll::parse_enrollment_uri`). Duplicate or unknown query parameters are refused. Test:
      `tests::enrollment_uri_round_trips_and_rejects_bad_schemes_or_tokens`.
- [x] Before sending `POST /__node/enroll`, the managed install verifies the controller gateway's TLS certificate
      SHA-256 fingerprint against `fp` from the enrollment URI in constant time (`enroll::verify_cert_fingerprint`); a
      mismatch aborts before the token is sent. Test: `tests::certificate_fingerprint_pinning_refuses_mismatch`.
- [x] The managed install generates its ECDSA P-256 keypair locally (`state::generate_node_keypair_and_spki_pem`) and
      sends only `public_key_pem` (`-----BEGIN PUBLIC KEY-----`). Any `EnrollRequest` or `EnrollAnswer` containing a
      `PRIVATE KEY` block is refused (`wire::EnrollRequest::validate`, `wire::EnrollAnswer::validate`).
- [x] The controller stores only `sha256(token)` (`PendingEnrollment.token_sha256_hex`), compares incoming tokens in
      constant time (`subtle::ConstantTimeEq`), enforces `now_ms <= expires_at_ms`, checks `node` and `region` match
      the invitation, and marks `used = true` atomically on first redemption (`state::RemoteControllerState::redeem_enrollment`).
      Replaying a used or expired token is refused with `EnrollmentTokenUsed` or `EnrollmentTokenExpired`. Test:
      `tests::single_use_enrollment_redeems_once_and_refuses_replay_or_expired_token`.
- [x] Every post-enrollment `RemoteRequest` on the controller checks the mTLS-authenticated peer `NodeId` from the
      client certificate (`authenticated_peer`), verifies that `is_authorized_peer` is true (enrolled and not revoked),
      and refuses any request whose body `node` disagrees with `authenticated_peer` (`PeerIdentityMismatch`). On a
      managed install, `handle_controller_push` verifies that `authenticated_caller == self.controller_node`. Test:
      `tests::revoked_install_is_refused_and_unreachable_controller_keeps_running_with_backoff`.
- [x] A managed install has at most one controller (`controller_node`, `controller_url`); enrolling with another
      controller requires `remote leave` first (`docs/remote-management.md` §5.4).

### What crosses (and what never leaves an install)

- [x] Every wire decoder (`decode_enroll_request`, `decode_enroll_answer`, `decode_request`, `decode_response`) enforces
      `bytes.len() <= MAX_REMOTE_WIRE` (`1 MiB`) before parsing JSON (`wire::check_wire_slice`).
- [x] Every wire encoder and decoder runs `wire::contains_forbidden_secret_bytes` / `contains_forbidden_secret_value`,
      which refuses any payload containing:
      - player account keys (`RRK_...`, case-insensitive);
      - admin secret keys (`RRA_...`, case-insensitive);
      - `secrets.env` variables (`REFORGED_ADMIN_API_KEY`, `JWT_SECRET`, `SESSION_SECRET`, `TURN_STATIC_AUTH_SECRET`,
        `AUTH_SIGNING_PRIVKEY_B64`);
      - PEM private keys (`-----BEGIN PRIVATE KEY-----`, `EC PRIVATE KEY`, `RSA PRIVATE KEY`, `OPENSSH PRIVATE KEY`);
      - password hashes (`$2a$`, `$2b$`, `$2y$`, `$argon2`);
      - raw SQLite database files (`SQLite format 3\0`).
      Test: `tests::secret_leak_check_refuses_secrets_env_account_keys_private_keys_and_sqlite_headers`.
- [x] Merging a controller's shared `host.toml` and per-node override (`merge::merge_managed_host_toml`) never overwrites
      per-host identity tables (`[ports]`, `[paths]`, `[database]`, `[network]`, `[remote]`, `[mongo]`, `[wine]`),
      per-host listener keys (`server.host`, `server.game_address`, `server.mode`, `server.port`,
      `history.bucket_*`), or any key listed in `[remote] local_overrides`. Test:
      `tests::config_merge_applies_shared_and_node_overrides_while_preserving_local_identity_and_overrides`.
- [x] Remote `Backup` writes only to the managed install's own `.workspace/backups/` folder, its own `DH4` S3 bucket, or
      its `DH7` log-ship peer, returning only metadata (`backup_name`, `files_copied`, `bytes`) and never database rows
      or files.

### Roles (`DH2`), guarded actions (`DH6`), and audit actor (`DD23`)

- [x] Every `QueuedRemoteCommand` carries an `OperatorEnvelope { controller_node, key_id, role, issued_at_ms }`, where
      `key_id` is an identifier (never `RRA_...` and never `"system"`), and maps to the change-log actor
      `"remote:<controller_node>:<key_id>"` (`OperatorEnvelope::actor_string`).
- [x] Every `RemoteCommand` enforces its minimum `RemoteRole` (`Viewer` for read-only queries/doctor/lookup,
      `Moderator` for kick/timeout/ban, `Admin` for lifecycle/setup/modules/backup/db, `Owner` for `fresh_start`,
      live `HistoryRevert`, `Update`, and `Detached`) on both controller submission and managed-install validation.
- [x] Destructive remote commands (`Setup { fresh_start: true }`, `HistoryRevert { dry_run: false, .. }`) are marked
      `is_guarded() == true`. They cannot execute without `approved_by` naming a second operator (`Admin` or `Owner`)
      with `approver.key_id != requested_by.key_id` after the cooling-off delay (`execute_after_ms`), and can be
      canceled while waiting (`state::RemoteControllerState::approve_guarded`, `cancel_guarded`,
      `wire::QueuedRemoteCommand::validate`). Test:
      `tests::guarded_actions_require_second_operator_with_distinct_key_id_and_delay`.

### Lifetime, revocation, and offline behaviour

- [x] Revoking an install (`RemoteControllerState::revoke_install`) immediately sets `revoked = true`, clears queued
      commands, and causes `is_authorized_peer` and `handle_remote_request` to refuse all further traffic from that
      node with `RevokedPeer`.
- [x] When the controller is unreachable, a managed install continues running on its cached `host.toml` revision,
      never stops a running server or wipes local state, and backs off heartbeat retries (`5s -> 10s -> 20s -> 40s ->
      60s`, `state::offline_backoff_delay_ms`). Test:
      `tests::revoked_install_is_refused_and_unreachable_controller_keeps_running_with_backoff`.

## Open points

- **Multi-controller federation.** By design (`docs/remote-management.md` §5.4, §10), a managed install belongs to at
  most one controller at a time (`remote leave` detaches it before re-enrolling elsewhere).

## Fuzzing

A short run of `remote_wire` happens on every PR (CI's `fuzz` job). For a 24-hour campaign from `fuzz`:

```bash
cargo +nightly fuzz run remote_wire -- -max_total_time=86400
```

Signed: Silent Spud (2026-10-30, `feat/r1-remote-security-and-fuzz`)
