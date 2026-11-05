<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: Rustic Isles sign-in (RX3)

The checklist for `crates/rustic-isles-auth`, the MIT sign-in area dainty-restricted's Rustic Isles module calls
(RD27, `docs/port/spec-rustic-isles-auth.md`). `SECURITY_REVIEW.md` indexes it once the integration pass adds the row.

How to use it: before the isles adapter switches to this crate, copy this list into the change, check each item in
the code (not from memory), link the code or test that shows it, and sign at the end with the date and the commit
reviewed. An item that does not hold is a bug to fix first, not a box to leave open.

## What it is and where the boundary is

The crate answers the game client's sign-in routes, the Epic Online Services stand-in, the hub's encryption keys and
the launcher API (49 routes, `routes.rs`). It runs inside the Rustic Isles backend (port 61000), which listens on
loopback behind the gateway; the gateway is the only public port and already refuses the host-only headers
(`x-rustic-isles-gameserver-apikey` and the old `x-mysticparadox-*` names) from the internet.

- **In:** HTTP requests isles routes to `Auth::handle`, or the crate's own `Auth::router()` in tests. The peer comes
  from `ConnectInfo<ConnectionInfo>`; `x-forwarded-for` counts only from a loopback peer (`client.rs`).
- **Out:** the store trait (`AuthStore`, isles' SQL), the seams (`Seams`: activity, operations, realms, notices, game
  login, erasure, the home API), SMTP to `RUSTIC_ISLES_SMTP_HOST` (`mail.rs`), and HTTPS to `discord.com`
  (`discord.rs`).
- **Files:** the runtime manifest, read from `<LAUNCHER_UPDATE_ROOT>/runtime/client/<channel>/windows-x86_64/latest.json`
  only (`runtime.rs`).

Code: `crates/rustic-isles-auth/src/` (`game.rs`, `launcher.rs`, `tokens.rs`, `hashes.rs`, `password.rs`, `totp.rs`,
`email.rs`, `mail.rs`, `discord.rs`, `policy.rs`, `runtime.rs`, `launcher_build.rs`, `enckeys.rs`, `limiter.rs`,
`client.rs`, `jobs.rs`, `routes.rs`).

Fuzz targets in `fuzz`:

- `rustic_isles_auth_text`: usernames, email addresses and masks, launcher reports and versions, runtime names and
  versions, encryption-key tokens, client addresses and rate-limit keys, TOTP secrets and query strings never panic;
  a valid username is 3 to 16 ASCII letters and digits with a letter, a valid email has one `@`, a mask never holds
  the local part's middle, and a normalized token loses one leading `bearer` and is trimmed.
- `rustic_isles_auth_tokens`: JWT validation of arbitrary text never panics and accepts nothing the test key didn't
  sign; the runtime artifact-set check of arbitrary JSON against a fixed manifest never panics and accepts only sets
  whose files and hashes are the manifest's.

## Checklist

### Credentials and secrets

- [ ] A game server key is a credential: its HMAC is compared with every stored hash, never trusted by presence; no
      match is 401 (`game.rs`, `game_auth_inner`).
- [ ] Never logged: the signing keys, `API_KEY_HASH_SECRET`, game server and account keys, passwords and their
      hashes, refresh tokens, exchange and completion codes, email codes, the Discord secret and verifier, the SMTP
      password and the SMTP reply text. Encryption-key tokens appear only as fingerprints (`enckeys::fingerprint`);
      an address only masked; mail failures by code (`MailError`).
- [ ] `Debug` of `LauncherAccount`, `RefreshSession`, `ExchangeCode`, `DiscordTransaction`, `EmailCode`, `SmtpConfig`,
      `DiscordConfig` and `TokenKeys` leaves out hashes, secrets and verifiers (`store.rs`, `mail.rs`, `discord.rs`,
      `tokens.rs`).
- [ ] Stored only as hashes: opaque tokens as SHA-256, keys and email codes as HMAC-SHA256 under a secret of at least
      32 characters (start fails without one), passwords as Argon2id `m=19456,t=2,p=1` (`hashes.rs`, `password.rs`).
- [ ] Stored hashes are compared in constant time after hex decoding, and a lookup touches every record
      (`hashes::first_match`).
- [ ] Argon2 runs on the blocking pool, not the async threads (`password::*_blocking`).

### Tokens

- [ ] RS256 only: a header naming another `alg` (or `none`) is refused before the signature (`tokens.rs`, `verify`).
- [ ] `iss` and `aud` must be present and accepted; the three kinds have their own audiences, so a launcher or store
      token never opens a game route and the reverse (`tests/jobs.rs`, `tokens_keep_their_kinds_apart`).
- [ ] `exp` present and not passed, `nbf` honoured, no leeway. Validation is on the system clock while signing uses
      the crate's clock (spec §14 item 13, kept).
- [ ] A launcher access token is honoured only while its refresh family has an unrevoked, unexpired session
      (`launcher.rs`, `bearer_inner`).

### Enumeration and single use

- [ ] Login and verify answer the same 401 for an unknown address and a wrong password, with a dummy Argon2
      verification on a miss (`check_password`).
- [ ] Resend and forgot always answer `{"sent":true}` and mail after answering (`code_request`,
      `mail_in_background`); reset reveals a ban only after a right code. Registration's `AUTH_EMAIL_TAKEN` is the one
      admitted leak.
- [ ] Exchange codes (60 s), completion codes (60 s), OAuth states (5 min) and email codes (5 min, 5 tries) are
      consumed atomically by the store (`AuthStore` docs; `MemoryStore` holds one lock per operation).
- [ ] A malformed email code costs no try; a wrong one counts a try in one write; the fifth removes it
      (`accept_code`, `tests/jobs.rs`).
- [ ] Refresh rotates on every use; reuse of a revoked token revokes its family; logout, logout-all and password reset
      end sessions (`tests/launcher.rs`).

### Admission

- [ ] Pending and rejected accounts get no session; banned and disabled never get a code mailed
      (`policy::assert_not_blocked` before `issue_code`).
- [ ] A game token is refused (403) for a launcher account that is banned, disabled, pending, rejected, timed out or
      without a username (`token_eligible`).
- [ ] Exchange redemption re-checks the build, eligibility, the channel and the published runtime hash, then binds the
      realm through the realms seam (`launcher_exchange`).

### Limits and input

- [ ] The attempt windows and the request budget hold at most 16 384 keys each; a new key at the cap evicts the one
      that expires first (`limiter.rs`, `windows_are_bounded`).
- [ ] Per-IP and per-account or per-address windows on register, login, game sessions, every code route and every
      mail send (§4.7's table; `launcher.rs`).
- [ ] Bodies the crate parses itself are capped at 100 KiB, must start with `{` or `[`, and refuse a non-UTF-8
      charset and any content encoding (`util::read_body`).
- [ ] Email validation is linear; API keys at the token route are 1 to 256 UTF-16 units; artifact names are 1 to 96
      safe characters and a report has at most as many entries as known names; launcher versions and hashes have fixed
      grammars (`email.rs`, `runtime.rs`, `launcher_build.rs`).
- [ ] The manifest path is fixed under the update root; nothing the client sends goes into it (`runtime::manifest_path`).

### CORS, Discord and SMTP

- [ ] CORS applies to `/launcher/` paths only, with an exact-match allowlist; an unknown origin is 403 whatever the
      method; game routes never get CORS headers (`launcher_cors`).
- [ ] Discord: PKCE S256, a single-use 5-minute state, `identify` only, HTTPS only with webpki roots and 15 s per
      request; the callback redirects only to the configured deep-link scheme with a code-only error (`discord.rs`,
      `discord_callback`).
- [ ] SMTP: on `starttls` the login never goes in clear (no `STARTTLS` in the `EHLO` answer is `ETLS`); certificates
      are checked against webpki roots with SNI the host; every read and write has a 20 s timeout (`mail.rs`).
- [ ] The code mail carries nothing the user typed, escapes every value and has no `<img`, `<script` or `<link`
      (`render_code_mail`).

### Known weak spots (kept for parity, spec §14)

- [ ] The encryption-key most-recent fallback can hand one player's key to another when a request names nothing
      (`enckeys::lookup`); host runtimes with a roster never get it.
- [ ] Dev mode signs in as any id, but needs `ALLOW_NO_AUTH_DEV_MODE=true` and `NODE_ENV` other than `production`.
- [ ] The `INVALID` sentinel and the bearer-less game server act as a configured account (`rewrite_sentinel`,
      `no_player_account`).
- [ ] The session-ticket path (DN5) skips the eligibility check and names the caller by global id (spec §14 item 3).
- [ ] Discord completion skips email verification under `RUSTIC_ISLES_EMAIL_VERIFICATION=required` (spec §14 item 12).
- [ ] `AUTH_ACCOUNT_TIMED_OUT` answers 500 at game sessions (spec §14 item 1).

## Sign-off

Reviewed at commit `________` on `________` by `________`.
