<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: Rustic Isles Reforged API (RX3)

The checklist for `crates/rustic-isles-reforged`, the MIT crate holding Rustic Isles' Reforged API (RD27;
`docs/port/spec-rustic-isles-reforged-public.md` §19 and `-admin.md` §18). One page for the crate, both halves, as
the two specs ask.

How to use it: before a change to this crate merges, and before its cutover is signed (RD10), copy this list into the
change, check each item in the code (not from memory), link the code or test that shows it, and sign at the end with the
date and the commit reviewed. An item that does not hold is a bug to fix first, not a box to leave open.

## What it is and where the boundary is

The crate answers 62 backend rows (44 public, 18 admin) inside the Rustic Isles backend (port 61000), which listens on
loopback behind the gateway. The gateway refuses `/admin` and the host-only `/dainty/api/*` prefixes from outside
(core's `HOST_ONLY_PREFIXES`); the admin API is off unless `RUSTIC_ISLES_ADMIN_API` or `RUSTIC_ISLES_MANAGER_ADMIN`
is on (core §6.3).

- **In:** core's handler calls; launcher bearers (auth's `launcher_auth_bearer`), account keys (`RRK_`), the update
  publisher's address and key, the sync token, the metrics token, admin session cookies with CSRF tokens, the manager
  operator credential, the host admin key, Discord OAuth callbacks, Guard reports and envelopes, uploads.
- **Out:** the store (`IslesStore`; player and admin writes are `account_write`s with the home check), the home API's
  key and directory registration, the Director's `/api/instances` over plain HTTP on loopback (2 s), Discord through
  auth's `DiscordApi`, the economy's inventory transaction (tester max), the chat crate's home operations.
- **Files:** the update root (`LAUNCHER_UPDATE_ROOT`), the log root (`LAUNCHER_LOG_STORAGE_DIR`), the backend
  game-data folder (status classification, `tester_max_kit.json`).

Code: `crates/rustic-isles-reforged/src/` (public half at the root, admin half in `admin/`).

Fuzz targets in `fuzz`: `rustic_isles_reforged_guard`, `rustic_isles_reforged_text`, `rustic_isles_reforged_paths`,
`rustic_isles_reforged_admin_auth`, `rustic_isles_reforged_admin_text`.

## Public half

### Untrusted input

- [ ] Every file name built from a request is checked as a segment (ASCII letters, digits, `.`, `_`, `-`, not `.` or
      `..`) before a path is joined (`util::is_segment`, `updates.rs`, `logs.rs`).
- [ ] A log upload refuses a caller userId that is not a plain account folder before any write (coordinator decision,
      item 6; `logs::upload`).
- [ ] The operator's view and the sync routes never follow a symbolic link (`symlink_metadata`) and serve only regular
      files; the operator's view checks the session's canonical path is strictly inside the root's (decision item
      11; `logs::locate_session`, `logs::sync_file`).
- [ ] Immutable publishes (the runtime DLL, a Guard manifest) are written create-new; a replay is a 409.
- [ ] Bodies: JSON 100 KiB (core's reader), log files 10 MiB, runtime uploads 200 MiB, envelopes' payloads 256 KiB;
      Guard reports at most 32 modules, file names 260 units, signers 256; the Director read 2 s and 4 MiB; the
      status 500 players and 100 instances.

### Signatures

- [ ] The server only verifies, never signs: Ed25519 with the two compiled-in SPKI keys (`guard_manifest::Trusted`);
      a test key pair only through the `Trusted` slot.
- [ ] Runtime and extra-file uploads are refused before anything is written when the signature fails.
- [ ] Guard heartbeats are signed by the session's ephemeral key over the 17-line material; an old challenge or a
      replayed sequence is refused (tests `guard_flow`).

### Secrets

- [ ] The publisher key, the sync token, the metrics token are compared in constant time and never logged.
- [ ] Account keys are stored only as auth's HMAC and compared in constant time over every row.
- [ ] The request log redacts kill-session tokens, JWT-shaped segments and the secret query names
      (`observe::sanitize_url_for_log`).
- [ ] No `Debug` of state, records or settings prints a key, token, challenge or public key.

### Trust boundary

- [ ] Publisher routes need an allow-listed address (the client address behind a loopback proxy, or the peer) and the
      key; the body read keeps the request's peer for that check (a test publishes from 127.0.0.1 and is refused from
      10.0.0.5).
- [ ] The sync routes need their own token; every operator view (logs, notices, operations, activity, Guard) runs
      inside the admin wrapper.

### Known weak spots (kept as parity; listed for the maintainer)

- The metrics route is open to loopback without a token outside `NODE_ENV=production`.
- The launcher-build verdict is always unknown (the pin is looked up on `win64`), so
  `RUSTIC_ISLES_UNOFFICIAL_LAUNCHER=refuse` never refuses (public spec §21 item 1).
- An expired current Guard manifest blocks the next publish (§21 item 10).
- Attempt windows are in memory and per module (a restart resets them).
- The defaults `../MysticLauncherLogs`, `https://paradox.mysticfox.dev` and the `pranav158` release prefix are kept for
  parity: a host that sets nothing advertises and allows domains DaintyLass does not control.

## Admin half

### Credentials

- [ ] The session cookie is `HttpOnly; SameSite=Strict; Path=/admin/v1`, 8 hours, stored only as its SHA-256; every
      state change carries the CSRF token (constant time); the session's account is re-checked on every request and
      the session revoked when it loses `admin` or eligibility.
- [ ] The manager operator credential: loopback peer, no forwarding header, the five-line HMAC-SHA256 (method,
      target, timestamp, operator id, body SHA-256) in constant time, 60 s back and 5 s ahead, a 120 s per-module
      nonce set, a JSON body only; checked before any handler (`admin::auth::verify_operator`). A request signed by
      `crates/admin`'s `operator_signature` is admitted (test `a_request_crates_admin_signs_is_admitted`).
- [ ] With the manager switch on, the module refuses to start without a 32-character secret.
- [ ] The host routes: `DAINTY_ADMIN_KEY` of 32 units or more, a loopback peer, no `x-dainty-gateway` or
      `x-forwarded-for`, the key compared by SHA-256 in constant time.
- [ ] Core's admin authenticator slot (the `/admin/` rifts rows) runs the same origin check and credentials; core now
      hands it the method, target and body (the commit before the crate).

### Sign-in

- [ ] Six attempts per client address per 15 minutes; the password, the `admin` role and eligibility; TOTP when a
      secret is set, and production without one is a 503.
- [ ] Discord: an allow list of numeric ids, PKCE S256, the state HMAC-bound to an HttpOnly `SameSite=Lax` cookie,
      5-minute single-use transactions, the binding burned on every callback.

### Clocks

- [ ] Every admin trust decision (session expiry, the attempt windows, TOTP, the operator window and nonces) reads the
      system clock, never the replay clock (admin decisions, item 1).

### Origins and self-protection

- [ ] A cross-origin call is answered only for a listed origin (production lists none by default); every admin answer
      is `no-store` with a deny-all CSP.
- [ ] An admin cannot ban, reject, disable or delete their own account; deletion needs the typed name and a reason;
      every change is audited in the same write with actor, IP and request id.

### Known weak spots (kept as parity)

- A missing email answers the login without a password check (timing tells whether an address has an account).
- The origin check passes a request without `Origin`; browser protection rests on `SameSite=Strict` and CSRF.
- The operator's display name is unsigned; the audit records the signed id.
- The Discord state is consumed before the binding is checked: a stray callback burns a legitimate attempt.
- The release-gates route reveals the active player count to any loopback caller.
- `SetAccountStatus` bans without revoking sessions; the next refresh is refused by auth.

## Sign-off

Reviewed by: ______ Date: ______ Commit: ______
