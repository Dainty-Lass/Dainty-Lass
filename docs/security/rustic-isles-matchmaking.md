<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: Rustic Isles matchmaking and parties (RX3)

The checklist for `crates/rustic-isles-matchmaking`, the MIT matchmaking-parties area of Rustic Isles (RD27,
`docs/port/spec-rustic-isles-matchmaking.md` §16). It is a trust-boundary component: it parses client JSON and
headers on 18 backend rows, the realms side's candidate JSON, the Director's HTTP answers, and it runs the QoS pong
listener.

How to use it: before a change to this crate merges, and before its cutover is signed (RD10), copy this list into the
change, check each item in the code (not from memory), link the code or test that shows it, and sign at the end with the
date and the commit reviewed.

## What it is and where the boundary is

- **In, from players over the backend**: the candidate and party routes' bodies (`gameMode`, `gameArgs`,
  `playerHuntId`, `buildId`, `partyId`, `instanceId`, `recipientPlayerId`), path params (`:candidateId`,
  `:inviterId`, `:targetId`) and the `x-forwarded-for` header.
- **In, from the realms side** (`rifts_seam::RealmsHooks`, `IslesCore`): candidates as JSON text, accounts as userIds.
- **In, from the Director**: HTTP/1.1 answers to `POST /api/matchmaker/handle-matchmaking-for-player` and
  `GET /api/hub-status` (`DEPLOYSERVER_URL`, loopback, trusted).
- **Out**: the store (reads, home-checked party writes, system outbox writes), the home API's `ring_*` calls, the
  module context's event bus (`isles.party.headline`), the realms hooks.
- **Listener**: the QoS pong on `127.0.0.1` only.

Code: `crates/rustic-isles-matchmaking/src/` (`routes.rs`, `controller.rs`, `status.rs`, `party.rs`, `rules.rs`,
`candidate.rs`, `state.rs`, `outbox.rs`, `startup.rs`, `director.rs`, `rifts_core.rs`, `hooks.rs`).

Fuzz targets in `fuzz`:

- `rustic_isles_matchmaking_inputs`: any bytes as a join body (through `serde_json`), an `x-forwarded-for` value with
  any peer, a candidate's JSON, a `QOS_PORT` value and a build id never panic; a candidate that reads writes back and
  reads again the same; the client address is always one of the entries; `qos_plan` gives a port in 1-65535 or none;
  a party id is always the room spelling, `_` and padded base64.
- `rustic_isles_matchmaking_director_answer`: any bytes as an HTTP/1.1 answer never panic, never read past the
  input, and a de-chunked body is never longer than the input.

## Checklist

### Identity is the token's

- [ ] Every route but `GET /QoS` authenticates through auth's game auth (`routes.rs` `game_auth`), or takes the
  `AuthData` the pipeline already put in the request; a refusal is answered as it is (test `the_scenario`: an
  unauthenticated status is an empty 401).
- [ ] The caller's own userId keys every read and write; `:candidateId` is ignored by the five leave shapes.
- [ ] A party route acts on the caller's party only; the store enforces leadership for kick and promote and the home
  for every party write (`Parties::write` is an `account_write` on the acting account).
- [ ] The member-left hook runs after a kick only when the kick removed the target (a departure from Node, spec §15
  item 8): another player cannot make the realms side drop someone from a forming hunt (`Parties::kick` compares the
  target's party before and after in one transaction).

### The cancel is authoritative

- [ ] A leave cancels the caller's own candidate, and only those party members whose stored candidate is the same
  forming instance (`controller.rs` `cancel_for_leave`; test `a_party_hunt_cancelled_by_its_leader`).
- [ ] A candidate already told to travel is never touched (`too-late`; test `a_leave_after_the_release_is_too_late`).
- [ ] A hub candidate is never cancelled (`hub-not-cancelled`).

### The client's party id is not trusted

- [ ] An ISLAND join that names a party id other than the stored party travels solo (`rules.rs`
  `resolve_matchmaking_party`; unit test `party_resolution`, and the solo leg of `the_scenario`).
- [ ] Offline members are left out of an ISLAND (or realms-filtered) roster: online is chat presence or a game touch
  in the last 75 s.

### Listeners

- [ ] The QoS pong binds `127.0.0.1` only, on the module's `qos` port, and answers a fixed body on every method and
  path (`startup.rs` `QosListener`; test `the_qos_listener_answers_every_path`). Publishing the public port is the
  gateway's decision.
- [ ] A bad `QOS_PORT` starts no listener and is not fatal; `off` or `0` starts none.
- [ ] The backend routes are core's (127.0.0.1).

### Secrets

- [ ] A ring redirect's session ticket goes only to the caller (the body and `x-dainty-session-ticket`) and is
  never logged (`routes.rs` `ring_join`).
- [ ] No setting here is a secret; `HttpDirector`'s `Debug` prints the authority only; `MatchmakingState`'s prints
  counts only.
- [ ] The `AuthData` claims are never logged; the leave logs body key names only, sanitised to `A-Z a-z 0-9 _ . -`
  and capped at 12 × 40 characters, never values.
- [ ] Client text in the join log (mode, args, hunt id) is cut to 120 characters with control characters escaped (a
  departure, spec §15 item 13); the shapes are JSON-quoted and cut to 40.

### Inputs are bounded

- [ ] Bodies go through core's reader (100 KiB; the 400, 413 and 415 pages).
- [ ] Game presence is capped at 100,000 tracked users with a 2 h prune.
- [ ] A candidate from the realms side must be an object with string `HuntId`, `CandidateId` and `GameSessionId`;
  `Port` and `ReadyAt` read 0 unless integers; a set from rifts that is not one is refused `INVALID_CANDIDATE`.
- [ ] A failed launch during a queue pop clears the launching mark (a departure, spec §15 item 4), so one Director
  error cannot refuse every later join of that hunt (test `a_failed_pop_clears_the_launching_mark`).

### Known weak spots (spec §15, §16)

- The Director client reads its answer to the end without a size cap (§15 item 16); the Director is on loopback and
  trusted, and the exchange is bounded to 300 s.
- Queue growth is bounded only by joins (one entry per join, a player possibly twice); hub session ids grow with the
  distinct `host:port` pairs the Director chooses.
- The outbox puts recipients' userIds in the event bus payload (the chat plugin's to deliver, chat §13 item 3).
- The in-memory tables are lost on restart: a candidate mid-search is gone (the realms side restores its own).
- Two client-address readings: matchmaking's keeps `x-forwarded-for` entries over 64 bytes and answers the leftmost
  when every entry is loopback, unlike auth §4.6 (§15 item 9).
- `AuthData` from a session ticket names the account by its global id, so one player can appear under two keys across
  nodes (§15 item 18).

## Sign-off

Reviewed by: ______ on ______ at commit ______.
