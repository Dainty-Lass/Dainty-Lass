<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# Clean-Room Behaviour Specification: `crates/rustic-isles-replay` (RD27)

> **Ports moved 2026-11-10.** The loopback services' defaults are now backend 41000, Director 41001, content
> 41002, admin 41003, allowlist 41005, chat 41006 and QoS 41010, below the dynamic range Windows lends to Hyper-V,
> WSL and Docker. host.toml's `[ports]` moves any of them, and on Windows setup and start move one Windows has lent
> out (a private gateway's 61000 to 41080). The numbers below are the ones recorded upstream: read each 610xx
> service port as its 410xx counterpart. A private gateway's default stays 61000.

> **Scope and purpose (RD27):** this is the clean-room behaviour specification for an MIT crate,
> `rustic-isles-replay`: the shadow replay harness of Rustic Isles (RU8) and its command line. It reads the traffic
> capture the gateway writes, sends each recorded request again to one or two targets (or compares one target with
> the recorded answers), substitutes the capture's placeholders with working credentials, pins each request's clock
> and id seed through core's replay headers, learns the ids a target minted so later requests follow them,
> normalizes what legitimately differs, and classifies and counts what remains. It is unit 10 of
> [`spec-rustic-isles-core.md`](spec-rustic-isles-core.md) §16. Its source is dainty-restricted's
> `modules/rustic-isles/crates/isles-core/src/replay.rs` and the `replay` subcommand of `crates/isles-db`
> (`src/replay.rs`, the replay parts of `src/lib.rs` and `src/main.rs`), with restricted's `crates/dainty-host` wiring of
> `reforged-host db replay`; all of it is AGPL-3.0-only. An implementer who never sees them writes the MIT crate from
> this document alone.
>
> Everything below is stated from observable contracts: the capture lines it accepts, the bytes it sends, the
> normalization rules, the report's JSON and text, the command line and its exit codes, and the tests. No
> implementation code is reproduced and no private helper is named. Patterns are given as the shapes they match, not
> as source. Wire names the targets depend on (`x-isles-replay-now-ms`, `x-isles-replay-seed`,
> `x-rustic-isles-gameserver-apikey`, `x-mystic-party-revision`, `paradox-backend`) are interface facts and are kept.
>
> Written against dainty-restricted `e0e238b4434316bb210677d88bc59e3448688794` (worktree
> `splashloader-integration-90130d`; the harness last changed in `47c5498`, born in `0a29728`) and dainty-lass
> `eb92e8ac64a953badb78b50bbf336522e7099116` (branch `feat/rustic-isles-clean-room`).

---

## 1. Provenance and licensing classification

### 1.1 Evidence used

| Source | What it says |
| --- | --- |
| `modules/rustic-isles/NOTICE.md` | Every crate and file of the module is AGPL-3.0-only |
| File headers (`//!`) and `git log` | `isles-core/src/replay.rs` and `isles-db/src/replay.rs` are RU8, written for Dainty Lass (`0a29728`), changed by `3362df4` (RU9: gateway-redacted field alignment), `9ad97c6` (rename) and `47c5498` (docs). One author, the maintainer. No Paradox TypeScript is ported in them |
| `isles-db/tests/replay_test.rs` | Authored tests: parsing, substitution, request preparation, normalizer and diff classes, id learning, CLI parsing and key maps, two end-to-end runs against a composed 1.14.7 module (described only, §13) |
| `modules/rustic-isles/docs/shadow-replay.md` | The host-facing description (partly stale: §15 items 1, 2) |
| dainty-lass `crates/dainty-gateway/src/capture.rs`, `docs/port/spec-rd27-gateway-front.md` §12 | The capture's writer: the format this crate reads (MIT, read for the format only) |
| dainty-lass `crates/dainty-hostkit/src/cli.rs` (`DbReplayArgs`), `commands/db.rs` | `reforged-host db replay`'s flags (MIT) and its stub (`shadow replay is not registered in this build`) |
| dainty-restricted `crates/dainty-host/src/lib.rs` (replay command) | How restricted fills the flags' defaults from `secrets.env` and prints the result |
| dainty-lass `scripts/replay-capture.mjs` | MIT convenience wrapper that runs `reforged-host db replay` (§10.4) |

### 1.2 Classification

Category A throughout (Dainty Lass code, no Paradox code). The maintainer could relicense it, but the MIT crate's
shape differs (it depends on the MIT core and auth, not `isles-core`), so **this spec covers all of it and the
implementer writes all of it** (core §17 item 11).

| Restricted file | Rewritten as |
| --- | --- |
| `isles-core/src/replay.rs` | §2 to §9 (capture, substitution, id learning, request, transport, normalizer, comparison, chat, runner, report) |
| `isles-db/src/replay.rs` | §10.1 (options), §10.2 (key map), §10.3 (run) |
| `isles-db/src/lib.rs` (replay usage and flag parser), `src/main.rs` (replay arm) | §10.5 (the standalone CLI) |
| `crates/dainty-host/src/lib.rs` (replay command) | §10.4 (the `reforged-host db replay` defaults and output) |
| `isles-db/tests/replay_test.rs` | §13 (described only) |

### 1.3 What stays in dainty-restricted

- Every recorded capture corpus, key map, migrated snapshot and report, and every golden (`tests/golden/**` of any
  crate). They never come to this repository (§12).
- The end-to-end tests that compose the 1.14.7 module (`build_live_1147_server` and the two runs of §13.3): they
  need the AGPL areas, so they stay with the goldens and run there against this crate.
- `isles-db`'s `migrate`, `verify` and `export-mongo` (unit 11).

---

## 2. Conventions

- **Times**: a record's `t` is read as `Date.parse` reads it (`service_kit::js::date_parse`), giving Unix ms.
- **JSON text**: whenever this crate writes JSON that a target or a comparison sees, it is `JSON.stringify`'s
  compact form in insertion order (`service_kit::jsjson::stringify`, `serde_json` with `preserve_order`).
- **Sides**: a run has a **left** and a **right** target. Each side keeps its own learned-id map (§5) and its own
  normalizer state (§7); nothing is shared between them except the corpus.
- **SHA-256 hex** is lowercase, 64 characters.

---

## 3. Capture format (input)

### 3.1 Files

`--corpus` is a file or a directory.

- A **directory**: its direct entries (not recursive) whose extension is exactly `jsonl` (case-sensitive), sorted
  by full path. The gateway names its files `gateway-<YYYY-MM-DD>.jsonl` (gateway-front §12.2), so path order is day
  order.
- A **file**: read whatever its name.
- A file or directory that cannot be read is an I/O error (§9.5): `I/O error at <path>: <message>`.

### 3.2 Lines

Each line is trimmed; a blank line, a line that is not a JSON object, a line without a string `kind` or without a
non-negative integer `conn`, and a line whose `kind` is neither `http` nor `chat`, are **skipped silently** (a
truncated last line, as when the gateway is still writing, is the expected case).

**`kind: "http"`** also needs string `method` and string `target`, or it is skipped. The fields read:

| Field | Type, default | Meaning |
| --- | --- | --- |
| `t` | string, `""` | ISO time the gateway wrote the line |
| `src` | string, `gateway` | The writer's source name |
| `conn` | integer (required) | Connection number |
| `seq` | integer, `0` | Request sequence number (the gateway's is global, from 1) |
| `ip` | string, `127.0.0.1` | Client address as the gateway saw it |
| `method`, `target` | strings (required) | Method; path and query, as redacted by the gateway |
| `route` | string, `backend` | The gateway's upstream label (`backend`, `content`, `ws`, `network`, `gateway`, `enroll`, `node`; `director` only in hand-made corpora, §15 item 14) |
| `reqHeaders` or `req_headers` | object, `{}` | Request headers (name → string, or array of strings) |
| `reqBody` or `req_body` | body (§3.3), null | Request body |
| `status` | integer 0 to 65535, else `0` | Answer status |
| `resHeaders` or `res_headers` | object, `{}` | Answer headers |
| `resBody` or `res_body` | body, null | Answer body |
| `ms` | integer, `0` | Time taken |
| `aborted` | boolean, `false` | The answer did not complete |

When both spellings of a header or body field are present, the snake_case one wins. The gateway writes camelCase;
the snake_case form is the harness's own (tests and hand-made corpora).

**`kind: "chat"`** reads `t`, `src` (as above), `conn` (required) and these optional fields: `event` (`open`,
`close`), `dir` (`in` from the game, `out` from the server), `text` (the frame, redacted), `omitted` (size of a
frame past the cap), `compressed` (size of a compressed frame), `ip`, `target`, `via`, `reason`. A field of the
wrong type reads as absent.

### 3.3 Bodies

A body value is one of: `null`; `{"json": <any>}`; `{"form": [[k, v], ...]}`; `{"text": "<string>"}`;
`{"binary": <size>}`; `{"omitted": <size>}`. Reading it:

- Not an object: **null**.
- An object with a `json` key: **JSON** of that value (even `{"json": null}`, which is JSON null, not a null body).
- Else a `form` array: **form**; each item that is a two-element array becomes a pair, a non-string element reading
  as the empty string; items of any other shape are dropped.
- Else a string `text`: **text**. Else an integer `binary`: **binary**. Else an integer `omitted`: **omitted**.
- Anything else: **null**.

`binary` and `omitted` are **lossy**: the bytes are not in the capture.

### 3.4 Order

All records of all files are sorted, stably, by (time of `t` in ms, an unreadable `t` counting as 0; then file
index; then line index). The run then works on that one sequence; a record's **index** is its position in it.

### 3.5 Placeholders the gateway leaves (what the harness must fill)

| Placeholder | The gateway wrote it for | Filled with (§4) |
| --- | --- | --- |
| `<token:SUBJECT>` | A JWT whose claims had a string `userId` (else a non-null `sub`) of 1 to 64 characters from `[A-Za-z0-9_-]` | A fresh token for `SUBJECT` |
| `<token>` | A JWT with no such subject | A fresh token for the subject `UID-replay-anon` |
| `<key:HEX64>` | An account key (`RRK_...` or `UUK_...`), as the SHA-256 hex of the key | The raw key from the key map, else a stand-in |
| `<gskey>` | The value of a game-server key header (`x-rustic-isles-gameserver-apikey`, `x-rustic-isles-gameserver-key`) | The game-server key |
| `<plain:ACCOUNT>` | The body of an XMPP SASL `<auth>` (base64 of `\0ACCOUNT\0TOKEN`) | Base64 of `\0ACCOUNT\0<fresh token for ACCOUNT>` |
| `<redacted>` | A secret removed for good (long hex and token-like runs, secret-named fields, passwords, the gateway secret) | Nothing: it marks the record inexact (§4.3) |

---

## 4. Substitution

### 4.1 Per-side credentials

Each side has: an optional HS256 secret, an optional RS256 signing key (base64 of a PEM RSA private key, the
`AUTH_SIGNING_PRIVKEY_B64` form; PKCS#1 or PKCS#8), an optional game-server key, an optional gateway secret (§6.2),
a key map (SHA-256 hex → raw account key) and a **fallback-keys** switch. The command line always turns fallback
keys on (§10.3).

### 4.2 Minting a token for a subject at time `now` (ms)

`iat` is `now` in whole seconds (floor division, so negative times round down); `exp` is the later of `iat + 86400`
and `4102444800` (2100-01-01), so a replayed token never expires during a run whatever the capture's age (§15 item 5).

- **With a signing key** (RS256): header alg `RS256`, typ `JWT`; claims `userId` = subject, `iat`, `exp`,
  `aud` `paradox-backend`, `iss` `paradox-backend` (the game account token of auth §8's table, which the isles
  targets accept).
- **Else** (HS256) with the side's secret, or `replay-default-jwt-secret-key-32b` when none: header exactly
  `{"alg":"HS256","typ":"JWT"}`; claims `userId` and `sub` = subject, `iat`, `exp`, `iss` `rusticisles`,
  `aud` `rustic-isles-game`; signature HMAC-SHA-256 over `base64url(header).base64url(claims)`, all base64url without
  padding. (No isles target accepts it; it exists for targets that use a shared secret, §15 item 6.)

Claim order inside the token is not significant: every comparison normalizes tokens (§7.2). A signing key that is
not valid base64 or not an RSA PEM is a **token error** (`token signing error: invalid base64 signing key: ...`,
`... invalid RSA PEM signing key: ...`), which substitution turns into an inexact record (§4.3), never a failed run.

### 4.3 Substituting a string

In this order, each step on the result of the previous:

1. If the text contains `<redacted>` anywhere, the result is **inexact** (it is still sent).
2. Each `<token:SUBJECT>` (SUBJECT = one or more characters other than `>`) → a token for SUBJECT. A minting error
   leaves the placeholder and marks inexact.
3. Each `<token>` → a token for `UID-replay-anon` (one token minted for all of them in the string).
4. Each `<key:HEX64>` (exactly 64 hex digits, either case) → the key map's raw key for its lowercase form; else, with
   fallback keys on, `RRK_replay_` followed by the first 24 characters of the lowercase hex; else the placeholder is
   left and the result is inexact.
5. Each `<gskey>` → the side's game-server key; without one, left in place and inexact.
6. Each `<plain:ACCOUNT>` → standard base64 (with padding) of NUL, ACCOUNT, NUL, a token for ACCOUNT. A minting error
   leaves it and marks inexact.

A JSON value is substituted recursively: every string (object values and array items, **not** object keys) as
above; inexact if any part is. Numbers, booleans and null are unchanged.

### 4.4 Key map helper

Registering a raw account key adds (SHA-256 hex of its UTF-8 bytes → the key) to a side's map and returns the hex.
The file form is §10.2.

---

## 5. Learned ids

A target mints ids (characters, item instances, grants, parties, candidates, game sessions) that differ from the
ones recorded. Each side keeps a map **recorded id → that target's id** and applies it to every later request it
sends, so a request that names a recorded character reaches the character this target made.

### 5.1 Learning

After each live answer whose body parses as JSON, when the record's recorded answer body is JSON, the two are walked
together:

- **Objects**: for each key of the recorded object that the live object also has, the pair of values is walked with
  that key as the **hint**. Keys only on one side are ignored.
- **Arrays**: walked pairwise up to the shorter length, keeping the enclosing hint.
- **Two strings**: when they differ, the live one is not empty, and the recorded one **looks like a dynamic id**
  (below), the pair is added unless the recorded id is already mapped (**first mapping wins**). Otherwise, when both
  strings start with `{` and both parse as JSON, the parsed values are walked with no hint (double-encoded data, such
  as a character's or loadout's `data`).
- Anything else teaches nothing.

A recorded string **looks like a dynamic id** when:

- the hint is one of `id`, `characterId`, `instanceId`, `partyId`, `candidateId`, `gameSessionId`, `grantId` and the
  string is neither empty nor `INVALID`; or
- it contains a grant id (`gs-` then a UUID, any case), a party id (§7.4), a UUID (8-4-4-4-12 hex, any case, on word
  boundaries) or 32 lowercase hex on word boundaries; or
- it contains an instance-id-shaped word (26 characters of `[A-Z0-9]`, or 16 to 24 lowercase hex, on word
  boundaries) **and** the whole string contains a digit.

**Cross-learning**: when the left side is live and the record's recorded answer body is null (nothing to learn
from), the left and right live answers (both JSON) are walked the same way into the **right** side's map, so the
right follows the left's ids.

### 5.2 Applying

To a string: every mapped recorded id is replaced by its target id as a **plain substring** (no word boundaries),
longest recorded id first, ties in byte order of the recorded id, each replacement working on the result of the
previous ones. To a JSON value: recursively, **object keys included**, and strings inside arrays. An empty map
changes nothing. (Chained replacement is kept, §15 item 9.)

---

## 6. The request a target receives

### 6.1 Preparing an HTTP record for one side

Let `now` be the record's `t` in ms, or **1793016000000** (2026-10-26T12:00:00Z) when `t` does not parse (§15 item 4).
A prepared request is inexact from the start when the record is aborted or either body is lossy.

- **Target**: learned ids applied, then substituted (§4.3). For a `backend` record, a target of exactly
  `/game/login` becomes `/login`, and one starting `/game/login?` becomes `/login?` with the same query (the gateway's
  `/game/` mount, undone because the harness talks to the backend directly).
- **Headers**, in the recorded object's order: each name lowercased; `host`, `content-length`, `transfer-encoding`,
  `connection` and `keep-alive` dropped; a value that is an array becomes its string items joined with `, `, and any
  other non-string its JSON text; learned ids applied, then substituted. An inexact header marks the request inexact,
  **except** an `authorization` header whose value starts (case-insensitive) `basic <redacted>` on a target starting
  `/account/api/oauth/token` (the client's Basic credentials there are not secrets the answer depends on).
- **Body**:
  - null, binary, omitted: empty.
  - JSON: learned ids applied, substituted, then written as `JSON.stringify` text; `content-type: application/json`
    is appended when the recorded headers had none.
  - form: each key and value, ids applied and substituted, percent-encoded as UTF-8 leaving only ASCII letters,
    digits and `-` `.` `_` `*` as they are (space is `%20`, not `+`), joined `k=v` with `&`;
    `content-type: application/x-www-form-urlencoded` appended when the recorded headers had none.
  - text: ids applied and substituted, sent as its UTF-8 bytes.
- **Appended headers**, after the recorded ones and in this order:
  1. `x-forwarded-for: <ip>` when the recorded headers had no `x-forwarded-for` and the record's `ip` is not empty.
  2. With a gateway secret: any `x-gateway-secret` header removed, then `x-gateway-secret: <secret>` (§15 item 7).
  3. With clock pinning (§6.3): `x-isles-replay-now-ms: <now>` (decimal ms) and
     `x-isles-replay-seed: <conn>:<seq>:<method>:<target>`, where the target is the **record's raw target**, before
     ids, substitution and the `/game/login` rewrite, so both sides send the same seed.

Recorded `x-isles-replay-*` headers are not removed; a capture that recorded them would send them twice (§15 item 12).

### 6.2 Which target, and which address

- A `director` record goes to the side's Director address when it has one, else to its backend address. Every other
  replayed record goes to the backend address.
- A side is **recorded** (its answers are the capture's own, §6.5), **http** (a backend base URL, optional Director
  URL, optional chat address) or **router** (an in-process backend router and optional Director router, §6.4).

### 6.3 Clock and id pinning

Pinning is on by default and is forced off for a recorded side. The two headers make the target's request run in a
replay scope (core §9.1): its clock reads the record's time and its new ids are drawn from SHA-256 of the seed, so two
targets with the same starting state mint the same ids for the same request. **The target only honours them when its
module setting `RUSTIC_ISLES_REPLAY_HEADERS` is on** (core §17 item 1); off, it ignores them silently and the run
shows the difference as unpinned times and ids (normalized, §7) and as learned-id churn (§14 item 6, §16 question 1).

### 6.4 Transport (HTTP)

The harness speaks HTTP/1.1 over plain TCP itself, one connection per request:

- **Address**: the base URL trimmed, a leading `http://` removed, everything from the first `/` dropped; with no
  `:` the port is 80. `https://` is not supported (the text is taken as a host and the connect fails).
- **Request**: `<METHOD> <path> HTTP/1.1`, then `Host: <host:port as given>`, `Connection: close`,
  `Content-Length: <body bytes>` (always, `0` included), then the prepared headers (minus the five dropped names),
  `\r\n`, then the body. A prepared target that does not start with `/` is sent as `/`.
- **Answer**: read to end of stream. Everything before the first `\r\n\r\n` is the head (a missing terminator is a
  transport error, `malformed HTTP response (missing header terminator)`); the status is the second word of the first
  line (`invalid HTTP status line: "<line>"` when absent or not a number). Header lines split at the first `:`, names
  lowercased and trimmed, values trimmed, a repeated name joined with `, `. When `transfer-encoding` contains
  `chunked`, the body is de-chunked (hex size before any `;`, stopping at a 0 size, a bad size or a short chunk);
  `content-length` is not consulted. The body becomes text by lossy UTF-8.
- Connect, write and read failures are transport errors: `HTTP transport error calling <base><target>: <message>`.

**Router** sides serve the router on a fresh `127.0.0.1:0` listener for each request (`service_kit::serve`, 50 ms
grace), send the request as above, then stop the listener. This lets restricted's goldens drive a composed module
in-process.

### 6.5 The recorded answer

From the record: status; headers with names lowercased and values joined as in §6.1; body text: JSON as
`JSON.stringify` text, form as `k=v` pairs joined with `&` **without** percent-encoding, text as is, null, binary
and omitted as empty. A recorded side is inexact for a record when the record is aborted or either body is lossy.

---

## 7. Normalization

Applied to both sides' answers before comparison, the recorded side included, so placeholders and minted values on
both sides meet in the same canonical form.

### 7.1 Settings

Three switches, all on by default and not exposed on the command line: normalize timestamps, normalize dynamic ids,
compare raw JSON text (§8.3).

### 7.2 Tokens

In a string, first `<token:X>` → `<jwt:X>` and `<token>` → `<jwt>` (so a recorded placeholder equals a live token).
Then each run shaped `eyJ` + `[A-Za-z0-9_-]+` `.` `[A-Za-z0-9_-]+` `.` `[A-Za-z0-9_-]+` is checked: exactly three
parts, a non-empty signature, header and claims base64url-decode (trailing `=` ignored) to JSON **objects**, and the
header has a non-empty string `alg`. A run that passes becomes `<jwt:SUB>` (SUB = a non-empty string `userId`, else a
non-empty string `sub`) or `<jwt>`; one that fails is left as it is, so a corrupt token shows as a difference. The
signature is never verified.

### 7.3 Strings

A string normalizes, in this order:

1. When it starts with `{`, ends with `}` and parses as JSON: the parsed value is normalized (§7.5, no hint) and the
   result is its `JSON.stringify` text. Nothing below applies.
2. Tokens (§7.2).
3. With hint `refresh_token`, `refreshToken` or `challengeSignature`: a result of 12 or more characters not starting
   `<jwt` becomes `<opaque-token>` (and nothing below applies).
4. With hint `nonce`: a non-empty result becomes `<nonce>` (and nothing below applies).
5. `code_challenge=` followed by 40 to 64 of `[A-Za-z0-9_-]` → `code_challenge=<challenge>`.
6. Timestamps (switch): ISO times (`YYYY-MM-DDTHH:MM:SS`, optional fraction, then `Z` or `±HH:MM`, on word boundaries)
   → `<date>`, **except** those starting `2085-09-09` or `2099-` (game sentinels, kept); login times
   `YYYY.MM.DD-HH.MM.SS` → `<login-time>`; display dates `Mon D, YYYY` (English three-letter month, 1 or 2 digit day)
   → `<display-date>`.
7. Dynamic ids (switch), in this order, each numbered by **first appearance on this side over the whole run** (§7.4):
   grant ids → `gs-<uuid#N>`; party ids → `<party#N>_<suffix>`; UUIDs → `<uuid#N>`; instance-id-shaped words →
   `<iid#N>` when the word contains a digit or the hint is `instanceId`, else unchanged.

### 7.4 Numbering

Four independent counters per side, each starting at 1, keyed by the matched text:

| Kind | Shape | Key | Rendered |
| --- | --- | --- | --- |
| Grant | `gs-` + UUID, any case | lowercased | `gs-<uuid#N>` (its own counter, not the UUID one) |
| Party | one of: `[a-z0-9-]+` `-` 16 to 32 lowercase hex; 32 lowercase hex; a ULID (26 of Crockford `[0-9A-HJKMNP-TV-Z]`) `.` `[a-z0-9-]+`; then `_` and a suffix of `[A-Za-z0-9+/=-]+` (word boundary before) | the part before `_<suffix>` | `<party#N>_<suffix>` |
| UUID | 8-4-4-4-12 hex, any case, word bounded | lowercased | `<uuid#N>` |
| Instance | 26 of `[A-Z0-9]`, or 16 to 24 lowercase hex, word bounded | as is | `<iid#N>` |

Because the counters persist across records and across headers and bodies, co-reference is preserved: an answer that
repeats one id where the other side has two distinct ids shows a difference (§13.2 case N3).

### 7.5 JSON values

Recursively: strings by §7.3 with the enclosing object key as the hint (array items inherit the array's hint);
numbers under the hint `uptimeSeconds` or `processId` become `0`, other numbers unchanged; object keys are kept as
they are, in order; booleans and null unchanged.

### 7.6 Raw text (a body that is not JSON on at least one side)

`"uptimeSeconds"` and `"processId"` followed by optional spaces, `:`, optional spaces and digits are rewritten to
`"uptimeSeconds":0` / `"processId":0`; then the whole text normalizes as a string with no hint (§7.3).

### 7.7 Headers

Names lowercased. Dropped: `date`, `etag`, `x-powered-by`, `connection`, `keep-alive`, `content-length`,
`transfer-encoding`, `x-rustic-isles-request-id`, `x-request-id`, `server`, `access-control-allow-origin`,
`access-control-allow-methods`, `access-control-allow-headers`, `access-control-allow-credentials`, `vary`.
`content-type` is canonicalized (split at `;`, each part trimmed and lowercased, empty parts dropped, joined with
`; `). Every other header's value is trimmed and normalized as a string with the header name as the hint (sharing the
side's counters). So `cache-control`, `location`, `ratelimit`, `ratelimit-policy` and application headers such as
`x-mystic-party-revision` are compared.

### 7.8 Gateway redaction alignment

The gateway removed some values from the recorded answer for good. After both bodies are normalized, they are walked
together (objects by common keys, arrays to the shorter length, the key as hint): when exactly one side's string is
exactly `<redacted>` and the other side's value "is a secret", the other is set to `<redacted>` too. A value "is a
secret" when the hint contains, case-insensitively, any of `key`, `token`, `secret`, `pass`, `auth`, `jwt`, `sig`,
`cred`, `recovery`, `invite`, `cookie` (the gateway's secret-name list), or the value contains a run of 40 or more hex
digits or of 100 or more `[A-Za-z0-9_+/=-]`. A `<redacted>` under any other name still differs (§13.2 case R2).

---

## 8. Comparison

### 8.1 Difference kinds

| Kind | Fields | Raised when |
| --- | --- | --- |
| `StatusDiff` | `left`, `right` (numbers) | The statuses differ |
| `HeaderDiff` | `header`, `left`, `right` (string or null) | A normalized header differs or is on one side only |
| `ValueDiff` | `path`, `left`, `right` (JSON) | Normalized JSON bodies differ at a path |
| `TextDiff` | `left`, `right` (strings) | JSON equal in value but not in text, or non-JSON bodies differ |
| `ChatStanzaDiff` | `conn`, `index`, `left`, `right` (string or null) | A chat connection's normalized server stanza differs at an index |

### 8.2 One HTTP pair

The differences of one record come in this order: the status (if different); header differences over the union of
both sides' normalized header names in **byte order of the name**; then the body. Left headers are normalized before
right headers, and both before the bodies (this order fixes the numbering, §7.4).

### 8.3 Bodies

- **Both parse as JSON**: each is normalized (left first), then aligned (§7.8), then compared structurally from the
  path `$`: equal values give nothing; two numbers equal as doubles give nothing; arrays compare index by index over
  the longer length (`$[i]`), a missing item reading as null; objects compare over the union of keys in byte order
  (`$.key`, the key written as it is, no quoting), a missing member reading as null; any other mismatch (types or
  values) is one `ValueDiff` at that path.
  When there is **no** `ValueDiff` and raw-text comparison is on (the left side's setting): the normalized values'
  `JSON.stringify` texts are compared, and for each side whether its received body differs from the
  `JSON.stringify` text of its own un-normalized parse (whitespace, number spelling, escapes). If the texts differ, or
  only one side's body is non-canonical, one `TextDiff` is raised whose sides are the received body for a
  non-canonical side and the normalized text otherwise. Key order counts here (`{"alpha":1,"beta":2}` against
  `{"beta":2,"alpha":1}` is a `TextDiff`).
- **Otherwise** (either side not JSON, empty bodies included): both normalize as raw text (§7.6); different texts are
  one `TextDiff` of the normalized texts. Two empty bodies are equal.

### 8.4 Classification of an entry

An entry (one HTTP record, or one chat connection) is **inexact** when either side's preparation was (§6.1, §6.5,
§9.3). Then: no differences and exact → **exact matched**; no differences and inexact → **inexact matched**;
differences and inexact → **inexact diffed**; differences and exact → **unexplained diffed**. A run **passes** when
no entry is unexplained diffed.

---

## 9. The run

### 9.1 HTTP records, in corpus order

For each HTTP record:

1. **Skipped** (counted in `skipped_records`, no entry) when: a filter is set and neither the record's raw target nor
   its route contains it; the record is aborted; its route is `content` or `network`; or its route is `director` and
   the **right** side cannot reach a Director (http without a Director URL, router without a Director router; a
   recorded right side always can). The left side's Director is not checked (§15 item 13).
2. Otherwise counted in `http_replayed`; the **left** side answers (recorded, or prepared and sent, then learning,
   §5.1), then the **right** side the same way; then cross-learning (§5.1); then comparison (§8). Sides run one after
   the other, never concurrently, so the second side sees the first's effects only through the learned maps.
3. An entry is recorded for every replayed record, with or without differences.

A transport error on either side **ends the run** with that error (no report).

### 9.2 Chat records

Chat records are not replayed in place. With a filter set, a chat record is skipped (counted) unless the **filter
text itself contains** `chat` (§15 item 11). Kept records are grouped by `conn`. After every HTTP record, the
connections are replayed in ascending `conn` order, each against the left side and then the right side, using each
side's learned map as it stands after all the HTTP records.

### 9.3 One chat connection on one side

The connection is inexact when any of its records has `omitted` or `compressed`.

- **A side with a chat address** (http side with `chat` set; `host:port`): one plain TCP connection (not WebSocket,
  not TLS). For each `in` record with text, in order: learned ids applied, substituted (with that record's time),
  written as its bytes; then one read of up to 8192 bytes with a 50 ms timeout, and when it returns bytes they are one
  **server stanza** (lossy UTF-8). Events and `out` records are not sent. A connect or write failure is a transport
  error (`chat://<addr>`), which ends the run.
- **Any other side** (recorded, router, or http without a chat address): the server stanzas are the recorded `out`
  texts with the side's learned ids applied; the `in` texts are only substituted, to learn whether they would be
  inexact.

### 9.4 Comparing chat

From each side's stanza list, drop the **filtered** ones: empty after trimming, keepalive pings (containing both
`urn:xmpp:ping` and `<ping`), and anything containing `replay-barrier-`. Normalize the rest: trimmed; the `id`
attribute value of a `<stream:stream ...>` opening (single or double quotes) → `<stream>`; any `id='srv-...'` or
`id="srv-..."` value made of hex and `-` → `srv-<id>`; every ISO time → `<date>` (no sentinel exception here). Then
compare index by index over the longer list; each mismatch is one `ChatStanzaDiff` with the connection, the index
and each side's stanza or null.

### 9.5 Errors

| Error | Text |
| --- | --- |
| I/O | `I/O error at <path>: <message>` |
| Configuration | `invalid replay configuration: <message>` |
| Token | `token signing error: <message>` (only inside substitution; never ends a run) |
| Transport | `HTTP transport error calling <target>: <message>` |
| JSON | `JSON serialization error: <message>` |

---

## 10. Interfaces

### 10.1 Library (names are the contract the restricted callers and goldens are written against)

| Item | Contract |
| --- | --- |
| `CaptureBody { Null, Json, Form, Text, Binary, Omitted }` | §3.3: `from_wire(Option<&Value>)`, `to_wire()` (the inverse shape), `is_lossy()` |
| `HttpRecord`, `ChatRecord`, `CaptureRecord { Http, Chat }` | §3.2 fields; `timestamp()`, `timestamp_ms()` |
| `parse_capture_line(&str) -> Option<CaptureRecord>`, `parse_capture_str(&str) -> Vec<_>` (line order), `load_corpus(&Path) -> Result<Vec<_>, ReplayError>` (§3.4 order) | §3 |
| `ReplayTargetAuth { jwt_secret, signing_key_b64, gs_key, gateway_secret, key_map, deterministic_fallback_keys }` | §4.1; `mint_token(subject, now_ms)`, `register_account_key(raw) -> hex`; `with_test_keys(gs_key)` under the `test-support` feature (§11.2). `Debug` shows which secrets are set, never their values |
| `mint_rs256_jwt(subject, privkey_b64, now_ms)`, `mint_hs256_jwt(subject, secret, now_ms)` | §4.2 |
| `Substituted<T> { value, inexact }`, `substitute_str`, `substitute_value` | §4.3 |
| `learn_ids(recorded, actual, &mut map)`, `apply_ids_str`, `apply_ids_value` | §5 |
| `PreparedHttpRequest { method, target, headers, body, inexact }`, `prepare_http_request(record, auth, id_map, pin_clock)` | §6.1. `Debug` hides header values and the body (they carry minted tokens and keys) |
| `ReplayHttpResponse { status, headers (sorted map), body_text }`, `from_recorded(&HttpRecord)` | §6.5 |
| `send_http_request(base_url, &PreparedHttpRequest)`, `serve_ephemeral(router)`, `dispatch_router_request(router, req)` | §6.4 |
| `DiffKind` | §8.1; serialized with a `kind` tag (§10.6) |
| `NormalizerConfig { normalize_timestamps, normalize_dynamic_ids, check_raw_json_text }` (all `true` by default), `IdNormalizer::new(config)` with `normalize_str(text, hint)`, `normalize_value(value, hint)`, `normalize_raw_text(text)` | §7 |
| `normalize_jwts_in_str`, `normalize_headers`, `IGNORED_RESPONSE_HEADERS`, `compare_http_responses(left, right, &mut left_norm, &mut right_norm)` | §7.2, §7.7, §8 |
| `is_filtered_chat_stanza`, `normalize_chat_stanza`, `compare_chat_stanzas(conn, left, right)` | §9.4 |
| `ReplayEndpoint { Recorded (default), Http { base_url, director_url, chat_addr }, Router { backend, director } }` | §6.2; `Debug` shows the URLs and, for a router, only whether a Director is present |
| `ReplayTarget { name, endpoint, auth, pin_clock }`: `recorded()` (name `recorded`, no pinning), `http(name, url, auth)`, `router(name, backend, director, auth)` (both pinning) | §6.2 |
| `ReplayConfig { left, right, normalizer, filter }`, `run_replay(&[CaptureRecord], &ReplayConfig) -> Result<ReplayReport, ReplayError>` | §9 |
| `ReplayEntryResult`, `ReplayReport` with `passed()` | §10.6 |
| `ReplayError { Io, Config, Token, Transport, Json }` | §9.5 |
| `ReplayOptions`, `load_key_map(&Path)`, `run_replay_cli(&ReplayOptions)` | §10.2, §10.3 |
| `pub mod cli`: `parse_args`, `USAGE`, `main_from(args) -> ExitCode` | §10.5 |

### 10.2 Key map file (`--key-map`)

JSON, either:

- an **array**: each string item is a raw key, mapped from its SHA-256 hex; other items ignored; or
- an **object**: for each member whose value is a string, a name of exactly 64 hex digits maps (lowercased) to that
  value; any other name maps the value's own SHA-256 hex to it; non-string values ignored.

Errors (configuration): `failed to read --key-map <path>: <error>`, `invalid JSON in --key-map <path>: <error>`,
`--key-map <path> must be a JSON object or array of strings`. The same map goes to both sides.

### 10.3 Running from options

`ReplayOptions`: `corpus` (path), `left` and `right` (each a base URL, or `recorded`/`corpus` in any case for the
recorded side), `left_director`, `right_director`, `left_chat`, `right_chat`, per-side `jwt_secret`, `signing_key`,
`gs_key`, one `gateway_secret` (both sides), `key_map_path`, `pin_clock` (default true), `filter`, `out_path`.
Defaults when built without values: left `recorded`, right empty, pinning on. `Debug` prints the paths, URLs,
filter and switches and, for each secret, only whether it is set.

The run loads the key map (if any), then the corpus, builds each side (names `left` and `right`, fallback keys on,
pinning = `pin_clock` and not recorded), runs §9 with default normalization and the filter, and, with `out_path`,
writes the report as pretty JSON (two-space indent, no trailing newline) to that file, replacing it
(`failed to write --out <path>: <error>`). Corpus and run errors surface as configuration errors carrying the
inner message.

### 10.4 `reforged-host db replay` (the host kit's command)

The flags are `DbReplayArgs` in dainty-lass's `crates/dainty-hostkit/src/cli.rs` (the clap tree; unchanged by this row):
`--corpus` (required), `--left` (default `recorded`), `--right` (default `http://127.0.0.1:61000`),
`--left-director`, `--right-director`, `--left-chat`, `--right-chat`, `--jwt-secret`, `--left-jwt-secret`,
`--right-jwt-secret`, `--signing-key`, `--left-signing-key`, `--right-signing-key`, `--gs-key`, `--left-gs-key`,
`--right-gs-key`, `--gateway-secret`, `--key-map`, `--pin-clock`, `--no-pin-clock`, `--filter`, `--out`, `--json`.

What the command does with them (today restricted's `run_db_command` hook; §11.1 for where it goes):

- **Secrets default from the host's `secrets.env`** (`<config>/secrets.env`, a missing file reading as empty):
  `AUTH_SIGNING_PRIVKEY_B64` (signing key), `JWT_SECRET`, `GAMESERVER_API_KEY`, `GATEWAY_SECRET`, each trimmed and
  ignored when empty. Per side: the side's flag, else the both-sides flag, else `secrets.env`.
- Pinning: on unless `--no-pin-clock` is given without `--pin-clock`.
- Runs §10.3 on a single-threaded runtime (`could not start runtime for db replay: <error>`); a run error is the
  host kit's error `db replay failed: <error>` (exit 1, printed by the host kit as `Error: ...`).
- Output (§10.6) with the prefix `reforged-host db replay:`. Exit 0 when the run passes, **2** when it does not.
- A clap usage error is clap's own (exit 2).

### 10.5 The standalone CLI (`isles-db replay` today)

Restricted's `isles-db` keeps its `replay` subcommand until unit 11; it becomes a call to this crate's `cli`, which
behaves as follows (flags as §10.4 except where noted):

- Parsing walks the arguments in order; each value flag takes the next argument. `-h`/`--help` anywhere returns help.
  `--jwt-secret`, `--signing-key` and `--gs-key` set **both** sides at that point, so a later per-side flag overrides
  one side and a later both-sides flag overrides both (last wins; unlike §10.4, §15 item 15). An unknown argument:
  `unexpected flag "<arg>"`. A value flag at the end: `--corpus requires a path`, `--left requires a URL or
  'recorded'`, `--right requires a URL`, `--left-director requires a URL` (and right), `--left-chat requires a
  host:port` (and right), `--jwt-secret requires a secret` (and the per-side ones), `--signing-key requires a base64
  key` (and per-side), `--gs-key requires a key` (and per-side), `--gateway-secret requires a secret`, `--key-map
  requires a path`, `--filter requires a substring`, `--out requires a file path`.
- **Required**: `--corpus` (`missing required --corpus <path>`), `--left` (`missing required --left
  <url|recorded>`), `--right` (`missing required --right <url>`). No `secrets.env` defaults here.
- Exit codes: **0** help (usage on stdout) or a passing run; **1** a run error (`isles-db replay failed: <error>` on
  stderr; the prefix is the caller's program name); **2** a parse error (`error: <message>`, a blank line, then the
  usage, on stderr) or a run with unexplained differences.
- Output as §10.6 with the caller's prefix (`isles-db replay:`).

The usage text lists every flag with a one-line description (the `Replay Options:` block of restricted's usage
is the model: same flags, same order; the wording is the implementer's own).

### 10.6 Report

**JSON** (`--json` on stdout, pretty; `--out` to a file): an object with, in order, `total_records` (all records
parsed, skipped included), `http_replayed`, `chat_connections_replayed`, `skipped_records`, `exact_matched`,
`inexact_matched`, `inexact_diffed`, `unexplained_diffed`, `status_diffs`, `header_diffs`, `value_diffs`,
`text_diffs`, `chat_stanza_diffs` (counts of each kind over the run), `duration_ms` (wall time of the run), and
`entries`. Each entry: `index`, `kind` (`http` or `chat`), `conn`, `seq`, `method`, `target` (the record's raw
target), `route`, `inexact`, `left_status`, `right_status`, `diffs`. A chat entry has `seq` 0, `method` `XMPP`,
`target` `chat:conn:<conn>`, `route` `chat`, both statuses null, and `index` = the number of entries before it (not a
corpus index, §15 item 10). A diff is an object whose first member is `kind` (the variant name) followed by the
variant's fields in §8.1's order, e.g. `{"kind":"StatusDiff","left":200,"right":500}`,
`{"kind":"HeaderDiff","header":"content-type","left":"application/json","right":null}`.

**Text** (without `--json`):

```text
<prefix> PASS|DIFF (records=N, http=N, chat_conns=N, skipped=N, exact_matched=N, inexact_matched=N, inexact_diffed=N, unexplained_diffed=N)
  diffs by kind: status=N, header=N, value=N, text=N, chat_stanza=N
  [DIFF|INEXACT] #<index> <method> <target> (conn=N, seq=N, route=<route>): N diff(s)
    - <diff>
```

`PASS` when the run passes. One `[...]` line per entry with differences (`INEXACT` for an inexact entry), then one
`- <diff>` line per difference, written in Rust's derived `Debug` form of the variant, e.g.
`StatusDiff { left: 200, right: 500 }`, `HeaderDiff { header: "content-type", left: Some("a"), right: None }`,
`ValueDiff { path: "$.x", left: String("a"), right: Number(1) }` (serde_json's `Debug` for values). With `--json`
only the JSON is printed. `--out` is written in both modes.

---

## 11. The seam

### 11.1 Placement

- One crate, `rustic-isles-replay` (lib, MIT) in `crates/rustic-isles-replay`, with the `cli` module of §10.5. No
  binary: restricted's `isles-db` (until unit 11) and the host kit call the library.
- Dependencies: dainty-lass `rustic-isles` (only `replay::REPLAY_NOW_HEADER` and `REPLAY_SEED_HEADER`; the harness
  never enters a replay scope itself), `service-kit` (`js::date_parse`, `jsjson::stringify`, `serve`), and workspace
  crates for HTTP (`axum` for the router endpoint, `tokio`), `serde`/`serde_json` (`preserve_order` in its own
  manifest, core §17 item 8), `regex`, `sha2`, `hmac`, `base64`, `percent-encoding`, `thiserror`, and the RS256
  signer the workspace already uses for auth (aws-lc-rs). `rustic-isles-auth` only under the `test-support` feature,
  for its throwaway signing keys.
- **Seam changes for the integration pass** (not this row's): the crate in `[workspace.dependencies]`; the host kit's
  `db replay` (whose stub says `shadow replay is not registered in this build`) calling `run_replay_cli` with §10.4's
  defaults, which needs `rustic-isles-replay` as a dependency of `crates/dainty-hostkit` and moves the defaulting out of
  restricted's `run_db_command` hook; restricted's `isles-db replay` and its host hook then become thin calls or go.

### 11.2 Test keys (from auth)

`ReplayTargetAuth::with_test_keys(gs_key)` (feature `test-support`) is a side with the RS256 private key of
`rustic_isles_auth::test_support::throwaway_signing_keys()` (the process-wide pair auth's tests sign with; auth
§12.2 uses such a pair), the given game-server key, no HS256 secret, no gateway secret, an empty key map and fallback
keys on. A target built with the matching public key accepts the tokens it mints.

### 11.3 What the targets must have

- `RUSTIC_ISLES_REPLAY_HEADERS=on` in each live module's settings for pinning to work (core §17 item 1). Nothing else
  in core changes for this crate: it uses core only through the two header names on the wire.
- The same signing key pair, game-server key and gateway secret as the harness side that talks to them, and, for a
  stateful corpus, a database restored from the snapshot taken when the capture began (restricted's
  `isles-db migrate`/`verify`).

### 11.4 What the gateway gives

The capture lines of §3 (gateway-front §12), from `CAPTURE_DIR`. The gateway is not in the replay path: the harness
talks to the backend, Director and chat directly. Dropping client-sent `x-isles-replay-*` headers at the gateway is a
separate gateway row (core §17 item 1).

---

## 12. What stays in dainty-restricted

- Recorded corpora (they are personal data), key maps (raw account keys), snapshots and reports.
- Every golden, and the end-to-end replay tests that compose the 1.14.7 module (§13.3); they depend on this crate's
  library once it lands.
- `isles-db` and its other subcommands until unit 11; `docs/shadow-replay.md` (host-facing, to be corrected per §15
  items 1 and 2).
- The integration pass decides whether `scripts/replay-capture.mjs` stays (§10.4 and §15 item 16).

---

## 13. Test cases

### 13.1 What may be copied

Nothing. The restricted tests are authored code and their corpora compose AGPL areas. The cases below are written
from this spec with synthetic data; an implementer writes its own fixtures (ids, keys, and secrets made up; the RS256
key from §11.2).

### 13.2 Unit cases (data tables)

**Parsing** (§3)

| # | Input | Expected |
| --- | --- | --- |
| P1 | Seven lines: two `http` (one form body with a `<key:...>` and a Basic `<redacted>` header, one null body), a chat `open`, a chat `in` with `<plain:...>`, a chat `out`, an `http` with `{"binary":128}` request and `{"omitted":65536}` answer, and a truncated last line | Six records; the truncated line ignored; the form pairs in order; binary and omitted are lossy, form is not; the chat open has `conn` and `event` |
| P2 | `{"kind":"http",...,"req_body":{"text":"hello=world"},"res_body":{"text":"<pre>Bad Request</pre>"}}` | Text bodies both ways |
| P3 | `{"kind":"unknown"}`; a line without `conn`; `{"kind":"http","conn":1}` (no method) | Skipped |
| P4 | Directory with `b.jsonl` (t = 17:00:02) and `a.jsonl` (t = 17:00:01), plus `c.txt` | Two records, `a`'s first; `c.txt` ignored |
| P5 | Same `t` in two files; same `t` on two lines | File order, then line order |
| P6 | Gateway camelCase `reqHeaders`/`resBody` | Read like snake_case |
| P7 | `{"json":null}` | A JSON body of null, not a null body |

**Substitution** (§4), with an HS256 secret, game-server key `gs-k`, a registered raw key `K`, fallback keys off unless
said

| # | Input | Expected |
| --- | --- | --- |
| S1 | `Bearer <token:UID-alice>` | `Bearer eyJ...`, exact; its normalization is `Bearer <jwt:UID-alice>` |
| S2 | `Bearer <token>` | Normalizes to `Bearer <jwt:UID-replay-anon>`, exact |
| S3 | `code=<key:{hex of K}>` | `code=K`, exact |
| S4 | `<key:ffff...ff>` (unknown), fallback off / on | Inexact, unchanged / exact, `RRK_replay_ffffffffffffffffffffffff` |
| S5 | `<gskey>` with / without a key | `gs-k` / inexact |
| S6 | `<auth><plain:UID-alice></auth>` | Base64 decoding to three NUL-separated parts: empty, `UID-alice`, a token normalizing to `<jwt:UID-alice>` |
| S7 | JSON `{"secret":"<redacted>"}` | Inexact |
| S8 | RS256 side (§11.2), `Bearer <token:UID-bob>` | Exact; normalizes to `Bearer <jwt:UID-bob>`; the target's game-token check accepts it |
| S9 | Signing key `not base64` | `<token:...>` stays, inexact; no error |
| S10 | `now` = 1792688400000 | `iat` 1792688400, `exp` 4102444800 |

**Request preparation** (§6.1)

| # | Input | Expected |
| --- | --- | --- |
| Q1 | POST `/account/api/oauth/token`, conn 3, seq 2, ip `10.0.0.5`, t `2026-10-22T17:00:00.000Z`, headers `x-rustic-isles-gameserver-apikey: <gskey>` and `authorization: basic <redacted>`, form `grant_type=exchange_code`, `exchange_code=<key:..>`; gateway secret `gw`; pinning on | Exact; body `grant_type=exchange_code&exchange_code=<raw key>`; headers include the game-server key, `x-forwarded-for: 10.0.0.5`, `x-gateway-secret: gw`, `x-isles-replay-now-ms: 1792688400000`, `x-isles-replay-seed: 3:2:POST:/account/api/oauth/token`; a form content type appended |
| Q2 | Same `authorization` on another path | Inexact |
| Q3 | `backend` record `/game/login?x=1` | Sent to `/login?x=1`; the seed still names `/game/login?x=1` |
| Q4 | Recorded `host`, `content-length`, `Connection` headers | Not forwarded (the transport adds its own) |
| Q5 | Form value `a b+c/é` | `a%20b%2Bc%2F%C3%A9` |
| Q6 | Pinning off | No replay headers |
| Q7 | Learned map `{"65f..01": "999..44"}`, target `/loadout/u/65f..01/all` | Sent with the learned id; seed with the recorded id |

**Normalization and comparison** (§7, §8)

| # | Left | Right | Expected |
| --- | --- | --- | --- |
| N1 | HS256 token for `UID-player1` | RS256 token for the same | Both `<jwt:UID-player1>` |
| N2 | `eyJnot_base64_json.eyJalso_bad.sig` | — | Unchanged |
| N3 | Headers `date`, `etag`, `x-powered-by`, `x-rustic-isles-request-id`, `content-type: application/json; charset=UTF-8`, `x-mystic-party-revision: 3`; JSON with a token, `createdAt` ISO, a UUID `id`, a 16-hex `instanceId`, a party id `local-<24 hex>_us-east-1`, a `gs-<uuid>` grant, `uptimeSeconds` 120, `processId` 4412 | Other `date`, a `content-length`, other request id, `charset=utf-8`, same revision; the same shapes with other values | No differences |
| N4 | Then (same normalizers) `{"primary":U1,"secondary":U1}` | `{"primary":U9,"secondary":U8}` (new UUIDs) | Exactly one `ValueDiff` at `$.secondary`, `<uuid#1>` against `<uuid#2>` |
| N5 | 200, `content-type: application/json`, `{}` | 500, `text/html`, `{}` | A `StatusDiff` 200/500 and a `HeaderDiff` on `content-type` |
| N6 | `{"alpha":1,"beta":2}` | `{"beta":2,"alpha":1}` | Exactly one `TextDiff` with those two texts |
| N7 | `{"a":1}` | `{ "a": 1 }` | A `TextDiff`, the right side being its received text |
| N8 | `{"n":1}` | `{"n":1.0}` | No `ValueDiff`; a `TextDiff` (right is non-canonical) |
| N9 | Headers `Date`, `Content-Type: Application/JSON ; Charset=UTF-8`, `X-Custom: <uuid>` | — | `date` dropped; `application/json; charset=utf-8`; `x-custom` `<uuid#1>` |
| N10 | `"2085-09-09T00:00:00.000Z"`, `"2099-01-01T00:00:00Z"` | — | Kept |
| N11 | `{"refresh_token":"abcdefghijkl"}`, `{"nonce":"x"}`, `code_challenge=` + 43 chars | — | `<opaque-token>`, `<nonce>`, `code_challenge=<challenge>` |
| N12 | `{"data":"{\"id\":\"<uuid>\"}"}` | — | The inner JSON normalized and re-stringified |

**Redaction alignment** (§7.8)

| # | Recorded | Live | Expected |
| --- | --- | --- | --- |
| R1 | `game.executableSha256`, `token_type`, `refresh_token`, `payload.huntpass_id`, `payload.bounty_token_id` all `<redacted>` | A 64-hex hash, `bearer`, an opaque string, `season43`, `TOKEN_BOUNTY_DRAFT` | No differences |
| R2 | `{"displayName":"<redacted>"}` | `{"displayName":"CanarySlayer"}` | One difference |

**Chat** (§9.4)

| # | Input | Expected |
| --- | --- | --- |
| C1 | `<iq ... id='srv-1' type='get'><ping xmlns='urn:xmpp:ping'/></iq>`; `<iq type='result' id='replay-barrier-1'/>` | Both filtered |
| C2 | `<stream:stream from='chat.steelyard.ca' id='abc-123' version='1.0'>` | `id='<stream>'` |
| C3 | Left: stream `id='s1'`, a ping, `<iq id='srv-222' type='result'/>`; right: stream `id='s2'`, `<iq id='srv-999' type='result'/>` | No differences |
| C4 | `<presence from='a'/>` against `<presence from='b'/>` on conn 9 | One `ChatStanzaDiff` conn 9, index 0 |
| C5 | Two stanzas against one | A diff at index 1 with right null |

**Ids** (§5)

| # | Input | Expected |
| --- | --- | --- |
| L1 | Recorded `{"payload":{"id":"65f1a2b3c4d5e6f7a8b9c0d1","instanceId":"aaaa1111bbbb2222"}}`, live with other values | Both pairs learned |
| L2 | Same recorded id seen twice with two live ids | The first kept |
| L3 | Hint `id`, recorded `INVALID` | Not learned |
| L4 | Map `{"ab": "X", "abc": "Y"}` on `abcd` | `Yd` (longest first) |

**Key map and CLI** (§10)

| # | Input | Expected |
| --- | --- | --- |
| K1 | `["RRK_alpha_key","UUK_beta_key"]` | Two entries keyed by their SHA-256 hex |
| K2 | `{"<64 hex upper>":"RRK_a","name":"RRK_b"}` | Lowercased hex → `RRK_a`; hex of `RRK_b` → `RRK_b` |
| K3 | `42` | `--key-map <path> must be a JSON object or array of strings` |
| K4 | `replay --corpus c.jsonl --left recorded --right http://127.0.0.1:61000 --right-director http://127.0.0.1:61001 --gs-key g --jwt-secret j --gateway-secret w --filter /character --out r.json --json` | Options: both sides' gs key and HS256 secret set, right Director set, pinning on, filter, out, JSON output |
| K5 | `--left-jwt-secret a --jwt-secret b` / `--jwt-secret b --left-jwt-secret a` | Left `b` / left `a` (CLI order); for §10.4, left `a` in both |
| K6 | Missing `--right`; `--bogus`; `--out` last | Exit 2 with the messages of §10.5 |
| K7 | `Debug` of options, side credentials, endpoints, prepared requests | No secret value, key, token or body text appears |

**Runner** (§9), with a fake target that answers fixed bytes

| # | Input | Expected |
| --- | --- | --- |
| U1 | Records with routes `content`, `network`, an aborted one, a `director` one with no right Director | All skipped and counted; no entries |
| U2 | Filter `/character`; a chat record | Chat skipped; with filter `chat` the chat kept and every HTTP record without `chat` in target or route skipped |
| U3 | A lossy record whose answers differ | `inexact_diffed`; the run passes |
| U4 | A target that closes without a head terminator | The run fails with a transport error |
| U5 | Chunked answer `5\r\nhello\r\n0\r\n\r\n` | Body `hello` |
| U6 | A record whose recorded answer has no body, left and right live and minting different ids | The right's later requests use the right's ids (cross-learning) |
| U7 | `--out` | Pretty JSON with the field order of §10.6; diffs tagged by `kind` |
| U8 | Passing / failing run through `cli` | Exit 0 / 2; text lines as §10.6 |

### 13.3 Behaviour the restricted tests fix (rewrite there against this crate)

- **Target against target**: two fresh 1.14.7 modules on SQLite with the same game data, the same throwaway key
  pair, game-server key and pinning, driven by a 34-record corpus (server status, OAuth exchange with a key map,
  `/login`, game session, characters get/put/post with a learned character id, loadouts, inventory, progression with a
  `gs-<uuid>` grant, hunt pass, escalation, bounties, balance, store SKUs and a purchase token, candidates, parties,
  four Director routes, a chat stream with SASL, roster and a ping): passes, 30 HTTP records replayed, every left
  status 200 or 204; then the same corpus through `run_replay_cli` with a key-map file and `--out` passes and writes
  the file.
- **Recorded against live, with chat**: a recorded corpus (server status with `ratelimit` headers, a Director
  readiness answer, a chat connection with SASL success) against a live module and a mock chat server that answers
  `<success .../>` to `<auth ...>` passes with 2 HTTP records and 1 chat connection.

These need the composed AGPL module and stay in restricted (§12); a failure there is a bug on this side.

---

## 14. Security behaviour (for `docs/security/rustic-isles-replay.md` and RX3)

1. **The signing key mints any player's game token.** The harness holds `AUTH_SIGNING_PRIVKEY_B64`, the game-server
   key, the gateway secret and raw account keys. None of them is ever logged, printed, written to the report or shown
   by `Debug` (decision: secrets never in `Debug`; restricted's options and credentials derive `Debug` with values,
   which the port does not keep).
2. **Secrets on the command line.** `--signing-key`, `--jwt-secret`, `--gs-key` and `--gateway-secret` (and the
   per-side forms) put secrets in the process list and shell history, against AGENTS.md's rule for host settings. The
   host kit defaults them from `secrets.env`, which is the path to document; the flags are kept for parity (§16
   question 2).
3. **Where requests go.** Only to the URLs and chat addresses the operator gives, never to an address read from the
   corpus. The transport is plain HTTP and TCP with no TLS, so targets should be loopback; the harness does not
   enforce it (§16 question 3).
4. **Captures are personal data** (gateway-front §12): the harness reads them and never writes them back; corpora and
   reports stay out of this repository (§12).
5. **The report can hold live secrets.** Normalization masks tokens, opaque refresh tokens, nonces and challenges,
   but a minted value it does not recognize (a 64-hex exchange code or key, a 48-hex account key) appears verbatim in a
   `ValueDiff` or `TextDiff` when the sides disagree, in the text output and in `--out`. The report is written with
   default permissions (§16 question 4).
6. **Replay headers.** The harness is the only intended sender of `x-isles-replay-*`; the targets accept them only
   with `RUSTIC_ISLES_REPLAY_HEADERS` on, which a production module leaves off (core §17 item 1).
7. **Untrusted input.** Corpus lines (from the host's own gateway, but files can be edited) and target answers are
   parsed without panicking: bad lines are skipped, a bad answer is a transport error. Memory is not bounded: an
   answer is read to end of stream and a corpus is loaded whole.
8. **Fuzz targets** (`fuzz/`): the capture line parser and body reader; the normalizer (`normalize_str`,
   `normalize_value`, header normalization, chat stanza normalization) and the comparison of two arbitrary answers;
   the raw HTTP answer parser with chunked decoding; the key-map loader. Each must not panic.

---

## 15. Behaviour that looks wrong but is kept (parity), and departures

Departures (decisions taken):

- **D1. No legacy names.** Restricted also drops `x-mysticparadox-request-id` from compared headers (for captures
  from before the rename) and also applies the `/game/login` rewrite to route `metagame` (a Dauntless Revived name,
  migration guide §3: `metagame` became `backend`). Neither is carried: only the names of §6.1 and §7.7.
- **D2. Secrets never in `Debug`** (§10.1, §14 item 1).

Kept as they are (each a candidate decision row for the maintainer):

1. **Stale docs.** `shadow-replay.md` says the bare `<token>` subject is `UID-replay0000000000000000000000`; the
   harness uses `UID-replay-anon`. This spec follows the harness.
2. **Stale docs.** It names capture files `capture-<ms>-<pid>.jsonl` and shows snake_case fields; the gateway writes
   `gateway-<day>.jsonl` with camelCase fields. Both field spellings are read (§3.2).
3. **Seq is global**, not per connection, so the seed `<conn>:<seq>:...` is unique per run only because the gateway's
   seq is.
4. **Two different fallbacks for an unreadable `t`**: 0 for ordering, 1793016000000 for the pinned clock.
5. **Replayed tokens expire no earlier than 2100**, whatever the record's time.
6. **The HS256 token** carries an issuer and audience no isles target accepts; with no signing key and no secret it
   signs with a fixed public default. It can only work against a non-isles target.
7. **`x-gateway-secret`** is injected, but nothing in either repository reads that header (the gateway's mark is
   `x-reforged-gateway`). Kept as dead parity (§16 question 5).
8. **`RRK_replay_<24 hex>` fallback keys** are not valid account keys (`RRK_` + 48 hex), so a target refuses them;
   against two live targets both refuse alike, so it matches; against a recorded side it differs.
9. **Learned-id replacement is plain substring and chained**: a short recorded id can hit inside an unrelated string,
   and one replacement's output can be rewritten by a later mapping.
10. **Chat entries' `index`** is the entry count, not a corpus index, so it can collide with an HTTP entry's index.
11. **The filter on chat is inverted**: chat records survive only when the filter text contains `chat`, not when a
    record matches the filter.
12. **Recorded replay headers are forwarded** as well as the harness's own (§6.1).
13. **Only the right side's Director is checked** before a `director` record; a left side without one sends it to its
    backend.
14. **The gateway never labels a record `director`** (its routes are `backend`, `content`, `ws`, `network`,
    `gateway`, `enroll`, `node`), so the Director path is reached only by hand-made corpora; `ws`, `gateway`, `enroll`
    and `node` records are replayed against the backend.
15. **Flag precedence differs** between the standalone CLI (argument order) and the host kit (per-side beats
    both-sides).
16. **`scripts/replay-capture.mjs` is stale**: it looks for `reforged-host` under a `rust/` folder and falls back to
    `cargo run --manifest-path rust/Cargo.toml -p dainty-server`, neither of which exists in dainty-lass; it forwards
    its arguments to `reforged-host db replay` and exits with the child's status (1 when it cannot start).
17. **Live chat is raw TCP with one 50 ms read per sent frame**: a server that answers in two writes, or slowly, loses
    stanzas; WebSocket framing is not spoken, so it works only against a plain XMPP-over-TCP listener. A router side
    compares chat against the recorded `out` frames rather than replaying.
18. **Nothing sends `replay-barrier-` stanzas**; the filter for them exists for test servers that echo one.
19. **Chat normalizes every ISO time**, unlike HTTP's sentinel exception.
20. **Grant ids render as `gs-<uuid#N>`** on their own counter, so a grant's N is unrelated to the UUID counter.
21. **Any transport error ends the run** without a report; there is no per-record tolerance or retry.
22. **A form answer recorded by the gateway** is rebuilt as unencoded `k=v&...`, which never equals a live encoded
    form body; such records always diff.

---

## 16. Open questions

1. **Detecting an ungated target.** With `RUSTIC_ISLES_REPLAY_HEADERS` off the run silently loses pinning. Should the
   harness check first (for example, two requests that should mint the same seeded id) and refuse or warn? Not in
   parity; proposed as a follow-up row.
2. **Secret flags.** Keep `--signing-key`, `--jwt-secret`, `--gs-key`, `--gateway-secret` (parity, already in the
   MIT clap tree), or replace them with `--*-file` paths and `secrets.env` only? Changing the flags is the clap tree's
   owner's call.
3. **Loopback only?** Should the harness refuse non-loopback targets, as `isles-db migrate` refuses remote Postgres
   without a flag?
4. **Report permissions.** Write `--out` with owner-only permissions (0600 where modes exist), as the gateway does for
   captures?
5. **`x-gateway-secret`.** Drop it, keep it, or send the gateway's real `x-reforged-gateway` mark (which changes how
   the target reads the client address)? Kept as is until decided.
6. **Hostkit wiring.** Does the integration pass make `crates/dainty-hostkit` depend on this crate (the stub becomes the real
   command and restricted's hook drops its replay arm), or keep the replay behind restricted's hook? §11.1 proposes
   the first.
7. **`replay-capture.mjs`.** Fix its paths (`target/` at the workspace root, `reforged-host` from dainty-restricted's
   `server-console`), or remove it?

---

## Coordinator decisions (2026-11-05, before implementation)

These settle §16 and override anything above that disagrees.

- **Item 1:** the harness warns (stderr, once) when a target's first answer shows it ignored the pinning; it does not
  refuse.
- **Item 2:** the flags stay as they are here (the clap tree is hostkit's); moving secrets off the command line is a
  hostkit row for the maintainer.
- **Item 3:** parity (no loopback check); candidate decision row.
- **Item 4: taken.** The `--out` report is created owner-only (0600 on Unix; default ACL on Windows).
- **Item 5:** parity (`x-gateway-secret` kept).
- **Item 6:** replay stays behind restricted's hook for now; wiring hostkit to the crate is the integration pass's.
- **Item 7:** out of this unit; `scripts/replay-capture.mjs` is a separate row.
- **§15:** parity, as listed.
