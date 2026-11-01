# Sign-in and access

Anyone can sign in to the dashboard. What they see depends on their tier, and the server can ask
newcomers to accept its terms first.

## Signing in

[Better Auth](https://www.better-auth.com) handles every way in (`apps/dashboard/src/lib/auth/server.ts`).
There are no passwords:

| Way in     | Needs                                       | Notes                                                    |
| ---------- | ------------------------------------------- | -------------------------------------------------------- |
| Discord    | `DASHBOARD_DISCORD_CLIENT_ID`, `_SECRET`    | The only one that lets guild roles set a tier            |
| Twitch     | `DASHBOARD_TWITCH_CLIENT_ID`, `_SECRET`     |                                                          |
| Email code | `DASHBOARD_SMTP_URL`, `DASHBOARD_MAIL_FROM` | Six digits, five minutes, three tries, stored hashed     |
| Passkey    | nothing                                     | Added from **Your account** after signing in another way |

Register `<DASHBOARD_BASE_URL>/api/auth/callback/discord` and `.../callback/twitch` as the redirects.
Passkeys are bound to the hostname of `DASHBOARD_BASE_URL`, so changing the public hostname invalidates
them.

A Discord or Twitch account with the same confirmed address as an existing one joins it. On **Your
account**, people link more providers and add passkeys; the last way in cannot be removed.

`DASHBOARD_AUTH_SECRET` signs session cookies and encrypts the provider tokens stored for guild checks.
When it is unset, one is generated into `<data dir>/auth-secret` on first start; keep that file with the
database.

## Tiers

| Tier      | Sees                                                                                        |
| --------- | ------------------------------------------------------------------------------------------- |
| user      | Overview (players and controllers only), Players (no chat, reports or history), Controllers |
| moderator | Everything a user sees, plus reports, chat logs and the chat filter; can moderate players   |
| admin     | Everything a moderator sees, plus the audit log and undo, Members, account access, Settings |
| owner     | Everything, including changing Settings and making other owners                             |

[`permissions.ts`](../apps/dashboard/src/lib/auth/permissions.ts) has the capability table; pages and
actions check capabilities, never tiers.

Where a tier comes from, re-decided at every sign-in (`resolveSignIn` in `lib/staff.ts`):

1. An owner listed in `DASHBOARD_OWNER_DISCORD_IDS`, or by confirmed address in `DASHBOARD_OWNER_EMAILS`.
2. A tier set by hand on the **Members** page ("Set here"). It sticks until the member is set back to
   **Follow Discord**.
3. The highest tier any of the member's Discord roles maps to (**Settings → Discord roles**), read with
   their own Discord token. Taking the role away in Discord takes the tier at the next sign-in. When the
   guild cannot be read, the last tier stands.
4. Otherwise, user.

## Onboarding

With **Settings → Onboarding → Ask newcomers to accept the terms** on, a user who is not approved sees
only the welcome page: a display name, an optional in-game name, and the terms (Markdown). Moderators and
above never see it.

Someone is approved when they:

- hold the guild's **approved role** (checked at sign-in),
- are approved by an admin on the Members page, or
- accept the current terms on the welcome page.

Accepting adds the approved role in Discord when the member has Discord linked and
`DASHBOARD_DISCORD_BOT_TOKEN` is a bot in the guild with Manage Roles, ranked above that role. If Discord
refuses, the member is still approved and the audit log says the role was not added.

Saving changed terms makes a new version. Everyone approved by accepting is asked again; approvals from
the guild role or an admin stand.

## Audit

Sign-ins (with the method), sign-outs, refusals, linked and unlinked providers, added and removed
passkeys, onboarding and the role grant are all in the audit log.
