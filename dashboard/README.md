# dainty-dashboard

The DaintyLass user interfaces on one theme: a **remote management dashboard** for a server's staff, the
**launcher** page players use, and the **server manager** page hosts use. All three are Astryx on Next.js
(through vinext) on Bun, and share `@dainty/theme`.

| Path | What |
| --- | --- |
| [`apps/dashboard`](apps/dashboard) | The dashboard: a server app with Discord sign-in, a SQLite database and the game's admin APIs behind it |
| [`apps/launcher`](apps/launcher) | The launcher page, exported as static files for the Tauri launcher |
| [`apps/server-manager`](apps/server-manager) | The server manager page, exported as static files for the Tauri server GUI |
| [`packages/theme`](packages/theme) | `@dainty/theme`: the Astryx theme, the CSS stack, the colour-mode provider and the brand mark |
| [`packages/bridge`](packages/bridge) | `@dainty/bridge`: the launcher's Tauri bridge (synced from dainty-lass), a browser stand-in, and the shared desktop catalog |

## The dashboard

What staff do on it:

- **Overview**: players online, hunts running, open reports, flagged chat, the health of each service the
  dashboard talks to, and the latest changes.
- **Players**: search every account, see who is online and where, and kick, time out, ban or lift a ban.
  Admins change an account's access and game roles. Each player page has their chat, the reports against
  them and every staff action taken on them.
- **Reports**: a queue from the game, Discord and staff. Claim, add notes, see the chat around the reported
  moment, and resolve with a sanction in one step, or dismiss.
- **Chat logs**: search by text, player, channel, room and time; flagged lines only; CSV export (audited).
- **Chat filter**: whole-word, anywhere and regex terms that flag, mask or block, per channel, with a tester.
  Look-alikes (`id1ot`) and spacing tricks (`i.d.i.o.t`) are caught by the non-regex kinds.
- **Audit log**: every change, with what it replaced, filterable and exportable. Most changes have **Undo**.
- **Staff** and **Settings**: who may sign in and with what role; Discord role sync; moderation reasons;
  retention. Both keep a revision history.

### Rollback

Two kinds, both themselves audited and undoable:

- **Undo one change** from the audit log: a ban becomes an unban, an edited filter term gets its old pattern
  back, a role change is reversed, a report goes back to the state it was in. Undoing writes a new change
  that points at the one it undid.
- **Restore a revision** of the chat filter, the settings or the staff list: the whole document as it was,
  written again as the newest version. Nothing newer is deleted.

A kick cannot be undone, and neither can a timeout that has already run out.

### Sign-in

Discord OAuth2 with PKCE. Someone gets in when they are on the Staff page, when their Discord id is in
`DASHBOARD_OWNER_DISCORD_IDS` (which is how the first owner gets in), or when a role they hold in the
server's Discord guild is mapped to a dashboard role under Settings. Access granted by a guild role is
re-read at every sign-in. Roles: owner, admin, moderator, viewer
([`permissions.ts`](apps/dashboard/src/lib/auth/permissions.ts) has the table).

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
public origin; register `<DASHBOARD_BASE_URL>/api/auth/discord/callback` as the Discord app's redirect.

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

AGPL-3.0-only, as the rest of DaintyLass ([LICENSE](LICENSE)).
