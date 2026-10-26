<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# Spike: `pq-sys` bundled under `MultiConnection` (DF7)

Can `store` offer Postgres beside SQLite through Diesel's `MultiConnection` (DD15) with a bundled libpq, so no
host or developer installs one, on Windows MSVC and Linux? First planned as RB3's spike under RD15; DF7 carries it.
Run on 2026-10-26.

**Verdict: yes, with `pq-sys`'s `bundled_without_openssl` feature, not `bundled`.** It builds and runs on all three
targets with no system library and no new tool, and keeps `openssl-sys` banned. The price is no TLS to Postgres,
which loopback and a Compose network don't need. Two additions to `deny.toml` and one to `NOTICE.md`, below.

## Setup

A toy crate outside the repository, on the workspace's versions: Diesel `=2.3.14` with the workspace's features plus
`postgres`, `diesel_migrations` 2.3, `libsqlite3-sys` 0.38 bundled, toolchain 1.99.1, and on Windows the
workspace's `+crt-static`. It had a `#[derive(MultiConnection)]` enum over `PgConnection` and `SqliteConnection`,
one table (`Integer` key, `Text`, `Bool`, `BigInt`, `Nullable<Binary>`), one migration per dialect, and ran on both
engines: the migrations, an insert in a transaction, an insert with `RETURNING`, a select, raw SQL with binds, a
raw upsert and an r2d2 pool over the enum. Postgres was a throwaway `postgres:17` container on 127.0.0.1 with
SCRAM-SHA-256 passwords and TLS turned on, so both plaintext and TLS clients could be tried.

`pq-sys` 0.7.6 builds libpq from `pq-src` 0.3.13 (libpq 18.6). Its features:

- `bundled` = libpq plus `pq-src/with-openssl`, which needs `openssl-sys`: a system OpenSSL, or OpenSSL built
  from source with `openssl-sys/vendored`.
- `bundled_without_openssl` = libpq alone. Password auth (SCRAM) still works; libpq has its own SHA-256.

## Result per OS

Clean release builds of the toy, every dependency included, with the registry already fetched. Binaries stripped.
Linux ran in Docker Desktop on the same PC, so its times compare with each other, not with Windows.

| Build | Windows MSVC | Debian trixie (`rust:1.99.1-slim-trixie`) | Alpine musl (`rust:1.99.1-alpine`) |
| --- | --- | --- | --- |
| SQLite only (today) | 20.4 s, 2.15 MB | 31 s, 2.60 MB | 57 s, 2.74 MB |
| + Postgres, `bundled_without_openssl` | **73.0 s, 2.77 MB (+0.62 MB)** | **34 s, 3.12 MB (+0.52 MB)** | **82 s, 3.30 MB (+0.56 MB)** |
| + Postgres, `bundled` (system OpenSSL) | fails: no OpenSSL found | fails until `libssl-dev`; then 32 s, 3.13 MB, links `libssl.so.3`, `libcrypto.so.3` | fails until `openssl-dev openssl-libs-static`; then 74 s, 8.87 MB static |
| + Postgres, `bundled` + `openssl-sys/vendored` | fails: needs a native Perl | 37 s, 8.76 MB (+6.2 MB) | 81 s, 8.92 MB |

- **`bundled_without_openssl`** worked against Postgres on every target, through the same apt line the repository's
  `docker/Dockerfile` already runs (`build-essential pkg-config`). On Windows the binary imports only system DLLs
  (`Secur32`, `SHELL32`, `ADVAPI32`, `WS2_32` added): no `libpq.dll`, no VC++ redistributable (M38 holds). On
  Linux, `ldd` shows nothing beyond libc. The default `sslmode=prefer` connects in plaintext.
  `sslmode=require` is refused at connect: `sslmode value "require" invalid when SSL support is not compiled in`.
- **System OpenSSL** breaks the one-file binary (M38): every runtime image (`debian:trixie-slim`,
  `node:22-trixie-slim`) and every host would need `libssl3`, and Windows has no system OpenSSL.
- **Vendored OpenSSL on MSVC** was not built. OpenSSL's `Configure` needs a native Windows Perl (Strawberry);
  Git for Windows' MSYS Perl fails (`Can't locate Locale/Maketext/Simple.pm`, and OpenSSL doesn't support MSYS
  Perl for `VC-WIN64A`). `openssl-src` passes `no-asm`, so NASM isn't needed. It would add Perl to every
  developer's PC and to DX3's MSVC runner, add about 6 MB per binary, and bring in a second TLS stack beside
  rustls (D10). Each TLS build connected with `ssl = true`, and `sslmode=require` worked.
- The one extra cost of `bundled_without_openssl` is compile time on MSVC: `pq-src` compiles about 100 C files with
  `cl.exe` once per clean build (+53 s here, +3 s on trixie).

## What `MultiConnection` can't express

Diesel 2.3.14, checked by compiling it and by running it on both engines. **Works through the enum:**
`insert_into(..).values(..)` for one row, `.returning(..)` / `get_result` (SQLite ≥ 3.35, already on), selects,
`transaction`, `Bool`, `BigInt`, `Binary`, `Nullable`, r2d2 pools of the enum, and `MigrationHarness`, though each
dialect is still its own `embed_migrations!` set, picked by variant.

**Fails to compile through the enum:**

- **`on_conflict(..)`, `on_conflict_do_nothing()`.** `MultiBackend` declares `DoesNotSupportOnConflictClause`. The
  metagame has 19 (RV3). Both engines accept the same text, which was run on both:
  `INSERT .. ON CONFLICT (id) DO UPDATE SET body = excluded.body` through `sql_query`. The other way is a match on
  the variant that runs the same DSL on the inner connection, written once with a macro.
- **`insert_or_ignore_into`**: SQLite only. Use `ON CONFLICT DO NOTHING` instead.
- **Batch insert** (`.values(&vec![..])`) fails on `InsertWithDefaultKeyword`. Match on the variant, or insert one row
  at a time inside the transaction.
- **`immediate_transaction`** is `SqliteConnection`'s. `store`'s writer (`BEGIN IMMEDIATE`) matches on the variant;
  Postgres takes a plain `transaction`.
- **Types beyond** integers, floats, text, binary, bool, date, time and timestamp: `jsonb`, arrays,
  `timestamptz` and UUIDs each need hand-written `HasSqlType`, `FromSql` and `ToSql` for `MultiBackend` and for
  each engine.

**Compiles but differs at run time (raw SQL):**

- **Placeholders.** `?` is a syntax error on Postgres. `$1` works on both, but SQLite treats `$N` as a *named*
  parameter numbered by first appearance: `SELECT $2 AS a, $1 AS b` swaps the two values on SQLite and not on
  Postgres (run on both). Rule for portable SQL: `$1..$n` in order of first appearance. The metagame has 52
  `sql_query` calls written with `?`.
- **SQL the dialects don't share.** `PRAGMA`, `datetime('now')`/`now()`, `json_extract`/`->>`,
  `group_concat`/`string_agg`, case in `LIKE`, and typing: `SET isAdmin = 1` fails on a Postgres `boolean`.
  Unquoted camelCase identifiers (`userId`) fold to lower case on Postgres. Diesel's DSL quotes them; raw SQL must
  too.
- **`AnyConnection::establish(url)`** tries each variant in order. With a wrong Postgres password it falls through to
  SQLite and reports only `Invalid connection url for multiconnection`, losing libpq's error. `store` should build
  the variant itself from the engine `host.toml` names.
- SQLite-only parts stay behind a variant match: PRAGMAs, the online backup (`rusqlite`), Drizzle adoption.

## Proposed `[workspace.dependencies]` lines (not applied)

The workspace `Cargo.toml` is a seam (section 5.1). DF7 would change the two Diesel lines and add one:

```toml
diesel = { version = "2.3", default-features = false, features = ["sqlite", "postgres", "r2d2", "returning_clauses_for_sqlite_3_35"] }
diesel_migrations = { version = "2.3", features = ["sqlite", "postgres"] }
# Bundled libpq without OpenSSL: no system libpq, no Perl on Windows, openssl-sys stays banned (D10, DF7 spike).
pq-sys = { version = "0.7", default-features = false, features = ["bundled_without_openssl"] }
```

`store` then adds `pq-sys.workspace = true` (only to set the feature; Diesel pulls `pq-sys` itself). The new crates
are `pq-sys` (MIT OR Apache-2.0) and `pq-src` (PostgreSQL). `bitflags` 2, `byteorder`, `itoa`, `libc`, `cc`,
`vcpkg` and `pkg-config` are already in `Cargo.lock`.

## `deny.toml`

`cargo deny check licenses bans` with the repository's `deny.toml` on the toy:

- **`pq-src`'s `PostgreSQL` licence is rejected**: it isn't in `[licenses] allow`. Add `"PostgreSQL"`, an OSI-approved
  permissive licence (BSD/MIT style) that fits AGPL-3.0-only. Nothing else in the `bundled_without_openssl` graph
  fails, and the bans pass.
- **`openssl-sys` stays banned.** Under `bundled` it fails `bans`. The ban is what keeps a later change from
  switching to `bundled` quietly, so leave it. A feature ban makes the reason explicit:
  `features = [{ crate = "pq-sys", deny = ["bundled"] }]` under `[bans]`.
- Tested with both changes: the `bundled_without_openssl` graph passes, and `bundled` fails on both the crate ban
  and the feature ban. `[graph] all-features = true` turns on only workspace members' features, so a member feature
  that turned on `pq-sys/bundled` would be caught too.
- **`NOTICE.md`** (integration pass): libpq is linked into every binary that links `store`. The PostgreSQL licence
  asks for its copyright notice to ship with them.

## For DF7

- Postgres connections are plaintext. The host kit's Postgres (RH3, RD15) binds 127.0.0.1 and Compose's runs on the
  internal network, so that is enough. A host's own `postgres_url` pointing off the machine has no TLS: `setup`
  should refuse a non-loopback host, or the hosting docs should say it isn't supported. If remote TLS is needed
  later, the choices are vendored OpenSSL (Perl on Windows, lifting the ban) or a libpq-free driver, which
  `MultiConnection` can't wrap.
- RV3's metagame queries are the bulk of the porting work: the upserts, the `?` placeholders and the
  dialect-specific raw SQL above.
- Not tried: Unix-socket connections, Windows ARM64, the launcher (which keeps no database, DD15).
