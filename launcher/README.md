<!-- SPDX-License-Identifier: MIT -->

# launcher

The launcher's JavaScript side (RD27), MIT: the isolation hook Tauri runs in front of every page call, the
scripts the Tauri shells and the release workflow run, and the tests of both. The Rust shell itself
(`restricted/launcher/src-tauri`) is AGPL and takes `isolation/hook.js`, `isolation/index.html` and
`scripts/` from here.

| Path | What it is |
| --- | --- |
| `isolation/` | `hook.js` and `index.html`; `commands.js` is generated in restricted and copied in, never committed |
| `scripts/restricted-data.ts` | copies `commands.js` and ui's restricted data from `restricted/` |
| `scripts/dashboard-page.ts` | builds or serves a dashboard page for a shell (`build\|dev`, `launcher\|server-manager`) |
| `scripts/release-config.ts` | writes the release builds' `tauri build --config` overlays |
| `scripts/sign-windows.ts` | Authenticode-signs one file; Tauri's `signCommand` |
| `test/` | `bun test`: the hook against the real table, ui's page API through the hook, the scripts |
| `e2e/` | the desktop end-to-end runs (Windows, built bundles, by hand) |

The restricted folder is `DAINTY_RESTRICTED_DIR`, or `restricted/` in this repository.

```sh
bun install --frozen-lockfile
bun run typecheck
bun run test
bun run e2e:desktop   # Windows only, after restricted's bundles are built
bun run e2e:remote    # also needs the stack docker/remote/test.sh leaves running
```

### Development mode

`tauri dev` over the MIT shell, `crates/dainty-launcher-app`, from here or the repository root. Each starts its
page's dev server first (`crates/dainty-launcher-app/scripts/page.ts dev <page>`).

```sh
bun run dev           # the launcher with realms, against the launcher page on 4321
bun run dev:manager   # Dainty Lass Manager, against the server-manager page on 4322
```
