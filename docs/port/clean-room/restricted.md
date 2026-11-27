<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room record: retiring `restricted/`

On 2026-11-26 the maintainer had the AGPL `restricted/` folder deleted, keeping only what was still useful, remade as
MIT. Since RB15 it had held no code, only docs and data. This page records what it held, what each part became and
how the new text was kept clean of the old (RD27).

## Method

- `restricted/` was removed from the working tree before any writer started.
- Each writer was given only the heading outline of the page it replaced, as a list of topics, and was told never to
  open the old text: not in the main checkout, not through `git show` or `git log -p`, and nothing from Dauntless
  Revived or Mystic Paradox.
- Each writer wrote from the MIT code, the MIT docs and the specs in `docs/port/`. Where the code couldn't confirm
  something, it was left out or flagged for the maintainer rather than recalled.
- The data writer worked from the MIT reader (`crates/rustic-isles-1-14-7/src/data.rs`), its tests and the specs,
  under rulings V-1 to V-4 and Q-2 (`docs/port/rulings-2026-11-13.md`). Ids and game GUIDs are facts (V-1); the design
  values (V-2) are this project's own, chosen fresh.

## What each part became

| Was in `restricted/` | Now |
| --- | --- |
| `docs/hosting.md`, `hosting-linux.md`, `hosting-docker.md` | `docs/hosting.md`, `docs/hosting-linux.md`, `docs/hosting-docker.md`, rewritten (English only) |
| `docs/players.md`, `canary.md`, `launcher-release.md` | `docs/players.md`, `docs/canary.md`, `docs/launcher-release.md`, rewritten |
| `modules/rustic-isles/docs/` (installing the game, the SDK, the game data, scope and legal) and the game-data READMEs | `docs/sdk-and-game-data.md`, expanded; `modules/rustic-isles/game-data/1.14.7/README.md` |
| `docs/security/` checklists with no MIT successor (module panels, host start, host game data, keystore, launcher network, runtime download, Host page) and rustic-rifts' relay credentials | The same names under `docs/security/` (`rifts-relay-credentials.md` for the last), rewritten; indexed in `SECURITY_REVIEW.md` |
| `docs/security/turn-relay.md` | Dropped: no TURN listener runs in code any more; the relay credentials it covered are in `rifts-relay-credentials.md` |
| `docs/security/region-ping.md`, `relay.md`; rustic-rifts' `admission.md`, `guard.md`, `host.md` | Dropped: `docs/security/region-ping.md`, `launcher-relay.md`, `rifts-transport.md`, `rifts-guard.md` and `rifts-host.md` already cover today's code |
| `SECURITY_REVIEW.md` | Folded into the root `SECURITY_REVIEW.md` |
| `modules/rustic-isles/version-data/1.14.7/version_data.json` (V-2's design) | `crates/rustic-isles-1-14-7/data/version_data.json`, new values beside the MIT `version_facts.json` |
| `modules/rustic-isles/game-data/1.14.7/` placeholders and `store-captures.json` | `modules/rustic-isles/game-data/1.14.7/`, written fresh from the readers' shapes; the pin's commit, paths and hashes are facts |
| `docs/paradox-patches.md`, `paradox-runtime-patches.md`, the route and rifts inventories, `mit-parity-report.md`, the RU10 spike | Dropped: closed records, kept in git history |
| `LICENSE.txt`, `NOTICE.md`, `ADDITIONAL_TERMS.md` and the modules' copies, `README.md`, `AGENTS.md`, `SECURITY.md`, `.gitleaks.toml`, `.gitignore` | Dropped with the AGPL material they covered; the root `README.md` credits Mystic Paradox, Dauntless Revived and Undaunted |
| `launcher/README.md`, `launcher/brand/icon.png` | Dropped: nothing read them |
| rustic-rifts' `run-relay-selftest.ps1`, `tools/mint-relay-test-file.cjs` | Dropped: Node tooling for the retired AGPL relay |

## Code and build changes

- A checkout's layout is `modules/rustic-isles` at the root (`crates/dainty-hostkit/src/paths.rs`, the launcher
  kit's `WorkspacePaths`); in a checkout, the version data and facts come from `crates/rustic-isles-1-14-7/data/`.
- `setup` moves a checkout's real game data out of the old `restricted/modules/rustic-isles` (`oldlayout.rs`), and
  the root `.gitignore` ignores what is left there.
- The server release, the launcher release, Docker and CI copy from the new places; the server image is labelled
  MIT OR Apache-2.0, and `deny.toml` no longer allows AGPL-3.0-only.
- `bun run restricted-data` is now `bun run bridge-data`.

## Left for the maintainer

- Read the new design values (the starting wallet, the reward cache plan, the legacy starter and the default
  appearance) and play a new character through them: nothing is played against the real client in CI.
- The guides are English only; the old ones had Portuguese sections.
- DL5's legal read now covers an all-MIT repository.
