# dainty-legacy

Old installs, carried into [DaintyLass](https://git.silentspud.com/silentspud/dainty-lass). DaintyLass serves one
game, Dauntless 1.14.7 on [rustic-isles](https://git.silentspud.com/silentspud/rustic-isles); this plugin is what is
left of everything older:

| Import | From | What carries |
| --- | --- | --- |
| `revived` | A Dauntless Revived 1.4.4 server's metagame database (SQLite, made by the Node or the Rust metagame) | Accounts, active bans and timeouts, Discord links, invite codes |
| `offline-save` | A player's 1.4.4 offline save: a `.drsave` file or the bare offline database | That player's account and Discord link |
| `reforged` | A Dauntless Reforged Revival Node-kit server: `host.json`, `secrets.env`, the old `.workspace` layout, MongoDB | The whole install, upgraded in place |

Characters, inventory and progression don't map across game versions: a 1.4.4 import counts them and leaves them
behind. 1.4.4 account keys don't carry either (1.4.4 kept SHA-256 hashes, 1.14.7 keeps HMACs), so an imported player
signs in through the launcher with their Discord account. Every import reports what it wrote, what was already there,
what it skipped and why, and what it left behind; `--dry-run` writes nothing. Sources are never written.

## Running it

It is a DaintyLass plugin (`ModuleKind::Plugin`, id `legacy`, for 1.14.7) that runs as its own process (DD5), so its
AGPL code never links into DaintyLass's MIT core. From the host's console, while DaintyLass runs:

```text
dainty-host legacy import revived      /srv/revived/metagame.db --env-file /srv/dainty/.workspace/config/backend.env
dainty-host legacy import offline-save /home/sam/Sam.drsave     --to /srv/dainty/isles.db --dry-run
```

`dainty-host legacy import <kind>` is `module run legacy import-<kind>`; only the console may run one, not the admin
API. The same executable runs an import from the command line, which is how an old Node kit is upgraded before
DaintyLass first starts:

```text
dainty-legacy import reforged /srv/old-kit --to /srv/dainty/isles.db --db-tool /srv/dainty/dainty-host
dainty-legacy import reforged /srv/old-kit --fresh-start
```

Then run `dainty-host setup`. The options:

- `--to <file>` or `--env-file <file>`: the 1.14.7 database, a SQLite file or the backend's env file the host kit
  writes (which also gives a Postgres database). The 1.14.7 server must have made it: start it once first.
- `--node <id>`: the node imported accounts are homed on (DD18); the running node by default, `local` on the command
  line.
- `--fresh-start` (`reforged` only): start 1.14.7 on a new database; MongoDB's files are kept as they are.
- `--db-tool <program>` (`reforged` only): the host kit's `dainty-host`, whose `db migrate` and `db verify` move and
  verify MongoDB; `dainty-host` from `PATH` by default. It finds the server folder from the working directory or its
  own location, so name the one in the new install. `--isles-db` is refused since isles-db was retired (RD27).

Every path is absolute. Only SQLite 1.4.4 databases are read: a 1.4.4 metagame that ran on Postgres is moved to
SQLite first (DaintyLass's `store::transfer::move_database`).

## Building

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked --no-fail-fast
```

The toolchain is pinned in `rust-toolchain.toml`. DaintyLass's `module-api` and `store` are path dependencies on the
monorepo root's crates (`../../crates/`). Fuzz targets are in `fuzz/` (`cargo +nightly fuzz run
database_readers`); the security checklist is [docs/security/importer.md](docs/security/importer.md), and what
dainty-lass removes now that this exists is [docs/dainty-lass-handover.md](docs/dainty-lass-handover.md).

## Layout

| Path | What it is |
| --- | --- |
| `crates/legacy-import` | The importers: `revived` (1.4.4 databases and saves), `reforged` (Node kits), `target` (1.14.7 accounts), `cli` |
| `crates/legacy-import/sql`, `migrations` | Dauntless Revived's frozen Drizzle history and Diesel baseline, byte for byte |
| `crates/legacy` | The plugin and the `dainty-legacy` executable |
| `fuzz/` | `cargo fuzz` targets, a workspace of its own |

## Licence

`AGPL-3.0-only` ([LICENSE](LICENSE)), with Mystic Paradox's additional terms for its material
([ADDITIONAL_TERMS.md](ADDITIONAL_TERMS.md)) and DaintyLass's MIT notice for the host-kit code ported here
([LICENSE-MIT.txt](LICENSE-MIT.txt)). Where each part comes from: [NOTICE.md](NOTICE.md).
