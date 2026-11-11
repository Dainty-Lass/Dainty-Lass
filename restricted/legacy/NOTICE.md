# NOTICE

dainty-legacy is licensed as a whole under the GNU Affero General Public License, version 3 only (`AGPL-3.0-only`);
see [LICENSE](LICENSE). Mystic Paradox's additional terms under AGPLv3 section 7 apply to the Mystic Paradox material
only ([ADDITIONAL_TERMS.md](ADDITIONAL_TERMS.md)). This file records where the parts of the repository come from.

## Modification notice

This repository (`silentspud/dainty-legacy` on `git.silentspud.com`) is the legacy-import plugin of
[Dainty Lass](https://git.silentspud.com/silentspud/dainty-lass). It contains code and data ported from Dauntless
Revived, through its Dainty Lass fork [legacy-revived](https://git.silentspud.com/silentspud/legacy-revived), and the
account schema of Mystic Paradox's 1.14.7 port, through its Dainty Lass fork Rustic Isles
([rustic-isles](https://git.silentspud.com/silentspud/rustic-isles)).
It is **not** an official release of Dauntless Revived, Undaunted, Mystic Paradox or any other project named below,
and none of them endorses it. Every change is recorded in the commit history. It started on 31 October 2026.

Since 11 November 2026 the plugin and the `dainty-legacy` executable are Dainty Lass's MIT `crates/dainty-legacy`,
rewritten clean-room from a specification (`docs/port/spec-legacy-import.md`, RD27) and not derived from this code.
What stays here is the AGPL importer, kept as that rewrite's test oracle; nothing here ships.

## Where the parts come from

### Dauntless Revived

- [Dauntless Revived](https://github.com/mixutin/dauntless-revived) (`mixutin/dauntless-revived` at `e1c5fec`), as
  maintained in `silentspud/legacy-revived` `plan` at `fac6efd`, `AGPL-3.0-only`. Dauntless Revived is a modified
  version of Undaunted.
- Ported, with how each was changed:
  - `crates/dainty-legacy-import/sql/drizzle/*.sql`: the Node metagame's Drizzle migrations
    (`legacy/UndauntedMetagame/src/drizzle`), byte for byte, since their SHA-256 is checked.
  - `crates/dainty-legacy-import/migrations/revived/0001_baseline`: the Rust metagame's Diesel baseline
    (`rust/services/metagame/migrations/sqlite`), byte for byte.
  - `crates/dainty-legacy-import/src/revived/schema.rs`: `db.rs` and `drizzle_frozen.rs`, cut down to the schema.
  - `crates/dainty-legacy-import/src/revived/source.rs` and the offline-save checks in `revived/mod.rs`: `offline_save.rs`'s
    import and `db.rs`'s adoption, extended to server databases and Dainty Lass's change log.
  - `crates/dainty-legacy-import/src/revived/drsave.rs`: the Tauri launcher's `offline/save_format.rs`.

### Undaunted

- By gwog (Gregory Morford, [SyST3MDeV](https://github.com/SyST3MDeV)),
  [EisigesEis](https://github.com/EisigesEis) and
  [its other contributors](https://github.com/SyST3MDeV/Undaunted/graphs/contributors).
- Upstream: [github.com/SyST3MDeV/Undaunted](https://github.com/SyST3MDeV/Undaunted), `AGPL-3.0-only`. The metagame
  whose database this repository reads is Undaunted's work, modified by Dauntless Revived.

### Mystic Paradox

> "Mystic Paradox's Dauntless 1.12.0 and 1.14.7 ports and related modifications were developed by MysticFox / Pranav
> Karande. See NOTICE.md for contribution and provenance information."

- [Mystic Paradox](https://github.com/pranav158/Mystic-Paradox), Copyright (C) 2026 MysticFox / Pranav Karande,
  `AGPL-3.0-only` with the additional terms in [ADDITIONAL_TERMS.md](ADDITIONAL_TERMS.md), as ported to Rust in
  `silentspud/rustic-isles` `plan` at `af9c125`.
- Used here: the 1.14.7 account tables an import writes (`crates/dainty-legacy-import/src/target.rs` writes them as
  `isles-core`'s migrations `0001_accounts`, `0020_auth` and `0200_reforged_invites` make them, and
  `crates/dainty-legacy-import/src/testing/isles-1-14-7.sql` copies those migrations for the tests), isles-core's soft-delete
  table list, and `isles-db`'s account-id derivation (which dainty-lass's MIT `rustic-isles-db` keeps).

### Dainty Lass

- [Dainty Lass](https://git.silentspud.com/silentspud/dainty-lass) `plan` at `fb46a4a`, Copyright (c) 2026 the
  Dainty Lass contributors, MIT ([LICENSE-MIT.txt](LICENSE-MIT.txt)).
- `module-api` and `store` are dependencies, not copies.
- Ported from the host kit (`rust/crates/dainty-hostkit`), with the MIT notice kept in each file:
  `crates/dainty-legacy-import/src/reforged/layout.rs` (`commands/setup/paradox.rs`, `paths.rs`),
  `reforged/hostjson.rs` (`hosttoml.rs`) and `reforged/mongo.rs` (`commands/update.rs`, `commands/db.rs`).

No game files and no player data are in this repository: every database the tests use is built when they run.
"Dauntless" is a trademark of its owners; dainty-legacy is not affiliated with or endorsed by Phoenix Labs or Epic
Games.
