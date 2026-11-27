<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# The canary week, the traffic capture and the shadow replay

The maintainer's hands-on steps for two rows of [`MIGRATION_PLAN.md`](../MIGRATION_PLAN.md)'s "What's left":

- **RU8**: record a live tester corpus with the gateway's traffic capture (RD19) and replay it against a second server
  with `dainty-host db replay`.
- **RU9**: the canary week (RD13): a server on the Linux kit with a copy of a real database and invited testers, then
  each host through `update`.

Nothing here is an agent's job (RD18). The default was switched to the Rust services in code on 2026-10-28 without
this week (RD2); these steps are what checks it in the field. There is no Node and no MongoDB left to compare against
(RB15): the baseline is what the canary itself recorded.

## 1. Turn on the traffic capture

The gateway records every request, every answer and every chat message that passes through it, redacted, as JSON
lines (`crates/dainty-gateway/src/capture.rs`). It is off unless `CAPTURE_DIR` is set.

| Variable | Meaning | Default and range |
| --- | --- | --- |
| `CAPTURE_DIR` | The folder for the daily files. Turns the capture on | unset (off) |
| `CAPTURE_RETENTION_DAYS` | Days a daily file is kept before the gateway deletes it | 14, from 1 to 90 |
| `CAPTURE_MAX_BODY_BYTES` | Bytes of each body kept; a longer one is recorded as omitted, with its length | 262144, from 0 to 16777216 |

These are not `settings.toml` fields, and `setup` writes none of them into `gateway.env`. The gateway reads them from
its environment, and `dainty-host` passes the environment it was started in through to the gateway. So set them where
the server is started:

```powershell
# Windows, or a test run in a terminal
$env:CAPTURE_DIR = "C:\dainty\traffic"   # an absolute path, outside the checkout
bin\dainty-host serve
```

```sh
# The Linux kit's systemd unit: a drop-in, so a later setup that rewrites the unit keeps it
sudo systemctl edit dainty-lass.service
#   [Service]
#   Environment=CAPTURE_DIR=/srv/dainty/traffic
sudo systemctl restart dainty-lass.service
```

The gateway logs `traffic capture is on: ...` with the folder and the retention when it starts. If it can't make the
folder it refuses to start, and after a write error it stops capturing and logs `traffic capture stopped`.

> **Maintainer:** whether a capture setting should become a `settings.toml` field (AGENTS.md's rule for new settings)
> is open. Until then, remove the drop-in or the variable after the week.

### What a file holds

One file per UTC day, `gateway-<YYYY-MM-DD>.jsonl`, created owner-only on Unix (the folder 0700, the files 0600).
Each line is one record; `t` (ISO time) and `src` (`gateway`) come first.

- `kind: "http"`: the connection and sequence numbers (`conn`, `seq`), the client's `ip`, `method`, `target`, the
  gateway's `route`, the request's and the answer's headers and bodies (`reqHeaders`, `reqBody`, `resHeaders`,
  `resBody`), `status`, `ms`, and `aborted` when the client left before the answer ended.
- `kind: "chat"`: a chat tunnel's `open` (with `ip` and `target`), each message with its direction `dir` (`in` or
  `out`) and its `text` (or `omitted` / `compressed` with a length), and its `close`.

A body is recorded as `json`, `form`, `text`, `binary` (its length only) or `omitted` (over the cap).

**What is redacted before a line is written:** cookies and the gateway's own header are dropped; the
`authorization` header and secret-named headers and fields keep only a placeholder; JSON web tokens become
`<token:...>`, account keys `<key:<sha256 of the key>>`, the game server key `<gskey>`, a chat sign-in's account
`<plain:<account>>` and its password `<redacted>`; long hex and base64 runs become `<redacted>`; and the gateway's
and the allowlist's secrets are replaced wherever they appear. The placeholders carry no secret, only what the
replay needs to put working credentials back.

**What is not redacted:** client IP addresses, player names, chat text and game state. The capture is personal data.
Keep the folder on the canary, readable by the server's user only, never commit it or post it, and delete it when the
replay is done. `docs/security/gateway.md` is the checklist.

## 2. Replay a capture against a second server

`dainty-host db replay` (`crates/dainty-host/src/lib.rs`, on `crates/rustic-isles-replay`) reads the capture and sends
every recorded request again, with working credentials, to a server, then compares the answers.

- **The left side** is the baseline: `recorded` (the default), the answers in the capture itself, or a backend URL.
- **The right side** is the server under test: a backend URL, `http://127.0.0.1:41000` by default.

It talks to the backend directly, not through the gateway, so it sends the gateway secret the way the gateway does.

### 2.1 Prepare the target server

1. **The same state.** Take a backup of the canary's store just before the capture starts (`dainty-host backup`
   writes it into `.workspace/backups/`), and restore it on the target, so the replayed requests meet the data the
   recorded ones met.
2. **The same game data and version data.** Copy the canary's `modules/rustic-isles/game-data/1.14.7` (or a pack from
   `dainty-host gamedata --export`) and use the same release.
3. **The same secrets, or name them.** By default the replay reads `AUTH_SIGNING_PRIVKEY_B64`, `JWT_SECRET`,
   `GAMESERVER_API_KEY` and `GATEWAY_SECRET` from `.workspace/config/secrets.env` of the folder it runs in. A target
   with other secrets takes `--right-signing-key`, `--right-jwt-secret`, `--right-gs-key` and `--gateway-secret` (and
   the `--left-...` ones when the left side is a live server too).
4. **Pinned clock and ids.** The replay sends `x-isles-replay-now-ms` and `x-isles-replay-seed` with each request, so
   the target answers with the recorded time and makes the same ids. The backend honours them only with
   `RUSTIC_ISLES_REPLAY_HEADERS=on` in its environment. Set it on the target only, never on the canary or a server
   players use. `--no-pin-clock` sends no such headers. When the target ignores them, the replay warns once.
5. **Account keys.** The capture holds each key only as `<key:sha256>`. For a request that needs the real key, give a
   `--key-map` file: a JSON array of raw keys, or an object mapping the SHA-256 (or any name) to the raw key. Without
   it those requests are sent with what is left and count as inexact. The key map is the testers' keys: keep it with
   the capture and delete it with it.

```powershell
bin\dainty-host db replay --corpus C:\dainty\traffic --right http://127.0.0.1:41000 `
  --right-director http://127.0.0.1:41001 --key-map C:\dainty\keys.json --out C:\dainty\replay.json
```

| Option | Meaning |
| --- | --- |
| `--corpus <path>` | A capture file, or a folder of `*.jsonl` files |
| `--left <url\|recorded>`, `--right <url>` | The two sides (above) |
| `--left-director`, `--right-director <url>` | The Director's URL; without the right one, Director records are skipped |
| `--left-chat`, `--right-chat <host:port>` | A plain XMPP-over-TCP address to replay the chat connections against |
| `--filter <text>` | Only the HTTP records whose target or route contains it |
| `--out <file>` | Writes the full JSON report there |
| `--json` | Prints the JSON report instead of the summary |

> **Maintainer:** confirm which address `--right-chat` takes on a composed server (the chat service listens on 41006
> behind the gateway); the code describes it only as plain XMPP over TCP.

`scripts/replay-capture.mjs` is a wrapper that passes its arguments to `dainty-host db replay`. As it stands it looks
for `dainty-host` in the repository root or under `rust/target/`, and otherwise falls back to `cargo run` on a
`rust/Cargo.toml` and a `dainty-server` package that this layout doesn't have; its help also gives
`http://127.0.0.1:61000` as the right side, the private gateway's port, where the command's default is the backend's
41000. Run `dainty-host db replay` directly until the script is brought up to date.

### 2.2 Read the report

The summary's first line is `dainty-host db replay: PASS` or `DIFF`, with the counts:

| Count | Meaning |
| --- | --- |
| `records` | Lines read, skipped ones included |
| `http`, `chat_conns` | HTTP records and chat connections replayed |
| `skipped` | Left out by `--filter` or the replay's route rules (Director records without `--right-director`, for one) |
| `exact_matched` | No difference, and the request was sent exactly as recorded |
| `inexact_matched` | No difference, but the request could not be rebuilt exactly |
| `inexact_diffed` | A difference, on a request that could not be rebuilt exactly: look, but it may be the request's fault |
| `unexplained_diffed` | A difference on an exact request. Any of these fails the run |

A request is inexact when the client aborted it, a body was cut by `CAPTURE_MAX_BODY_BYTES`, or a placeholder could
not be filled (a key missing from the key map). The second line counts the differences by kind: `status`, `header`,
`value` (the JSON differs at a path), `text` (the JSON is equal in value but not byte for byte, or a non-JSON body
differs) and `chat_stanza`. Then each entry with a difference, `[DIFF]` or `[INEXACT]`, with its method, target,
connection, sequence and route, and each difference under it.

Before comparing, the replay normalises what legitimately differs: the ids each side mints (it learns the mapping
from the answers), tokens, and the headers that always change. A `text` difference matters: the game parses the JSON
text, so a reordered field or a number written differently is a real difference (MIGRATION_PLAN section 3).

The command exits 0 on PASS, 2 when anything is unexplained, and 1 when the run itself failed.

## 3. The canary week (RU9)

1. **Set up the canary** on the Linux kit, from the release under test, on a clean Debian 13 (`hosting.md`). Give it
   a copy of a real server's store: the SQLite file (`.workspace/config/dainty-lass.db`) or a Postgres dump, taken
   from a backup.
2. **Turn on the capture** (section 1) before the first tester joins.
3. **Invite the testers** (`dainty-host invite`). Over the week, the corpus should cover login, Ramsgate, a hunt, chat
   in the City room, Lady Luck's Store and the platinum store, loadouts, bounties, the launcher's API and the
   Director's game-server calls. Ask testers for a runtime download and a ban and unban too.
4. **Run the front-door check** with the Tauri launcher against the canary's gateway and content server: register
   with an invite, the status page, the runtime download.
5. **Run the smoke list** (MIGRATION_PLAN section 3) at the start and after every change to the canary: register with
   an invite, login, Ramsgate, a hunt, chat in the City room, runtime download, ban, clean stop.
6. **Replay** the week's capture (section 2) against a second server restored from the backup taken at the start.
   RU8 is done when `unexplained_diffed=0` on a corpus that covers step 3's list.
7. **Move each host** with `dainty-host update`. After each move, run the smoke list, and replay the RU8 corpus
   against the moved server's copy: it must answer as it did before.
8. **Turn the capture off** and delete the capture folder, the key map and the reports, which hold personal data.

> **Maintainer:** MIGRATION_PLAN's RU9 row still mentions a host on MongoDB importing first with
> `dainty-legacy import reforged` (`db migrate`, `db verify`). Whether any host still needs that path is your call;
> it is left out of the steps above.

Write down for the plan: the release and commit, the dates, the counts from each replay, every unexplained
difference and what was done about it, and anything a tester reported. That record, not this page, closes RU8 and RU9.
