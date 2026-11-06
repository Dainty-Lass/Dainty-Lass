# Rustic Isles goldens against the MIT module

What the composed clean-room module (`rustic-isles-1-14-7` at dainty-lass `3d0a64a8`, `spec_with` and restricted's
version data) answers to every recorded golden under `crates/isles-mit-parity/tests/golden/`, compared with the recorded
answers after each golden's own normalisation. The replay is `crates/isles-mit-parity` (AGPL test crate, a member of
restricted's workspace; see "How it runs"). Run on 2026-11-05, SQLite only, and again after the move into the
workspace with the same result.

**Result: 890 steps replayed, 890 matched, 0 mismatched; 34 steps could not be driven through the composed module.**
There are no class (a) MIT bugs to report from these goldens.

## Summary

| Suite | Golden | Steps | Matched | Mismatched | Not replayable |
| --- | --- | ---: | ---: | ---: | ---: |
| `auth` | `auth/golden.json`, `auth/scenario.json` | 215 | 215 | 0 | 0 |
| `bounties_store` | `bounties_store/1.14.7.json` | 80 | 80 | 0 | 0 |
| `characters` | `characters/node-1.14.7.golden.json`, `characters/scenario.json` | 67 | 67 | 0 | 0 |
| `chat` | `chat/chat.json` | 44 | 44 | 0 | 0 |
| `game_data` | `game_data.json` | 48 | 48 | 0 | 0 |
| `inventory` | `inventory/expected.json`, `inventory/scenarios.json` | 75 | 75 | 0 | 0 |
| `matchmaking_parties` | `matchmaking-parties.json`, `scenario.json` | 38 | 38 | 0 | 0 |
| `progression` | `progression/scenarios-1.14.7.json` | 133 | 133 | 0 | 32 |
| `reforged_api` | `reforged_api.json` | 143 | 143 | 0 | 1 |
| `request_budget` | `request_budget.json` | 5 | 5 | 0 | 1 |
| `game_data_edges` | hand-written from the Node source, kept beside `game_data.json` | 35 | 35 | 0 | 0 |
| `chat_edges` | hand-written from the Node source, kept beside `chat/chat.json` | 7 | 7 | 0 | 0 |
| **Total** | | **890** | **890** | **0** | **34** |

The answers cover every status the goldens record (200 to 503, 302 redirects, 413, 415, 429), not a run of matching
refusals: `target/tmp/mit-parity/<suite>.json` lists each suite's actual statuses.

## Mismatches

None. Each step's normalised status, compared headers and body were equal.

### (a) MIT bugs

None found.

### (b) Recorded departures the specs allow

| Where | What | Decision |
| --- | --- | --- |
| `game_data`: `1.14.7: patchnotes`, `1.14.7: patchnotes, encoded language` (`GET /patchnotes/en/1.14.7.123`, `GET /patchnotes/pt%2DBR/1.0`) | The descriptions and the six list items are the implementer's own prose. Compared on structure: every `description` text and `list` entry masked, the rest (keys, order, titles, types, date, permalink, release version, language) byte for byte. Equal | spec-rustic-isles-gamedata §12 item 1 and its decision ("own patch-notes copy ... restricted's two patch-notes golden cases compare structure only") |
| `request_budget`: the host-capability case | The golden sends `x-mystic-host-capability`; the replay sends `x-rustic-host-capability`, the only name the module accepts. The case is then not replayable (below) | D3, spec-rustic-isles-1-14-7 §7.2, spec-rustic-isles-core §17 (header `x-rustic-host-capability`, no aliases) |
| every suite | `RUSTIC_ISLES_REPLAY_HEADERS=on` in the env file, so the replay clock header counts | spec-rustic-isles-core §17 item 1 |

### (c) Harness gaps, closed before this report

The goldens were recorded with parts of the backend stubbed. Through the composed module those parts are real, so
the replay supplies what the stub did:

- **Stand-in callers.** The goldens name the caller in a test header. The replay sends the real credentials: a
  player is an RS256 game token for that `userId` (signed with the run's key), a game server the game server key
  registered from `GAMESERVER_API_KEYS`, with that player's token when the stand-in named one, and nothing when it
  named nobody.
- **Mail and Discord** (`auth`). A recording mail transport and the stub Discord (`code-<id>`) are put in the
  composition's auth seams. Every other part of the composition is the version crate's own.
- **The Director** (`matchmaking_parties`). The recording used a fake Director. A fake DeployServer on 127.0.0.1:0
  answers `POST /api/matchmaker/handle-matchmaking-for-player` with `203.0.113.10:7777` and `GET /api/hub-status` with
  Ramsgate ready there; `DEPLOYSERVER_URL` points at it.
- **Seed rows** (`chat`, `matchmaking_parties`, `bounties_store`, `inventory`, `progression`, `characters`'
  Slayer's Path step). Written through the MIT store before or between requests: the accounts, characters,
  inventories, wallets with exactly the seeded balances, progression tracks, the journey row and the open reforge.
- **The character lookup** (`bounties_store`). The recording's fake gave every known account a character `char-1`.
  `p1` gets `char-1` and `p2` gets `char-1-p2` (character ids are unique in the store) with empty inventories, and
  `p1` gets 100 Steel Marks before the first token step, the balance the recording's fake wallet held.
- **Purchase-token clock** (`bounties_store`). A purchase token is signed at the module's clock and checked at the
  system clock (auth §5.1, §14 item 13). A token minted under the golden's pinned 2026-10-26 is already expired today,
  so the pinned clock is released from the first `/token/` step on.
- **The starter bootstrap** (`inventory`). The recording stubbed it: an existing inventory was served as it was. The
  composed module repairs a seeded inventory that has no loadout with the starter kit (characters §7.2), so five reads
  carry up to 47 extra starter items. Items whose catalog id is a starter id and which the recording lacks are left
  out of the comparison; the remaining items, their order and every other field are compared as recorded.
- **The chat clock** (`chat`). Node's recording ticked `2026-10-20T12:00:<n>.000Z` per clock read, and the module has
  no way to inject a per-read clock. Each request carries the replay clock header at `12:00:<step>`, and ISO times in
  the bodies are compared as `<iso>`.
- **The request-budget route** (`request_budget`). The golden's `GET /character` answered a test `{"ok":true}`; in the
  composed module it is the real route (401 without a token). An allowed hit is compared as "not refused" with the
  budget's `ratelimit`, `ratelimit-policy` and `retry-after` headers; the refused hit's status, content type and body
  are compared byte for byte.
- **Express's error pages** (`bounties_store`, `progression`). As the goldens' own comparison does: an Express HTML
  error page is compared by its start, and a recorded Express 404 page for a path no route takes matches the module's
  unstubbed 404.

## Not replayable through the composed module (34 steps)

| Suite | Steps | Why |
| --- | --- | --- |
| `progression` | 27 requests in the `host` role, 5 `guard` steps | A player-host runtime (a capability with a roster) and a scripted rifts guard. The recording faked both. The module runs dedicated-only here (`RUSTIC_RIFTS_P2P_MODULE=off`), and a real runtime needs realms and a live hosted session. Their host-write expectations (writes asked, tags, would-refuse notes) belong to these steps only. The gameserver and player steps after them still matched |
| `request_budget` | `host capability bucket allowed at t=15000ms ...` | The host bucket is the one rifts' guard names for a capability it recognises. The recording's guard knew `valid-cap`, the dedicated-only module knows none, and a real one needs a live host. The module answered from the client bucket (429, `"2-in-1min"; r=0; t=45`), which is what core says for a capability the guard doesn't map |
| `reforged_api` | `release gates, legacy key and a bad guard mode` | The case puts `MYSTPAX_OLD` and `MYSTPAX_AGAIN` in the env file. Composed, the Director refuses to start on them (`Rename these environment keys to the RUSTIC_ISLES_ prefix: MYSTPAX_AGAIN, MYSTPAX_OLD`; spec-rustic-isles-director §11, its decision item 1 keeps the refusal), so the module never serves the release gate. The recording served the backend alone |

Also not covered: the Postgres halves (`DAINTY_TEST_POSTGRES_URL` was not set; the replay has no Postgres path yet),
the Director's hunt-manifest digest (`reference/1.14.7/shared/hunt-manifest/golden-digest.json`, rustic-rifts' hunt
tables, not an HTTP golden), and the shadow-replay harness's own tests, which have no recorded gateway capture to
replay.

## How it runs

`crates/isles-mit-parity` is a member of restricted's workspace. It was a workspace of its own until restricted
switched to dainty-lass's MIT `rifts-*` crates (RD27): before that, restricted's `modules/rustic-rifts` members had the
same package names and versions, and one lockfile couldn't hold both. Since the AGPL isles-core and isles-db were
removed (RD27) it holds everything it took from them: the goldens (`tests/golden/`, isles-core's `tests/golden/` before),
the characters normaliser, the inventory recorder's `normalize` and the progression replay's `normalise`
(`tests/common/normalise.rs`), the character data and starter kit constants (`tests/common/starter.rs`), the Reward
Cache plan (`tests/common/reward_cache.rs`), the storage token seeds (`tests/common/src_tokens.txt`) and isles-db's
legacy banner defaults. The values isles-core wrote inline in its normalise and bootstrap code (the series fill, the
trigger node, the banner customization) are written out in `tests/common/version_data.rs`.

Each suite is a test file. It starts the module in process, as the version crate's own composition tests do: a fresh
SQLite file, every port 127.0.0.1:0, one env file, and the golden's game data folder. Where a step changes the
settings, the module restarts on the same database. The suite then sends the recorded requests over HTTP and writes
its tally to `target/tmp/mit-parity/<suite>.json` (`CARGO_TARGET_TMPDIR`): steps, matched, mismatched and skipped, then for each mismatch the step
number, method and path, and both normalised answers. A mismatch never fails the test, so every suite runs to the
end.

```
cargo test -p isles-mit-parity -j 6 --locked
```

It also runs with the rest of `cargo test --workspace`.

The version data is restricted's 1.14.7 character data and starter kit in `version_data.json`'s shape
(spec-rustic-isles-1-14-7 §4.3), handed to `spec_with`: the whole document the host ships, so `rewardCachePlan`,
`storageTokenSeeds` and `legacyStarter` are in it too, although they feed the game-data generators and the db tool,
which no HTTP golden reaches. The `version_data` test keeps `version-data/1.14.7/version_data.json` equal to it.
Rerun on 2026-11-05 after the move out of isles-core: 890 of 890 again.
