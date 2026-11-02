# Security Review Index

Index of trust-boundary security checklists (`docs/security/*.md`, RX3/DX2) and `cargo fuzz` targets (`fuzz/fuzz_targets/*.rs`) for dainty-restricted's AGPL components. The MIT components' checklists are dainty-lass's (RD27).

## Checklists (`docs/security/`)

| Component | Checklist | Fuzz target(s) (`fuzz/fuzz_targets/`) | Row |
| --- | --- | --- | --- |
| Gateway (TLS, origin routing, rate limiting, capture, module routes, node links) | [docs/security/gateway.md](docs/security/gateway.md) | `gateway_policy.rs`, `gateway_policy_legacy.rs`, `gateway_capture.rs`, `gateway_modules.rs`, `gateway_network.rs` | RS1, DF4a, DN3 |
| Allowlist (Windows Firewall / nftables UDP port admission, module port ranges) | [docs/security/allowlist.md](docs/security/allowlist.md) | `allowlist_input.rs` | RS2, DF4d |
| Content server (runtime manifests, DLL downloads, branding, news, module versions) | [docs/security/content.md](docs/security/content.md) | `content_request.rs`, `content_range.rs`, `content_news.rs`, `content_runtime_index.rs` | RS3, DF4b |
| Module panels (declarative UI schema, action commands, bridge validation) | [docs/security/module-panels.md](docs/security/module-panels.md) | `module_frame.rs` | DF8, DU3 |
| Host kit (`setup`, `host.toml`, `secrets.env`, `start`, `serve`, `update` MongoDB migration & verification) | [docs/security/host-start.md](docs/security/host-start.md) | `host_config.rs`, `host_console.rs` | RH1–RH3, RH6, RU9 |
| Host game-data and SDK pipeline (`gamedata` in-process Rust generators, `sdk`, `build-runtime`, `publish`) | [docs/security/host-gamedata.md](docs/security/host-gamedata.md) | `host_gamedata_input.rs` | RH4, RH5, RB11 |
| TURN relay (`turn-rs` listener exception, `turn-rest` shared secret, credential lifetimes, relay-only mode, UDP relay port range) | [docs/security/turn-relay.md](docs/security/turn-relay.md) | `allowlist_input.rs`, `modules/rustic-rifts/fuzz/fuzz_targets/turn_framing.rs` | DD24, RF4, DX2 |
| Chat plugin (`plugins/chat`: XMPP over WebSocket, MUC, whispers, federation) | [docs/security/chat.md](docs/security/chat.md) | `chat_stanza.rs`, `chat_federation.rs` | DN10a, DN10b |
| Launcher keystore (OS credential store + AES-256-GCM fallback, account keys, Epic tokens) | [docs/security/keystore.md](docs/security/keystore.md) | `launcher_input.rs` | RA3, DE2 |
| Launcher network client (pinned TLS, SSRF blocks, redirects, server probe) | [docs/security/launcher-net.md](docs/security/launcher-net.md) | `launcher_net.rs`, `launcher_files.rs` | RA5, RA6 |
| Launcher region ping (pinned TLS RTT measurement, `ServerHello` timing) | [docs/security/region-ping.md](docs/security/region-ping.md) | `launcher_region.rs` | DN7 |
| Launcher runtime download and loader verification (`mystic_loader.ini`, SHA-256) | [docs/security/runtime-download.md](docs/security/runtime-download.md) | `runtime_input.rs` | RA8, RA10 |
| Launcher local loopback relay (`127.0.0.1:61100`, PID owner check, XMPP upgrade, mid-session re-point & key stripping) | [docs/security/relay.md](docs/security/relay.md) | `relay_input.rs` | RA9, DN6 |
| Launcher Host page and supervised `dainty-lass-host` driver | [docs/security/host-page.md](docs/security/host-page.md) | `launcher_host.rs` | RA11, RA12 |

## Module repository checklists

| --- | --- | --- | --- |
| `modules/rustic-isles` | [modules/rustic-isles/docs/security/db-migrate.md](modules/rustic-isles/docs/security/db-migrate.md), [rate-limit.md](modules/rustic-isles/docs/security/rate-limit.md) | `modules/rustic-isles/fuzz/fuzz_targets/` (`auth_token.rs`, `bounties_store.rs`, `characters_loadouts.rs`, `chat_presence.rs`, `director_protocol.rs`, `gamedata_config.rs`, `inventory_payload.rs`, `matchmaking_tickets.rs`, `mongo_migrate.rs`, `progression_grant.rs`, `reforged_api.rs`) | RU5–RU7, RB11, RB13, RU9 |
| `modules/rustic-rifts` | [modules/rustic-rifts/docs/security/rifts-transport.md](modules/rustic-rifts/docs/security/rifts-transport.md), [rifts-guard.md](modules/rustic-rifts/docs/security/rifts-guard.md), [rifts-server.md](modules/rustic-rifts/docs/security/rifts-server.md), [rifts-host-bridge.md](modules/rustic-rifts/docs/security/rifts-host-bridge.md) | `modules/rustic-rifts/fuzz/fuzz_targets/` (`abi_C_strings.rs`, `envelope_parse.rs`, `guard_attest.rs`, `host_bridge_parse.rs`, `realm_invite.rs`, `turn_framing.rs`) | RF3–RF7 |
