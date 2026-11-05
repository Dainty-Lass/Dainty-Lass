# Security review: TURN relay (`turn-rs`, DD24 / RF4 / DX2)

Scope: the TURN relay process (`turn-rs` or `coturn`) managed by the host kit for player-hosted realms (DD14,
DD24), the `turn-rest` short-lived HMAC credentials minted by `modules/rustic-rifts/crates/rifts-relay`, the
launcher and transport relay-only mode (`P2P_FORCE_SDR` / `force_relay`), and the UDP relay port range opened by
the allowlist firewall code.

## 1. The DD24 listener exception

| Rule | Where | Checked |
| --- | --- | --- |
| The gateway is the only listener off loopback across all server services, **except** for the TURN relay when realms are enabled on 1.14.7 (DD24). A STUN/TURN server must bind the host's public IPv4 on UDP/TCP 3478 (and optional TLS 5349) plus its UDP relay port range so ICE agents behind NAT can allocate relay addresses | `docs/dainty-lass-plan.md` DD24, `docs/architecture.md`, `modules/rustic-rifts/crates/rifts-transport/src/turn.rs` | Separate OS process (`turn-rs`), started only when realms/TURN are configured; carries only STUN/TURN frames and never holds database, admin, or gateway secrets |
| `turn-rs` runs as an unprivileged child process supervised by the host kit; a crash or compromise of the relay process does not expose the backend, `store`, or `secrets.env` | `modules/rustic-rifts/crates/rifts-transport/src/turn.rs` | `TurnServerProcess` passes only the rendered `turn-rs` config (`static-auth-secret`, `realm`, listener addresses, and `port-range`) |

## 2. Shared secret and `turn-rest` HMAC credentials

| Rule | Where | Checked |
| --- | --- | --- |
| The shared secret (`P2P_TURN_REST_SECRET` in `secrets.env` / `backend.env`, `static-auth-secret` in `turn-rs`) must be at least 16 characters of high-entropy random text and is never sent to clients; clients receive only a time-limited username `<expiry_unix>:<tag>` and `base64(HMAC-SHA1(secret, username))` | `modules/rustic-rifts/crates/rifts-relay/src/lib.rs` (`TurnRestConfig::validate`, `issue_turn_rest_credentials`), `rifts-transport/src/turn.rs` (`TurnServerConfig::validate`) | Unit tests in `rifts-relay` and `rifts-transport` reject empty or short (`< 16` char) secrets and control characters |
| Credential lifetime (`P2P_TURN_CREDENTIAL_TTL_SECONDS`) is clamped to `[60, 86400]` seconds (default 3600s) so a leaked credential expires automatically without rotating the server secret | `modules/rustic-rifts/crates/rifts-relay/src/lib.rs` (`MIN_TTL_SECONDS`, `MAX_TTL_SECONDS`) | Tested at boundary values in `rifts-relay` |
| The `turn-rest` username binds an opaque per-account HMAC tag (`relay_account_tag`, `acct_<24 hex>`) instead of the raw account ID or display name, so STUN/TURN packets on the wire never leak player identifiers while the backend can still enforce per-account relay quotas | `modules/rustic-rifts/crates/rifts-relay/src/lib.rs` (`relay_account_tag`) | Deterministic HMAC tag tested in `rifts-relay`; raw account ID never appears in `turn_username` |
| Transport secrets (`turn_username`, `turn_credential`, and per-peer AES-256-GCM pair keys) are written to a restricted temporary file by the launcher and deleted immediately after the transport session reports readiness | `modules/rustic-rifts/README.md`, `crates/rifts-host/src/lib.rs` | Secrets file deleted on ready or startup failure |

## 3. Relay-only mode (`P2P_FORCE_SDR` / `force_relay`) and encryption

| Rule | Where | Checked |
| --- | --- | --- |
| When relay-only mode is active (`P2P_FORCE_SDR=true` on the backend or `force_relay=true` on the session), `rifts-relay` requires at least one `turn:`/`turns:` URL, `ParadoxTransport.dll` gathers and accepts only `relay` ICE candidates, and the backend strips host/srflx/prflx candidates from `POST /launcher/v1/p2p/hunts/:sessionId/signal`, so neither host nor joiner learns the other's IP address | `modules/rustic-rifts/crates/rifts-relay/src/lib.rs`, `crates/rifts-realms/src/lib.rs`, `src/ice_link.cpp` | `ParadoxTransportTest.exe parse`, `hub --relay-local`, and `rifts-relay` unit tests verify non-relay candidate stripping and `force_relay` enforcement |
| The TURN relay never sees plaintext game traffic: every datagram between host and joiner is sealed end-to-end with the per-pair, per-generation AES-256-GCM key (`pair_crypto.*`, random starting counter, 1024-frame replay window) before it enters `libjuice` / `turn-rs` | `modules/rustic-rifts/src/pair_crypto.cpp`, `crates/rifts-transport/src/lib.rs` | `ParadoxTransportTest.exe crypto` and fuzz target `control_frame` |

## 4. Port range and firewall rules

| Rule | Where | Checked |
| --- | --- | --- |
| The TURN listener ports (UDP/TCP 3478, optional TLS 5349) and the UDP relay allocation range (`port-range`, default `49152`–`65535` or the host's configured subset) are validated (`port_range_start >= 1024`, `port_range_start <= port_range_end`, listener ports outside the allocation range) before `turn-rs` starts and before the allowlist firewall rules open them | `modules/rustic-rifts/crates/rifts-transport/src/turn.rs` (`TurnServerConfig::validate`, `firewall_port_ranges`) | Unit tests in `rifts-transport::turn` verify privileged-port rejection, inverted range rejection, and listener/range overlap rejection |
| When the server stops or realms are disabled, the host kit stops `turn-rs` and removes its firewall rules alongside the allowlist cleanup | `modules/rustic-rifts/crates/rifts-transport/src/turn.rs`, `crates/gateway/src/allowlist/` | Process termination and rule teardown verified in `rifts-transport` and `allowlist` tests |
