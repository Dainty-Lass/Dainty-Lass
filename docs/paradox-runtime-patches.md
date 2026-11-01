# Applying a runtime patch

A change to the game runtime (`Rustic-Isles.dll`) or its loader (`winmm.dll`) that was written against Mystic
Paradox's tree, as a `git diff` with paths starting `ParadoxRuntime/` or `tools/RuntimeLoader/`, is applied in two
places since DS6: the runtime in the rustic-isles repository (`native/runtime/` there, mounted here at
[`modules/rustic-isles`](../modules/rustic-isles)), the loader in this one
([`native/runtime-loader`](../native/runtime-loader), DD13). The runtime is the one part every server shares: a
player's DLLs speak HTTP to whichever backend answers, so a fix to the game's behaviour helps every host.

## Applying a patch

With the patch saved as `patch.diff`:

1. Turn Paradox's paths into each repository's, then apply each part with a three-way merge, so a patch whose
   context moved still applies or stops with conflict markers. The runtime part, from `modules/rustic-isles` on a
   branch of rustic-isles:

   ```bash
   sed -e 's#\([ab]\)/ParadoxRuntime/#\1/native/runtime/#g' patch.diff > runtime.diff
   git apply --numstat --include='native/runtime/*' runtime.diff   # every path should start with native/runtime/
   git apply --3way --include='native/runtime/*' runtime.diff
   ```

   The loader part, from this repository's root:

   ```bash
   sed -e 's#\([ab]\)/tools/RuntimeLoader/#\1/native/runtime-loader/#g' patch.diff > loader.diff
   git apply --3way --include='native/runtime-loader/*' loader.diff
   ```

   Once rustic-isles' integration pass has merged the runtime part, this repository moves its
   `modules/rustic-isles` pin to that commit. A patch saved on Windows may have CRLF line endings; convert it to
   LF first (`dos2unix patch.diff`).
2. If it does not apply, read what the patch does and make the change by hand. A conflict usually means an
   earlier runtime change the patch builds on is missing: take that one first.
3. A patch that also changes the backend or the Director (a new environment variable the Director passes
   on, a route the runtime calls) is taken for its runtime part only. Write down what the other part did in
   the commit message; the Rust backend and Director (the RB rows in MIGRATION_PLAN.md, the RU rows in
   dainty-lass-plan.md) decide whether they need it.
4. Commit it alone, as `NNNN-runtime-<name>` like the patches before it, with what it does as the body and a
   line naming where it came from. Add a row to [paradox-patches.md](paradox-patches.md).
5. Build the runtime with a host's SDK (`modules\rustic-isles\native\runtime\_build.bat`,
   [sdk-and-game-data.md](sdk-and-game-data.md#2-the-runtime)) and play a hunt with it before it is released.
   Hosts rebuild and publish it after `reforged-host update`, which says so when the runtime or the loader changed.

## Keeping track

The highest number taken is the latest `NNNN-` commit under `native/` here or in rustic-isles
(`git log --oneline -- native/` in each); a patch
is skipped only on purpose, with the reason in the commit that takes the next one. The runtime is C++ and keeps
no record of its own, so the table in [paradox-patches.md](paradox-patches.md) is the inventory.
