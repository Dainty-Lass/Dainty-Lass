# Security Review Index

Index of trust-boundary security checklists (`docs/security/*.md`, RX3/DX2) and `cargo fuzz` targets (`fuzz/fuzz_targets/*.rs`) for dainty-lass's MIT components. The AGPL components' checklists (the host
shell, the Tauri launcher, the game modules and `restricted/`'s realms crates) and their fuzz targets are in [restricted/SECURITY_REVIEW.md](restricted/SECURITY_REVIEW.md) (RD27).

## Checklists (`docs/security/`)

| Component | Checklist | Fuzz target(s) (`fuzz/fuzz_targets/`) | Row |
| --- | --- | --- | --- |
| Protocol (invites, account keys, statuses, usernames) | none yet | `protocol_input.rs` (`http_body.rs`, `proxy_headers.rs`: dainty-restricted) | RA1 |
| Gateway policy and TLS (`crates/gateway`) | [docs/security/gateway.md](docs/security/gateway.md) | `gateway_policy.rs`, `gateway_modules.rs`, `gateway_capture.rs` | RS3, DF4a, DN3, RD27 |
| Allowlist helper and its IPC | [docs/security/allowlist.md](docs/security/allowlist.md) | `allowlist_input.rs` | RS4, DF4d, RD27 |
| Content server (`crates/content`) | [docs/security/content.md](docs/security/content.md) | `content_request.rs`, `content_news.rs`, `content_runtime_index.rs` | RS2, RD27 |
| Chat plugin (`crates/chat`) | [docs/security/chat.md](docs/security/chat.md) | `chat_stanza.rs`, `chat_federation.rs` | DN10a, DN10b, RD27 |
| Region ping (gateway `/network`, launcher probe) | [docs/security/region-ping.md](docs/security/region-ping.md) | `gateway_network.rs`, `gateway_policy.rs`, `launcher_region.rs` | DN7, DN8 |
| Admin GraphQL API (keys, roles, guarded actions, history, module control, network) | [docs/security/admin-api.md](docs/security/admin-api.md) | `admin_request.rs`, `network_admin.rs` | RS4, DF4c, DH2, DH3, DH6, DN11 |
| Controller (in-process service supervision, module registry, in-process 1.14.7 backend and Director) | [docs/security/controller.md](docs/security/controller.md) | (`host_console.rs`: dainty-restricted) | RC1, DF3, RU9 |
| Module host (`module-api`, length-prefixed framing, SHA-256 verification, capabilities) | [docs/security/module-host.md](docs/security/module-host.md) | (`module_frame.rs`: dainty-restricted) | DF2 |
| Module libraries (`plugin-host`, the C ABI in `module-api::plugin`, pinned loading) | [docs/security/plugin-host.md](docs/security/plugin-host.md) | `plugin_bytes.rs` (frames: `module_frame.rs`, dainty-restricted) | RD27 |
| Change log, hash chain, revert, object-locked bucket and peer log shipping | [docs/security/history.md](docs/security/history.md) | `history_input.rs`, `history_record.rs`, `history_bucket.rs`, `network_logship.rs` | DH1, DH3–DH5, DH7 |
| Server network (CA, mTLS node links, directory, home API, session tickets, instance registry, matchmaking ring, regional Ramsgate, cluster join) | [docs/security/network.md](docs/security/network.md) | `network_link.rs`, `network_directory.rs`, `network_home.rs`, `network_ticket.rs`, `network_admin.rs`, `network_logship.rs`, `network_ring.rs` | DN2–DN6, DN8, DN9, DN11, DN13 |
| Epic Games login, manifest parser and CDN chunk installer (`crates/epic`, host, launcher) | [docs/security/epic.md](docs/security/epic.md) | `epic_manifest.rs` | DE1–DE6 |
| Remote management (`crates/remote`: one-time enrollment URIs, certificate pinning, mTLS `Service::Remote`, secret non-export, `host.toml` override preservation, `DH6` guarded approval) | [docs/security/remote-management.md](docs/security/remote-management.md) | `remote_wire.rs` | DD27, RM1–RM4 |
| Rustic Rifts co-op transport and admission (`crates/rifts-transport`, `rifts-admission`) | [docs/security/rifts-transport.md](docs/security/rifts-transport.md) | `transport_frame.rs`, `transport_pipe.rs`, `transport_files.rs`, `admission_control.rs`, `admission_ticket.rs` | RD27 |
| Rustic Rifts Guard (`crates/rifts-guard`) | [docs/security/rifts-guard.md](docs/security/rifts-guard.md) | `guard_evidence.rs`, `guard_grants.rs` | RD27 |
| Rustic Rifts realm hosting on the launcher (`crates/rifts-host`) | [docs/security/rifts-host.md](docs/security/rifts-host.md) | `host_ready_line.rs`, `host_peer_line.rs`, `host_transport_config.rs` | RD27 |

Fuzz targets with no checklist row yet: `relay_signal.rs`, `runtime_input.rs`, `launcher_host.rs`.
