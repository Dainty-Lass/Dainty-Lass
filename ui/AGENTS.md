# AGENTS.md

Project-specific guidance for AI coding agents. The README says what is here; this says what is not obvious.

## Layout

A Bun workspace (`bunfig.toml` pins the isolated linker: import only what a package declares).

| Path              | What                                                                                                                              |
| ----------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `apps/web`        | Every page, one source, built by `DAINTY_MODE` (below): the dashboard, the launcher and the server manager                        |
| `apps/agent`      | Deprecated (D1): `dainty-remote` replaces it. Runs on each extra controller; dials the dashboard ([docs/fleet.md](docs/fleet.md)) |
| `packages/fleet`  | The game calls and the agent protocol, shared by the dashboard and the agent                                                      |
| `packages/theme`  | The one theme. `bun run theme:build` compiles `src/dainty.theme.ts` to `built/`, which is committed                               |
| `packages/bridge` | `src/generated/` is copied from `crates/dainty-launcher-kit/generated` and never committed here: `bun run bridge-data`            |

## One app, three modes

`apps/web` builds as one of three things, picked by `DAINTY_MODE` (`mode.mjs`; `dashboard` when unset):
`dashboard` is the Bun server, `launcher` and `manager` are static exports for the Tauri shells (no server,
no request APIs). A file in `src/app` is a route only in the modes its extension names (`pageExtensions`):
`page.dashboard.tsx`, `page.launcher.tsx`, `page.manager.tsx`, or `layout.desktop.tsx` for both shells.
Anything else (components, `lib/`, client files beside a page) is shared and builds in every mode that
imports it, so a shell's route must never import server code (`lib/db`, `lib/auth`, `next/headers`).
`MODE` (`src/mode.ts`) is inlined at build time for code that differs by mode. Each build empties `dist/`, and vinext
runs one dev server per app, so build or serve one mode at a time. The shells' catalogs are `messages/launcher` and `messages/manager`, loaded under
the `launcher.` and `manager.` namespaces; the dashboard's is `messages/en.json`.

## Rules that read like overcaution

- **Every change staff make is audited, with what it replaced.** A new mutation goes through a function in
  `apps/web/src/lib` that calls `recordAudit` in the same transaction (`tx`), never straight from an
  action. If it can be undone, give it `revertible: true` and an inverse in `lib/audit/revert.ts`, and list
  the action in `components/AuditTable.tsx`'s `REVERTIBLE`.
- **Config that rolls back as a whole writes a revision.** The chat filter, settings and staff list call
  `ensureBaseline` before the change and `writeRevision` after, in the same transaction.
- **`tx` bodies are synchronous.** bun:sqlite commits when the callback returns; anything awaited inside
  runs after the commit. Do the game calls first, then the transaction.
- **The game answers before the dashboard records.** Moderation reaches the admin API or the backend first;
  the audit event is written only when the game accepted it.
- **Every game call names its controller.** `adminApi(controllerId)` and `backend(controllerId)` reach the
  local one directly and any other through its agent; rows that come from or act on a controller carry its
  `controller_id`. A new game call is a named operation in `packages/fleet/src/ops.ts`, or the agent
  refuses it: never widen that allowlist for convenience.
- **Pages read status, they do not fetch it.** Controllers report every 15 s (`lib/fleet/status.ts`); a page
  that calls every controller on render does not scale to a fleet.
- **The dashboard is one process.** The agent registry is in memory (`lib/fleet/registry.ts`).
- **Secrets are never shown or logged.** Settings shows whether each is set, never the value. The ingest key
  is compared by digest.
- **A new setting is an entry in `lib/settings.ts`** (and its validation in `settings-change.ts`), not an
  environment variable. The environment holds only what the process needs before the database: paths,
  secrets, the public origin.
- **No passwords.** Sign-in is Discord, Twitch, an email code or a passkey (`lib/auth/server.ts`);
  `emailAndPassword` stays off. Every new session goes through `signInMember`, which decides the tier.
- **Pages and actions check capabilities, not tiers.** A page hides what `can()` refuses and the server
  does not fetch it: users never receive reports, chat or audit rows, moderators never receive audit rows.
- **A migration that rebuilds a table runs with foreign keys off** (`lib/db/index.ts`): drizzle's rebuild
  drops the old table, which with keys on would null every row pointing at it.
- **The desktop pages render nothing from a request.** They are prerendered to files; read the bridge in
  client components only.

## Design

[docs/design.md](docs/design.md) is the look, copied from Caddy Proxy Manager: neutral with one runtime
accent (pink by default), `#f1f1f1` / black pages, a tonal current page, stat tiles with a hued edge and
number, rows for dense data. It is written once, in `packages/theme`; an app never restyles it.

## Astryx

Components first: no `<div>` layout, no hex or pixel values in components, tokens for everything. The
exceptions are where Astryx has no component: `packages/theme/src/brand.css` (the colouring in `variants.css`, the brand mark, the
console, the hero, the title bar's drag region) and `apps/web/src/app/globals.css` (the sparkline). Add a rule
there rather than an inline `style`. Look a component up with `bunx astryx component <Name>` from
`packages/theme`, which has the CLI installed.

## Text

Every string a person reads is in a catalog: `apps/*/messages/*.json` and `packages/bridge/messages`. The
dashboard is English; the launcher and server manager are English and Brazilian Portuguese, with the same
keys and placeholders (their `tests/catalog.test.ts` checks). Never build a sentence by concatenation.
Server actions return an error code (`lib/errors.ts`), and the catalog's `errors.*` holds the sentence.

## Checks

From the root: `bun run typecheck`, `bun run test`, `bun run lint`. The dashboard's tests run on an
in-memory database (`apps/web/tests/setup.ts`).

## Commits

Never add a `Co-Authored-By` trailer or any other line crediting an AI tool, in a commit or a pull request
description (as in dainty-lass). Comments are terse and say why.
