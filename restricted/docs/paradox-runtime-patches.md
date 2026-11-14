# Applying a runtime patch

The AGPL game runtime (`Rustic-Isles.dll`) retired at R21 (2026-11-14) and its source is no longer in this tree
(its last commit is the tag `agpl-runtime-final`). A runtime change written against Mystic Paradox's tree, as a
`git diff` with paths starting `ParadoxRuntime/`, is no longer applied here: it reaches the MIT
`rustic-isles-runtime.dll` through a clean-room spec, as the root's `docs/port/migration-guide.md` section 4
describes. A `tools/RuntimeLoader/` part goes through the loader's spec (`docs/port/spec-runtime-loader.md` at the
root), and the root's MIT `native/runtime-loader` follows it in a commit of its own, written without the patch
open (RD27). A backend or Director part is recorded in [paradox-patches.md](paradox-patches.md).
