# Clean-room record: the root workspace files (RD27)

The root `Cargo.toml`'s workspace layout, `deny.toml`, `clippy.toml`, `rustfmt.toml`, `rust-toolchain.toml` and
`.cargo/config.toml` began as copies of Dauntless Revived's. They were rewritten from the list below: what each file
must do for this repository, stated without the old text. Every setting is a project fact (a decision row, an
AGENTS.md rule or a pinned version), so the new files keep the behaviour and share only the values. The new files
use their own order, grouping and comments.

## What each file must do

### `Cargo.toml` (workspace)

- Resolver 3; every package is a directory under `crates/` (RD11).
- `fuzz/` (cargo fuzz, nightly) and `restricted/` (AGPL, RD27) are workspaces of their own and stay out.
- Shared package fields: version `0.0.0`, edition 2024, MSRV 1.99, licence MIT, the Forgejo repository URL,
  nothing published to crates.io.
- One version per dependency for the whole workspace, so `[workspace.dependencies]` lists every in-tree crate by
  path and every third-party crate at its pinned version and features (unchanged by this row; regrouped only).
  `dainty-module-api` carries `version` and `registry = "silentspud"` so it can be packaged (DF1), and
  `server-controller` stays as the alias restricted's launcher uses.
- Lints from AGENTS.md: rustc denies `unsafe_code` (only `dainty-plugin-host` opts out) and `unused_must_use`,
  and warns on `missing_docs`; clippy denies `unwrap_used`, `expect_used`, `dbg_macro` and `todo`, and warns on an
  `#[allow]` without a reason. CI runs clippy with `-D warnings`, so the warnings fail it too.
- Release builds: fat LTO, one codegen unit, `opt-level = 3`, and panics that unwind, because servers catch a
  panicking request instead of exiting.

### `clippy.toml`

- `unwrap` and `expect` are allowed in tests only.

### `rustfmt.toml`

- `max_width = 120`; everything else is rustfmt's default (edition comes from `Cargo.toml`).

### `rust-toolchain.toml`

- Rust 1.99.1, minimal profile, with `rustfmt` and `clippy`.

### `.cargo/config.toml`

- Windows MSVC builds link the C runtime statically (one file per platform, no redistributable).
- The bundled SQLite is compiled with the options that make SQL behave as the Node services' did: no
  double-quoted string literals, `LIKE` never matching BLOBs, the math and percentile functions, WAL with
  `synchronous=NORMAL`, and a 16 MB default page cache.
- `aws-lc-sys` uses its prebuilt NASM objects, so Windows builds need no NASM (D10).
- `DAINTY_RELEASE_REPOSITORY` defaults to `SilentSpud/DaintyLass`; the environment overrides it.
- The `silentspud` registry is Forgejo's Cargo index, so a dependency naming it resolves.

### `deny.toml`

- Checked with all features on.
- Advisories: yanked crates are refused; two unmaintained, build-path crates with no known vulnerability are
  ignored with a reason (`RUSTSEC-2024-0370` via GTK3 under Tauri, `RUSTSEC-2026-0441` via webrtc-util's nix).
- Licences: only those that can ship inside the AGPL-3.0-only release built from `restricted/` (AGPL-3.0-only,
  Apache-2.0 with or without the LLVM exception, BSD-2/3-Clause, BSL-1.0, CDLA-Permissive-2.0, ISC, MIT, MIT-0,
  MPL-2.0, OpenSSL, Unicode-3.0, Zlib), at confidence 0.9, without complaining about entries no crate uses.
- Bans: duplicate versions warn; wildcard requirements are refused except for path dependencies; one
  `libsqlite3-sys` per build (M17); no `openssl-sys` (D10) and no `pq-sys` (Postgres goes through tokio-postgres).
- Sources: crates.io only, no unknown registry or git source.

## Check

The rewrite kept `cargo metadata` (packages, resolved dependencies and features) and `Cargo.lock` identical, and
`cargo fmt --check`, clippy with `-D warnings`, the tests and `cargo deny check` passed at the root, with
`restricted/` still checking against the root crates.
