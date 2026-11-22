# AGENTS.md

Project-specific guidance for AI coding agents.

`restricted/` is the AGPL-3.0-only subtree of the Dainty Lass monorepo (RD27). Since RB15 it holds no code and no
Cargo workspace: only docs and the data the MIT server reads at run time. Every rule for code, branches, commits,
comments and user-facing text is the root's [`AGENTS.md`](../AGENTS.md); read it first.
[`docs/port/migration-guide.md`](../docs/port/migration-guide.md) says where an upstream change goes now.

## Layout

| Path | What it is |
| --- | --- |
| `docs/` | Host and player guides, `paradox-patches.md` (the record of the patches to Mystic Paradox), `paradox-runtime-patches.md`, and `security/` for the components first reviewed here |
| `modules/rustic-isles/` | The 1.14.7 module's AGPL data: `version-data/1.14.7` (Mystic Paradox's version data, read at start from the file `settings.toml`'s `version_data` names) and `game-data/1.14.7` (`backend/`, with `raw/`, `director/`, `exports/`, `captures/` and the `store-captures.json` pin), plus Paradox's licence, notice and docs. The module itself is the root's MIT `rustic-isles-*` crates |
| `modules/rustic-rifts/` | Docs, the relay self-test (`run-relay-selftest.ps1`, `tools/`) and Paradox's licence. The `rustic-rifts-*` crates are the root's |
| `launcher/` | `brand/icon.png`, today's icon (L2) |

The server release is the `git archive` of this folder plus the root's binaries, so what is here ships to hosts.

## Rules

- **Code never comes here.** Anything that would be ported from DR, Paradox or the Reforged Revival line reaches the
  root through a clean-room spec instead (RD27).
- **Game data** goes in `modules/rustic-isles/game-data/1.14.7/`, examples only: no game files, generated SDKs or
  exported game data beyond the examples (`../AGENTS.md`, "Nothing from the game is committed"). Lady Luck's Store
  and the platinum store are built on each server from `modules/rustic-isles/game-data/1.14.7/store-captures.json`.
- **The hosting docs move with a setting.** The settings table in `docs/hosting.md`, and `hosting-linux.md` and
  `hosting-docker.md` wherever a host sees the option. Delete the rows when the setting goes.
- **Never edit** `NOTICE.md`, `README.md` or `SECURITY_REVIEW.md` from a row; they belong to the integration pass.

## Licensing

AGPL-3.0-only, with additional terms for the Mystic Paradox material (`ADDITIONAL_TERMS.md`,
`modules/rustic-isles/ADDITIONAL_TERMS.md`). Keep Paradox's top-level files under `modules/rustic-isles/`. The
integration pass records each source in `NOTICE.md`.
