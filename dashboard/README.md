# dainty-dashboard

The Dainty Lass user interfaces on one theme: a **remote management dashboard** for a server's staff, the
**launcher** page players use, and the **server manager** page hosts use. All three are Astryx on Next.js
(through vinext) on Bun, and share `@dainty/theme`.

| Path                                         | What                                                                                                                        |
| -------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| [`apps/dashboard`](apps/dashboard)           | The dashboard: a server app with passwordless sign-in, a SQLite database and the game's admin APIs behind it                |
| [`apps/launcher`](apps/launcher)             | The launcher page, exported as static files for the Tauri launcher                                                          |
| [`apps/server-manager`](apps/server-manager) | The server manager page, exported as static files for the Tauri server GUI                                                  |
| [`apps/agent`](apps/agent)                   | The agent each extra controller runs, so one dashboard manages them all ([docs/fleet.md](docs/fleet.md))                    |
| [`packages/theme`](packages/theme)           | `@dainty/theme`: the Astryx theme, the CSS stack, the colour-mode provider and the brand mark                               |
| [`packages/fleet`](packages/fleet)           | `@dainty/fleet`: the game calls and the dashboard–agent protocol, shared by both sides                                      |
| [`packages/bridge`](packages/bridge)         | `@dainty/bridge`: the launcher's Tauri bridge (synced from dainty-lass), a browser stand-in, and the shared desktop catalog |

## The dashboard

What staff do on it, for one controller or a fleet of them ([docs/fleet.md](docs/fleet.md)):

- **Controllers**: every host running game servers, its health, players and hunts, paired with a
  one-time code. Each controller besides the dashboard's own runs a small agent that connects out to the
  dashboard, so no controller opens a port and each keeps its own secrets.

- **Overview**: players online, hunts running, open reports, flagged chat, the health of each service the
  dashboard talks to, the latest changes, and performance: each component's phase, uptime, restarts, CPU
  and memory, game servers, request rates and latency per upstream, and chat by verdict.
- **Players**: search every account, see who is online and where, and kick, time out, ban or lift a ban.
  Admins change an account's access and game roles. Each player page has their chat, the reports against
  them and every staff action taken on them.
- **Reports**: a queue from the game, Discord and staff. Claim, add notes, see the chat around the reported
  moment, and resolve with a sanction in one step, or dismiss.
- **Chat logs**: search by text, player, channel, room and time; flagged lines only; CSV export (audited).
- **Chat filter**: whole-word, anywhere and regex terms that flag, mask or block, per channel, with a tester.
  Look-alikes (`id1ot`) and spacing tricks (`i.d.i.o.t`) are caught by the non-regex kinds.
- **Audit log**: every change, with what it replaced, filterable and exportable. Most changes have **Undo**.
- **Members** and **Settings**: everyone who has signed in and their tier; Discord role sync; onboarding
  and server terms; moderation reasons; retention. Both keep a revision history.
- **Your account**: each person's own sign-in methods and passkeys.

### Rollback

Two kinds, both themselves audited and undoable:

- **Undo one change** from the audit log: a ban becomes an unban, an edited filter term gets its old pattern
  back, a role change is reversed, a report goes back to the state it was in. Undoing writes a new change
  that points at the one it undid.
- **Restore a revision** of the chat filter, the settings or the staff list: the whole document as it was,
  written again as the newest version. Nothing newer is deleted.

A kick cannot be undone, and neither can a timeout that has already run out.

### Sign-in

Discord, Twitch, a code by email, or a passkey; never a password. Anyone can sign in, and their tier
decides what they see: users get the overview, players and controllers; moderators add reports and chat;
admins add the audit log, members and settings; owners can change settings. Tiers come from the
Members page, from Discord roles, or from `DASHBOARD_OWNER_DISCORD_IDS` / `DASHBOARD_OWNER_EMAILS`
(how the first owner gets in). The server can also ask newcomers to accept its terms first, which can
add an "approved" role in Discord. [docs/access.md](docs/access.md) has the details.

### Running it

```bash
bun install
bun run theme:build
cp apps/dashboard/.env.example apps/dashboard/.env   # then fill it in
bun run dev                                           # http://localhost:3000
```

For production, `bun run build` and then `bun run start` (or the [Dockerfile](docker/dashboard.Dockerfile)).
The dashboard reaches the admin API and the backend on loopback, so it runs on the game server or on a
machine with an SSH tunnel to it. Put it behind a TLS reverse proxy and set `DASHBOARD_BASE_URL` to the
public origin; register `<DASHBOARD_BASE_URL>/api/auth/callback/discord` (and `/twitch`) as the redirects.

[`docs/integration.md`](docs/integration.md) covers what the dashboard needs from the server: the admin API
key, the manager-operator secret, and the ingest API the backend posts chat and reports to.

## The launcher and the server manager

Static exports (`output: "export"`) that the Tauri shells in dainty-lass serve from their `frontendDist`.
They talk to Rust only through the bridge in `packages/bridge`, which is copied from
`dainty-lass/rust/launcher/page`; run `bun run bridge:sync` after the bridge spec changes there. In a plain
browser (`bun run dev:launcher`, `bun run dev:server-manager`) the bridge is a stand-in that walks the
phases with sample data. Both speak English and Brazilian Portuguese, following the launcher's language
setting.

## Checks

```bash
bun run typecheck
bun run test
bun run lint
```

## Licence

AGPL-3.0-only, as the rest of Dainty Lass ([LICENSE](LICENSE)).
