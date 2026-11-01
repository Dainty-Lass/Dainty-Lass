# Wiring the dashboard and the pages into a Dainty Lass server

What the dashboard needs from the server, the API the server's backend uses to send it chat and reports,
and where the launcher and server manager pages plug into the Tauri shells.

## Environment

All of the dashboard's environment is in [`apps/dashboard/.env.example`](../apps/dashboard/.env.example).
Everything a person tunes lives in the database instead (Settings in the dashboard).

| Variable                                           | From                                                   | What                                                             |
| -------------------------------------------------- | ------------------------------------------------------ | ---------------------------------------------------------------- |
| `DASHBOARD_ADMIN_API_KEY`                          | `secrets.env`: `REFORGED_ADMIN_API_KEY`                | Bearer key for the admin service's GraphQL (`127.0.0.1:61003`)   |
| `DASHBOARD_MANAGER_OPERATOR_SECRET`                | `secrets.env`: `MYSTICPARADOX_MANAGER_OPERATOR_SECRET` | Signs calls to the backend's `/admin/v1` as the manager operator |
| `DASHBOARD_ADMIN_API_URL`, `DASHBOARD_BACKEND_URL` |                                                        | Defaults are the host kit's loopback ports                       |
| `DASHBOARD_INGEST_KEY`                             | new: 32+ random characters                             | What the backend sends with chat and reports                     |
| `DASHBOARD_DISCORD_CLIENT_ID`, `_SECRET`           | a Discord application                                  | Sign-in ([access.md](access.md))                                 |
| `DASHBOARD_DISCORD_BOT_TOKEN`                      | that application's bot, with Manage Roles              | Adds the approved role when someone accepts the terms            |
| `DASHBOARD_TWITCH_CLIENT_ID`, `_SECRET`            | a Twitch application                                   | Sign-in                                                          |
| `DASHBOARD_SMTP_URL`, `DASHBOARD_MAIL_FROM`        | any SMTP relay                                         | Email sign-in codes                                              |
| `DASHBOARD_AUTH_SECRET`                            | new: 32+ random characters, or generated               | Signs sessions, encrypts stored provider tokens                  |
| `DASHBOARD_OWNER_DISCORD_IDS`, `_EMAILS`           |                                                        | Who is an owner at sign-in                                       |
| `DASHBOARD_BASE_URL`                               |                                                        | The public origin, for the OAuth redirect and Secure cookies     |

The backend must run with `MYSTICPARADOX_MANAGER_ADMIN=on` for the signed calls to be accepted. The host
kit's `setup` already writes it, with the secret, for the admin service (dainty-lass `docs/admin-api.md`).

Every staff action reaches the game as that staff member: the admin API gets their Discord id in
`x-reforged-operator-id`, and the backend's signed calls carry it in `X-Paradox-Operator-Id`, so the
backend's own audit trail names them as `manager:<discord id>` alongside the dashboard's. A member with
no Discord linked is sent as `dashboard-<member id>`.

## Ingest API

The backend sends chat and reports to the dashboard and fetches the chat filter from it. Every request
carries `Authorization: Bearer <DASHBOARD_INGEST_KEY>`; a missing or wrong key is a 401.

### `POST /api/ingest/chat`

Up to 500 lines a batch. A line whose `id` is already stored is skipped, so a batch can be retried.

```json
{
  "messages": [
    {
      "id": "stanza-id",
      "at": 1793500000000,
      "channel": "city",
      "roomId": "ramsgate-1",
      "sender": { "userId": "acct-id", "name": "Moonwick" },
      "recipient": null,
      "body": "anyone up for heroic shrike?",
      "verdict": "allowed"
    }
  ]
}
```

`channel` is `city`, `party`, `guild`, `direct` or `system`; `recipient` is for direct messages; `at` is
milliseconds or an ISO string. `verdict` is what the backend did with the line (`allowed`, `flagged`,
`masked`, `blocked`); without it the dashboard rates the line itself. Send the original text, not the
masked one: moderators need to read what was said. The answer is
`{ "stored": n, "duplicates": n, "rejected": n }`.

### `POST /api/ingest/reports`

One report. A report whose `id` was already filed answers 200 with `"duplicate": true` and the first
report's id; a new one answers 201.

```json
{
  "id": "report-id",
  "source": "game",
  "reporter": { "userId": "acct-id", "name": "Moonwick" },
  "target": { "userId": "acct-id", "name": "Grevel" },
  "category": "harassment",
  "description": "What the player wrote",
  "context": { "roomId": "ramsgate-1", "instanceId": "hub-1", "at": 1793500000000 }
}
```

`category` is one of `harassment`, `hate_speech`, `cheating`, `griefing`, `spam`, `inappropriate_name`,
`real_money_trading`, `other`. `context.at` is when it happened; the report page shows the room's chat
around it. A Discord bot posts the same shape with `"source": "discord"`.

### `GET /api/ingest/chat-filter`

The enabled terms, with the filter's revision as an `ETag`: send `If-None-Match` and an unchanged filter is
a 304. Polling every 30 seconds is plenty.

```json
{
  "version": 12,
  "terms": [
    { "id": 3, "pattern": "idiot", "matchType": "word", "action": "mask", "channels": null }
  ]
}
```

`matchType` is `word` (the word on its own), `substring` (anywhere) or `regex` (JavaScript syntax,
case-insensitive). The first two also match look-alikes and separators: `a`→`[a@4]`, `e`→`[e3]`,
`i`→`[i1!|]`, `o`→`[o0]`, `s`→`[s5$]`, `t`→`[t7]`, `l`→`[l1|]`, with `[\s._-]*` allowed between letters.
[`filter.ts`](../apps/dashboard/src/lib/chat/filter.ts) is the reference implementation. `action` is `flag`
(deliver and mark), `mask` (deliver with the match replaced by asterisks) or `block` (do not deliver; the
sender sees it as sent). `channels: null` means every channel.

## Still to do in dainty-lass

These are changes to the server and the Tauri shells, which this repository does not own:

1. **The backend posts chat.** `realtime/RoomService.ts` and `DirectChatService.ts` would queue each
   delivered stanza and post batches to `/api/ingest/chat`, skipping silently while the dashboard is not
   configured. A new setting (`dashboard_url`, `dashboard_ingest_key`) goes through `host.toml` and
   `secrets.env` as dainty-lass AGENTS.md describes.
2. **The backend enforces the filter.** Fetch `/api/ingest/chat-filter`, compile it as `filter.ts` does,
   and apply it before delivery.
3. **In-game reports.** The 1.14.7 client's report flow, or a `/report` chat command, posting to
   `/api/ingest/reports`.
4. **The Tauri shells load these pages.** Point `rust/launcher/src-tauri/tauri.conf.json`'s `frontendDist`
   at `apps/launcher/dist/client`, and the server GUI's at `apps/server-manager/dist/client` (today both
   bins share the launcher's `index.html`). `beforeBuildCommand` becomes
   `bun run --cwd <dainty-dashboard>/apps/launcher build`. Tauri hashes the exported pages' inline scripts
   into the CSP at build time, so `script-src 'self'` holds.
5. **Window title and identifier.** `window.rs` still titles the window "Dauntless Reforged Revival
   Launcher"; the page now draws its own title bar, so only the OS task switcher shows it.
