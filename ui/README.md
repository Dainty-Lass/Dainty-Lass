# dainty-lass-ui

The front-end page served by the [DaintyLass](https://git.silentspud.com/silentspud/dainty-lass) Tauri launcher and
server GUI (`ui/` submodule). Built with Astro, Preact, TypeScript and Tailwind CSS in a Bun workspace.

## Workspace layout

| Package | What |
| --- | --- |
| `apps/web` | Astro static application: player screens (servers, invites, PLAY, runtime acceptance, Epic library & download, realms), server GUI screens (Host, status, declarative module panels, guarded actions, change-log history & revert) |
| `packages/bridge` | Typed Tauri bridge client and generated types (`src/generated/types.ts`) |
| `packages/i18n` | Language lookup and string catalogs (`catalogs/en.json`, `catalogs/pt-BR.json`) |
| `packages/ui` | Shared UI primitives |

## Build and test

```bash
bun install --frozen-lockfile
bun run typecheck
bun run lint
bun run test
bun run build
```

## Licence

Distributed as a whole under the GNU Affero General Public License, version 3 only (`AGPL-3.0-only`,
[LICENSE-AGPL.txt](LICENSE-AGPL.txt)). All application, component, bridge and i18n lookup code written for this
project is also licensed under the MIT License ([LICENSE-MIT.txt](LICENSE-MIT.txt)), while the two language catalogs
(`packages/i18n/catalogs/en.json` and `pt-BR.json`, derived from Dauntless Revived and the Dauntless Reforged Revival
line) and the generated bridge types (`packages/bridge/src/generated/types.ts`) remain `AGPL-3.0-only`. See
[NOTICE.md](NOTICE.md) for full provenance and attribution.
