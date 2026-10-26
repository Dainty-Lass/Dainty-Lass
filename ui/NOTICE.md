# NOTICE

dainty-lass-ui is the page the DaintyLass launcher shows. Its parts carry two licences, split by where
they came from (DaintyLass's `docs/dainty-lass-plan.md`, DD26). The repository as a whole is distributed
under the GNU Affero General Public License, version 3 only (`AGPL-3.0-only`,
[LICENSE-AGPL.txt](LICENSE-AGPL.txt)), which the MIT parts allow.

## MIT

[LICENSE-MIT.txt](LICENSE-MIT.txt), by the DaintyLass contributors: everything not listed under AGPL below.
That is the screens and the app (`apps/web`), the components (`packages/ui`), the bridge's own code
(`packages/bridge/src`, except `generated/`) and the catalogs' lookup code (`packages/i18n/src`,
`packages/i18n/test`). Written for this project, from none of the projects named below.

## AGPL-3.0-only

- **The two language catalogs**, `packages/i18n/catalogs/en.json` and `pt-BR.json`. Their text is the
  launcher text of the Dauntless Reforged Revival line (its `ReforgedLauncher/src/shared/i18n/` files, and
  `revived.ts` for the Dauntless 1.4.4 keys; by Freeza and that project's contributors,
  [github.com/FreezaMotoqueiro/Dauntless-Reforged-Revival](https://github.com/FreezaMotoqueiro/Dauntless-Reforged-Revival),
  AGPL-3.0-only), which is Dauntless Revived's launcher text
  ([github.com/mixutin/dauntless-revived](https://github.com/mixutin/dauntless-revived), AGPL-3.0-only, by
  mixutin and its contributors) rewritten for Dauntless 1.14.7. `err_compat_runtime_missing` is Dauntless
  Revived's own. The text is modified here: the product's name is DaintyLass, the backend's dialog text is
  DaintyLass's, and the keys after `err_edition_other` were added; the history records each change. The
  launcher keeps these strings word for word in both languages (the plan's section 6), so they keep their
  licence.
- **The bridge's generated types**, `packages/bridge/src/generated/types.ts`, written by DaintyLass from its
  launcher's bridge (`rust/launcher/src-tauri/src/bridge`), which follows Dauntless Revived's launcher.

## Not in this repository

No game files and nothing exported from the game.
