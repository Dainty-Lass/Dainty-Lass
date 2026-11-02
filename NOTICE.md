# NOTICE

dainty-lass is the MIT half of DaintyLass (RD27): the code here was written for this project, or rewritten
clean-room from behaviour specs, from none of the AGPL projects DaintyLass builds on, and is licensed under the MIT
License ([LICENSE-MIT.txt](LICENSE-MIT.txt)). Every AGPL part, with the attributions and additional terms that come
with it (Mystic Paradox, Dauntless Revived, Undaunted, the Dauntless Reforged Revival line), is in
[dainty-restricted](https://git.silentspud.com/silentspud/dainty-restricted), whose NOTICE.md records it. A program
built from both is distributed under the AGPL-3.0-only as a whole; the MIT grant is on this repository's own source.

It is **not** an official release of Mystic Paradox, Dauntless Revived, Undaunted or any other project named in
dainty-restricted's notice, and none of them endorses it.

## This project

By the DaintyLass contributors (`Copyright (c) 2026 Silent Spud`). Each crate's `Cargo.toml` says `license = "MIT"`,
and CI fails a package that does not.

- **Born here**: `crates/controller`, `crates/module-api` (DF2, the module framework and frame protocol),
  `crates/epic` (DE1, written from Epic's manifest and chunk formats as Legendary documents them),
  `crates/history-ship` (DH4), `crates/remote` (DD27/RM2), `crates/network` (DN4), `crates/admin`,
  `crates/backend` and `crates/director` (stubs), the fuzz targets in `fuzz/`, and the UI in `ui/` (whose two
  AGPL inputs, the language catalogs and the generated bridge types, are dainty-restricted's and are copied in at
  build time).
- **Clean-room MIT rewrites and extracted MIT kits** (`DD26`, `DL2` answered No on 2026-10-30; behaviour specs in
  `docs/port/spec-*.md` and clean-room provenance records in `docs/port/clean-room/*.md`): `crates/protocol`,
  `crates/proxy`, `crates/service-kit`, `crates/store`, `crates/hostkit` (`DL3b`) and `crates/launcher-kit` (`DL4b`).
- **Not yet rewritten**: the workspace files (`Cargo.toml`, `rustfmt.toml`, `clippy.toml`, `deny.toml`,
  `.cargo/config.toml`) started from Dauntless Revived's at commit `e1c5fec` (AGPL-3.0-only). They are configuration
  rather than program code, but are to be rewritten before this repository is published as MIT-only (RD27).

### Other material

- The UI's npm dependencies are listed in `ui/bun.lock`, and the Rust crates' in `Cargo.lock`; each keeps its own
  licence (`cargo deny` checks the Rust ones).
- libpq, built from source by `pq-src` and linked into every binary that links `store` (DF7): PostgreSQL
  Licence. Portions Copyright (c) 1996-2025, PostgreSQL Global Development Group; Portions Copyright (c) 1994,
  The Regents of the University of California. Permission to use, copy, modify, and distribute this software
  and its documentation for any purpose, without fee, and without a written agreement is hereby granted,
  provided that the above copyright notice and this paragraph and the following two paragraphs appear in all
  copies. IN NO EVENT SHALL THE UNIVERSITY OF CALIFORNIA BE LIABLE TO ANY PARTY FOR DIRECT, INDIRECT, SPECIAL,
  INCIDENTAL, OR CONSEQUENTIAL DAMAGES, INCLUDING LOST PROFITS, ARISING OUT OF THE USE OF THIS SOFTWARE AND ITS
  DOCUMENTATION, EVEN IF THE UNIVERSITY OF CALIFORNIA HAS BEEN ADVISED OF THE POSSIBILITY OF SUCH DAMAGE. THE
  UNIVERSITY OF CALIFORNIA SPECIFICALLY DISCLAIMS ANY WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
  WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE. THE SOFTWARE PROVIDED HEREUNDER IS ON AN
  "AS IS" BASIS, AND THE UNIVERSITY OF CALIFORNIA HAS NO OBLIGATIONS TO PROVIDE MAINTENANCE, SUPPORT, UPDATES,
  ENHANCEMENTS, OR MODIFICATIONS.

## What this repository does not contain

No game files, no SDK generated from the game, no game data exported from it beyond the examples the services
ship with, and no store captures. Each host produces those from their own copy of the game and never shares
them through this project.
