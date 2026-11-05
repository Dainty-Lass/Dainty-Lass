<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# Clean-Room Behaviour Specification: `rustic-isles-economy`, part 2 of 3: progression (RD27)

> **Scope and purpose (RD27):** this is the clean-room behaviour specification of the **progression** part of the MIT
> crate `rustic-isles-economy` (unit 7 of [`spec-rustic-isles-core.md`](spec-rustic-isles-core.md) §16): XP tracks
> and their rank curves, the authoritative XP grant and its ledger, the rank confirm, the objective report and
> mastery, escalation season progress, the Slayer's Path (player journey map) and its rewards, encountered content,
> breadcrumbs, the Hunt Pass read, the bootstrap stubs, the session supplies refilled at login, and the XP half of the
> test multipliers. The other two parts of the same crate are
> [`spec-rustic-isles-economy-inventory.md`](spec-rustic-isles-economy-inventory.md) (inventory and the reforge
> reset) and [`spec-rustic-isles-economy-bounties.md`](spec-rustic-isles-economy-bounties.md) (bounties and the
> stores); §9 is where the three meet. The source is dainty-restricted's `modules/rustic-isles/crates/isles-core`
> files `src/areas/progression.rs` and `src/areas/progression/*`, with the XP half of
> `src/areas/matchmaking/multipliers.rs`, which are AGPL-3.0-only with Mystic Paradox's section 7 terms; an
> implementer who never sees them writes the MIT code from this document alone.
>
> Everything below is stated from observable contracts: routes and their bytes, the order of checks a client can
> tell apart, the rules as input/output tables, what is stored through the store spec's repositories, the settings,
> the calls other parts make, and the tests. No implementation code is reproduced and no private helper is named.
> Node names (`GrantProgressionXp`, ...) appear only as provenance. Game-derived values (rank curves, journey graphs,
> objective ids, escalation tables) are inputs read from the host's game data, never committed; §6 gives their
> formats.
>
> Written against dainty-restricted `e0e238b4434316bb210677d88bc59e3448688794` (worktree
> `splashloader-integration-90130d`; the files of this unit last changed in `a8681fe`) and dainty-lass
> `731f928af2c098c1c4d61e1448c0b80ce69609ef` (branch `feat/isles-auth-clean-room`).

---

## 1. Provenance and classification

| Restricted file (`crates/isles-core/src/areas/`) | Ports (Node, provenance only) | Rewritten as |
| --- | --- | --- |
| `progression.rs` | `routes/progression.ts`, the escalation, Hunt Pass and stub routes of `routes/system.ts` | §3 route table, §10.3 registration |
| `progression/req.rs`, `auth.rs`, `util.rs` | `req.AuthData`, `ResolveRequestUserId`, the `Is*Reader/Reporter/Writer` predicates, `extensions/p2p.ts`'s `hostWrites` | §2, §5 |
| `progression/tracks.rs` | `controllers/progression.ts` (curves, allowlist, host limits, grant ids) | §4.1 to §4.4 |
| `progression/xp.rs` | `GrantProgressionXp`, `CaptureProgressionObjectiveEvent`, the rank confirm | §3.4 to §3.6, §4.3, §4.8 |
| `progression/reforge.rs` | the XP-route half of `controllers/reforge.ts` | §4.5, §4.6 |
| `progression/mastery.rs` | `controllers/masteryReport.ts` | §4.9 |
| `progression/escalation.rs` | `controllers/escalationProgress.ts` | §4.10 |
| `progression/journey.rs` | `controllers/slayersPath.ts`, the journey half of `controllers/progression.ts` | §4.11, §4.12 |
| `progression/supplies.rs` | `sessionSupplies.ts` (Reforged) | §4.14 |
| `progression/ports.rs` | the calls into other areas | §9 |
| `progression/testkit.rs`, `testkit/golden.rs` | test support | §13 (described only) |
| `matchmaking/multipliers.rs` (XP half) | `testMultipliers.ts` | §4.7 |

All of it ports Paradox or Reforged Revival TypeScript (category B): everything is rewritten. The reward-items half of
the multipliers is the inventory part's; the start-up line stays where core §12 puts it.

---

## 2. Conventions

### 2.1 Answers

- **JSON** as core §2.1 / auth §2.2: `service_kit::json::Json` (JavaScript `JSON.stringify` numbers: `3`, never
  `3.0`; a non-finite number is `null`), `content-type: application/json; charset=utf-8`, keys in the order written
  here.
- **Envelope** `E(status, message, payload)` = `{"code":null,"message":<message>,"payload":<payload>}`. The ones used:
  - **OK**(p) = 200 `{"code":null,"message":"OK","payload":p}`;
  - **BAD** = 400 `{"code":null,"message":"Bad Request","payload":{}}`;
  - **FORBIDDEN** = 403 `{"code":null,"message":"Forbidden","payload":{}}`;
  - **CONFLICT** = 409 `{"code":null,"message":"Conflict","payload":{}}`.
- **Empty** n = status n, no body, no content type.
- **Hook reply** = a `rifts_seam::Reply` answered as it is: its status, `content-type: application/json;
  charset=utf-8`, its body text unchanged.
- **NO DB** = empty 500, logged (error) `progression: no database is configured`.
- **STORE** = empty 500, logged (error) `progression: {error}` (any repository error, `NotHome` and `Home`
  included).
- **NO ACCOUNT**(u) = BAD, logged (warn) `progression: no account for userId {u'}` where `u'` is the first 128
  characters of `u` with every character outside `!`..`~` replaced by `?`.

### 2.2 How every handler starts

In this order, for every route of §3 except where §3 says otherwise:

1. **Body**: core §3.9's reader with the default limit (100 KiB). Its errors (413, 415, 400 pages) answer before
   anything else, auth included. The result is **absent** (no JSON or form body) or a JSON value (a form is an
   object).
2. **Game auth**: auth §4.1 (`Auth::game_auth`); its refusal (401, 403, 500, a runtime's refusal) is the answer.
3. The **caller** derived from the auth result:

| Caller field | From |
| --- | --- |
| `user` | `AuthData.user_id` |
| `gs` | `AuthData.is_gameserver` |
| `host` | a player-host runtime authenticated the request (`AuthData.runtime` is some) |
| `roster` | the runtime's roster (empty ids dropped) |
| `capability` | the runtime's `rifts_seam::RuntimeAuth`, for the host-write hooks (§5; downcast from auth's opaque runtime handle, §9.7) |

A body field "`a.b`" is optional chaining on the JSON: absent for an absent body, for a hop that is not an object,
or for a missing key; a key present with `null` is present (`null`). "Non-null" means present and not `null`.

### 2.3 Predicates

| Name (provenance) | True when |
| --- | --- |
| **may read** t (`IsAuthorizedProgressionReader`) | `gs`, or `user` = t |
| **may report** t (`IsAuthorizedProgressionReporter`) | `gs`, or (not `host` and `user` = t) |
| **may read escalation** t | `gs`; else for a `host`: t is in `roster`; else `user` = t |
| **may write escalation** t | as **may report** |
| **host escalation writer** | `host` and not `gs` |
| **read gate** t (`RequireProgressionRead`) | t = `INVALID`, or **may read** t; otherwise FORBIDDEN, logged (warn) `Rejected cross-account progression read actor={user or "unknown"} target={t}` |
| **resolved user** (`ResolveRequestUserId`) | the route's `userId` param when non-empty, else body `accountId` when a non-empty string, else `user`, else `INVALID` |

**`Number(text)`** of a path param is JavaScript's: the text trimmed of white space and BOM; empty is 0; `0x`, `0o`,
`0b` prefixes (lower or upper case, no sign) read in that base; an optional sign then `Infinity`; otherwise an
optional sign and a decimal literal (`12`, `.5`, `5.`, `1e3`, `1E-2`); anything else (`12abc`, `1_0`, `inf`, `NaN`,
`-0x10`, `1 2`) is NaN. "An integer" means finite, no fraction and at most 2⁵³ in magnitude.

`INVALID` is the no-player sentinel (auth §4.1.1; for a game server's `GET`, auth has already replaced it in the path
when a path account is configured). Every comparison here is exact and case-sensitive.

### 2.4 Accounts, reads and writes

- **Account of** u: `accounts.find_by_user_id(u)` (store §6.1); none means no account. A read of u's rows for a u
  without an account reads as no rows (no error).
- **Writes** go through `IslesStore::account_write(account, actor, f)` (store §3.2, the home check), one closure one
  transaction, except §4.14's supply refill, which is a plain `write` (§15 item 14). The **actor** is
  `Actor::System` for a `gs` or `host` caller, else `Actor::Player { account }`, with the request id attached
  (`ActorContext::with_request`).
- **Time**: stored stamps are the store's `now_iso()` (store §10.1, replay-aware). Where a rule needs "now" in ms it
  says which clock (§4.5 uses the system clock, §15 item 3).

### 2.5 Track and objective rows (wire shapes)

- **Track row** (u, id, record or none): `{"phx_account_id":u,"progression_id":id,"progress":n,
  "confirmed_fremium_rank":n,"confirmed_premium_rank":n,"confirmed_date":s}`; without a record every number is 0 and
  `confirmed_date` is now (ISO); with one, the record's values and `confirmed_date` = its `updated_at`. `u` is the
  client's userId text as the request named it, never the account id.
- **Objective row** (u, record): the version hook `objective_row(u, ObjectiveRow)` (core §5.2), given objective id,
  value, completed count and, for both dates, the record's `updated_at` (§15 item 6). 1.14.7's hook writes
  `{"phx_account_id","objective_id","progress"(value),"completed_count","created_date","last_modified_date"}`.

---

## 3. Routes

The 25 rows of core Appendix A whose group is `progression` (B43, B44, B52, B108 to B129), all `Service::Backend`,
registered at these exact path strings. B130 and B131 (`DELETE /progression/:userId/:progressionId`, `POST
/progression/purchase/...`) are the inventory part's; B46 and B48 are game-data's.

| Row | Method | Path | § |
| --- | --- | --- | --- |
| B43 | GET | `/escalation/:escalationSeason/:userId` | 3.8 |
| B44 | POST | `/escalation/:escalationSeason/:userId` | 3.9 |
| B52 | GET | `/huntpass/:userId` | 3.1 |
| B108 | GET | `/encountered-content/:characterId/:contentType` | 3.12 |
| B109 | POST | `/encountered-content/query/:characterId` | 3.13 |
| B110 | POST | `/encountered-content/query` | 3.14 |
| B111 | POST | `/encountered-content/:characterId/query` | 3.13 |
| B112 | POST | `/encountered-content/:characterId` | 3.15 |
| B113 | POST | `/encountered-content` | 3.16 |
| B114 | POST | `/encountered-content/` | 3.16 (never reached: B113 takes it, core §4.3) |
| B115 | GET | `/progression/objectives/:userId` | 3.7 |
| B116 | GET | `/breadcrumbs/` | 3.17 |
| B117 | GET | `/breadcrumbs/:characterId` | 3.18 |
| B118 | POST | `/breadcrumbs/:characterId` | 3.19 |
| B119 | POST | `/progression/:userId/:progressionId/:amount` | 3.4 |
| B120 | POST | `/progression/:userId` | 3.5 |
| B121 | POST | `/progression/:userId/:progressionId/:rank/confirm/public` | 3.6 |
| B122 | GET | `/progression/:userId` | 3.2 |
| B123 | GET | `/pjm/:userId` | 3.20 |
| B124 | GET | `/weapon_tracker/:userId/:itemId` | 3.10 |
| B125 | POST | `/pjm/:userId` | 3.21 |
| B126 | GET | `/pjm` | 3.22 |
| B127 | GET | `/progression/tracked_objectives/:phxAccountId` | 3.11 |
| B128 | POST | `/progression/tracked_objectives/:phxAccountId` | 3.11 |
| B129 | GET | `/progression/:userId/:progressionId` | 3.3 |

Every route needs the database except where it answers before touching it; without one, the first store use answers
NO DB.

### 3.1 `GET /huntpass/:userId`

§2.2, then OK(the version hook `huntpass_payload(A)`), where A is game-data's served Hunt Pass id
(`rustic_isles_gamedata::progression_config::served_huntpass_id`: `GAMESERVER_HUNTPASS_ID` raw when set, else the
table's `active_huntpass_id`, JS-trimmed). 1.14.7's hook gives `{"huntpass_id":A,"season":A}`. No ownership check,
no database. Logged (info) `Huntpass (stubbed) -> huntpass_id={A}`.

### 3.2 `GET /progression/:userId`

1. §2.2; t = resolved user; read gate t.
2. **Known tracks** K: the 23 ids of §4.2's default list, then the table's `active_huntpass_id` (1.14.7: `season43`).
3. t = `INVALID`: OK([track row (t, k, none) for each k in K]), no database.
4. **Stale reforge repair** (§4.6), only when the reforge mode (§4.5) is `on` and the caller is **authoritative**
   for t: `gs`; or `host` and the guard allows a `Reforge` write for t with `parity: true` (not a probe); a player is
   never authoritative. A guard error is STORE-like: empty 500 (§5). With an account for t, the repair runs in one
   account write; a failure is logged (error) `[Reforge] stale-reforge recovery failed for {t}` and the read goes on.
   A `host` caller whose repair finished at least one track: logged (info) `[HostParity] reforge repair at a realm
   load user={t} tracks=[a, b]` and tagged `Reforge` `{"staleReforgesFinished":[ids]}`.
5. Rows: the live tracks of t's account in creation order. Answer OK(list): for each k in K in order, the track row
   of the first remaining stored row with that id (removed from the remaining) or a zero row; then every remaining
   stored row in creation order. Logged (info) `Progression fetched for userId {t} ({n} tracks, persisted)`.

### 3.3 `GET /progression/:userId/:progressionId`

1. **Before anything else** (before the body and auth): when the `userId` param is exactly `objectives`,
   `tracked_objectives` or `config`, empty 404 (Express's `next("route")` with no route after).
2. §2.2; t = the `userId` param, p = the `progressionId` param; read gate t.
3. t = `INVALID`, or p longer than 128 characters: OK(track row (t, p, none)), no database.
4. OK(track row (t, p, the live track p of t's account or none)). Logged (info)
   `Progression track fetched user={t} track={p} progress={n}` plus ` reforge=open` / ` reforge=closed` when the row
   holds a reforge.

### 3.4 `POST /progression/:userId/:progressionId/:amount` (the authoritative XP grant)

Checks in this order; the first that fails answers.

1. §2.2. u = `userId`, p = `progressionId`, a = `amount` (params).
2. **Grant id** g (§4.3), computed now (a game server that sent none gets one minted here, which draws one id from the
   replay id source even if the request is then refused).
3. Logged (info) `[ProgressionGrant] REQ user={u'} track={p'} amount={a'} grantId={present|absent|invalid} src={host|gs|other}`
   (primes: first 128 characters, non-printables `?`).
4. Neither `host` nor `gs`: FORBIDDEN.
5. u empty or `INVALID`: BAD.
6. p empty or longer than 128 characters: BAD.
7. p not an **approved XP track** (§4.2): BAD, logged with §4.2's reason.
8. g none: BAD.
9. a, read as JavaScript `Number(text)` (§2.3), must be an integer from 1 to 100 000 (`MAX_PROGRESSION_GRANT_AMOUNT`):
   else BAD.
10. **Host authorization** (only for `host`), §5.2's kind selection; a denial is its hook reply. Then the host
    ceiling (§4.4) on the request's amount a: a refusal answers **HOST LIMIT** unless it is one of the two
    observe-only reasons and the host policy is in observe mode (§4.4).
    HOST LIMIT = 403 `{"code":"HOST_WRITE_LIMIT","message":"This write is beyond what a hosted server may report.","payload":null}`.
11. **Federation forward** (only for `gs`): when u names an account homed on another node (§4.15), the grant is
    forwarded and answered OK(`{"progression_id":p,"xp":m,"level":1,"forwarded_to_home":true}`), m being the
    multiplied amount (§4.7). Nothing is stored here; no ledger, no reforge check.
12. No account for u: NO ACCOUNT(u).
13. **Reforge pre-check** when p is a reforgeable track or a prestige track (§4.5): read (one read) the ledger row of g
    and the weapon's ExperienceTrack row. Only when the ledger has no row for g: classify (§4.5); `Refuse`: CONFLICT
    (logged `[Reforge] DUP/REFUSED grant ...: {reason}`); a `host` grant to a prestige track whose plan is not
    `Prestige` (whatever the mode): CONFLICT.
14. m = the test XP multiplier applied to a (§4.7); logged (info) `[TestMultiplier] XP grant {a} -> {m} ...` when it
    changed.
15. **The grant transaction** (one account write), §4.3, with amount m. Outcomes: applied, replayed, CONFLICT (a
    reused grant id), CONFLICT (the reforge mark lost a race), HOST LIMIT (a host ceiling on m or a full player-level
    track, §4.4).
16. After it: each observe-mode note is sent to the guard (§5.3); an applied grant is counted by the activity port
    (§9.5: `xp(u, p, m)`); a `host`'s applied (not replayed) grant is tagged (§5.4); a Hunt Pass XP track (p is the
    table's or the served id) logs the monitor line (§4.3).
17. OK(`{"phx_account_id":u,"progression_id":p,"progress":n,"confirmed_fremium_rank":n,"confirmed_premium_rank":n,
    "confirmed_date":<updated_at>,"grant_id":g,"replayed":bool}`) from the track the transaction answered (for a
    replay, the ledger's stored result).

### 3.5 `POST /progression/:userId` (the objective report)

1. §2.2; t = resolved user (the path's `userId`; a body `accountId` is never used here because the param is
   non-empty).
2. `host`: the guard is asked for an `Objectives` write for t (not a probe); a denial is its hook reply (logged
   `Rejected host progression report actor=.. target=..`). Not `host` and not **may report** t: FORBIDDEN.
3. t = `INVALID`: OK(`{}`), nothing stored.
4. The report R is the body, or `{}` when absent. R fails §4.8's validation: BAD.
5. No account for t: NO ACCOUNT(t).
6. One account write appends an objective event: the account, R as JavaScript JSON text, received at now
   (`append_objective_event`, store §10.2).
7. Diagnostics logs (§11). `host`: tagged `Objectives` with §5.4's report summary.
8. **Persist?** Only when mastery persistence is on (§8): a `host` persists when the guard allows a `Mastery` write
   for t (not a probe; refused: logged `[MasteryMon] user={t} host report left diagnostics-only: write refused`); a
   `gs` persists; a player's own report never does.
9. Persisting: §4.9 in one account write; the answer's payload is
   `{"progress_tracks":[track rows],"objectives":[objective entries]}` (§4.9 "Answer"). A `host` is tagged
   `Mastery` with the plan summary. A failed write: logged (error) `[MasteryMon] user={t} report NOT applied: the
   database write failed` and the payload is `{}`.
10. OK(payload): `{}` unless step 9 produced one.

### 3.6 `POST /progression/:userId/:progressionId/:rank/confirm/public`

1. §2.2; u, p, r (params).
2. Neither `host` nor `gs`: FORBIDDEN.
3. u empty or `INVALID`: BAD.
4. r as `Number(text)` must be an integer >= 0: else BAD.
5. `host`: the guard is asked for a `RankConfirm` write for u (not a probe); a denial is its hook reply.
6. No account for u: NO ACCOUNT(u).
7. One account write (the curve is §4.1's for p):
   - for a `host`, the stored `confirmed_fremium_rank` before (0 without a row) is read first;
   - no curve for p: **rejected** `unknown_track`;
   - the required cumulative XP for r (§4.1) is none: **rejected** `rank_out_of_range`;
   - required > 0 and the stored progress (0 without a row) is below it: when p starts `MasteryTrack_` and the gap is
     at most 1 (`MASTERY_CREDIT_PER_OBJECTIVE`), progress is raised to the required value
     (`set_progress_if_greater`, logged `[MasteryMon] SYNC ...`); otherwise **rejected** `insufficient_xp`;
   - then `set_confirmed_fremium_rank(p, r)` (keeps the larger rank) answers the track.
8. Rejected: `{"code":null,"message":"Rejected: <reason>","payload":{}}`, 409 for `insufficient_xp`, else 400. A
   rejection writes nothing.
9. A `host` whose confirm raised the stored rank is tagged `RankConfirm`
   `{"track":p,"rank":r,"previousRank":before,"confirmedRank":after}`.
10. OK(`{"phx_account_id":u,"progression_id":p,"progress":n,"confirmed_fremium_rank":n,"confirmed_premium_rank":n,
    "confirmed_date":<updated_at>}`).

### 3.7 `GET /progression/objectives/:userId`

§2.2; t = resolved user; read gate t. t = `INVALID`: OK(`[]`). Else OK([objective row (t, record) for each live
objective of t's account in creation order]). Logged (info) `Objective progression fetched for userId {t} ({n}
objectives, persisted)`.

### 3.8 `GET /escalation/:escalationSeason/:userId`

§2.2; s, t (params). Not **may read escalation** t: **empty 403** (logged `Rejected cross-account escalation read
...`). OK(escalation payload of the live row (t's account, s), or of none) (§4.10).

### 3.9 `POST /escalation/:escalationSeason/:userId`

1. §2.2; s, t.
2. **host write** = not **may write escalation** t and **host escalation writer**. Neither may write nor host write:
   empty 403.
3. Host write: the guard is asked for an `Escalation` write for t (not a probe); a denial is its hook reply.
4. Host write: the tuning (§4.10, §6) is loaded. Loaded and s not one of its seasons: 403
   `{"code":"ESCALATION_SEASON_UNKNOWN","message":"This escalation season is not known.","payload":null}`
   (logged with s as a JSON string of its first 64 characters). Invalid: logged (warn) `[Escalation]
   game-data/escalation_tuning.json rejected ({problem}): host escalation writes keep the never-lower merge`, and the
   write goes on without a table. A non-host write never loads it.
5. No account for t: NO ACCOUNT(t).
6. The update U is the body, or `{}` when absent.
7. One account write, up to 8 attempts inside it: read the live row; merge (the dedicated merge, or the host merge
   with the season table when loaded; §4.10) at now; `save_if_version` with the read row's version (none when no
   row); saved: done. 8 misses: the data error `escalation progress save remained contended` (STORE).
8. Host write: tagged `Escalation` `{"season":s, ...summary}` (§4.10); a rejected level/XP or talents part is logged
   (warn) `Host escalation write partly refused ...`.
9. OK(escalation payload of the saved record). Logged (info) `Escalation progress saved season={s} user={t}
   level={n} version={n}`.

### 3.10 `GET /weapon_tracker/:userId/:itemId`

§2.2; read gate `userId`; OK(`{"TrackersInfo":[]}`).

### 3.11 Tracked objectives (`/progression/tracked_objectives/:phxAccountId`)

- `GET`: §2.2; read gate a (the param); OK of
  `{"current_set":"quest_slayer_links","omitted_quests":[],"phx_account_id":a,"tracked_craftables":[],"tracked_quests":[]}`.
- `POST`: §2.2 (the body is parsed, so a malformed one is refused, and then ignored); read gate a; OK(`null`), i.e.
  `{"code":null,"message":"OK","payload":null}`.

### 3.12 `GET /encountered-content/:characterId/:contentType`

§2.2; c = `characterId`; k = `Number(contentType)` (a JSON number, `null` when not finite). Logged (info)
`Querying encountered content for userId {user or "undefined"} and characterId {c}`.

- No `user`: OK(`{"content_types":[{"content":[],"content_type":k}],"success":true}`).
- **Character gate** (user, c): passes when user is `INVALID`; else when the character is owned (§4.13); refused:
  403 `{"code":"forbidden","message":"Character is not owned by the authenticated account.","payload":null}`.
- OK(`{"content_types":Q,"success":true}`), Q = §4.13's query with the list `[k]`.

### 3.13 `POST /encountered-content/query/:characterId` and `POST /encountered-content/:characterId/query`

§2.2; an absent body: empty 500. L = body `content_types` when non-null, else `[]`. Same log line. No `user`: OK with
`content_types` = for each element x of L (when L is an array; else none) `{"content":[],"content_type":x}`, and
`"success":true`. Character gate. L not an array: empty 500. Else OK(`{"content_types":Q,"success":true}`).

### 3.14 `POST /encountered-content/query`

§2.2; absent body: empty 500; L as above; logged (info) `Querying encountered content without characterId;
returning empty content`; the no-user answer of §3.13 (whoever calls).

### 3.15 `POST /encountered-content/:characterId`

§2.2; absent body: empty 500. T = body `content_type`, I = body `content_id` (present means present, `null`
included). No `user`, or T absent, or I absent: OK(`{}`). Character not owned (§4.13; the `INVALID` pass does not
apply here): **empty 403** (logged). No account: NO ACCOUNT. One account write: the live row for (c, account); its
`content` parsed as a JSON array (not parseable or not an array: data error `encountered content: ...`, STORE); no
row: a row is created with `[]`; the entry `{"content":I,"category":T}` is appended (no de-duplication) and the list
stored as JavaScript JSON text (`update_content`). OK(`{}`).

### 3.16 `POST /encountered-content` (and `/encountered-content/`)

§2.2 (body and auth still run), then OK(`{}`): the client posts here with an empty character id before its login
settles.

### 3.17 `GET /breadcrumbs/`

§2.2; OK(`{"breadcrumbs":[],"updateVersion":0}`). Logged (info) `Requested breadcrumbs without characterId; returning
empty breadcrumbs`.

### 3.18 `GET /breadcrumbs/:characterId`

§2.2; c. No `user`: OK(`{"breadcrumbs":[],"updateVersion":0}`). Not owned: empty 403. No account: NO ACCOUNT. One
account write (a read that creates): no live row for (c, account): created with `[]` and version 0, answer
`{"breadcrumbs":[],"updateVersion":0}`; a row: its `items` parsed as JSON (not JSON: data error `breadcrumbs: ...`,
STORE), answer `{"breadcrumbs":<parsed>,"updateVersion":<n>}`. OK(answer).

### 3.19 `POST /breadcrumbs/:characterId`

§2.2; absent body: empty 500. B = body `breadcrumbs`, V = body `updateVersion` (present, `null` included). The
requestor is `user` or the empty text; not owned: empty 403. No account: NO ACCOUNT. Stored text: B as JavaScript
JSON text, or `[]` when B is absent; stored version: V when it is a JSON number, truncated toward zero, else 0. One
account write: create the row when there is no live one for (c, account), else `update` it (no version guard).
OK(an object with `breadcrumbs`: B when present, then `updateVersion`: V when present, each as sent).

### 3.20 `GET /pjm/:userId`

1. §2.2; t; read gate t.
2. t = `INVALID`, or no saved journey for t: OK(`{"nodes":{},"update_version":1}`).
3. **Login reconciliation**: when the caller is authoritative for t (as §3.2 step 4, with kind `SlayersPath`) and t
   has an account: §4.12's reward reconciliation for the unlocked node ids (§4.11) of the stored map. Items granted
   and a `host` caller: tagged `SlayersPath` `{"loginCatchUp":[catalog ids]}`.
4. OK(`{"nodes":<the stored map, masked (§4.11)>,"update_version":<stored version>}`).

The stored map is the row's `nodes` parsed as JSON (not JSON: `{}`).

### 3.21 `POST /pjm/:userId`

1. §2.2; t.
2. Neither `host` nor `gs`: FORBIDDEN (it grants items).
3. `host`: the guard is asked for a `SlayersPath` write for t (not a probe); a denial is its hook reply.
4. Vs = body `update_version` when non-null. t = `INVALID`: OK(`{"nodes":{},"update_version":<Vs as sent, or 1>}`).
5. N = body `nodes` when non-null, else `{}`; v = Vs when it is a JSON number, `NaN` for any other present value
   (no coercion), 1 when absent.
6. No account: NO ACCOUNT.
7. One account write: §4.11's save (with the graph of §4.11 for a `host`, none for a `gs`).
8. The activity port is told the newly unlocked keys (§9.5, source `host` or `gameserver`), even when there are none.
9. A `host` whose plan unlocked, stripped or relocked anything: tagged `SlayersPath`
   `{"unlocked":[ids],"stripped":["id:reason"...]}` plus `"relocked":[ids]` when non-empty; stripped or relocked
   nodes are also logged (warn).
10. §4.12's reconciliation for **every** unlocked node of the saved map (a reward that failed earlier is retried).
11. OK(`{"nodes":<the saved map, masked>,"update_version":<the saved version>}`).

### 3.22 `GET /pjm`

§2.2. A `user` that is not `INVALID` with a saved journey: OK(masked map and version, as §3.20 step 4, no
reconciliation). Otherwise the backend file `slayers_path.json` is read fresh (no cache) and answered **as it is**
with 200 (it is already an envelope); no game data folder, a missing or unparseable file: empty 500 (logged).

---

## 4. Rules

### 4.1 Rank curves (`progression_config.json`)

**Building the curves**: for each element of `payload.paths` (not an array, or no file: no curves) that has a
non-empty string `progression_id` and a non-empty array `requirements`:

- each requirement gives (rank, xp): rank = `Number(rank_id)` truncated toward zero (NaN reads as 0); xp =
  `Number(xp_required)`, NaN reads as 0. `Number` here is JavaScript's on a JSON value (`null` 0, `true` 1, a numeric
  string its number, anything else NaN; `service_kit::js::number`);
- sorted by rank, stable;
- the **cumulative** list: a running total over the sorted pairs; a rank seen again keeps its first position and
  takes the later running total;
- `max_rank` = the last sorted rank;
- `prestige_xp_per_level` = `Number(prestige.xp_per_level)` when finite and > 0, else none.

A later path with the same id replaces the earlier one.

| Query | Result |
| --- | --- |
| **cap** (id) | the cumulative of `max_rank` (0 when absent) |
| **required** (id, r) | r < 0: none; r <= max: the cumulative of r, **0 when r has no entry** (§15 item 7); r > max: none without prestige, none above max + 100 000, else cap + (r − max) × per level |
| **rank for progress** (id, x) | no curve, or x not finite: none; else the largest rank id whose cumulative <= x, at least 0 |

**Loading**: the curves are built from the backend file once per process per path (core §8.1's `backend_file`). A
missing or unreadable file is logged (error) `progression_config.json: {error}` and gives **no curves, cached for
the life of the process** (a file generated later needs a restart; §15 item 8). No game data folder: no curves, not
cached. The read strips one leading BOM and parses JSON.

Worked example: requirements `[{2,20},{1,10},{3,5}]`, prestige 100 → max 3, cap 35; required(2) 30, required(0) 0,
required(5) 235, required(−1) none; rank for 31 is 2, for 5 is 0.

### 4.2 Track lists and the Hunt Pass ids

- **Default progression ids** (the list order of §3.2): `MasteryTrack_PlayerLevel`, `MasteryTrack_Weapon_` +
  {`Axe`, `ChainBlades`, `Hammer`, `Repeaters`, `Sword`, `Spear`, `Strikers`}, `ExperienceTrack_PlayerLevel`,
  `ExperienceTrack_Weapon_` + the same seven in the same order, `PrestigeTrack_Weapon_` + the same seven.
- **XP allowlist**: `MasteryTrack_PlayerLevel`, `MasteryTrack_Behemoth`, and the seven weapons of each of
  `MasteryTrack_Weapon_`, `ExperienceTrack_Weapon_`, `PrestigeTrack_Weapon_`, plus `ExperienceTrack_PlayerLevel` (24).
- **Active pass** = the table's `active_huntpass_id`; **served pass** = game-data's served id (§3.1).
- **Hunt Pass id shape**: starts, ASCII case-insensitively, with `season`, `eventpass_` or `d24_`.
- **Hunt Pass XP track**: the active or the served pass id.
- **Approved XP track** p: (p on the allowlist, or p = active pass, or (p = served pass and it has the Hunt Pass
  shape)) **and** p has a curve.
- **Reason** logged for an unapproved p (in this order): `Linked_Slayer_Slot_<digits>` → `Slayer Link track (the hunt
  pass also_progress target) - deliberately not approved`; no curve → `no rank curve in progression_config.json`;
  Hunt Pass shape → `hunt pass that is not served (served={id})`; else `not on the XP allowlist`.
- **Resolve track id** (used by bounties, §9.3): `selected_huntpass` is the active pass; any other id is itself when
  it has a curve, else none.
- **Weapons**: `Axe`, `ChainBlades`, `Hammer`, `Repeaters`, `Spear`, `Strikers`, `Sword`. **Reforgeable**:
  `ExperienceTrack_Weapon_<weapon>`. **Prestige track** `PrestigeTrack_Weapon_<weapon>` maps to the ExperienceTrack of
  the same weapon; **prestige track for** an ExperienceTrack id replaces its first `ExperienceTrack_` with
  `PrestigeTrack_`.

There are no daily or weekly resets in this part: tracks and objectives only grow, and the Hunt Pass "window" is
which id is served (game-data's rewrite of `GET /progression/config` and core §8.4 decide the dates the client
sees).

### 4.3 Grant ids and the ledger

**Sent ids**: the header id is `x-rustic-isles-grant-id`, else `idempotency-key` (the first value, when it is valid
header text); the body id is body `grant_id` when non-null, else body `grantId` as it is (possibly `null` or
absent).

| Situation | g |
| --- | --- |
| no header id and no body id (absent) and `gs` | `gs-` + the next replay-aware UUID (core §9.1 `next_uuid`) |
| header and body both present and the body id is not the same string | none |
| otherwise the header id, else the body id | that value when it is a string matching `^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$`, else none |

A body `grantId: null` counts as "sent" (no mint) and is then not a string (none); with a header present it also
fails the "same string" test (§15 item 2). The REQ log's word: `present` (g some), `absent` (nothing sent),
`invalid`.

**Request hash**: lowercase hex SHA-256 of the JavaScript JSON text of
`{"version":1,"userId":u,"progressionId":p,"amount":m}` (u as the request named it, m the multiplied amount).

**The transaction** (account write, actor §2.4), in order:

1. For a `host`: the ceiling (§4.4) on m. Refused: HOST LIMIT unless observe-only in observe mode, which adds a
   note.
2. The ledger row of g (`progression_grants.find_by_grant_id`). Present: when its hash, account, track or amount
   differs from this request's: CONFLICT (logged `Rejected XP grant reuse with different request ...`); else the
   answer is its stored result, **replayed**, nothing written.
3. For a `host` on a track with a total cap (§4.4): the stored progress (0 without a row) at or above the cap:
   HOST LIMIT unless observe-only in observe mode (a note). Decided after step 2 so a retry of the grant that filled
   the track still replays.
4. A `Prestige` or `Restore` plan (§4.5): the reforge mark (`update_reforge` compare-and-set). Lost: CONFLICT (the
   message `reforge {id} changed concurrently (prestige|restore)`).
5. `progression_tracks.increment(p, m)`; `progression_grants.insert_applied` with g, the account, p, m, the hash,
   the incremented track as result and now as `created_at`.
6. Logged (info) `[ProgressionGrant] grant={g} user={u} track={p} delta={m} old={before} new={after}
   source={host|gameserver} applied=1`.

A replayed grant is not counted by the activity port and not tagged.

**Monitor line** (when `RUSTIC_ISLES_ECONOMY_MONITOR` is on unless off, §8): `[HuntPassMon] GRANT user={u}
track={p} amount={a} progress={old}->{new} rank={r0}->{r1} confirmed={f}/{p} replayed={0|1} grant={g}`, where old =
new − a (the **unmultiplied** a, §15 item 4) and ranks are §4.1's (`?` when none).

### 4.4 Host limits (a player-hosted server may not report the impossible)

| Track p | Ceiling per grant |
| --- | --- |
| `ExperienceTrack_PlayerLevel` | 1 000 |
| starts `ExperienceTrack_Weapon_` | 1 000 |
| starts `PrestigeTrack_Weapon_` | exactly 1 (`amount_not_one`, limit 1) |
| starts `MasteryTrack_` | 0: never writable by a host (`track_not_writable`, limit 0) |
| a Hunt Pass XP track | 200 |
| anything else | no opinion (no check) |

Over the ceiling: `amount_over_ceiling`. **Total cap**: only `ExperienceTrack_PlayerLevel`, its curve cap truncated to
an integer when > 0; stored progress >= cap is `track_full` (limit the cap, with the stored value).

**Observe mode** = the guard's `host_policy_refuses()` answers false (an error reads as refusing). In observe mode
`amount_over_ceiling` and `track_full` only log: a note with what `xp` and detail
`user={u} {description} grant={first 96 characters of g}`, sent with `note_would_refuse` after the transaction. The
other reasons always refuse.

**Description**: `reason={r} track={first 64 characters of p, non-printables ?} amount={n} limit={n}` plus
` stored={n}` when known. Refusals are logged (warn)
`[HostWrite] refused kind={kind} target={u'} {description} grant={g'}`.

### 4.5 The reforge plan of a grant

**Mode** (`RUSTIC_ISLES_REFORGE`, §8): the setting JS-trimmed and ASCII lower-cased; unset follows the version hook
`reforge_rules()` (true `on`, false `off`); `off` and `log` are themselves, anything else is `on`.

**Classify** (mode, p, amount, the weapon's ExperienceTrack row, now ms): not `on`: `Normal`. With r = the row's
stored reforge (store §10.1):

| p | Condition | Plan |
| --- | --- | --- |
| a prestige track | r open, its +1 not yet granted, amount 1 | `Prestige` |
| a prestige track | r open, amount 1 (so already granted) | `Refuse` `this reforge's +1 was already granted` |
| a prestige track | r open, amount ≠ 1 | `Refuse` `prestige amount {n} is not 1` |
| a prestige track | no r, or r closed | `Refuse` `no open reforge for this weapon` |
| reforgeable, r with banked > 0 | r open, remaining = banked − restored > 0, amount = remaining | `Restore` |
| reforgeable, r with banked > 0 | remaining 0, amount = banked, now − opened < 15 min (an unparseable `openedAt` reads as long ago) | `Refuse` `banked {n} of reforge {id} was already restored` |
| anything else | | `Normal` |

The grant route's "now" here is the **system clock**, not the replay clock (§15 item 3); the inventory part passes
its own.

**The mark** for a plan: expected `{"status":"open","prestigeGranted":<r's>,"bankedRestored":<r's>}`; set
`{"prestigeGranted":true}` (Prestige) or `{"bankedRestored":<banked>}` (Restore); the reforge **closes** when, after
this mark, both follow-ups are done (Prestige: restored >= banked; Restore: +1 already granted), and then the set
also gets `"status":"closed"` and `"closedAt":<now ISO>` in that order. Applied: logged (info) `[Reforge]
PRESTIGE|BANKED user={label} track={experience track} reforge={id}` + ` restored={banked}` for a restore + ` CLOSE`.

### 4.6 The stale reforge repair

An **open** reforge on a reforgeable track whose `openedAt` is at least 2 minutes (`STALE_REFORGE_MS` = 120 000) before
now (the replay-aware ms clock; unparseable reads as long ago) is finished, in the caller's transaction, once:
compare-and-set expected `{"status":"open","prestigeGranted":<r's>,"bankedRestored":<r's>}`, set
`{"status":"closed","closedAt":<now ISO>,"prestigeGranted":true,"bankedRestored":<banked>}`; when that applied, the
ExperienceTrack gets `banked − restored` when > 0 and its prestige track +1 when the +1 had not been granted. Logged
(warn) `[Reforge] RECOVER user={u} track={t} reforge={id} stale since {openedAt}: restored {n}, prestige {+1|already
granted}`. The answer is the list of tracks it closed, in creation order. The inventory part calls the single-track
form (P4, §9.2).

### 4.7 The test XP multiplier (the XP hook of `testMultipliers.ts`)

**Reading**: inventory §10.1 owns the shared reader (I7, §9.1; each grant reads it again). Restricted's reading,
for reference (open question 1):

1. Each of `REFORGED_XP_MULTIPLIER`, `REFORGED_REWARD_MULTIPLIER`: unset or blank after a JS trim is 1; else
   `Number(text)` must be finite and in [1, 100], else the multipliers are **off** with reason
   `a multiplier is not a number from 1 to 100`.
2. Both 1: off, reason `not set`.
3. `REFORGED_SERVER_MODE` raw not exactly `private`: off, `this is not a private server`.
4. `REFORGED_SERVER_HOST` not a Tailscale address (JS-trimmed, Unicode lower-cased: an IPv4 in 100.64.0.0/10, or a
   name ending `.ts.net` whose labels before it are non-empty runs of `a-z`, `0-9`, `-`): off, `the server host is
   not a Tailscale address`.
5. Otherwise on with both values.

**Applying** to an XP grant a: xp factor 1 → a; else min(100 000, max(a, floor(a × xp))).

| a | xp | m |
| --- | --- | --- |
| 125 | 5 | 625 |
| 50 000 | 5 | 100 000 |
| 125 | 1 | 125 |
| 3 | 1.5 | 4 |

### 4.8 Objective report validation

R is valid when all hold:

- its JavaScript JSON text is at most 131 072 UTF-16 units;
- `progress_tracks` and `objectives` are each absent, `null` or an array (absent and `null` read as empty);
- at most 256 tracks and 512 objectives;
- every track: `progression_id` a string of 1 to 128 characters, each ASCII alphanumeric or `_`, `.`, `:`, `-`;
  `Number(progress)` finite and in [0, 10⁹];
- every objective: `objective_id` as `progression_id`; `Number(value)` finite in [0, 10⁹]; `completed_count` absent,
  `null`, or `Number(..)` finite in [0, 10⁶].

A body that is a JSON array is "an object without those keys" and is valid (§15 item 12).

### 4.9 Mastery (`masteryReport.ts`, 1.14.7)

Credit comes from the stored objective state, never from the increments the report claims.

**Card track** of an objective id: not starting `MasteryObjective_` → none (not mastery);
`MasteryObjective_Behemoth_...` → `MasteryTrack_Behemoth`; `MasteryObjective_Weapon_<code>_...` with an all-letters non-empty code → by code: `Axe`,
`Sword`, `Strikers`, `Hammer` → `MasteryTrack_Weapon_<same>`; `Pike`, `Spear` → `..._Spear`; `DP`, `Repeaters` →
`..._Repeaters`; `CB`, `ChainBlades`, `Chainblades` → `..._ChainBlades`; another code, or any other mastery shape →
**unmapped** (mastery, no card).

**Host limits** (a `host`'s report only): at most 64 newly completed objectives per report; values clamped to
100 000 000 and completed counts to 10; objective ids checked against **known objectives** (`mastery_objectives.json`
`objectiveIds`, read on each host report; a missing file, an empty list or an unreadable file (logged warn) falls back
to the **id grammar**: `MasteryObjective_Behemoth_` or `MasteryObjective_Weapon_` followed by at least two non-empty
`_`-separated runs of ASCII letters and digits, or `AchievementObjective_` followed by a non-empty run of ASCII
letters, digits and `_`; at most 128 bytes). An id refused is **ignored** with a reason: empty or over 128 bytes
`bad_id`; a `MasteryObjective_` id not known `unknown_mastery_id`; an `AchievementObjective_` id not known
`unknown_achievement_id`; anything else `unrecognised_id_family`. **Track caps**: the cap (§4.1, truncated, only
when > 0) of `MasteryTrack_PlayerLevel`, the seven `MasteryTrack_Weapon_*` and `MasteryTrack_Behemoth`; the stored
totals of the capped mastery tracks the report names are read inside the transaction.

**The plan** (inside one account write, after reading the stored state of the report's objective ids):

1. **Reported**: objectives in first-seen order; value and count are `floor(Number(x))` (absent/`null`/non-finite 0);
   a host's ignored ids are dropped (each reported once) and its clamps applied (noted `{id} value {v}->{max}` /
   `{id} count {c}->{max}`); a repeated id keeps the larger value and the larger count, each on its own.
2. **Claimed**: for each `progress_tracks` entry whose id starts `MasteryTrack_`, the sum of `max(0, floor(progress))`
   per id, first-seen order.
3. For each reported objective: **new** = it has a card (mastery) and its stored completed count is < 1 and the
   reported one >= 1. A new one past the host's 64: **over limit**: nothing of it is stored (the merged answer keeps
   the stored state if there is one) so a later report can still credit it. Otherwise **merged** = max(stored,
   reported) per field; a new one is **newly completed** and adds 1 (`MASTERY_CREDIT_PER_OBJECTIVE`) to
   `MasteryTrack_PlayerLevel` and 1 to its card track (unmapped: only the player track, and the id is listed
   unmapped).
4. **Credits**: per track in first-credit order, allowed = min(raw, claimed for that track) (a lower value is noted
   **capped** `{track} {raw}->{allowed}`); for a host with a cap and a stored total, at most max(0, cap − total)
   (noted **at cap** `{track} {allowed}->{room} (total {t}, cap {c})`); only allowed > 0 is credited.
5. **Writes**: `merge_objectives` for every reported objective except the over-limit ones (stored values are kept
   when larger, store §10.2); `increment` of each credited track by its amount.

**Answer** (§3.5 step 9): `progress_tracks` = a track row (§2.5, u = t) for each credited track (the incremented
record), then for each claimed track not credited its current record (or a zero row); `objectives` = for each merged
objective in order `{"phx_account_id":t,"objective_id":id,"progress":value,"completed_count":n,"created_date":now,
"last_modified_date":now}`.

**Logs**: with anything newly completed or claimed, (info) `[MasteryMon] user={t} newly=[..] credit={t+n, ..}
claimed={t+n, ..} totals={t=n, ..}` + ` capped=[..]` + ` UNMAPPED=[..]` when non-empty + ` objectives={n}`; a host
plan with ignored, over-limit, clamped or at-cap entries, (warn) `[HostWrite] limited kind=MASTERY target={t}
ignored={n}[first 10, ...] overLimit=.. clamped=.. atCap=..`.

| Stored | Report | Credits |
| --- | --- | --- |
| none | `Weapon_Sword_Generic_Kills1` completed 1; claims PlayerLevel 1, Sword 1 | PlayerLevel +1, Sword +1 |
| that objective completed 1 | the same report | none (merged value is the larger) |
| none | `Weapon_Axe_A_B` completed 1; claims only PlayerLevel 1 | PlayerLevel +1; capped `MasteryTrack_Weapon_Axe 1->0` |
| none | `AchievementObjective_Win` completed 1 | none; the objective is stored |
| host: PlayerLevel cap 3, total 2; four new Axe objectives (value 500, count 50) and `Bogus_Id`; claims PlayerLevel 9; limits 2 new, max value 100 | | `Bogus_Id` ignored `unrecognised_id_family`; 2 newly, 2 over limit; clamps noted; PlayerLevel +1 (at cap); 2 objectives stored |

### 4.10 Escalation

**Payload** of a record or none (none: zeros and empty lists):
`{"escalation_level":n,"next_level_xp":n,"talents_progress":[..],"unlock_progress":[..],"update_version":n}`.

**Dedicated merge** (previous P or none, update U, now): a field is U's key when U is an object. A **non-negative
int32** is a JSON number that is an integer in [0, 2 147 483 647].

| Field | Result |
| --- | --- |
| `escalation_level` | max(P's or 0, U's when a non-negative int32, else P's) |
| `next_level_xp` | U's when a non-negative int32, else P's or 0 |
| `talents_progress`, `unlock_progress` | U's when an array (replaces), else P's or `[]` |
| `update_version` | P's + 1, or 1 |
| `created_at` / `updated_at` | P's or now / now |

**Tuning** (`escalation_tuning.json`, §6; read on each host write, never cached): `{"seasons":{<id>:{maxLevel,
xpToReachLevel, unlocks, talents}}}`. One bad value rejects the whole file with the first problem, as text:
`the file is not a JSON object`; `"seasons" must be a non-empty object`; `a season id is empty`;
`seasons.{s} is not an object`; `seasons.{s}.maxLevel must be an integer 1..1000`;
`seasons.{s}.xpToReachLevel must hold one value per level ({max})`; `seasons.{s}.xpToReachLevel[{i}] must be a
positive integer` (1..2³¹−1); `seasons.{s}.unlocks must be a non-empty object`; `seasons.{s}.unlocks.{r}.level must be
an integer 1..{max}` (also for an empty reward id); `seasons.{s}.talents must be a non-empty object`;
`seasons.{s}.talents.{t}.pointsToUnlock must be a non-negative integer` (0..1000);
`seasons.{s}.talents.{t}.pointCosts must list the cost of each rank (1..100)`;
`seasons.{s}.talents.{t}.pointCosts[{i}] must be a non-negative integer` (0..1000); an empty talent id or a non-object
talent: `seasons.{s}.talents.{t} must be an object`. An unreadable file is `unreadable: {error}`. A missing file (or
no folder) is "missing", not invalid.

**Season checks** (level L, xp X, season S):

- **Level/XP**: L > max → `level {L} is above the season maximum {max}`; L < max and X >= `xpToReachLevel[L]` (0-based:
  the XP that level L+1 takes) → `next_level_xp {X} is not below the {need} XP level {L+1} takes`.
- **Talents** (a whole build): not an array → `talents_progress is not an array`; more entries than the season's
  talents → `{n} entries for a season with {m} talents`; an entry not an object `entry {i} is not an object`, with no
  string `talent_id` `entry {i} has no talent_id`, unknown `talent {"id"} is not in this season's table`, repeated
  `talent {"id"} appears twice`, rank not an integer in 1..ranks `talent {"id"} needs a rank of 1..{n}` (ids quoted
  as JSON strings, cut to 80 characters + `...`); points = the sum of the costs of the first `rank` ranks; points >
  min(L, max) → `{p} points spent with {e} earned (level {L})`; the purchases (gate, cost) sorted by gate must each
  have spent >= gate before them, else `a talent gated at {g} points is bought with {s} spent`. Answers the points.
- **Stored points** of a build: the same sum over entries that resolve (objects, known ids, first occurrence, rank an
  integer >= 1); others count 0.
- **Unlocks filter** at level L: an entry is accepted when it is an object whose string `reward_id` is in the
  season's unlocks and whose `collected` is absent, `false`, or `true` with L >= the unlock level; otherwise rejected
  (counted).

**Never-lower merge of entries** (stored list, incoming list, key): the key of an entry is the first non-empty string
among `talent_id`, `reward_id`, `unlock_id` (unlocks under a season table key on `reward_id` only). Stored entries
keep their order (duplicates merged), keyless stored entries go to the end; an incoming keyless entry is ignored; a
new key is appended unless there are already 512 keys (then ignored); an existing key merges field by field: a field
whose stored value is a number keeps it unless the incoming one is a number >= it; a stored `true` stays `true`; any
other field takes the incoming value; a kept value counts the entry as ignored once. Each stored key the host did not
send counts as ignored (the entry stays).

**Host merge** (P, U, now, season table S or none):

1. base = the dedicated merge.
2. requested level = U's when a non-negative int32, else P's. "Sent" = U has a valid level or a valid XP. With S and
   sent: the level/XP check on (requested, base XP).
3. Level and XP: rejected → P's pair stays; requested > P's level → (requested, base XP); equal → P's level and
   max(P's XP, base XP) (lower: a regression, ignored); lower → P's pair (a regression).
4. Talents: U's not an array → base's. With S: the build check at the merged level; valid → U's list **replaces**
   (a respec; fewer points than stored is `talentsReset`); invalid → P's list stays (all U's entries counted ignored).
   Without S: the never-lower merge of P's and U's.
5. Unlocks: U's not an array → base's. With S: filtered, then never-lower merged on `reward_id` (ignored += rejected).
   Without S: never-lower merged.
6. **Summary** (the tag, after `season`): `levelBefore`, `levelAfter`, `xpBefore`, `xpAfter`,
   `levelXpRegressionIgnored`, `talents` `{before,after,ignored}`, `unlocks` `{before,after,ignored}`, `version`;
   with S also `levelXpRejected` (when rejected), `talentPoints` `{before,after}`, `talentsReset: true` (when
   true), `talentsRejected` (when rejected).

### 4.11 The Slayer's Path (player journey map)

Node maps are JSON objects whose key order is the game's and is kept.

- **Unlocked** node: its `node_status` is the JSON number 1 (claimable) or 2 (unlocked).
- **Unlocked node ids** of a map: for each unlocked entry in order, its `node_id` when a non-empty string, else its
  key. **Newly unlocked** (stored, next): the **keys** of next's unlocked entries whose key is not unlocked in stored.
- **Merge** (stored map and version, incoming map and version; a non-object is `{}`): **stale** when incoming <=
  stored version: `{...incoming, ...stored}` (incoming's keys first, stored's values win) at the stored version;
  otherwise `{...stored, ...incoming}` at the incoming version.
- **Masked nodes** (`nodes_shown_locked`): the table's `masked_journey_nodes` (1.14.7: `Gauntlet_Unlock`) whose
  `unless` setting (1.14.7: `RUSTIC_ISLES_GAUNTLET`) is not `on` (JS-trimmed, ASCII case-insensitive). An answer
  shows a copy with each masked node that is present (an object) set to `node_status` 0; a save keeps a masked
  node's stored entry when it is unlocked in the stored map (so the client's masked echo never overwrites it).
- **The graph** (for a host's save): from `slayers_path_definitions.json` (§6; read once per process, a missing or bad
  file logged (error) `[slayers-path] FAILED to load slayers_path_definitions.json - node rewards will NOT be granted:
  ...` and gives no definitions) and the node keys of `slayers_path.json`'s `payload.nodes` (read each time): a
  child's parents are every definition listing it in `childNodes` (non-empty, not `None`, each parent once);
  **known** = definition ids, their children and the served node keys; **usable** = some definition loaded.

**Save** (one account write; v non-finite counts as 1, stored as an integer by truncation):

1. No row: the map to store is N (a non-object is `{}`) after the host plan against `{}` (for a `host`);
   `create_if_absent` (refused because a row of the account exists, deleted included: data error, STORE).
2. A row: stored = its `nodes` parsed (not JSON: `{}`); merged as above; masked nodes restored; then for a `host` the
   host plan (stored, merged). A stale save is logged (warn) `[pjm-stale] Stale journey save ...`.
   `update_if_revision` at the read revision (lost: data error `player journey update conflicted`, STORE).
3. Answers the stored map, the effective version and the newly unlocked keys.

**Host plan** (stored S, merged M, graph):

1. Each M entry that is unlocked, has a string `node_id` different from its key, and differs from S's entry under the
   key (compared as JSON text): stripped `node-id-mismatch`; S's entry put back, or the key removed when S has none.
2. Each S entry unlocked and not unlocked in M: S's entry put back, **relocked** listed.
3. Pending = M's unlocked keys not unlocked in S (in M's order); already unlocked = those unlocked in S.
4. Pending without a usable graph: `definitions-unavailable`; not known: `unknown-node`.
5. The rest are accepted in rounds: a node is accepted when it has no parents or any parent is unlocked or accepted;
   repeat while a round accepts something; what is left is `prerequisite`.
6. Each refused node gets S's entry back, or M's entry with `node_status` 0 (everything else the host sent kept) when
   S has none.
7. **Unlocked** list = accepted pending nodes in pending order; **stripped** = step 1's, then step 4's and step 6's in
   order.

Key order of M is kept throughout (`__proto__` and `constructor` are ordinary keys).

### 4.12 Slayer's Path rewards

For a list of unlocked node ids: when no node maps to an item (a definition's `rewards` strings, not empty, not
`None`): logged and nothing. Else the account's **first character** and its instanced items (§9.1); none: logged
(warn) `[slayers-path] cannot grant rewards for {u}: no character found`; an error: logged and nothing.

**Plan**: the wanted items, de-duplicated in node then reward order; **owned** = catalog ids of instanced items, except
a **legacy** item (`catalogId` starts `PART_` and its `instanceId` equals its `catalogId`), which does not count as
owned; to grant = wanted minus owned, **sorted**. For each, with stable = `{u}:{character}:{catalog}`:

- instance id = the first 26 hex digits of SHA-256(`pjm-instance:` + stable), upper-cased;
- transaction id = the first 32 hex digits of SHA-256(`pjm-grant:` + stable), or of `pjm-repair-instance:` + stable
  for a repair (legacy items of that catalog id exist), upper-cased;
- the item added: `{"catalogId":..,"instanceId":..,"itemData":<the first legacy item's itemData when non-null, else
  null>,"updateVersion":<its updateVersion when a number, else 0>}`; the legacy items are removed by (catalog,
  instance).

One inventory transaction per item (I3, §9.1); applied → granted (logged info); not applied or an error → logged,
retried on the next save. The answer is the granted catalog ids. Rewards never fail the route.

### 4.13 Encountered content and character ownership

**Owned** (u, c): false when u or c is blank after trimming or either is `INVALID`; else the account of u (none: false)
and the characters part's ownership check (§9.4; a failed lookup reads as not owned, logged).

**Query** (u, c, list L): L not an array: empty 500. The live row for (c, u's account): its `content` parsed as JSON
(not an array: empty 500); none: empty. For each category n from 0 to 7 **ascending** that L contains as a JSON
number equal to it (strings never match): `{"content":[C],"content_type":n}`, C being the `content` values of the
stored entries whose `category` is the number n. An entry without `content` contributes nothing.

### 4.14 Session supplies (`REFORGED_SESSION_SUPPLIES_FILE`, refilled at every game login)

`POST /login` is auth's (auth §3.1); its `GameLogin::session_supplies(account)` seam is implemented here (§10.3).
Never fails the login.

1. `get(REFORGED_SESSION_SUPPLIES_FILE)` unset, or the file missing: nothing.
2. The file is read (one BOM dropped) and parsed; any problem is logged (error) `[SessionSupplies] could not refill
   supplies for {u}: {problem}` and nothing happens. The format:
   `{"accounts":{"<userId>":{"<catalog id>":<quantity>, ...}}}`. Problems: not JSON (the parser's text);
   `expected {"accounts": {"<user id>": {"<catalog id>": <quantity>}}}`; `account "<u>": expected an object of catalog
   ids and quantities` (also an empty user id); `account {u}: invalid catalog id "<id>"` (1..128 bytes of ASCII
   alphanumerics, `_`, `-`); `account {u}: {id} is a currency (wallet), not a stacked item` (starts `CURRENCY_`);
   `account {u}: {id} is an instanced item, not a stacked one` (the inventory part's storage policy says instanced,
   I5, §9.1); `account {u}: quantity of {id} must be a whole number from 0 to 1000000`.
3. The login's userId has no entry, or an empty one: nothing.
4. For each of the account's live characters in order: the characters part's starter bootstrap is ensured first
   (§9.4); then up to 5 attempts: the character's inventory (none: done); its stacked items parsed (not JSON: data
   error, logged, stop); the **apply** below; unchanged: done; else `update_both_if_revision_matches` with the
   instanced text unchanged (lost: retry). Refilled: logged (info) `[SessionSupplies] refilled {n} item(s) for {u}
   (character {c})`. Writes are plain `write`s with actor `System` (no home check, §15 item 14).

**Apply** (stacked list, targets): each entry whose `catalogId` is a target: the first one is kept with `quantity`
set to the target (a target of 0 removes it); later duplicates are removed; any other entry is kept as it is; then
each target not seen and > 0 is appended as `{"catalogId":..,"quantity":..}`. "Unchanged" when no entry was removed,
added or had a different numeric quantity.

| Stacked | Targets | Result |
| --- | --- | --- |
| A 4, B 9, Z 1, A 7 | A 10, B 0, C 3 | A 10, Z 1, C 3 |
| A 10, Z 1, C 3 | the same | unchanged |
| (empty) | Q 0 | unchanged |

### 4.15 The federation forward of a grant (DN5)

Only for a `gs` grant, after the host checks. The account of u is **foreign** when u parses as a global id of kind
`Account` whose owner is another node, or u is a local userId whose account's global id is owned by another node.
Then: m = §4.7 on a; the hunt id is the `x-reforged-instance-id` header parsed as a global id, else the matchmaking
part's ring instance for the player (§9.5), else a new `Hunt` global id on this node; the module id is `isles-` + the
table's game version with `.` replaced by `-` (`isles-1-14-7`; invalid: STORE-like empty 500); the home is asked
`ring_forward_hunt_rewards` with `{hunt, module, account, xp_grant: {"track":p,"xp":m}}` (no loot, no objective
write). A failure: logged (error) and empty 500.

---

## 5. Host writes (the rifts guard)

### 5.1 The calls

Every call is on `Shared::rifts().guard()` (`rifts_seam::GuardHooks`):

| Use | Call | Outcome |
| --- | --- | --- |
| authorize | `authorize_host_write(capability, kind, AccountId(target), options)` | `Ok(None)` allowed; `Ok(Some(reply))` refused, answered as the hook reply; `Err` logged (error) `[HostWrite] authorize {kind} for {target} failed: {error}` and empty 500 |
| tag | `record_host_write(capability, {kind, target, summary: JSON text})` | never fails the request; an error is logged `[HostWrite] tag failed kind=.. target=..: {error}` |
| observe mode | `host_policy_refuses()` | an error reads as refusing |
| note | `note_would_refuse(what, detail)` | an error is logged |

A `host` caller without a capability (its opaque handle missing or not a `RuntimeAuth`, §9.7) is refused every
write with 403
`{"code":"forbidden","message":"This runtime cannot write player state.","payload":null}` and tags nothing.
"Not a probe" means `HostWriteOptions::default()`; a probe is `probe: true`; the authoritative check is `parity: true`.

### 5.2 The XP grant's kind

For a `host` grant, in order: probe `Xp`; if refused and p is a prestige track, probe `Reforge` (allowed: the kind is
`Reforge`); if still refused and p is the **active** pass id, probe `HuntPassXp` (allowed: kind `HuntPassXp`); if
still refused, ask `Xp` again without probe and take its answer as final. The kind chosen is what the tag names.

### 5.3 Notes

After an applied, replayed or refused-later grant, each note gathered (§4.4) is sent with `note_would_refuse`.

### 5.4 Tag summaries (JSON, keys in this order)

| Kind | Summary |
| --- | --- |
| `Xp`, `HuntPassXp` | `{"track":p,"amount":a,"grantId":g}` (a the request's, unmultiplied) |
| `Reforge` grant with a `Prestige` plan | `{"route":"xp","track":p,"amount":a,"grantId":g,"reforgeId":id}` |
| `Objectives` | `{"progressTracks":[{"id":<first 128 chars of String(progression_id)>,"progress":<Number>}...first 20],"progressTrackCount":n,"objectiveCount":n,"completedObjectiveCount":<objectives with Number(completed_count) >= 1>}` |
| `Mastery` | `{"objectives":n,"newlyCompleted":[first 20],"newlyCompletedCount":n,"credits":{track:n},"claimed":{track:n}}` + when non-empty `capped`, `unmapped`, `ignoredCount` and `ignored` (`id:reason`), `overLimitCount` and `overLimit`, `clamped`, `atCap` (each list its first 20) |
| `RankConfirm` | §3.6 step 9 |
| `Escalation` | `{"season":s}` + §4.10's summary |
| `Reforge` (list repair) | `{"staleReforgesFinished":[ids]}` |
| `SlayersPath` | `{"loginCatchUp":[ids]}` (§3.20) or `{"unlocked":[..],"stripped":[..]}` + `relocked` (§3.21) |

---

## 6. Game data read here (formats; never committed)

All are backend files of core §8.1, exported from the host's install or written by game-data's generators.

| File | Read | Shape used |
| --- | --- | --- |
| `progression_config.json` | once per process per path (§4.1) | `payload.paths[]`: `progression_id`, `requirements[]` `{rank_id, xp_required}` (per-rank increments), `prestige.xp_per_level` |
| `slayers_path.json` | each time (§3.22, the graph's served nodes) | an envelope whose `payload.nodes` is an object keyed by node id; each node `{node_id, node_status, objectives}` |
| `slayers_path_definitions.json` | once per process per path | an object keyed by node id: `childNodes` (strings; `None` for an empty slot), `rewards` (catalog ids; `None` for empty), `currencyCosts` (not read here) |
| `mastery_objectives.json` | each host report | `{"schema":1,"objectiveIds":[strings]}` |
| `escalation_tuning.json` | each host escalation write | §4.10 |

Each read strips one leading BOM and parses JSON; "missing" never fails a route except §3.22's.

---

## 7. Store use (store spec names)

| Repository (store §) | Calls |
| --- | --- |
| `accounts` (6.1) | `find_by_user_id` (every user lookup), `find` (the first-character user id) |
| `progression_tracks` (10.2) | `get_all_for_account`, `get`, `increment`, `set_progress_if_greater`, `set_confirmed_fremium_rank`, `append_objective_event`, `get_all_objectives_for_account`, `merge_objectives`, `get_objectives`, `update_reforge` |
| `progression_grants` (10.3) | `find_by_grant_id`, `insert_applied` |
| `escalation_progress` (10.4) | `get`, `save_if_version` |
| `player_journey` (10.5) | `find`, `create_if_absent`, `update_if_revision` |
| `breadcrumbs`, `encountered_content` (10.6) | `find`, `create`, `update`, `update_content` |
| `characters` (7.1) | `find` (ownership, via §9.4), `find_many_by_account` (supplies) |
| `inventories` (9.1) | `find_by_character_id`, `update_both_if_revision_matches` (supplies) |

`set_objective_if_greater`, `begin_reforge`, `claim_prestige_levels`, `spend` and `list_recent_for_account` are not
called by this part (the inventory part and others use some). The database is the composer's (`IslesStore`), held
in core's `Shared` area slot (§10.3).

---

## 8. Settings (read through core's `Settings`; none is new)

| Setting | Reading | Used |
| --- | --- | --- |
| `RUSTIC_ISLES_REFORGE` (switch `reforge`) | §4.5 (`raw`; unset follows `reforge_rules()`) | §3.2, §3.4 |
| `RUSTIC_ISLES_MASTERY_PERSISTENCE` (switch `mastery_persistence`) | set: `on_unless_off`; unset: the hook `mastery_persistence()` | §3.5 |
| `RUSTIC_ISLES_ECONOMY_MONITOR` | `on_unless_off` (unset on) | §4.3 monitor line |
| `RUSTIC_ISLES_GAUNTLET` (the table's `unless`) | `raw`, JS-trimmed, `on` any case | §4.11 masking |
| `GAMESERVER_HUNTPASS_ID` | through game-data's served id | §3.1, §4.2 |
| `REFORGED_XP_MULTIPLIER`, `REFORGED_REWARD_MULTIPLIER`, `REFORGED_SERVER_MODE`, `REFORGED_SERVER_HOST` | §4.7 (`raw`) | §3.4, §4.15 |
| `REFORGED_SESSION_SUPPLIES_FILE` | `get` | §4.14 |

Hooks and tables (core §5.2): `huntpass_payload`, `objective_row`, `reforge_rules`, `mastery_persistence`,
`active_huntpass_id`, `masked_journey_nodes`, `game_version`. No `MYSTICPARADOX_*` alias is read (core §17 item 2).

---

## 9. Calls into and out of this part

The three parts are one crate; these are the contracts between them, numbered as
[`spec-rustic-isles-economy-inventory.md`](spec-rustic-isles-economy-inventory.md) §12 numbers them (P1 to P7 out of
inventory into progression, I1 to I8 into inventory) and as
[`spec-rustic-isles-economy-bounties.md`](spec-rustic-isles-economy-bounties.md) §6.3 names them. "In tx" means the
call takes the caller's `&mut Tx` and `&Repositories` and adds its writes to the caller's transaction (one
`write`/`account_write` closure; committed on `Ok`, rolled back on `Err`); it never opens its own.

### 9.1 Progression calls the inventory part (inventory §12.3)

Each call uses a `TxContext` built from the module's `Shared`, the store slot and `InventoryPorts::default()`; this
part never calls the inventory handlers.

| # | Call | Inputs | Output | Errors | Atomicity |
| --- | --- | --- | --- | --- | --- |
| I2 | `inventory::get_inventory` | ctx, `Some(user id)`, the first character id (below), no request id | the inventory JSON; this part reads its `instancedItems` (none or not an array: `[]`) | `TxError`, taken as text (§4.12 logs it and grants nothing) | inventory §6.3, its own `account_write` (may write) |
| I3 | `inventory::run_inventory_transaction` | ctx, `TxRequest { user_id, character_id, transaction_id, add_instanced: [item], add_stacked: [], remove_instanced: [{"catalogId","instanceId"}...], remove_stacked: [], save_instanced: [] }`, `TxOptions { ownership_already_validated: true, diagnostic_label: Some("slayers-path-reward"), economy: Some(Economy { source: "OTHER", p2p_session_id: None, store_sku_id: None }), ..Default }`, no request id | `Done(_)` → granted; `NotOwned` → not applied | any `TxError` → its text (logged, retried on the next save) | one inventory transaction of its own |
| I7 | `multipliers::read_test_multipliers` (crate root) | settings | `TestMultipliers { xp, rewards, reason }` per inventory §10.1 | none | pure; this part applies `xp` (§4.7) |
| I5 | `inventory::storage_policy::StoragePolicy::load`, `entry` | shared, catalog id | instanced when the entry's storage is `Instanced` (missing file or entry: not instanced) | none | read only; §4.14's "is an instanced item" check |

The **first character** of §4.12 comes from the characters port `first_character_id(shared, store, actor, user)`
(inventory §11.3's `Characters`; actor `Player { account }`; user = the account's `user_id` via `accounts.find`, else
the account id text); `Ok(None)` is "no character". It may create the account's first character (the characters
crate's rule). I5 is listed in inventory §12.3 for auth only; progression makes the same read.

### 9.2 The inventory part calls progression (inventory §12.2)

| # | Call | Inputs | Output | Errors | Atomicity |
| --- | --- | --- | --- | --- | --- |
| P1 | `progression::reforge::reforge_mode` | shared | `ReforgeMode::{On, Log, Off}` (§4.5) | none | pure |
| P2 | `progression::reforge::classify_grant` | mode, progression id, amount, `Option<&ProgressionTrackRecord>` (the weapon's ExperienceTrack row), now ms | `ReforgePlan::{Normal, Prestige { experience_track, reforge }, Restore { experience_track, reforge }, Refuse(reason)}` (§4.5) | none | pure |
| P3 | `progression::reforge::mark_reforge_use` | tx, repos, account, &plan | `Ok(Ok(()))` marked, or nothing to mark (`Normal`, `Refuse`); `Ok(Err(ReforgeConflict(text)))`, text `reforge <id> changed concurrently (prestige|restore)` | `RepoError` | in tx; §4.5's mark (for `Restore`: expected as classified, set `bankedRestored` = banked, plus `status` `closed` and `closedAt` now when the +1 was already granted); the log label is the account id text |
| P4 | `progression::reforge::finish_stale_reforge` | tx, repos, account, user id, ExperienceTrack id, &WeaponReforge (open) | `bool`: whether this call closed it | `RepoError` | in tx; §4.6 |
| P5 | `progression::reforge::STALE_REFORGE_MS` | | 120 000 | | constant |
| P6 | `progression::tracks::{is_reforgeable_track, track_cap_xp, MAX_PROGRESSION_GRANT_AMOUNT, grant_request_hash, is_valid_grant_id}` | id; shared and id; —; user, track, amount; id | bool; `Option<f64>` (§4.1's cap, none without a curve); 100 000; §4.3's hash; §4.3's pattern | none | pure (the cap reads the cached curves) |
| P7 | repositories | `progression_tracks.{get, begin_reforge, increment, claim_prestige_levels}`, `progression_grants.{find_by_grant_id, insert_applied}` | store §10 | `RepoError` | the inventory part's own writes; they share the grant ledger with §4.3, so a grant id is unique across the XP grant and the banked purchase |

This part also offers `finish_stale_reforges` (the list form, §4.6), `prestige_track_for` and
`experience_track_for_prestige`; the inventory part does not need them.

### 9.3 The bounties-store part calls progression (bounties §6.3)

The SKU progression rule (an xp increment, or ranks up to a curve threshold) is the bounties part's, run in its store
transaction; it reads from here:

| Call | Inputs | Output | Errors | Atomicity |
| --- | --- | --- | --- | --- |
| `progression::resolve_track_id` | shared, id | `Option<String>` (§4.2: `selected_huntpass` → the active pass; else the id when it has a curve) | none | pure over the cached curves |
| `progression::curves` | shared | `Arc<Curves>`: `get(track) -> Option<&TrackCurve>`, `rank_for_progress(track, progress: f64) -> Option<i64>`, `TrackCurve::max_rank`, `TrackCurve::required_xp_for_rank(rank) -> Option<f64>` | none | pure (§4.1) |

`resolve_track_id` and `curves` are re-exported at `progression::` from `progression::tracks`. Progression makes no
call into the bounties part.

### 9.4 Progression calls the characters part (unit 5) and core

| Call | Contract |
| --- | --- |
| **owns** (shared, account, character id) → bool | `characters.find(character, account)` is some; a store error reads false (logged `progression: character lookup failed: ...`) |
| **ensure starter bootstrap** (shared, account, character id) | the characters crate's starter rule (inventory §11.3's `StarterBootstrap`) in one account write with actor `Player { account }`; never fails (errors ignored); without a database nothing |
| **first character id** | inventory §11.3's `Characters` port (§9.1) |
| core | `Shared` (settings, hooks, tables, game data root, `rifts`, `home()`, `auth()`), `replay::next_uuid`, `now_ms`, the store's `now_iso` |

### 9.5 Ports to later units

- **Activity** (Reforged API, unit 9; monitoring only, never fails a request, default nothing): `xp(user, track,
  amount)` on each applied grant; `slayers_path_unlocks(user, node keys, "host"|"gameserver")` after every journey
  save. The reforge reset's activity is the inventory part's `Observer::reforge_reset`.
- **`RingInstances`** (inventory §11.3's slot trait, filled by matchmaking, unit 8):
  `ring_instance_for_player(&GlobalId) -> Option<GlobalId>`; default none. Used only by §4.15.

### 9.6 Auth calls progression

`rustic_isles_auth::GameLogin::session_supplies(&GameAccount)` (auth §11.2 `Seams.game_login`): §4.14 for the
account's `id` (parsed as `AccountId`; unparseable: nothing) and `user_id`. Other users of the track API: the
Reforged API's tester kit (unit 9: approved track, cap, rank for progress, grant hash, the ledger) and its home
operations `progression.grant`/`progression.read` (core §7.8), which use the store directly.

### 9.7 The host capability

Every guard call needs the `rifts_seam::RuntimeAuth` of a host caller. Per the inventory spec's coordinator
decisions (item 2), `rustic_isles_auth::RuntimeCaller` carries an opaque handle (`Option<Arc<dyn Any + Send + Sync>>`,
added by the characters unit) that the realms seam fills; this part downcasts it to `rifts_seam::RuntimeAuth`. A
runtime caller whose handle is missing or of another type is §5.1's "host without a capability" (403, no tag).

---

## 10. The seam

### 10.1 Crate and module

`rustic-isles-economy` (lib, MIT, `crates/rustic-isles-economy`), module `progression` for this part, `multipliers`
at the crate root (shared with the inventory part). Dependencies as inventory §11.1, plus `rustic-isles-gamedata`
(`progression_config::served_huntpass_id`) and `module-api`'s `ModuleId`, `IdKind` and `HuntRewards` for §4.15.

### 10.2 Seam changes for the integration pass

- `crates/rustic-isles-economy` and `rustic-isles-economy` in the workspace and `[workspace.dependencies]` (shared
  with the other two parts), with `rustic-isles-gamedata` as a dependency.
- Auth's `RuntimeCaller` opaque handle (§9.7), owned by the characters unit.

### 10.3 Interface (names are the contract the composer and the other parts are written against)

| Item | Contract |
| --- | --- |
| `pub fn register(handlers: &mut Handlers, ports: ProgressionPorts)` | registers the 25 handlers of §3 on `Service::Backend` at their exact path strings (core §5.7) |
| `pub fn routes() -> &'static [(Method, &'static str)]` | the 25 in §3's order; a test checks them against core's `progression` rows |
| `pub struct ProgressionPorts` (`Clone`, `Default`) | `characters` (owns, starter bootstrap), `activity: Arc<dyn Activity>`, `ring: Arc<dyn RingInstances>` (inventory's trait), `inventory: InventoryPorts` (the `TxContext` of §9.1, and its `Characters` for the first character id); defaults as inventory §11.3, nothing for `activity` |
| area slot | the composer puts the `IslesStore` in core's slot; the curves and definitions caches are per module, keyed by path (inventory decisions, item 3) |
| `pub mod tracks` (re-exported: `progression::{resolve_track_id, curves}`) | `Curves` (`get`, `has`, `rank_for_progress`), `TrackCurve` (`max_rank`, `cap_xp`, `required_xp_for_rank`), `curves(&Shared)`, `resolve_track_id`, `is_approved_xp_track`, `track_cap_xp`, `is_reforgeable_track`, `experience_track_for_prestige`, `prestige_track_for`, `grant_request_hash`, `is_valid_grant_id`, `MAX_PROGRESSION_GRANT_AMOUNT`, `NO_PLAYER`, `DEFAULT_PROGRESSION_IDS` |
| `pub mod reforge` | `ReforgeMode`, `reforge_mode`, `ReforgePlan` (`kind()`: `normal`, `prestige`, `restore`, `refuse`), `classify_grant`, `mark_reforge_use`, `ReforgeConflict` (Display = its message), `finish_stale_reforge`, `finish_stale_reforges`, `STALE_REFORGE_MS` |
| `multipliers::multiply_xp_grant(amount: f64, max: f64, &TestMultipliers) -> f64` | §4.7, beside inventory's reader at the crate root |
| `pub mod mastery`, `escalation`, `journey`, `supplies` | the pure rules of §4.8 to §4.14 (`report_is_valid`, `plan_mastery_report`, `merge_escalation`, `merge_host_escalation`, `parse_tuning`, `merge_nodes`, `plan_host_journey`, `mask_nodes`, `plan_rewards`, `parse_session_supplies`, `apply_session_supplies`) for tests and fuzzing |
| `pub struct SessionSupplies` | implements `rustic_isles_auth::GameLogin` (§9.6) over `Shared` and the ports |

### 10.4 How it plugs into core

The composer calls `register` on its backend `Handlers` with real ports, puts the store in the slot and hands
`SessionSupplies` to auth's `Seams.game_login`. Every handler reads only its call's shared state, params, headers and
body.

---


## 11. Logs

English, never a setting's value or a secret. Beyond those named above: `Rejected XP grant attempt without gameserver
auth (userId=.., track=..)`, `Rejected XP grant: invalid/sentinel userId (..)`, `Rejected XP grant: invalid
progressionId (len=n)`, `Rejected XP grant: progression track is not approved (user=.., track=..): {reason}`,
`Rejected XP grant: missing or invalid authoritative grant id (...)`, `Rejected XP grant: invalid amount (..) for
user=.. track=..`, the rank-confirm equivalents (`Rejected rank confirm ...`, `Confirm public progression rank {r} for
userId {u} track {p} - progress preserved at {n}`), the report diagnostics (`[ProgressionObjective] user={t}
UNRECOGNIZED top-level key(s) [..] in a progression report - possible unhandled reward/grant payload, not applied.
Raw: {first 2000 characters}` when R has keys other than `progress_tracks`, `objectives`; then (info)
`[ProgressionObjective] user={t} mode=diagnostics-only tracksReceived={n} objectivesReceived={n}
authoritativeMutation=0 rawBytes={bytes}`), the sentinel debug lines, the Slayer's Path reconciliation lines
(`[slayers-path] ...`, `Slayer's Path login reconciliation granted [..] to {u}`, `Player journey map SAVED for userId
{u} ({n} nodes, v{v})`).

---

## 12. What stays in dainty-restricted

The goldens (`tests/golden/progression/*`, their scenario file and game data) and the golden harness
(`testkit/golden.rs`), run there against this crate once composed; `isles-1-14-7`'s hook payloads and tables (the
wire facts `huntpass_payload`, `objective_row`, `active_huntpass_id` = `season43`, `masked_journey_nodes`); the
multipliers' start-up line; the migrations (store unit). Restricted's composer drops `areas/progression*` and the
XP half of `matchmaking/multipliers.rs` when this lands.

---

## 13. Test cases (authored tests only; the goldens are not copied)

Tests run the handlers over `rustic-isles-store`'s temporary SQLite store with a test auth that sets the caller from
headers (role `gs`, `player`, `host`, none; a user; a host roster) and a recording guard (allow, refuse with a
reply, refuse some kinds only on probes, policy observe/enforce, recorded tags and notes). Game data is written by
the test into a temporary folder (synthetic curves, a five-node graph, a small tuning table): nothing from the game.

### 13.1 Reads

| Case | Expect |
| --- | --- |
| `GET /huntpass/u` as player | 200 OK `{"huntpass_id":A,"season":A}` (A the served id) |
| `GET /progression/u`, nothing stored | 24 zero rows: the 23 defaults then the active pass, `confirmed_date` now |
| same for `INVALID` | the same 24 rows, no store use |
| `GET /progression/other` as player u | 403 FORBIDDEN |
| no auth | auth's 401 |
| `GET /progression/u/objectives` | empty 404 before auth |
| `GET /progression/u/ExperienceTrack_PlayerLevel` nothing stored | a zero row |
| single track, p of 129 characters | a zero row, no store use |
| `GET /progression/objectives/INVALID` | OK `[]` |
| `GET /weapon_tracker/u/x` | OK `{"TrackersInfo":[]}`; other account 403 |
| tracked objectives GET / POST | the fixed payload / payload `null` |
| a stored track outside the defaults | appended after the 24 known rows |

### 13.2 Grants (curves: `ExperienceTrack_PlayerLevel` and weapon tracks with ranks 1..3)

| Case | Expect |
| --- | --- |
| gs grant 100, header id `g1` | OK, `replayed:false`, progress 100 |
| the same again | OK, the same answer with `replayed:true`, progress still 100 |
| `g1` with amount 50 | 409 CONFLICT |
| `g1` for another user | 409 |
| gs, no id | OK, `grant_id` `gs-<uuid>` (seeded replay: deterministic) |
| player, no id | 403 (auth checked before ids matter) |
| id `-x`, id with space, 129 characters | 400 |
| header `a`, body `grant_id` `b` | 400; body `grantId` only: used; `idempotency-key` only: used |
| amount `0`, `abc`, `1.5`, `100001` | 400; `1e3` is 1000; `100000` accepted |
| user `INVALID` | 400 |
| unapproved `Linked_Slayer_Slot_1`, a track without a curve, a long track | 400 |
| active pass id; the served pass when it differs and has the shape | accepted; another `season` id not served: 400 |
| prestige +1 with no open reforge | 409 |
| test multiplier x5 on a private Tailscale host | 125 → 625 stored; off on a public host |

### 13.3 Confirm, reforge, report

| Case | Expect |
| --- | --- |
| confirm rank 1 with enough XP | OK, `confirmed_fremium_rank` 1; again OK; rank 0 after keeps 1 |
| rank the XP does not reach | 409 `Rejected: insufficient_xp` |
| rank above max (no prestige) / unknown track | 400 `Rejected: rank_out_of_range` / `Rejected: unknown_track` |
| rank `x`, `1.5` | 400 BAD |
| a mastery rank one short / two short | synced and OK / 409 |
| open reforge (inventory's reset) then prestige +1 | OK; again 409; replay of the same id OK `replayed:true` |
| banked restore of the wrong size | an ordinary grant; of the remaining amount: marks and closes when +1 done; again 409 within 15 min |
| an open reforge older than 2 min, gs `GET /progression/u` | finished once: banked back, prestige +1; a second load changes nothing |
| report with first completions (gs) | §4.9's first table row; objectives listed with now dates |
| the same report again | no credit |
| report by a player token for itself | 200 `{}`, event stored, no objective rows |
| report `progress_tracks: 5` / negative value / bad id | 400 |
| report for `INVALID` | 200 `{}`, nothing stored |

### 13.4 Escalation, encountered content, breadcrumbs, journey

| Case | Expect |
| --- | --- |
| escalation GET nothing stored | zeros; another account: empty 403 |
| gs write `{level 9, xp 50}` then `{level 4}` | level stays 9, version 2 |
| bad values (`-1`, `1.5`, `"x"`) | ignored as omitted |
| player writes own season | allowed (may write); another player's: empty 403 |
| host write, unknown season with a table | 403 `ESCALATION_SEASON_UNKNOWN` |
| host write lowering the level, taking back `collected:true` | stored pair and flag kept, summary counts ignored |
| host respec to a valid build / 6 points at level 5 | replaced, `talentsReset` / kept, `talentsRejected` text |
| encountered add, add again, other category | entries appended in order |
| add without `content_type` | 200 `{}`, nothing stored |
| get type 3 / 9 / `abc` | its entries / `content_types: []` / `[]` |
| query with no body | empty 500; no list: `content_types: []` |
| another account's character | 403 forbidden body (reads), empty 403 (writes) |
| breadcrumbs first read creates; write echoes only sent keys | |
| pjm first gs save, read masks `Gauntlet_Unlock` | stored status 2, answered 0 |
| newer save adds nodes; stale save keeps stored nodes and version | spread orders of §4.11 |
| save echoing the masked status | stored unlock kept |
| host save: unknown node, broken prerequisite, node-id mismatch, relock attempt | §4.11's reasons; stored entries kept |
| host save without definitions | every new unlock `definitions-unavailable` |
| reward node unlocked | one transaction with the stable ids; a legacy `PART_` instance repaired |
| `GET /pjm` no save | the `slayers_path.json` file as it is |

### 13.5 Host limits and modes

| Case | Expect |
| --- | --- |
| host weapon grant 1000 / 1001 (enforce) | OK / 403 HOST_WRITE_LIMIT |
| 1001 in observe mode | OK, a note `xp` recorded |
| host grant to a mastery track | 403 (any mode) |
| host prestige grant that is not a reforge +1 | 409 |
| host Hunt Pass grant over 200 | 403 |
| a session that refuses `Xp` on probe but allows `HuntPassXp` | the grant is tagged `HuntPassXp` |
| weapon grant when only `HuntPassXp` is allowed | the final `Xp` answer (refused) |
| host fills the player track, then grants again (enforce / observe) | 403 / OK with a note; a replay of the filling grant replays |
| guard error on authorize | empty 500 |

### 13.6 Pure rules

Every table of §4 is a unit test; plus session supplies parsing (each error text of §4.14), the Tailscale host check
(`100.64.0.1` yes, `100.128.0.1` no, `a.b.ts.net` yes, `.ts.net` no), curves with duplicate ranks, the tuning
parser's error texts, and the escalation entry merge's 512 limit.

---

## 14. Security behaviour (RX3)

This part sits behind the backend's pipeline (loopback only, the gateway in front) but parses untrusted bodies from
game clients, game servers and player-hosted runtimes. Checklist page: `docs/security/rustic-isles-economy.md` (one for
the crate, a progression section).

- **Authority**: only a game server key or an accepted host runtime grants XP, confirms ranks or saves the journey;
  a player token may read only its own state and report only diagnostics. Mastery credit comes from stored objective
  state and is bounded by the claim.
- **Exactly once**: XP grants are keyed by a validated grant id with a request hash that binds user, track and amount;
  a reforge's +1 and banked restore are marked inside the grant's transaction.
- **Host runtimes** are bounded (§4.4, §4.9, §4.10, §4.11) and every write is authorized and tagged through the guard;
  a host never learns limits before it is authorized for the player.
- **Bounded input**: ids at most 128 characters of a fixed alphabet, report size and counts, escalation entries 512,
  amounts and quantities capped, integers checked before conversion (no `as` cast of an unchecked float, §15 item 9).
- **Log hygiene**: user and track text in logs is cut and non-printables replaced where Node did; raw report text is
  cut to 2000 characters. No secret passes through this part.
- **Game data** is the host's own, read with JSON parsing only; a path is never built from request text.
- **Fuzz targets** (`fuzz/fuzz_targets/`): `rustic_isles_economy_progression_report` (arbitrary JSON into §4.8 and the
  mastery plan with arbitrary stored state and limits: never panics, credits never exceed claims or caps);
  `rustic_isles_economy_journey` (arbitrary stored/incoming maps, versions and graphs into the merge, mask and host
  plan: never panics, a stored unlock is never lost); `rustic_isles_economy_escalation` (arbitrary records, updates
  and tuning text: never panics, a host merge never lowers level or a number). Plus `Number(text)` and the grant-id
  check on arbitrary text.

---

## 15. Behaviour that looks wrong but is kept (parity)

1. **The body is parsed before auth**: a malformed body gets its 400/413/415 page before an unauthenticated caller's
   401.
2. **Grant id edge cases**: a body `grantId: null` is "sent" (no mint for a game server) and then invalid; a header
   plus a non-string body id is a mismatch (400).
3. **The grant route's reforge classification uses the system clock**, not the replay clock (a replayed corpus may
   classify the 15-minute duplicate window differently).
4. **Monitor line arithmetic** uses the unmultiplied amount for "old" progress.
5. **A minted grant id consumes a replay id** even when the request is refused later.
6. **Objective rows' `created_date` is `updated_at`** (the store keeps no creation date in the record).
7. **A missing rank inside the curve requires 0 XP**, so its confirm always passes.
8. **A missing or bad `progression_config.json` is cached as "no curves"** for the life of the process; every grant
   then answers 400 until a restart. Likewise the definitions.
9. **Number readings follow the restricted port**: a NaN `rank_id` is rank 0, fractional ranks truncate, stored
   versions truncate.
10. **The federation forward has no idempotency**: a retried forward grants twice at the home.
11. **The objective event is stored for a player's diagnostics-only report** (it is a log, not state).
12. **A JSON array report is valid** and stores an event with no objectives.
13. **`GET /breadcrumbs/:characterId` writes** (creates the row); a `breadcrumbs: null` write stores `null`.
14. **The supply refill uses plain writes with actor `System`** (no home check), as restricted does.
15. **Encountered content appends duplicates** and answers categories in ascending order, not the request's.
16. **Empty 403s** on escalation, encountered-content and breadcrumbs writes, but enveloped 403s elsewhere.
17. **A host's progression read of another player is refused** (the read gate looks at `gs` and the token's user),
    so the stale repair and Slayer's Path catch-up for a host run only for its own account in practice.

---

## 16. Open questions

1. **The multiplier text rule differs between the two readings.** Inventory §10.1 (which owns the shared reader)
   accepts only ASCII digits, `.`, `-`, `+`, `e`, `E` parsed as a decimal float; restricted reads the value with
   JavaScript `Number`, so `0x5` (5) is on there and off under inventory's rule. §4.7 step 1 defers to inventory
   §10.1; the coordinator should confirm which is parity.
2. **Matchmaking's ring map** (§9.5) is unit 8's state; until it lands the slot answers none and a forwarded grant
   carries a fresh hunt id unless the header names one.
3. **The system-clock classification** (§15 item 3): a candidate decision row to use the replay clock; parity until
   then.
4. **Caching a failed game-data read** (§15 item 8): a candidate decision row to retry; parity until then.
5. **Characters ports** (§9.4) are needed before unit 5's crate exists; the composer wires restricted's
   implementations meanwhile.

---

## Coordinator decisions (2026-11-05, before implementation)

These settle the open questions and override anything above that disagrees.

- **Multiplier text rule: JavaScript `Number`** (parity: `0x5`, ` 2 `, `1e1` read as JavaScript reads them). The shared
  reader in inventory §10.1 follows this rule, not the digits-only one; the inventory spec's decisions say so.
- **I5:** progression's session-supplies refill calls I5 (`StoragePolicy::load` / `entry`); inventory §12.3's table
  is amended by its decisions.
- **Items 2-5:** parity; items 3 and 4 are candidate decision rows for the maintainer. The characters ports are
  `rustic-isles-characters`' (unit 5), which is built before this crate.
- **§15:** parity, as listed.
