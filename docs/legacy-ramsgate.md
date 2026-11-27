<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Importing Ramsgate Reborn saves

A Dainty Lass server can take players over from a Ramsgate Reborn server (Dauntless 1.12, a Mystic Paradox port).
Nothing is copied up front: the first time a player asks this server for their characters and has none, the backend
reads that player's save from the Ramsgate Reborn MongoDB and writes it in before the starter character would be
made. A player who never played there, or whose old account can't be matched, simply starts fresh.

The data facts are [`port/spec-legacy-ramsgate.md`](port/spec-legacy-ramsgate.md) (clean-room, RD27); the review
checklist is [`security/legacy-ramsgate.md`](security/legacy-ramsgate.md).

## Turning it on

1. Get a **read-only** MongoDB user on the Ramsgate Reborn database from its host, and its URI:
   `mongodb://reader:password@rr.example:27017/mysticparadox`. The database is `mysticparadox` when the URI names
   none. SRV URIs (`mongodb+srv://`) are not supported. The connection is not encrypted, so reach it over a network
   you trust (a VPN or an SSH tunnel).
2. Give it to setup on stdin, so it never lands in a shell history, and restart the server:

   ```powershell
   Get-Content rr-uri.txt | bin\dainty-host setup --legacy-mongo-uri
   ```

   It is kept in `secrets.env` as `RUSTIC_ISLES_LEGACY_MONGO_URI` and written into `backend.env`. An empty line on
   stdin turns the import off. The backend logs `[legacy] Ramsgate Reborn saves are imported at a player's first
   character read` at start when it is on.

## How a player is matched

- **Discord.** Ramsgate Reborn signs everyone in with Discord, and keeps the link in `authIdentities`. A player who
  makes their account here with **Continue with Discord** ([oauth.md](oauth.md)) is matched by that Discord user id,
  with nothing else to do.
- **A claim code.** For a player whose old account had no Discord link, or who made their account here another way:
  staff make a code for their old user id, the player enters it in the launcher **before their first play**.

  ```text
  POST /admin/v1/legacy/claims   {"sourceUser": "<Ramsgate Reborn user id>", "reason": "asked in #support"}
  -> {"code": "7Q2KD-M4XBR-0P9TC-HV3JN", "expiresAt": "...", "requestId": "..."}
  ```

  The code is shown once, works for 7 days and only once; only its SHA-256 is stored, and the request is in the
  admin audit log. The player's launcher sends it to `POST /dainty/api/LegacyClaim` (`{"Code": "..."}`), which
  answers `{"Status": "claimed"}` or `{"error": ...}`: `invalid_code`, `unknown_code` (wrong or expired),
  `code_used`, `account_has_save`, `save_taken` or `already_played`.

Each old account fills one account, once, and each account takes at most one old save (`legacy_imports`).

## What carries

| Ramsgate Reborn | 1.14.7 | Notes |
| --- | --- | --- |
| `characters`, `inventories`, `loadouts`, `breadcrumbs`, `encounteredContent` | the same | every character; items keep their instance ids |
| `wallets` | `wallets` | **platinum never carries** |
| `playerJourney`, `progressionTracks`, `progressionObjectives` | the same | |
| `escalationProgress` | `escalationProgress` | reshaped: `season` and `payload` to `seasonId` and the five values |
| `bounties`, `bountyDrafts` | `bountyStates` | folded into one document, in board order |
| `cooldowns` | `cooldownStates` | gatherables older than a day are left out |
| `trackedObjectives`, `playtime` | (none) | left behind and counted |
| accounts, keys, sessions, friends, roles | (none) | never carried; the account here is the player's own |

An old account that is banned (while the ban runs), disabled, or not approved is left behind, and the player starts
fresh. The rows written and what was left behind are kept in the account's `legacy_imports.report`. The write is one
transaction as the `legacy` module, so the change log can revert it (DD23).

## Open questions

These are the spec's section 8; check them against a real Ramsgate Reborn dump before opening the import to players.

- The `provider` value the live Ramsgate Reborn sign-in writes for Discord: the importer looks for `discord`.
- No 1.14.7 catalog check yet: every 1.12 item and currency id but platinum is carried, so an item 1.14.7 lacks
  reaches the player's inventory. Passing the host's catalog export (`catalog_1_14_7.jsonl`) to the importer drops
  them instead.
- Whether a 1.12 escalation season id means anything in 1.14.7.
- Nothing here was played against the real client: the tests are a mock MongoDB and a 1.14.7 store.
