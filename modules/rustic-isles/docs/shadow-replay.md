# Shadow Replay Harness & Normalizer (`isles-db replay`)

The RU8 shadow replay harness (`isles_core::replay` / `isles_db::replay` and the `isles-db replay` CLI subcommand) replays real HTTP and XMPP chat traffic captured by the DaintyLass Rust gateway against two backend targets—or against a recorded baseline and a live backend—and verifies wire-level behavioral parity across all Rustic Isles areas (`reforged-api`, `auth`, `game-data`, `characters-loadout`, `inventory`, `progression`, `bounties-store`, `matchmaking-parties`, `chat`, and `isles-director`).

---

## 1. Recording a Capture Corpus (`CAPTURE_DIR`)

The DaintyLass gateway (`crates/gateway/src/capture.rs`) records every forwarded HTTP exchange and XMPP chat frame as newline-delimited JSON (`.jsonl`) when `CAPTURE_DIR` is set in the gateway environment:

```powershell
$env:CAPTURE_DIR = "R:\captures\session-2026-10-22"
```

### Capture Record Format

Each line in `<CAPTURE_DIR>/capture-<ms>-<pid>.jsonl` is either an HTTP record (`"kind": "http"`) or a chat frame (`"kind": "chat"`):

```json
{"t":"2026-10-22T17:00:02.000Z","kind":"http","conn":1,"seq":2,"ip":"127.0.0.1","method":"POST","host":"127.0.0.1:8421","target":"/account/api/oauth/token","route":"backend","req_headers":{"content-type":"application/x-www-form-urlencoded"},"req_body":{"form":[["grant_type","exchange_code"],["exchange_code","<key:4a8b...>"]]},"status":200,"res_headers":{"content-type":"application/json; charset=utf-8"},"res_body":{"json":{"access_token":"<token:UID-a1b2c3d4e5f6a7b8c9d0e1f2>","token_type":"bearer"}},"ms":4}
{"t":"2026-10-22T17:00:31.010Z","kind":"chat","conn":10,"dir":"in","text":"<auth xmlns='urn:ietf:params:xml:ns:xmpp-sasl' mechanism='PLAIN'><plain:UID-a1b2c3d4e5f6a7b8c9d0e1f2></auth>"}
```

The gateway automatically scrubs credentials before writing to disk while preserving stable identity correlation tokens:

| Placeholder | Meaning in Capture | Replay Substitution |
|---|---|---|
| `<token:UID-...>` | JWT whose payload had `userId` or `sub` equal to `UID-...` | Re-signed as a valid RS256 (`--signing-key`) or HS256 (`--jwt-secret`) JWT for `UID-...` at the record's timestamp |
| `<token>` | JWT without a string `userId`/`sub` claim | Re-signed for the default replay subject `UID-replay0000000000000000000000` |
| `<key:sha256>` | Raw `RRK_...` / `UUK_...` account key hashed with SHA-256 | Replaced with the raw key from `--key-map <file.json>` (or deterministic `RRK_replay_<hash[..24]>`) |
| `<gskey>` | `x-mysticparadox-gameserver-apikey` / `x-mysticparadox-gameserver-key` | Replaced with `--gs-key` (`GAMESERVER_API_KEY`) |
| `<plain:UID-...>` | Base64 SASL `PLAIN` payload `\0<account>\0<jwt>` | Re-encoded as base64 `\0UID-...\0<fresh-jwt>` |
| `<redacted>` | Irreversibly scrubbed secret (e.g., Basic client credentials or passwords) | Marked `inexact = true` unless on a known non-secret route |

---

## 2. Migrating & Verifying the Database Snapshot

Before replaying a stateful capture corpus against both Node (`left`) and Rust (`right`), restore the MongoDB snapshot taken at the start of the capture session and migrate it into the Rust `store` database (`SQLite` or `PostgreSQL`) using `isles-db migrate` and `isles-db verify`:

```powershell
# 1. Migrate the MongoDB snapshot into SQLite (or postgres://...)
cargo run -p isles-db -- migrate `
  --from "mongodb://127.0.0.1:27017/mysticparadox_snapshot" `
  --to "R:\captures\session-2026-10-22\isles.sqlite" `
  --node local `
  --game-data-dir "R:\game_data\1.14.7"

# 2. Verify row counts and canonical SHA-256 digests across all 22 collections
cargo run -p isles-db -- verify `
  --from "mongodb://127.0.0.1:27017/mysticparadox_snapshot" `
  --to "R:\captures\session-2026-10-22\isles.sqlite" `
  --node local `
  --game-data-dir "R:\game_data\1.14.7"
```

---

## 3. Running `isles-db replay`

### Target-Against-Target Replay (Node vs. Rust or Rust vs. Rust)

Start the left target (e.g., the reference Node backend on `127.0.0.1:8421` and Director on `127.0.0.1:9421`) and the right target (e.g., `isles-1-14-7` on `127.0.0.1:61000` and Director on `127.0.0.1:61001`) against their respective snapshots, then run:

```powershell
cargo run -p isles-db -- replay `
  --corpus "R:\captures\session-2026-10-22" `
  --left "http://127.0.0.1:8421" `
  --left-director "http://127.0.0.1:9421" `
  --right "http://127.0.0.1:61000" `
  --right-director "http://127.0.0.1:61001" `
  --signing-key $env:AUTH_SIGNING_PRIVKEY_B64 `
  --gs-key $env:GAMESERVER_API_KEY `
  --key-map "R:\captures\session-2026-10-22\keymap.json" `
  --out "R:\captures\session-2026-10-22\replay-report.json"
```

### Recorded Corpus vs. Live Rust Target

You can also replay a recorded `.jsonl` corpus directly against a single live Rust target without running a second server by passing `--left recorded`:

```powershell
cargo run -p isles-db -- replay `
  --corpus "R:\captures\session-2026-10-22\capture.jsonl" `
  --left recorded `
  --right "http://127.0.0.1:61000" `
  --right-director "http://127.0.0.1:61001" `
  --signing-key $env:AUTH_SIGNING_PRIVKEY_B64 `
  --gs-key $env:GAMESERVER_API_KEY `
  --key-map "R:\captures\session-2026-10-22\keymap.json"
```

### Key Map File Format (`--key-map`)

The `--key-map` JSON file can be either a JSON array of raw `RRK_...` / `UUK_...` account keys (the harness hashes each key with SHA-256 automatically) or an explicit `{"<sha256-hex>": "RRK_..."}` map:

```json
[
  "RRK_player1_account_key_0123456789abcdef",
  "UUK_player2_account_key_fedcba9876543210"
]
```

### Clock & Seed Pinning (`--pin-clock`)

When `--pin-clock` is enabled (the default), the replay harness sends two headers on every HTTP request to live targets:

- `x-isles-replay-now-ms`: the capture record's timestamp (`t`) in epoch milliseconds, pinning `season_window::now_ms()` and all area clocks (`auth`, `reforged-api`, `inventory`, `progression`, `bounties-store`, `characters-loadout`, `matchmaking-parties`, and `chat`).
- `x-isles-replay-seed`: a deterministic seed (`<conn>:<seq>:<method>:<target>`) that pins generated character IDs, starter/minted item `instanceId`s, progression `grantId`s, and party IDs inside `ReplayScope`.

In addition, when a response creates a new character or item instance, `learn_ids` records the mapping from the corpus's recorded ID to each live target's actual ID and rewrites subsequent request URLs, headers, and bodies automatically.

---

## 4. Normalizer Rules & Diff Classifications

Before comparing responses from `left` and `right`, the normalizer applies the following rules:

1. **JWT Verification & Canonicalization**: Every `eyJ...` token in response bodies or headers is base64url-decoded and verified to have a valid JSON header (`"alg"`) and JSON payload. Valid tokens are replaced with `<jwt:<userId>>` (or `<jwt>` when no subject claim is present). Malformed tokens are left untouched so structural corruption is caught immediately.
2. **Opaque Tokens & Nonces**: `refresh_token`, `refreshToken`, and `challengeSignature` fields are normalized to `<opaque-token>`, `nonce` to `<nonce>`, and PKCE `code_challenge=...` query parameters to `code_challenge=<challenge>`.
3. **Volatile HTTP Headers**: Transport headers (`date`, `etag`, `x-powered-by`, `connection`, `keep-alive`, `content-length`, `transfer-encoding`, `x-mysticparadox-request-id`, `x-request-id`, `server`, `vary`, and CORS headers) are stripped, while `content-type` (canonicalized) and application headers (e.g., `x-mystic-party-revision`) are compared.
4. **Timestamps & Dynamic IDs**: Non-sentinel ISO-8601 timestamps (`<date>`), login timestamps (`<login-time>`), display dates (`<display-date>`), `uptimeSeconds` / `processId`, and dynamically generated UUIDs (`<uuid#N>`), 16-hex instance IDs (`<iid#N>`), party IDs (`<party#N>_<region>`), and progression grant IDs (`gs-<uuid#N>`) are numbered by first appearance per side so co-reference structure is strictly preserved.
5. **XMPP Chat Stanzas**: Server stream IDs (`id='<stream>'`) and server-generated stanza IDs (`id='srv-<id>'`) are normalized, and asynchronous server keepalive pings (`<ping xmlns='urn:xmpp:ping'/>`) and replay barriers are filtered out before comparing outbound stanza sequences.

### Diff Kinds

Every discrepancy is classified into one of five [`DiffKind`] variants:

- `StatusDiff { left, right }`: HTTP status code mismatch.
- `HeaderDiff { header, left, right }`: Normalized HTTP response header mismatch.
- `ValueDiff { path, left, right }`: Semantic JSON value mismatch at JSONPath `path`.
- `TextDiff { left, right }`: Wire-text or JSON object key-ordering mismatch when semantic JSON values match (or on non-JSON bodies).
- `ChatStanzaDiff { conn, index, left, right }`: Outbound XMPP stanza mismatch on connection `conn`.

### Exit Codes

- `0`: Replay completed with `unexplained_diffed == 0`.
- `1`: Runtime or I/O error loading the corpus or connecting to a target.
- `2`: Invalid CLI arguments or one or more unexplained diffs (`unexplained_diffed > 0`).
