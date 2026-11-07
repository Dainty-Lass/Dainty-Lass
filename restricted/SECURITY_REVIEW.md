# Security Review Index

Index of trust-boundary security checklists (`docs/security/*.md`, RX3/DX2) and `cargo fuzz` targets (`fuzz/fuzz_targets/*.rs`) for `restricted/`'s AGPL components. The MIT components' checklists are in [../SECURITY_REVIEW.md](../SECURITY_REVIEW.md) (RD27), the gateway's, the allowlist's, content's and chat's among them; the libraries built here from those crates are covered there.

## Checklists (`docs/security/`)

| Component | Checklist | Fuzz target(s) (`fuzz/fuzz_targets/`) | Row |
| --- | --- | --- | --- |
| Module panels (declarative UI schema, action commands, bridge validation) | [docs/security/module-panels.md](docs/security/module-panels.md) | `module_frame.rs` | DF8, DU3 |
| Host kit (`setup`, `host.toml`, `secrets.env`, `start`, `serve`, `update` MongoDB migration & verification) | [docs/security/host-start.md](docs/security/host-start.md) | `host_config.rs`, `host_console.rs` | RH1–RH3, RH6, RU9 |
| Host game-data and SDK pipeline (`gamedata` in-process Rust generators, `sdk`, `build-runtime`, `publish`) | [docs/security/host-gamedata.md](docs/security/host-gamedata.md) | `host_gamedata_input.rs` | RH4, RH5, RB11 |
| TURN relay (`turn-rs` listener exception, `turn-rest` shared secret, credential lifetimes, relay-only mode, UDP relay port range) | [docs/security/turn-relay.md](docs/security/turn-relay.md) | dainty-lass's `allowlist_input.rs` | DD24, RF4, DX2 |
| Launcher keystore (OS credential store + AES-256-GCM fallback, account keys, Epic tokens) | [docs/security/keystore.md](docs/security/keystore.md) | `launcher_input.rs` | RA3, DE2 |
| Launcher network client (pinned TLS, SSRF blocks, redirects, server probe) | [docs/security/launcher-net.md](docs/security/launcher-net.md) | `launcher_net.rs`, `launcher_files.rs` | RA5, RA6 |
| Launcher region ping (pinned TLS RTT measurement, `ServerHello` timing) | [docs/security/region-ping.md](docs/security/region-ping.md) | dainty-lass's `launcher_region.rs` | DN7 |
| Launcher runtime download and loader verification (`mystic_loader.ini`, SHA-256) | [docs/security/runtime-download.md](docs/security/runtime-download.md) | dainty-lass's `runtime_input.rs` | RA8, RA10 |
| Launcher local loopback relay (`127.0.0.1:61100`, PID owner check, XMPP upgrade, mid-session re-point & key stripping) | [docs/security/relay.md](docs/security/relay.md) | `relay_input.rs` | RA9, DN6 |
| Launcher Host page and supervised `dainty-host` driver | [docs/security/host-page.md](docs/security/host-page.md) | dainty-lass's `launcher_host.rs` | RA11, RA12 |

## Game module checklists

| Module | Checklist | Fuzz targets | Row |
| --- | --- | --- | --- |
| `modules/rustic-isles` | None here: the 1.14.7 module is dainty-lass's MIT `rustic-isles-*` crates (RD27), checked in dainty-lass's `docs/security/rustic-isles*.md` | dainty-lass's `fuzz/fuzz_targets/rustic_isles_*.rs` | RU5–RU7, RB11, RB13, RU9 |
| `modules/rustic-rifts` | None here: the rifts crates are the root's MIT `rustic-rifts-*` (RD27), checked in the root's `docs/security/rifts-transport.md`, `rifts-guard.md` and `rifts-host.md`. The four pages in `modules/rustic-rifts/docs/security/` describe the retired AGPL crates and are kept for reference; [relay-credentials.md](modules/rustic-rifts/docs/security/relay-credentials.md) has no MIT counterpart yet | the root's `fuzz/fuzz_targets/` (`admission_control.rs`, `admission_ticket.rs`, `guard_evidence.rs`, `guard_grants.rs`, `relay_signal.rs`, `transport_*.rs`) | RF3–RF7, RD27 |
