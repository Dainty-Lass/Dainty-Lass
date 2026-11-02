# Security Review Index

Index of trust-boundary security checklists (`docs/security/*.md`, RX3/DX2) and `cargo fuzz` targets (`fuzz/fuzz_targets/*.rs`) for dainty-lass's MIT components. The AGPL components' checklists (gateway, allowlist, content, chat, the host
shell, the launcher, the game modules and realms) and their fuzz targets are in dainty-restricted's own index (RD27).

## Checklists (`docs/security/`)

| Component | Checklist | Fuzz target(s) (`fuzz/fuzz_targets/`) | Row |
| --- | --- | --- | --- |
| Protocol (invites, account keys, statuses, usernames) | [docs/local-contracts.md](docs/local-contracts.md) | `protocol_input.rs` (`http_body.rs`, `proxy_headers.rs`: dainty-restricted) | RA1 |
| Admin GraphQL API (keys, roles, guarded actions, history, module control, network) | [docs/security/admin-api.md](docs/security/admin-api.md) | `admin_request.rs`, `network_admin.rs` | RS4, DF4c, DH2, DH3, DH6, DN11 |
| Controller (in-process service supervision, module registry, in-process 1.14.7 backend and Director) | [docs/security/controller.md](docs/security/controller.md) | (`host_console.rs`: dainty-restricted) | RC1, DF3, RU9 |
| Module host (`module-api`, length-prefixed framing, SHA-256 verification, capabilities) | [docs/security/module-host.md](docs/security/module-host.md) | (`module_frame.rs`: dainty-restricted) | DF2 |
| Module libraries (`plugin-host`, the C ABI in `module-api::plugin`, pinned loading) | [docs/security/plugin-host.md](docs/security/plugin-host.md) | `plugin_bytes.rs` (frames: `module_frame.rs`, dainty-restricted) | RD27 |
| Change log, hash chain, revert, object-locked bucket and peer log shipping | [docs/security/history.md](docs/security/history.md) | `history_input.rs`, `history_record.rs`, `history_bucket.rs`, `network_logship.rs` | DH1, DH3–DH5, DH7 |
| Server network (CA, mTLS node links, directory, home API, session tickets, instance registry, matchmaking ring, regional Ramsgate, cluster join) | [docs/security/network.md](docs/security/network.md) | `network_link.rs`, `network_directory.rs`, `network_home.rs`, `network_ticket.rs`, `network_admin.rs`, `network_logship.rs`, `network_ring.rs` | DN2–DN6, DN8, DN9, DN11, DN13 |
| Epic Games login, manifest parser and CDN chunk installer (`crates/epic`, host, launcher) | [docs/security/epic.md](docs/security/epic.md) | `epic_manifest.rs` | DE1–DE6 |
| Launcher 1.4.4 legacy client path (`dauntless-revived://`, client patcher, relay) | [docs/security/launcher-legacy.md](docs/security/launcher-legacy.md) | `launcher_legacy.rs` | RL1–RL5 |
| Remote management (`crates/remote`: one-time enrollment URIs, certificate pinning, mTLS `Service::Remote`, secret non-export, `host.toml` override preservation, `DH6` guarded approval) | [docs/security/remote-management.md](docs/security/remote-management.md) | `remote_wire.rs` | DD27, RM1–RM4 |

## Module repository checklists

| Repository | Checklist | Fuzz targets | Row |
| --- | --- | --- | --- |
