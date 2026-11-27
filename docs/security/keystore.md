<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the launcher's key store (RX3)

The checklist for where the launcher keeps what proves who the player is: the account keys (`DLK_...`), one per
server, and the one Epic session. The code is `crates/dainty-launcher-kit/src/platform/keystore.rs` and its modules
`format.rs`, `os.rs`, `legacy.rs` and `epic.rs`, with the callers in `controller/join.rs` and `controller/account.rs`
and the log's redactor in `platform/log.rs`. The Epic tokens themselves are `dainty_epic::auth::Token`
(`crates/dainty-epic`, see also [epic.md](epic.md)). Written from the MIT code (RD27); `SECURITY_REVIEW.md` indexes
it.

How to use it: before a launcher release, copy this list into the release commit, check each item in the code (not
from memory), link the code or test that shows it, and sign at the end with the date and the commit reviewed.

## What it is and where the boundary is

- **In, from the player's own files (untrusted: anything running as the player can rewrite them):** `<id>.key2`,
  the old launcher's `<id>.key` and `Local State` under `<userData>/keys` and `<userData>`, the plain recovery copy
  `<id>.txt` under `<Documents>/DaintyLass/Account Keys`, `keys/epic.session`, and a backup file the player picks
  with `import_key_file`.
- **In, from the page (through the bridge):** a pasted key (`use_existing_key`, at most `KEY_TEXT_MAX` = 8192
  characters) and the save and import commands, which take no path; the path comes from the OS file dialog.
- **In, from the OS:** DPAPI for the current user on Windows; on Linux a 32-byte file key in the Secret Service.
- **Out:** sealed files, the recovery copy, a backup file where the player says, and a key handed to the network
  client (`net::hostapi`, see [launcher-net.md](launcher-net.md)). The key never goes back to the page.

There is no listener here. The `unsafe` code is the DPAPI calls in `os.rs` (`dpapi::protect`, `unprotect` and
`take`), each with a SAFETY note.

Fuzz target in `fuzz`: `launcher_files` (`settings.json` through `sanitize`, the decrypted text of a key file through
`format::parse_text`, and the GCM and CBC framing through `split_gcm` and `split_cbc`).

## Checklist

### One key per server, bound to its certificate

- [ ] A slot's file name is `slot_id`: lower-case hex of the first 12 bytes of SHA-256 of `public:<host>:<port>` or
  `<host>:<port>`, so a host or port can never pick a path (tests `slot_ids_from_the_spec`,
  `slot_ids_differ_by_mode_and_are_24_hex`).
- [ ] `KeyStore::save` refuses a key that fails `is_plausible_account_key`, and a public slot without a fingerprint
  that passes `is_valid_fingerprint` (64 lower-case hex). A private slot stores none (tests `save_refusals`,
  `private_slots_store_no_certificate`).
- [ ] `KeyStore::load` answers a public slot only for the exact fingerprint it was saved with, and never for `None`.
  Every caller passes the stored server's pin (`controller/join.rs` `load_key`, `refresh_key_flag`;
  `controller/realms.rs`; `controller/versions.rs`).
- [ ] A changed certificate needs the player's explicit accept: `join` returns `CertChanged` with the old fingerprint
  unless `accept_new_certificate` is set, and only then calls `KeyStore::rebind`. `rebind` refuses a private slot or a
  bad fingerprint (test `rebind`).
- [ ] Carrying an account to a moved server (`carry_account`) loads with `None`, so only a private slot's key moves. A
  public key never follows a host change without its certificate.
- [ ] The decrypted text is `DLK2\n<fp or ->\n<key>`, or the older `RRK2`, or a bare first-format key. Anything else is
  no key: another product's `DRK2`, an upper-case fingerprint, a trailing newline or invalid UTF-8
  (`format::parse_text`, test `refuses_what_is_not_a_key`).

### Sealing

- [ ] Windows: `Dpapi` seals for the current user with `CRYPTPROTECT_UI_FORBIDDEN`. The DPAPI buffer is copied out and
  freed once with `LocalFree` (`os.rs` `dpapi::take`; test `dpapi_round_trip`).
- [ ] Linux: `SecretServiceKey` uses AES-256-GCM with a random 12-byte nonce for each seal (`seal_rr1`). The file is
  `rr1` + nonce + ciphertext and tag. The 32-byte key is made from the OS RNG on first use and kept in the Secret
  Service. A wrong key, a flipped bit or a cut prefix fails to open (test `rr1_round_trip_and_tamper`).
- [ ] An OS with neither has `Unavailable`. Then `save` refuses with `Unavailable`, and a `.key2` file that exists
  but can't be opened is an error, not a silent fallback (test `unavailable_encryptor_with_a_key2_file_is_an_error`).
- [ ] A `.key2` file that fails to open, or opens to foreign text, is no key. The plain recovery copy does not stand in
  for it (tests `undecryptable_key2_is_no_key_even_with_a_recovery_copy`, `foreign_text_in_a_key2_file_is_no_key`).

### Reading the old launcher's keys (`legacy.rs`)

- [ ] `.key` files are only read, never written. Windows: the DPAPI-sealed AES-256 key in `Local State`
  (`master_key`, `DPAPI` prefix), then `v10` + nonce + GCM. Linux: AES-128-CBC under PBKDF2-HMAC-SHA1 of `peanuts`
  (`v10`) or of the Secret Service password (`v11`), whole non-empty blocks only (`split_cbc`).
- [ ] A migrated key is written as `.key2`, read back and compared. On a mismatch the new file is deleted and the
  old one is kept (tests `electron_file_is_migrated_and_kept`, `failed_migration_removes_the_new_file`).
- [ ] An unreadable `.key` falls through to the recovery copy, and a store with no reader ignores `.key` files (tests
  `unreadable_electron_file_falls_back_to_the_recovery_copy`, `store_without_electron_reader_ignores_key_files`).

### The key never leaves where it belongs

- [ ] Every key read or saved goes on the log's secret list (`log::register_secret`). The redactor also masks any
  `DLK_` key, `-AUTH_PASSWORD=` value and user API key header by pattern, and a pattern that failed to build hides
  the whole line (`Redactor::redact`; test `no_key_reaches_the_log`).
- [ ] `logout` and `KeyStore::remove` unregister the key and delete all three copies. The first real failure is
  reported only after all three were tried (test `remove_deletes_every_copy_and_unregisters`).
- [ ] No error, `Debug` or `Display` carries a key: `KeyStoreError` and `SealError` have no data, and `Stored`,
  `Backup`, `KeyStore` and `RequestOptions` hide it in `Debug` (tests `debug_hides_the_key`,
  `backup_debug_hides_the_key`).
- [ ] The bridge has no command that returns a key. `use_existing_key` and `import_key_file` take a key in, and
  `adopt_key` saves it only after the current server's `GetUserInfo` accepts it.
- [ ] `import_key_file` reads at most `MAX_KEY_FILE_BYTES`, and only the `Key:` line's key is taken
  (`extract_account_key`).

### Files and folders

- [ ] Every key file, the recovery copy and the backup are written by `write_private`: a `.tmp` file opened `0600`
  on Unix (and set to `0600` again, since a leftover keeps its mode), flushed with `sync_all` and renamed over the
  target. A failed write removes the `.tmp` (test `write_private_replaces_and_leaves_no_tmp`).
- [ ] `load` remakes a missing recovery copy, and a failed recovery write only warns, so saving the sealed key never
  depends on Documents being writable (tests `load_remakes_a_missing_recovery_copy`,
  `recovery_copy_failure_only_warns`).
- [ ] **Known gap:** on Windows `write_private` sets no ACL. The recovery copy is the plain key and inherits the
  Documents folder's ACL, so whatever syncs Documents (OneDrive, for example) gets a copy.

### The Epic session (`epic.rs`, DD22, DE2)

- [ ] `keys/epic.session` is sealed by the same `Encryptor` and never has a Documents copy. Its name can't collide
  with a slot's `.key2` or `.key`.
- [ ] Both tokens go on the log's secret list when saved or loaded, and stay there after sign out (test
  `no_code_or_token_reaches_any_log` in `tests/epic_session_log.rs`).
- [ ] `load` drops a session whose refresh token expired, an unknown format version `v`, or an empty account id or
  token. `Token` shows its text only through `expose`.
- [ ] **Known gap:** `EpicSessionStore::save` writes its `.tmp` with `std::fs::write`, not `write_private`. On Unix
  the file is not `0600` and is not flushed before the rename. The content is sealed, so this exposes ciphertext only.

### Settings

- [ ] `settings.json` never holds a key. `sanitize` keeps only known fields, and a saved server must be a valid host:
  a private one must be loopback, Tailscale or MagicDNS (`is_private_mode_host`), and a public one must carry a
  valid `fp`. At most `MAX_SAVED_SERVERS` servers are kept, and per-server maps are keyed by a 24-hex slot id
  (`settings.rs` `sanitize_server`, `is_server_id`; fuzz target `launcher_files`).

## Open points

- The recovery copy, once `.key2` is missing, is read as plain text and believed. A process running as the player can
  plant a key and fingerprint there, and the launcher will then send that key to the server. The plant needs the
  player's own write access, so it gains nothing a local attacker doesn't have already. Decide whether to keep it.
- `Encryptor::open` on Linux takes the first matching Secret Service item, unlocked first. Any app in the same session
  can read that item, which is the Secret Service's own model.
- The redactor ignores secrets shorter than 6 UTF-16 units. Keys and tokens are longer, so this matters only if a
  shorter secret is ever registered.
- A pasted key is sent to the current server's `GetUserInfo` before it is saved, whichever server it was made for.
  The backup's `Server:` line is not compared.

## Sign-off

| Date | Commit reviewed | Reviewer | Notes |
| --- | --- | --- | --- |
| | | | |
