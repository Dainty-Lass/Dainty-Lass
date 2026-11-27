<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the Ramsgate Reborn importer (RX3)

The checklist for the on-demand importer in `crates/dainty-legacy/src/ramsgate/` and the claim codes in
`crates/rustic-isles-launcher-api/src/legacy_claims.rs`, written from
[`docs/port/spec-legacy-ramsgate.md`](../port/spec-legacy-ramsgate.md) (RD27). The host's guide is
[`docs/legacy-ramsgate.md`](../legacy-ramsgate.md).

How to use it: copy this list into the review, check each item in the code (not from memory), link the code or test
that shows it, and sign at the end with the date and the commit reviewed.

## What it is and where the boundary is

At a player's first character read, with no characters yet, the backend asks the importer for an old save. It reads
one player's documents from a Ramsgate Reborn (1.12) MongoDB and writes them into the 1.14.7 database. Nothing the
old database holds is trusted: another operator runs it, and its players wrote most of it.

- **In:** the old MongoDB (`RUSTIC_ISLES_LEGACY_MONGO_URI`, a secret in `secrets.env`); the 1.14.7 account's
  Discord link (`auth_identities`) or a redeemed claim (`legacy_claims`); a claim code a player types
  (`POST /dainty/api/LegacyClaim`, public through the gateway); a staff request for a code
  (`POST /admin/v1/legacy/claims`, host-only).
- **Out:** the account's save rows (characters, loadouts, inventories, progression, escalation, journey,
  breadcrumbs, bounty and cooldown states, wallet), and one `legacy_imports` row, in one transaction as the `legacy`
  module (the change log, DD23); a claim row and an `admin_audit` row.
- **Who triggers it:** any signed-in player, once, by asking for their characters. A claim is made only by staff.

## Checklist

### The old database is only read

- [ ] The client sends `hello`, authentication, `find` and `getMore` and nothing else (`rustic-isles-db`'s
      `MongoClient`; no write command exists on it).
- [ ] Every read is filtered to one player (`ramsgate/fetch.rs`): `authIdentities` by provider and subject,
      `accounts` by `_id`, the rest by `userId`, `_id` or the player's character ids. Nothing reads a whole
      collection.
- [ ] A filter's keys are refused when they hold a NUL (`find_matching`); its values are the account's own ids.

### Whose save it is

- [ ] A Discord match needs the 1.14.7 account's own Discord link (made by an OAuth sign-in), and exactly one
      `authIdentities` row for that subject in the old database; two are not guessed between.
- [ ] A claim match needs a code staff made for that old user id and the player redeemed while signed in; the code
      is 100 random bits, stored only as its SHA-256, shown once, good for 7 days, used once.
- [ ] A redeem is refused once the account has characters (the import would never run), when the account already
      has a claim or an import, and when the old save is already in another account.
- [ ] Wrong and expired codes get the same answer (`unknown_code`); the route shares the gateway's sign-in rate
      limit (`policy.rs`, `LimitClass::Register`).
- [ ] `legacy_imports`' key (source, old user) and unique (account, source), checked again inside the write
      transaction, keep one old save in one account and one account to one old save, also under two first logins
      at once (`tests/legacy.rs`, `tests/ramsgate.rs`).
- [ ] A ban in force, `disabled`, and approval `pending` or `rejected` on the old account bar the import
      (`fetch::barred`).

### What is written

- [ ] Only documents section 4's ownership rule gives the player are carried; another player's rows, even under the
      same filter, are dropped (`transform::owned`).
- [ ] The old user id is replaced in every string and key, in its four forms, and the import is refused when any
      form of it is still found anywhere (`transform::Refused::Leftover`).
- [ ] No `accounts`, `launcher_accounts`, `auth_identities`, keys, sessions, roles or friends are written:
      `write_player_save` reads only the save areas and drops `accounts` rows.
- [ ] Platinum, 1.14.7's paid currency, never carries (`filter_wallet`).
- [ ] The write runs only while the account still has no character, and an existing save is never overwritten.

### Failure

- [ ] Any failure (the old database down, a bad document, a refused save) is logged and the player gets the
      starter character: a login is never refused because of the importer (`FirstSave::import`).
- [ ] The connect and each read time out (`rustic-isles-db`'s 30 s socket timeouts), so a dead old server delays a
      player's first character read by up to that, once, not every login.
- [ ] The URI is never logged or put in an error: `MongoUri`'s `Debug` hides the password, the importer's `Debug`
      shows only the parsed URI, and setup's refusal names no part of it.

### Not covered

- [ ] Item and currency ids are checked against the 1.14.7 catalog only when one is passed; the composer passes
      none yet, so every 1.12 id but platinum carries (see the guide's open questions).
- [ ] No fuzz target: the transform's input is a database the host chose to trust for reading. Add one
      (`legacy_ramsgate_transform`) if the import is ever opened to files players upload.

Signed: ____ on ____, commit ____.
