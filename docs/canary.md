# Canary Rollout, Shadow Replay, and Rollback Runbook (`RU9`, `RB2`, `RB12`)

This runbook documents the production shadow-replay workflow, the rollback switch to the pre-cutover Node/MongoDB stack, and the 7-day canary gate required before removing the Node reference trees (`RB15`).

---

## 1. Turning On Gateway Traffic Capture (`RD19`, `RB2`)

The Rust gateway (`rust/services/gateway`) includes a built-in traffic capture tap (`gateway::capture::CaptureWriter`). It is **off by default** and activates only when `CAPTURE_DIR` is set in `.workspace/config/gateway.env`.

### 1.1 Configuration in `.workspace/config/gateway.env`

Add the following settings to `.workspace/config/gateway.env` on the server whose traffic you want to record:

```dotenv
# Absolute path to the directory where daily capture JSONL files are written.
# Created with mode 0700 (files 0600 on Unix).
CAPTURE_DIR="/opt/reforged/.workspace/capture"

# Number of days to keep daily capture files before automatic pruning (1..90, default: 14).
CAPTURE_RETENTION_DAYS=14

# Maximum request or response body size (in bytes) recorded inline before omission (0..16777216, default: 262144).
CAPTURE_MAX_BODY_BYTES=262144
```

Restart the server (`reforged-host start`, `reforged-host serve`, or `systemctl restart dainty-lass.service`). On startup, the gateway logs:

```text
traffic capture is on: every request and answer is recorded, redacted, for the shadow replay (personal data; see docs/security/gateway.md)
```

### 1.2 Output Format and Secret Redaction

The gateway writes one file per UTC day named `gateway-YYYY-MM-DD.jsonl` inside `CAPTURE_DIR`. Each line is a single JSON object with `"kind": "http"` (an HTTP request/response pair) or `"kind": "chat"` (an XMPP WebSocket lifecycle event or frame):

- **Session tokens (`JWT`)**: Never written in cleartext. Any JWT (`eyJ...`) in headers, bodies, or query strings is replaced with `<token:UID-...>` (preserving only the unverified `userId` / `sub` claim so the replay runner can mint a fresh token for the target server) or `<token>` when no subject claim is present.
- **Launcher account keys (`RRK_...` / `UUK_...`)**: Replaced with `<key:<sha256-hex>>` (the lowercase 64-character SHA-256 digest of the raw key).
- **Game-server API keys (`x-rustic-isles-gameserver-apikey`)**: Replaced with `<gskey>`.
- **XMPP SASL `PLAIN` frames (`<auth ...>...</auth>`)**: Replaced with `<plain:UID-...>` so the replay runner can synthesize a valid base64 `\0<userId>\0<fresh-jwt>` frame for the target server.
- **Other secret-named fields and long hex/token strings**: Any JSON or form field whose name matches `(?i)(key|token|secret|pass|auth|jwt|sig|cred|recovery|invite|cookie)` (unless already converted to `<token:...>` or `<key:...>`) and any 40+ character hex string or 100+ character token string is replaced with `<redacted>`.
- **Cookies and internal headers**: `cookie`, `set-cookie`, and `x-reforged-gateway` headers are stripped before writing.

> **Privacy note:** Capture files still contain player usernames, account IDs, IP addresses, and play history. Keep `CAPTURE_DIR` on local encrypted storage with `0600` permissions, never commit real capture files to Git, and delete them once replay verification completes.

---

## 2. Replaying a Capture Against a Second Server (`RU8`, `RB2`)

Shadow replay is built into `reforged-host db replay` (backed by `isles_db::run_replay_cli` and `isles_core::replay`) with a convenience wrapper at `scripts/replay-capture.mjs`.

### 2.1 Preparing the Target Server (`--right`)

1. Start from a database snapshot (`dainty-lass.db` or PostgreSQL dump) taken **before** the captured session began so account state, character `updateVersion`, inventory instances, and progression tracks start from the same baseline.
2. If the captured session includes `/account/api/oauth/token` (`grant_type=exchange_code`), `/reforged/api/ServerStatus`, or `/reforged/api/GetUserInfo` calls authenticated with a launcher account key (`RRK_...` or `UUK_...`), provide a `--key-map <keymap.json>` file so `<key:<sha256>>` placeholders resolve to the raw key registered on the target server:
   - Either a JSON array of raw keys (the runner computes each key's SHA-256 automatically):
     ```json
     [
       "RRK_51e9d7eb..."
     ]
     ```
   - Or a JSON object mapping lowercase SHA-256 hex digests to raw keys:
     ```json
     {
       "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef": "RRK_51e9d7eb..."
     }
     ```
3. Run `reforged-host db replay` (or `node scripts/replay-capture.mjs`) from the target server's directory (so it automatically reads `AUTH_SIGNING_PRIVKEY_B64`, `JWT_SECRET`, `GAMESERVER_API_KEY`, and `GATEWAY_SECRET` from `.workspace/config/secrets.env`), or pass the credentials explicitly on the command line:

```bash
# Replay recorded gateway capture against a running local target server on port 61000:
reforged-host db replay \
  --corpus /path/to/capture \
  --left recorded \
  --right http://127.0.0.1:61000 \
  --right-director http://127.0.0.1:61001 \
  --right-chat 127.0.0.1:61006 \
  --key-map /path/to/keymap.json \
  --out /path/to/replay-report.json

# Or via the Node wrapper script:
node scripts/replay-capture.mjs \
  --corpus /path/to/capture \
  --left recorded \
  --right http://127.0.0.1:61000 \
  --key-map /path/to/keymap.json \
  --out /path/to/replay-report.json
```

You can also replay a capture **live against two servers simultaneously** (for example, the legacy Node backend on `--left http://127.0.0.1:62000` and the Rust backend on `--right http://127.0.0.1:61000`) to compare their live responses side-by-side.

### 2.2 Reading the Diff Report

`reforged-host db replay` prints a summary line and exits with code `0` when `unexplained_diffed == 0` (`PASS`) or code `2` when any unexplained diff is found (`DIFF`):

```text
reforged-host db replay: PASS (records=32, http=25, chat_conns=1, skipped=0, exact_matched=26, inexact_matched=0, inexact_diffed=0, unexplained_diffed=0)
  diffs by kind: status=0, header=0, value=0, text=0, chat_stanza=0
```

Report counters and diff classifications in `--out <replay-report.json>`:

| Field | Meaning |
| --- | --- |
| `total_records` | Total JSONL records loaded across all `.jsonl` files in `--corpus` (sorted chronologically by `t`). |
| `http_replayed` | Number of HTTP requests replayed against `--left` and `--right`. |
| `chat_connections_replayed` | Number of XMPP WebSocket sessions (`conn`) replayed and compared stanza-for-stanza. |
| `skipped_records` | Records skipped because of `--filter`, `aborted: true`, or gateway-internal routes (`content`, `network`). |
| `exact_matched` | Entries where all placeholders resolved deterministically and normalized responses matched with zero diffs. |
| `inexact_matched` | Entries marked `inexact` (e.g. truncated/omitted body or unmapped `<key:...>` without `--key-map`) that still matched. |
| `inexact_diffed` | Entries marked `inexact` whose responses differed (expected when an unmapped key or omitted body prevents exact replay). |
| `unexplained_diffed` | **Release-blocking counter:** exact entries whose normalized responses differed. Must be `0` for `PASS`. |
| `StatusDiff` | HTTP status code mismatch (`left` vs `right`, e.g. `200` vs `500`). |
| `HeaderDiff` | Normalized response header mismatch (`content-type`, `ratelimit`, custom `x-reforged-*` / `x-mystic-*` headers). Volatile transport headers (`date`, `etag`, `content-length`, `connection`, `transfer-encoding`, `server`, CORS) are stripped automatically. |
| `ValueDiff` | JSON value mismatch at a specific JSONPath (`$.payload.loadouts[0]...`) after normalizing timestamps (`<date>`, `<login-time>`, `<display-date>`), JWTs (`<jwt:UID-...>`), gateway-redacted secret fields (`<redacted>`), and first-appearance dynamic IDs (`<uuid#1>`, `<iid#1>`, `<party#1>`, `gs-<uuid#1>`). |
| `TextDiff` | Raw wire serialization mismatch (such as JSON object key ordering or number formatting) when semantic values match, or non-JSON body text mismatch. |
| `ChatStanzaDiff` | XMPP server stanza mismatch on connection `conn` at stanza index `index` after normalizing stream IDs (`<stream>`), server stanza IDs (`srv-<id>`), and timestamps, and filtering keepalive pings. |

---

## 3. Rollback Switch to the Pre-Cutover Node + MongoDB Stack (`RB12`)

Until `RB15` removes the Node reference trees after the 7-day canary window, operators can roll back to the legacy Node backend + Director + MongoDB stack without losing player progress earned on the Rust server.

### 3.1 Step 1 — Export SQLite / PostgreSQL Player Data Back to MongoDB

Before starting the Node services, export the current SQLite (`dainty-lass.db`) or PostgreSQL store back into the MongoDB `reforged` database using `reforged-host db export-mongo`:

```bash
# Dry-run first to inspect collection document counts:
reforged-host db export-mongo \
  --from /opt/reforged/.workspace/config/dainty-lass.db \
  --to mongodb://127.0.0.1:27017/reforged \
  --dry-run

# Perform the live export into MongoDB:
reforged-host db export-mongo \
  --from /opt/reforged/.workspace/config/dainty-lass.db \
  --to mongodb://127.0.0.1:27017/reforged
```

`db export-mongo` reverses the `RB11` schema transforms (`accounts`, `launcher_accounts`, `api_keys`, `characters`, `loadouts`, `inventories`, `progressions`, `huntpasses`, `escalations`, `bounties`, `quests`, `Guilds`, `moderation`, `invites`, `friendships`, `social_blocks`) idempotently into MongoDB.

### 3.2 Step 2A — Docker Rollback (`--profile rollback`)

In `docker/compose.yml`, the default stack runs `dainty-lass` (`server-minimal`) on SQLite with zero MongoDB or Node containers. To switch to the rollback stack:

```bash
# 1. Stop the default Rust stack:
docker compose -f docker/compose.yml down

# 2. Start the rollback profile (mongo + mongo-init + Node backend + Node director + gateway + content + allowlist):
docker compose -f docker/compose.yml --profile rollback up -d
```

### 3.3 Step 2B — Native Windows / Linux Rollback (`--features paradox`)

On a native host installation:

1. Build `reforged-host` with the `paradox` cargo feature enabled:
   ```bash
   cd rust
   cargo build --release -p host -p server-console --features paradox --locked
   ```
2. Ensure MongoDB (`mongod`) and Node.js (`>= 20.19`) are installed (`reforged-host doctor` checks both when built with `--features paradox`).
3. Run `reforged-host setup` and `reforged-host start` (or `reforged-host serve`). When compiled with `--features paradox`, `hostkit::serve::compose` supervises `mongod`, the Node `ParadoxBackend` (`build/server.js`), and the Node `ParadoxDeployServer` (`dist/server.js`) in place of the in-process Rust `isles-1-14-7` module.

---

## 4. The 7-Day Canary Gate (`RU9`, `RB12`, `RB15`)

Per `docs/dainty-lass-plan.md` (`RU9`) and `MIGRATION_PLAN.md` (`RB12`, `RB15`), the maintainer's canary gate requires:

1. **Canary Deployment**: Run the default Rust `1.14.7` stack (`isles-1-14-7` + `isles-director` + `store` + `gateway` + `chat`) on the maintainer's live canary server for **7 consecutive days** with real players and `CAPTURE_DIR` enabled.
2. **Pass Criteria During the 7-Day Window**:
   - Zero unhandled panics in server logs (`server.log`).
   - Zero account, character, inventory, loadout, progression, quest, or guild data corruption incidents.
   - Shadow replay (`reforged-host db replay`) of the captured daily `.jsonl` files reports `unexplained_diffed: 0` (`PASS`).
   - No rollback (`--features paradox` / `--profile rollback`) triggered during the 7-day window (if a fix requires restarting or rolling back, the 7-day clock resets once the fix is deployed).
3. **Post-Canary Cleanup (`RB15`)**: Once the 7-day canary gate passes, delete the Node reference trees (`modules/rustic-isles/reference/1.14.7/{ParadoxBackend,ParadoxDeployServer}`, `services/{gateway,content,allowlist}`, `packages/shared`, `host/`) and remove the `--features paradox` cargo feature and `--profile rollback` Docker services.

---

## 5. End-to-End Synthetic Capture and Shadow Replay Proof

The end-to-end pipeline (live `Server A` with `CAPTURE_DIR` enabled in `.workspace/config/gateway.env` -> synthetic client session over the gateway covering 25 HTTP endpoints and XMPP WebSocket SASL `PLAIN` login -> redaction verification on `gateway-YYYY-MM-DD.jsonl` -> `node scripts/replay-capture.mjs` against a second live `Server B` cloned from the pre-session database snapshot) was executed and verified on `2026-11-01`.

### 5.1 Captured Routes Exercised Through the Gateway

1. `GET /reforged/api/RegistrationStatus`
2. `GET /reforged/api/ServerStatus` (with `x-reforged-user-api-key: RRK_...` -> redacted to `<key:sha256>`)
3. `GET /reforged/api/GetUserInfo` (with `x-reforged-user-api-key: RRK_...` -> redacted to `<key:sha256>`)
4. `POST /account/api/oauth/token` (`grant_type=exchange_code&exchange_code=RRK_...` -> redacted to `<key:sha256>`, response `access_token` redacted to `<token:52c7ed2e-99e4-463c-8d08-7f509bc9de02>`)
5. `POST /login` (with `Authorization: Bearer <jwt>` -> redacted to `Bearer <token:52c7ed2e-99e4-463c-8d08-7f509bc9de02>`)
6. `GET /accountinfo`
7. `GET /tags`
8. `PUT /gamesession/epic`
9. `GET /character?userId=<userId>&version=1` (auto-provisions starter character + starter inventory/loadout instances)
10. `POST /character` (updates character `data` and increments `updateVersion` from `0` to `1`)
11. `GET /loadout/<userId>/<characterId>/all`
12. `POST /loadout/<userId>/<characterId>/0`
13. `POST /loadout/<userId>/<characterId>/active/0`
14. `GET /inventory/<userId>/<characterId>`
15. `GET /progression/<userId>`
16. `GET /huntpass/<userId>`
17. `GET /escalation/Escalation_Season_00/<userId>`
18. `GET /bounty/game-data`
19. `GET /bounty/<userId>`
20. `GET /balance`
21. `POST /candidate/player/register`
22. `POST /candidate/player/alive`
23. `GET /candidate/regions`
24. `POST /party` (creates party with dynamic `partyId`, learned and mapped by `learn_ids`)
25. `GET /party/invites`
26. `WS ws://127.0.0.1:61004/` (`Sec-WebSocket-Protocol: xmpp` -> `<open .../>`, `<auth xmlns="urn:ietf:params:xml:ns:xmpp-sasl" mechanism="PLAIN">...</auth>` redacted to `<plain:52c7ed2e-99e4-463c-8d08-7f509bc9de02>`, `<success xmlns="urn:ietf:params:xml:ns:xmpp-sasl"/>`, clean close)

### 5.2 Recorded Proof Output

```text
=== 1. Running reforged-host setup in server-a ===
=== 2. Starting server-a to seed CanarySlayer account ===
Seeded CanarySlayer: {
  userId: '52c7ed2e-99e4-463c-8d08-7f509bc9de02',
  uukPrefix: 'RRK_51e9d7eb...'
}
=== 3. Cloning pre-session snapshot to server-b and writing keymap.json ===
=== 4. Enabling CAPTURE_DIR on server-a and running synthetic session ===
=== 5. Verifying capture file and secret redaction ===
Captured files: [ 'gateway-2026-11-01.jsonl' ]
Captured JSONL lines: 32
Redaction verified: <key:...>, <token:...>, <plain:...> present: { hasKeyTag: true, hasTokenTag: true, hasPlainTag: true }
=== 6. Starting server-b and running shadow replay ===
Replay stdout:
reforged-host db replay: PASS (records=32, http=25, chat_conns=1, skipped=0, exact_matched=26, inexact_matched=0, inexact_diffed=0, unexplained_diffed=0)
  diffs by kind: status=0, header=0, value=0, text=0, chat_stanza=0

Replay exit code: 0
```
