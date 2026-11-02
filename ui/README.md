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

MIT ([LICENSE-MIT.txt](LICENSE-MIT.txt)). The language catalogs and the generated bridge types are AGPL-3.0-only and
come from dainty-restricted at build time (`bun run restricted-data`, run by `build`, `dev`, `typecheck` and `test`);
see [NOTICE.md](NOTICE.md).
