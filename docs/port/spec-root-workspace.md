# Spec: the root workspace files (RD27)

What the root MIT workspace's six configuration files must do. Facts only: the rewrite is written from this page
(record: `clean-room/root-workspace.md`). Acceptance: `cargo metadata` resolves the same packages, `Cargo.lock`
doesn't change, and `cargo fmt --check`, clippy with `-D warnings` and `cargo deny check` give the same results.

## 1. `Cargo.toml`

### 1.1 Workspace

| Key | Value |
| --- | --- |
| members | every directory in `crates/` (glob) |
| exclude | `fuzz` (RB15 removed restricted's workspace) |
| resolver | 3 |

### 1.2 Inherited package fields

| Field | Value |
| --- | --- |
| version | `0.0.0` |
| edition | 2024 |
| rust-version | 1.99 |
| license | MIT |
| publish | false |
| repository | `https://git.silentspud.com/silentspud/dainty-lass` |

### 1.3 Lints

| Tool | Lint | Level |
| --- | --- | --- |
| rustc | `unsafe_code` | deny |
| rustc | `unused_must_use` | deny |
| rustc | `missing_docs` | warn |
| rustc | `unexpected_cfgs` | warn, with check-cfg `cfg(tokio_unstable)` (docs/tokio-console.md) |
| clippy | `unwrap_used`, `expect_used`, `dbg_macro`, `todo` | deny |
| clippy | `allow_attributes_without_reason` | warn |

### 1.4 Release profile

`opt-level = 3`, `lto = "fat"`, `codegen-units = 1`, `panic = "unwind"` (request handlers catch panics).

### 1.5 In-tree dependencies

Each is `{ path = "crates/<dir>" }` unless the table says otherwise.

| Key | Directory | Extra |
| --- | --- | --- |
| `dainty-admin`, `dainty-chat`, `dainty-content`, `dainty-controller`, `dainty-epic`, `dainty-gateway`, `dainty-history-ship`, `dainty-http`, `dainty-host`, `dainty-hostkit`, `dainty-inject`, `dainty-launcher-kit`, `dainty-manager-server`, `dainty-plugin-host`, `dainty-protocol`, `dainty-proxy`, `dainty-remote`, `dainty-service-kit`, `dainty-service-plugin`, `dainty-store` | same as key | — |
| `dainty-module-api` | same | `version = "0.1.0"`, `registry = "silentspud"` (DF1) |
| `network_plugin` | `dainty-network` | `package = "dainty-network"`, `default-features = false` |
| `rustic-rifts-admission`, `-guard`, `-host`, `-realms`, `-relay`, `-seam`, `-transport` | same as key | — |
| `rustic-isles`, `rustic-isles-1-14-7`, `rustic-isles-1-14-7-plugin`, `rustic-isles-auth`, `-characters`, `-chat`, `-db`, `-director`, `-economy`, `-gamedata`, `-launcher-api`, `-matchmaking`, `-replay`, `-store` | same as key | — |

### 1.6 Third-party dependencies

`dnf` = `default-features = false`. Features are listed as a set; their order carries no meaning.

| Crate | Version | dnf | Features |
| --- | --- | --- | --- |
| aes | 0.9 | | |
| aes-gcm | 0.11 | | |
| anstream | 1 | | |
| argon2 | 0.5 | yes | alloc, password-hash |
| async-graphql | 7 | yes | |
| aws-lc-rs | 1.18 | | |
| axum | 0.8 | yes | http1, json, query, tokio, form, ws |
| base64 | 0.23 | | |
| bytes | 1 | | |
| cbc | 0.2 | | alloc |
| chrono | 0.4 | yes | clock, std |
| clap | 4 | | derive |
| console-subscriber | 0.5 | yes | |
| crossterm | 0.29 | | |
| dbus-secret-service | 4.1 | | crypto-rust |
| diesel | 2.3 | yes | sqlite, postgres_backend, r2d2, returning_clauses_for_sqlite_3_35, i-implement-a-third-party-backend-and-opt-into-breaking-changes |
| diesel-async | 0.9 | yes | postgres, async-connection-wrapper, r2d2 |
| diesel_migrations | 2.3 | | sqlite, postgres |
| dotenvy | 0.15 | | |
| egs-api | =0.14.0 | | |
| flate2 | 1 | | |
| futures-util | 0.3 | yes | sink, std |
| gethostname | 1 | | |
| hex | 0.4.3 | | |
| hmac | 0.13 | | |
| http | 1 | | |
| http-body | 1 | | |
| http-body-util | 0.1 | | |
| hyper | 1 | yes | client, server, http1 |
| hyper-rustls | 0.27 | yes | http1, aws-lc-rs, webpki-tokio, tls12 |
| hyper-util | 0.1 | yes | tokio, service |
| libsqlite3-sys | 0.38 | | bundled |
| md-5 | 0.11 | | |
| open | 5.4 | | shellexecute-on-windows |
| p256 | 0.14 | | ecdsa, pem, pkcs8, std |
| pbkdf2 | 0.13 | yes | hmac |
| percent-encoding | 2 | | |
| rand | 0.10 | | |
| ratatui | 0.30 | yes | crossterm |
| rcgen | 0.14.10 | yes | aws_lc_rs, pem, crypto |
| regex | 1 | | |
| rtc-ice | 0.21.1 | yes | crypto-aws-lc-rs |
| rtc-shared | 0.21.1 | yes | ifaces |
| rtc-turn | 0.21.1 | yes | crypto-aws-lc-rs |
| rusqlite | 0.40 | | backup |
| rustls | 0.23 | yes | aws_lc_rs, std, tls12 |
| sansio | 1.0.1 | | |
| serde | 1 | | derive |
| serde_json | 1 | | float_roundtrip |
| sha1 | 0.11 | | |
| sha2 | 0.11 | | |
| subtle | 2 | | |
| sys-locale | 0.3 | | |
| sysinfo | 0.33 | yes | system |
| tauri | 2.12 | | isolation, tray-icon |
| tauri-build | 2.7 | | isolation |
| tauri-plugin-dialog | 2.8 | | |
| tauri-plugin-updater | 2.14 | yes | rustls-tls, system-proxy |
| tempfile | 3 | | |
| thiserror | 2 | | |
| time | 0.3 | | |
| tokio | 1 | | rt, macros |
| tokio-metrics | 0.5 | yes | |
| tokio-rustls | 0.26 | yes | aws_lc_rs, tls12 |
| tokio-tungstenite | 0.29 | yes | connect, handshake |
| tokio-util | 0.7 | | io |
| toml | 0.9 | | |
| toml_edit | 0.25 | yes | parse, display |
| tower | 0.5 | | util |
| tracing | 0.1 | | |
| tracing-log | 0.2 | | |
| tracing-subscriber | 0.3 | | fmt, registry, std, tracing-log |
| ts-rs | 12 | | |
| turn | 0.17.2 | | |
| twilight-gateway | 0.18 | yes | rustls-webpki-roots |
| twilight-http | 0.18 | yes | rustls-webpki-roots |
| twilight-model | 0.18 | | |
| twilight-util | 0.18 | | builder |
| ulid | 3 | | |
| url | 2 | | |
| uuid | 1.28.0 | yes | |
| webpki-roots | 1 | | |
| webrtc-util | 0.17 | yes | conn, vnet |
| windows | 0.62.2 | | |
| x509-parser | 0.18.1 | | |

Reasons a comment may cite: TLS and crypto are aws-lc-rs, never OpenSSL (D10); Diesel and rusqlite share one bundled
SQLite (M17); Postgres goes through tokio-postgres, no libpq; diesel's third-party-backend feature is for the store's
Postgres newtype in `MultiConnection`; `p256` types are public in the admission ticket API; Epic pins by
SHA-1 with MD5 beside it (DD22); `egs-api` is sign-in and library only (DE1); `ts-rs` generates the bridge types (L6);
`toml_edit` keeps a host's comments in `settings.toml`; `dbus-secret-service` is Linux key storage (RL5); `windows` is
COM for Windows Firewall; `turn` is a test TURN server; the `rtc-*`
crates are sans-IO ICE and TURN (spec-rd27-rifts D2); `console-subscriber` is the `tokio-console` feature only;
`tokio-metrics` is per-module task metrics on `/metrics`.

## 2. `clippy.toml`

`allow-unwrap-in-tests = true`, `allow-expect-in-tests = true`. Nothing else.

## 3. `rustfmt.toml`

`max_width = 120`. Nothing else (edition comes from `Cargo.toml`).

## 4. `rust-toolchain.toml`

Channel `1.99.1`, profile `minimal`, components `rustfmt` and `clippy`.

## 5. `.cargo/config.toml`

| Section | Setting | Value | Why |
| --- | --- | --- | --- |
| env | `LIBSQLITE3_FLAGS` | `SQLITE_DQS=0 SQLITE_LIKE_DOESNT_MATCH_BLOBS SQLITE_ENABLE_MATH_FUNCTIONS SQLITE_ENABLE_PERCENTILE SQLITE_DEFAULT_WAL_SYNCHRONOUS=1 SQLITE_DEFAULT_CACHE_SIZE=-16000` (this order) | SQL results match the Node services' better-sqlite3 |
| env | `AWS_LC_SYS_PREBUILT_NASM` | `1` | no NASM on Windows (D10) |
| env | `DAINTY_RELEASE_REPOSITORY` | `SilentSpud/DaintyLass`, not forced | default release source; the environment wins |
| target `x86_64-pc-windows-msvc` | rustflags | `-C target-feature=+crt-static` | no VC++ redistributable (M38) |
| registries `silentspud` | index | `sparse+https://git.silentspud.com/api/packages/silentspud/cargo/` | DD2, DF1 |

## 6. `deny.toml`

- **Graph:** all features.
- **Sources:** unknown registries denied, unknown git denied (crates.io only).
- **Licences** (version 2, confidence 0.9, unused entries allowed, since the list is what the AGPL-3.0-only server release can carry):
  AGPL-3.0-only, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause, BSD-3-Clause, BSL-1.0, CDLA-Permissive-2.0,
  ISC, MIT, MIT-0, MPL-2.0, OpenSSL, Unicode-3.0, Zlib.
- **Bans:** multiple versions warn; wildcards denied, path wildcards allowed. Denied: `libsqlite3-sys` with more than
  one version (M17), `openssl-sys` (D10), `pq-sys`.
- **Advisories** (version 2): yanked denied. Ignored, each with a reason:

| ID | Crate, path | Reason |
| --- | --- | --- |
| RUSTSEC-2024-0370 | proc-macro-error via gtk-rs 0.18 (Tauri 2, Linux GTK3) | unmaintained, build time, no vulnerability |
| RUSTSEC-2026-0441 | pin-utils via nix 0.26 (webrtc-util) | archived, no vulnerability |
| RUSTSEC-2026-0459 | bundled SQLite < 3.54, crafted sibling `-wal`, via libsqlite3-sys 0.38 | diesel 2.3.14 caps libsqlite3-sys < 0.39 (fix on its main, fcd8d71e); nothing opens a less-trusted database with its `-wal`; drop on diesel 2.3.15 |
