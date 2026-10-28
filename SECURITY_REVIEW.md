# Security Review Index

Index of trust-boundary security checklists (`docs/security/*.md`, RX3/DX2) and `cargo fuzz` targets (`rust/fuzz/fuzz_targets/*.rs`) across `dainty-lass` and its module submodules.

## Checklists (`docs/security/`)

| Component | Checklist | Fuzz target(s) (`rust/fuzz/fuzz_targets/`) | Row |
| --- | --- | --- | --- |
| Protocol (invites, account keys, statuses, usernames) | [docs/local-contracts.md](docs/local-contracts.md) | `protocol_input.rs`, `http_body.rs`, `proxy_headers.rs` | RA1 |
| Gateway (TLS, origin routing, rate limiting, capture, module routes, node links) | [docs/security/gateway.md](docs/security/gateway.md) | `gateway_policy.rs`, `gateway_policy_legacy.rs`, `gateway_capture.rs`, `gateway_modules.rs`, `gateway_network.rs` | RS1, DF4a, DN3 |
| Allowlist (Windows Firewall / nftables UDP port admission, module port ranges) | [docs/security/allowlist.md](docs/security/allowlist.md) | `allowlist_input.rs` | RS2, DF4d |
| Content server (runtime manifests, DLL downloads, branding, news, module versions) | [docs/security/content.md](docs/security/content.md) | `content_request.rs`, `content_range.rs`, `content_news.rs`, `content_runtime_index.rs` | RS3, DF4b |
| Admin GraphQL API (keys, roles, guarded actions, history, module control, network) | [docs/security/admin-api.md](docs/security/admin-api.md) | `admin_request.rs`, `network_admin.rs` | RS4, DF4c, DH2, DH3, DH6, DN11 |
| Controller (in-process service supervision, module registry) | [docs/security/controller.md](docs/security/controller.md) | `host_console.rs` | RC1, DF3 |
| Module host (`module-api`, length-prefixed framing, SHA-256 verification, capabilities) | [docs/security/module-host.md](docs/security/module-host.md) | `module_frame.rs` | DF2 |
| Module panels (declarative UI schema, action commands, bridge validation) | [docs/security/module-panels.md](docs/security/module-panels.md) | `module_frame.rs` | DF8, DU3 |
| Host kit (`setup`, `host.toml`, `secrets.env`, `start`, `serve`, `update`) | [docs/security/host-start.md](docs/security/host-start.md) | `host_config.rs`, `host_console.rs` | RH1–RH3, RH6 |
| Host game-data and SDK pipeline (`gamedata`, `sdk`, `build-runtime`, `publish`) | [docs/security/host-gamedata.md](docs/security/host-gamedata.md) | `host_gamedata_input.rs` | RH4, RH5 |
| Change log, hash chain, revert, object-locked bucket and peer log shipping | [docs/security/history.md](docs/security/history.md) | `history_input.rs`, `history_record.rs`, `history_bucket.rs`, `network_logship.rs` | DH1, DH3–DH5, DH7 |
| Server network (CA, mTLS node links, directory, home API, session tickets) | [docs/security/network.md](docs/security/network.md) | `network_link.rs`, `network_directory.rs`, `network_home.rs`, `network_ticket.rs`, `network_admin.rs`, `network_logship.rs` | DN2–DN5, DN11 |
| Chat plugin (`rust/plugins/chat`: XMPP over WebSocket, MUC, whispers, federation) | [docs/security/chat.md](docs/security/chat.md) | `chat_stanza.rs`, `chat_federation.rs` | DN10a, DN10b |
| Epic Games login, manifest parser and CDN chunk installer (`crates/epic`, host, launcher) | [docs/security/epic.md](docs/security/epic.md) | `epic_manifest.rs` | DE1–DE6 |
| Launcher keystore (OS credential store + AES-256-GCM fallback, account keys, Epic tokens) | [docs/security/keystore.md](docs/security/keystore.md) | `launcher_input.rs` | RA3, DE2 |
| Launcher network client (pinned TLS, SSRF blocks, redirects, server probe) | [docs/security/launcher-net.md](docs/security/launcher-net.md) | `launcher_net.rs`, `launcher_files.rs` | RA5, RA6 |
| Launcher region ping (pinned TLS RTT measurement, `ServerHello` timing) | [docs/security/region-ping.md](docs/security/region-ping.md) | `launcher_region.rs` | DN7 |
| Launcher runtime download and loader verification (`mystic_loader.ini`, SHA-256) | [docs/security/runtime-download.md](docs/security/runtime-download.md) | `runtime_input.rs` | RA8, RA10 |
| Launcher local loopback relay (`127.0.0.1:61100`, PID owner check, XMPP upgrade) | [docs/security/relay.md](docs/security/relay.md) | `relay_input.rs` | RA9 |
| Launcher Host page and supervised `dainty-lass-host` driver | [docs/security/host-page.md](docs/security/host-page.md) | `launcher_host.rs` | RA11, RA12 |
| Launcher 1.4.4 legacy client path (`dauntless-revived://`, client patcher, relay) | [docs/security/launcher-legacy.md](docs/security/launcher-legacy.md) | `launcher_legacy.rs` | RL1–RL5 |

## Module repository checklists

| Repository | Checklist | Fuzz targets | Row |
| --- | --- | --- | --- |
| `modules/rustic-isles` | [modules/rustic-isles/docs/security/db-migrate.md](modules/rustic-isles/docs/security/db-migrate.md) | `modules/rustic-isles/fuzz/fuzz_targets/` (`auth_token.rs`, `bounties_store.rs`, `characters_loadouts.rs`, `chat_presence.rs`, `director_protocol.rs`, `gamedata_config.rs`, `inventory_payload.rs`, `matchmaking_tickets.rs`, `mongo_migrate.rs`, `progression_grant.rs`, `reforged_api.rs`) | RU5–RU7 |
| `modules/rustic-rifts` | [modules/rustic-rifts/docs/security/rifts-transport.md](modules/rustic-rifts/docs/security/rifts-transport.md), [rifts-guard.md](modules/rustic-rifts/docs/security/rifts-guard.md), [rifts-server.md](modules/rustic-rifts/docs/security/rifts-server.md), [rifts-host-bridge.md](modules/rustic-rifts/docs/security/rifts-host-bridge.md) | `modules/rustic-rifts/fuzz/fuzz_targets/` (`abi_C_strings.rs`, `envelope_parse.rs`, `guard_attest.rs`, `host_bridge_parse.rs`, `realm_invite.rs`, `turn_framing.rs`) | RF3–RF7 |
| `modules/legacy-revived` | [modules/legacy-revived/docs/security/metagame.md](modules/legacy-revived/docs/security/metagame.md), [deploy.md](modules/legacy-revived/docs/security/deploy.md) | `modules/legacy-revived/rust/fuzz/fuzz_targets/` (`metagame_input.rs`, `metagame_Auth_and_ws.rs`, `deploy_input.rs`) | RV1 |
