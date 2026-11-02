<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Prompt for Gemini in Antigravity

Paste everything below the line into Antigravity as the task. It assumes the workspace is the `dainty-lass` repository
at `R:\source\dauntless\dainty-lass` (or a checkout of its `plan` branch); the other repositories sit beside it under
`R:\source\dauntless\`.

---

You are taking over a large, half-finished software migration. A different model (Claude) ran it until now and will
review your work afterwards, so write for a reviewer who has not seen your session: every claim you make must be backed
by something you ran, and everything you did not verify must be said plainly.

## What the project is

DaintyLass is a game-server launcher and server stack (for the game Dauntless, run on community servers) being ported
from Node.js to one Rust codebase and split across five Forgejo repositories under `git.silentspud.com/silentspud`:
`dainty-lass` (the app: controller, module framework, gateway, content, admin, host kit, Tauri launcher),
`dainty-lass-ui` (Astro + shadcn UI, mounted at `ui/`), `rustic-isles` (Rust port of the game backend and Director for
game versions 1.14.7 and 1.12.0), `rustic-rifts` (player-hosted realms), `legacy-revived` (the 1.4.4 server, in
maintenance mode). A sixth, `rustic-freeza`, does not exist yet and is blocked on the maintainer. Remotes are
`ssh://git@git.silentspud.com:222/silentspud/<repo>.git`. Everything lives under `R:\source\dauntless\` on a Windows PC.

The work is organised as rows in `docs/dainty-lass-plan.md` (DS, DF, DU, RU, RF, RV, RZ, DN, DE, DH, DX, DL). One row =
one branch. Finished branches are merged `--no-ff` into each repository's `plan` branch by the "integration pass"
(which is now you). `main` has never been touched; do not touch it.

## Your first actions, in this order

1. Read, in full, before changing anything: `AGENTS.md`, then `docs/port/handoff-2026-10-27.md` (the state of
   everything, the stopped rows with their exact WIP branches, the backlog, the known defects, the procedures, and
   what the reviewer will check), then the plan rows you are about to work on in `docs/dainty-lass-plan.md`, and
   `docs/port/migration-guide.md` when you touch code that came from Dauntless Revived or Mystic Paradox.
2. Do NOT use or modify the existing Antigravity worktree at
   `C:\Users\Nick\.gemini\antigravity\worktrees\reforged-revival\implement_app_structure` (branch `feat/app-structure`,
   a stale pre-rewrite branch). Work only in worktrees you create yourself with `git worktree add`, under
   `R:\source\dauntless\<repo>\.claude\worktrees\<slug>`.
3. Establish a baseline before you build on `plan`: fresh-clone `dainty-lass` `plan` with
   `git clone --recurse-submodules` into a SHORT path such as `R:\base` (set `core.longpaths=true`), then run the
   checks listed in section 10.2 of the handoff document and write down the results. If the baseline is not green,
   say so and fix or report that first.

## Hard rules (a violation means your work gets rejected)

- **No AI attribution anywhere.** No `Co-Authored-By`, no "Generated with ...", no tool or model credit lines in any
  commit message or description. Antigravity or git hooks may add such trailers by default: disable that, and before
  every push run
  `git log --format=%B origin/plan..HEAD | grep -iE 'co-authored|generated with|gemini|antigravity|claude'` which must
  print nothing.
- **Branch names are `{type}/{slug}`** (`feat/`, `fix/`, `docs/`, `ci/`, `deps/`, `test/`, `chore/`), never a
  tool-derived name. **Commit subjects name the row** (`DN11: ...`, `Merge DN11: ...`). The body says what and why and
  names the decision (DD/RD row) it follows.
- **No pull requests.** Push your branch, then integrate with `git merge --no-ff` into `plan` and push `plan` only
  after the full checks pass. **Never force-push `plan` or `main`.** `--force-with-lease` is allowed only on a feature
  branch you created.
- **Stay inside your row's files.** Shared seams (workspace `Cargo.toml`, `crates/protocol`, `crates/module-api`,
  the launcher bridge spec, `store` migrations, `services/host/src/cli.rs`, `.forgejo/`, generated files) get the
  smallest additive edit only, listed in the commit body. Never reformat or reorder them.
- **Never hand-edit or hand-merge generated files** (bridge types, `gen/strings.json`, `schema.graphql`,
  `isolation/commands.js`, permission sets). Regenerate with `RR_UPDATE_GENERATED=1 cargo test -p launcher generated`
  from `dainty-lass/rust`.
- **Both languages, appended at the end**: UI catalogs `ui/packages/i18n/catalogs/en.json` and `pt-BR.json`; same keys,
  same placeholders, same order; every new bridge error code needs a catalog message and a `model.ts` entry. Flag any
  pt-BR you are unsure of in the commit body.
- **Config rules**: a new setting is a `host.toml` field with a `setup` writer, a reader, a round-trip test and a docs
  row, in one commit; secrets go in `secrets.env`. Migration version numbers are unique across the whole database
  (list in the handoff document).
- **Security rules**: the gateway is the only listener off loopback; the account key (`RRK_...`) never leaves its home
  server, never crosses a node link or the launcher bridge, and is never logged; no private keys or `*.pem` in the
  tree; no `unwrap`/`expect`/`unsafe`/`todo!` outside tests (the lints enforce it).
- **Ports must match the Node originals byte for byte where a client can see it** (routes, JSON field order, status
  codes). Golden and parity tests are the contract: fix the Rust, never loosen the test.
- **Never do the maintainer's work**: the canary, cutovers, real Epic sign-in or installs, running against a real game
  client, publishing, creating the `rustic-freeza` repository, or any decision listed in section 8 of the handoff.
  Write the steps down instead.
- **If a tool or the system refuses an action** (for example killing a process, deleting files outside the repository),
  do not look for a way around it. Stop and ask the user.

## What to work on, in priority order

1. **Finish the stopped rows** (handoff section 4), each continuing from its pushed WIP branch: merge
   `legacy-revived` `feat/rv1-module-crate` first (4.9), then verify the Docker fix (4.5), controller network wiring
   (4.4), DN11 (4.1), DH7 (4.2), DN10b (4.3), then RV3 (4.8), RU7 (4.6), RF7 (4.7).
2. **The integration backlog** (handoff section 6): the documents rows may not edit, the missing `module-api`/`store`
   seams, duplicate types, the RU5 leftovers, and bumping the submodule pins and git-dependency pins.
3. **Rows not started** (handoff section 5) that are not blocked on the maintainer.

Work rows in parallel only if your environment can run independent agents in separate worktrees; otherwise do them one
at a time. Rebase onto the current `origin/plan` before reporting a row, because `plan` moves.

## How to report

After every row, print: repository, branch, tip SHA, merge SHA if integrated, the exact commands you ran and their
results (counts of tests passed and failed, copied from real output), what you did NOT verify, and new problems you
found. Append the same line to `docs/port/handoff-ledger.md` on `plan`. When you stop, leave a section "Where I left
off" at the end of `docs/port/handoff-2026-10-27.md` with the state of every branch you touched. Delete finished
worktrees' `target` and `ui/node_modules` folders to save disk, and list any leftover scratch folders for the
user.

Begin with step 1 of "Your first actions".
