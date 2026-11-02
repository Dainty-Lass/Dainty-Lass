# NOTICE

ui/ is the page the DaintyLass launcher shows, MIT-licensed ([LICENSE-MIT.txt](LICENSE-MIT.txt)), by the DaintyLass
contributors: the screens and the app (`apps/web`), the components (`packages/ui`), the bridge's own code
(`packages/bridge/src`) and the catalogs' lookup code (`packages/i18n/src`, `packages/i18n/test`). Written for this
project, from none of the projects named below.

## Inputs from dainty-restricted

Two inputs are AGPL-3.0-only and live in dainty-restricted, not here (RD27); `scripts/restricted-data.mjs` copies them
in before a build:

- **The two language catalogs**, `packages/i18n/catalogs/en.json` and `pt-BR.json`: the launcher text of the
  Dauntless Reforged Revival line ([github.com/FreezaMotoqueiro/Dauntless-Reforged-Revival](https://github.com/FreezaMotoqueiro/Dauntless-Reforged-Revival)),
  which is Dauntless Revived's launcher text ([github.com/mixutin/dauntless-revived](https://github.com/mixutin/dauntless-revived)),
  rewritten for Dauntless 1.14.7 and kept word for word (the plan's section 6).
- **The bridge's generated types**, `packages/bridge/src/generated/types.ts`, written from dainty-restricted's launcher
  bridge, which follows Dauntless Revived's launcher.

A page built with them is distributed under the AGPL-3.0-only, as dainty-restricted's NOTICE says.

## Not in this repository

No game files and nothing exported from the game.
