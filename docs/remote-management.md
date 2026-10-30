<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Remote management design (R0 — DD27, RM1–RM5)

This document specifies how one **DaintyLass Manager** install (or a headless controller) manages multiple **DaintyLass**
server installs across machines, containers, and regions. All code for remote management is written fresh under the MIT
licence (`Copyright (c) 2026 Silent Spud`) in `rust/crates/remote` with zero dependencies on AGPL crates.

---

## 1. Who is who

| Role | What runs it | `host.toml` `[remote]` setting | Description |
| --- | --- | --- | --- |
| **Controller** | A `DaintyLass Manager` desktop install (or a headless `dainty-lass-host` / `dainty-lass-server-console` install) with remote management enabled | `role = "controller"` | Holds the cluster/management CA (or enrollment issuer), maintains the authoritative shared configuration revision, displays the status/logs/history of every enrolled install, and dispatches operator commands to one install or a group. |
| **Managed install** | Any DaintyLass server install enrolled with the controller: headless servers (`dainty-lass-host`, `dainty-lass-server-console`, `dainty-lass-server-minimal`), Docker containers (`reforged-revival-server`), other `DaintyLass Manager` installs, or the regional nodes of a network (`DD16`) | `role = "managed"` | Enrolls once with the controller using a one-time enrollment code, pulls shared settings and runtime/gamedata packs from the controller, reports health/logs/doctor/migration/history back, and executes authenticated remote operator commands through the same local code paths (`DH2`, `DH6`, `DD23`). |
| **Standalone install** *(default)* | Any install that has not enabled controller mode or enrolled with a controller | `role = "standalone"` *(or `[remote]` omitted)* | Operates locally with zero network or remote-management traffic, exactly as before. |

### Can a player client (`dainty-lass-client`) also be a managed install?

We do **not** make `dainty-lass-client` a managed install by default, because a player's PC is not operator
infrastructure: the controller must never be able to execute host commands, inspect local files, or alter a player's
keystore.

However, a read-only **"server directory / launcher feed"** mode (where a player client optionally subscribes to a
community's URL to pull a curated list of server invites or region endpoints) is possible if desired. Per the `R0`
ground rules, this is left as **Open Question 1** in Section 10 for the maintainer to decide.

---

## 2. What a managed install pulls vs what is pushed, and local overrides

### 2.1 Shared artefacts and settings managed by the controller

Every controller state change increments a monotonic `config_revision: u64`. Each item below is tagged with the
revision at which it changed and its SHA-256 digest:

| Category | Contents | Delivery direction | How it is applied on the managed install |
| --- | --- | --- | --- |
| **Shared `host.toml` settings** | `[server]` (`registration`, `diagnostics_log_profile`), `[paradox]` (game-server frame rates, idle timeouts, max hunts, memory floor), `[switches]` (all `MYSTICPARADOX_*` switches and rate-limit budgets), `[realms]` (TURN mode/ports), `[history]` (retention days) | **Pushed** immediately on change when the install's gateway is reachable, **and pulled** on every heartbeat sync whenever `controller_revision > applied_revision` | Merged with the install's preserved per-host fields (§2.2), written atomically to `host.toml`, and applied via `setup` / hot-reload. |
| **Presets** | `preset/*.json` files | **Pushed** notification + **pulled** by content SHA-256 | Written atomically into `.workspace/presets/` (or active preset config) after SHA-256 verification. |
| **Module selection & settings** | `[modules].enabled` list, `[[modules.external]]` SHA-256 allowlist, and `[module_settings.<module_id>]` tables | **Pushed** notification + **pulled** on heartbeat sync | Written to `host.toml` and applied through `controller::ModuleRegistry` (`load`, `unload`, `module_settings_set`). |
| **Game data packs & published runtime** | Compiled game-data pack (`.workspace/gamedata/*.json` bundle) and published runtime (`MysticParadox.dll`, `winmm.dll`, `manifest.json`) | **Pulled** on demand when the manifest revision/SHA-256 in `SyncAnswer` differs from the install's local SHA-256 (triggered immediately by a lightweight push wake-up) | Streamed in chunks over `/__node/remote`, verified against the manifest's per-file SHA-256, and swapped atomically into `.workspace/gamedata` and `.workspace/runtime`. |
| **Invites** | Active invite codes (`code`, `for_user`, `uses_remaining`, `expires_at`, `revoked`) | **Pushed** on creation/revocation + **pulled** delta on heartbeat sync | Merged into the install's invite store so an invite created on the Manager works immediately on the managed server. |
| **Server name & branding** | Base `server.name`, `news.json`, and `.workspace/branding/*` assets | **Pushed** notification + **pulled** by SHA-256 | Written to `.workspace/config/news.json` and `.workspace/branding/`; `server.name` uses the controller's name or `<controller_name> (<install_label>)` if configured. |

### 2.2 Why both push and pull are used

- **Pull (heartbeat sync, always active)**: Every managed install initiates an outbound mTLS `RemoteRequest::Sync` call
  to its controller's gateway every `sync_interval_secs` (default `15` seconds) and on startup. This guarantees that
  managed installs behind NAT, residential firewalls, or outbound-only Docker networks stay in sync and receive queued
  commands without needing inbound port forwarding from the controller.
- **Push (low-latency wake-up and direct RPC, when reachable)**: When the managed install has a reachable gateway
  address (such as regional network nodes or servers on a tailnet/LAN), the controller also sends `RemoteRequest` calls
  directly to the managed install's gateway (`POST /__node/remote`) so operator actions (`start`, `stop`, `ban`,
  `doctor`, live log tailing, config saves) complete immediately in a single round-trip. If a direct push fails with
  `Unreachable`, the controller queues the command or revision wake-up for the install's next pull heartbeat.

### 2.3 Local overrides and per-host identity (never clobbered)

A managed install is a real machine with its own network interface, paths, and database. When merging a controller's
shared `host.toml` bundle into the managed install's local `host.toml`, the managed install **always preserves**:

1. **Per-host network and listener identity**:
   - `[server] host`, `[server] game_address`, `[server] mode` (`public` / `private` / `local`), and `[ports]`
     overrides;
   - `[network] node_id`, `[network] region`, and TLS certificate paths (`node.crt`, `node.key`, `cluster-ca.crt`).
2. **Per-host filesystem and database paths**:
   - `[paths]` / local Dauntless game-server binary path (`GAMESERVER_BINARY_PATH`) and Wine/Linux settings;
   - `[database]` engine and local connection target (`sqlite` file path or the node's own `postgres` URL), and
     `skip_mongo_migration`;
   - `[history]` local S3 bucket endpoint/prefix if configured per-node.
3. **Explicit local overrides (`[remote] local_overrides`)**:
   - Any dotted `host.toml` key listed in `[remote] local_overrides = ["paradox.max_hunts", "server.name"]` on the
     managed install is locked to the local value and ignored during controller merges.
4. **Per-install controller overrides**:
   - On the controller, the operator can also set per-install overrides keyed by `NodeId` (for example, giving node
     `eu-de-1` `server.name = "DaintyLass (Frankfurt)"` and `paradox.max_hunts = 8` while `us-or-1` gets
     `paradox.max_hunts = 4`). Precedence from highest to lowest is:
     1. Managed install's per-host identity fields and `[remote] local_overrides`;
     2. Controller's per-install override for `<node_id>`;
     3. Controller's shared baseline configuration.

---

## 3. What a managed install reports back

On every `Sync` heartbeat (and on demand when queried by the controller), each managed install reports a structured
`InstallReport`:

| Field group | Reported fields |
| --- | --- |
| **Identity & version** | `node_id`, `label`, `region`, `dainty_lass_version`, `commit_sha`, `os`, `arch`, `enrolled_at`, `applied_config_revision`. |
| **Status & component health** | Overall server state (`running`, `starting`, `stopping`, `stopped`, `degraded`), `uptime_secs`, and per-component status (`gateway`, `content`, `admin`, `allowlist`, `turn-rs`, and each loaded game module: state, listen port, last error). |
| **Player & instance counts** | `players_online`, `active_hunts`, `active_hubs`, `active_realms`, and the list of active game-server instances (`instance_id`, `version`, `kind`, `players`, `port`). |
| **Doctor summary** | `ok: bool`, `pass_count`, `warn_count`, `fail_count`, `checked_at_ms`, and the full vector of `DoctorCheck { id, status, summary, detail }`. |
| **Migration state** | `mongo_migration_state` (`not_needed`, `unmigrated`, `migrated`, `skipped`), `mongo_reachable: Option<bool>`, `verified_collections: Option<u32>`. |
| **History & change-log summary** | `head_seq: i64`, `head_hash: String`, `chain_verified: bool`, `last_change_at_ms: Option<i64>`, `last_actor: Option<String>`, `pending_guarded_count: u32`, and `ship_lag_secs: Option<u64>`. |
| **Logs (on request or streamed)** | Per-component and per-module structured log lines (`timestamp_ms`, `level`, `target`, `message`), fetched in bounded pages (`limit <= 500`, `since_seq`) when the operator opens the install's Status/Logs tab. |

---

## 4. What the controller can do remotely

Everything `DaintyLass Manager` does locally today works remotely against **one managed install** or **a group of
managed installs**:

| Operation | Remote command | Behaviour on the managed install |
| --- | --- | --- |
| **Start / Stop / Restart** | `RemoteCommand::Lifecycle { action: Start \| Stop \| Restart }` | Starts, stops, or restarts the managed install's embedded/supervised server components (`serve::compose`) without terminating the remote-management agent itself, so a stopped install can still be started again remotely. |
| **Run `setup`** | `RemoteCommand::Setup { examples: bool, fresh_start: bool }` | Runs `setup` with the current merged `host.toml` (generating any missing local secrets in `secrets.env`, writing `.env` files, and running or refusing MongoDB migration per `M1`–`M4`). Note: `fresh_start: true` is a guarded action (§4.1). |
| **Admin actions** | `RemoteCommand::AdminAction { action: Kick \| Timeout \| LiftTimeout \| Ban \| Unban \| Lookup, target, reason, duration_secs }` | Dispatches through the install's `admin` service and loaded game modules (`DF4c`, `DN11`), recording the remote operator's key ID as the change-log `Actor`. |
| **Module load / unload & settings** | `RemoteCommand::ModuleControl { module_id, action: Load \| Unload \| SetSettings(json) }` | Calls `controller::ModuleRegistry` to load/unload a compiled-in or allowlisted external module (`DD5`) or update its settings. |
| **History query & revert** | `RemoteCommand::HistoryQuery(filter)`, `RemoteCommand::HistoryVerify`, `RemoteCommand::HistoryRevert { filter, dry_run }` | Queries or verifies `store::history` on the install. A live revert (`dry_run: false`) is a guarded action (§4.1). |
| **Backup** | `RemoteCommand::Backup` | Runs a local SQLite online backup / Postgres dump into the install's own `.workspace/backups/` directory (or triggers an immediate `history-ship` snapshot to the install's own bucket/peer) and returns `{ ok, backup_name, files_copied, bytes }` — **never** the database contents. |
| **Update** | `RemoteCommand::Update { check_only: bool, version: Option<String> }` | Checks or applies a release self-update (`selfupdate`) on the managed install, running `setup` and verifying migration before switching. |
| **Database (`db`) commands** | `RemoteCommand::Db { subcommand: Migrate \| Verify \| ExportMongo }` | Runs `db migrate`, `db verify`, or `db export-mongo` locally on the managed install and returns the per-collection verification summary. |

### 4.1 Guarded actions (`DH6`) and audit actor (`DD23`)

Remote execution never bypasses local safety controls:

1. **Change-log actor (`DD23`)**: Every remote command carries a signed `OperatorEnvelope` naming the operator's key ID
   and role (`Actor::Admin { key_id: "remote:<controller_node>:<key_id>", role }`), never `Actor::System`. Every
   database write resulting from that command records that exact actor in `store`'s hash-chained change log.
2. **Two-person approval and delay (`DH6`)**: Destructive or bulk actions (`HistoryRevert { dry_run: false }`,
   `Setup { fresh_start: true }`, bulk account erasures, and `Update` downgrade) pass through `admin::guard` on the
   managed install (and on the controller for group actions). When `guarded_actions` requires a second operator's
   approval or a cooling-off delay:
   - Submitting the command creates a `PendingGuardedAction` (`id`, `action`, `requested_by`, `execute_after`,
     `requires_second_approver`);
   - A second operator with `Admin` or `Owner` role must approve the pending action (with a distinct `key_id` from
     `requested_by`) before the managed install executes it;
   - Either operator can cancel the pending action while it waits.

---

## 5. Transport, enrollment, ownership, and exposure

### 5.1 Reuse of existing transport and trust primitives

Remote management introduces **no new cryptographic protocol or listener**:

- **mTLS node links (`DN3`, `DD16`)**: All post-enrollment communication between the controller and managed installs
  runs over `plugins/network`'s mutual-TLS HTTP/1.1 node links (`POST /__node/remote`, added as `Service::Remote` in
  `network::link::Service`), with `x-node-time` clock-skew enforcement (`<= 30s`), `MAX_MESSAGE` (`1 MiB`) body caps,
  and peer `NodeId` extracted from the client certificate verified by `ClusterCa`.
- **Cluster CA (`network::cluster::ClusterCa`)**: When remote management is enabled on a controller that does not yet
  have a cluster CA, the controller initialises a `ClusterCa` in its local `<workspace>/.workspace/tls/cluster-ca.pem`
  (mode `0600`) and issues its own controller node certificate (`controller.crt`, `controller.key`). If the install is
  already part of a `DD16` network with an existing `cluster-ca.pem`, it reuses that CA.
- **Admin keys and roles (`DH2`)**: Operator commands issued from the controller require a valid `admin` key with the
  required `Role` (`Viewer` for status/logs/history queries, `Moderator` for kick/timeout/ban, `Admin` for
  start/stop/setup/modules/backup/revert, `Owner` for enrollment/revocation/update).

### 5.2 Enrollment flow (one-time enrollment code -> mTLS certificate)

1. **Minting the enrollment code (on the controller)**:
   - In `DaintyLass Manager` ("Managed installs" -> "Enroll install") or via CLI
     `reforged-host remote invite --node <id> --region <region>`, the controller generates a one-time enrollment code:
     ```text
     dainty-lass-remote://enroll?v=1&url=https%3A%2F%2Fhost.example.org%3A61000&fp=<sha256-hex>&node=<id>&region=<region>&token=<32-byte-base64url>
     ```
   - The controller stores only `sha256(token)`, the assigned `NodeId` and `Region`, `expires_at` (default 15 minutes),
     and `used = false`.
2. **Redeeming the enrollment code (on the managed install)**:
   - The operator runs `reforged-host remote enroll "<code-or-uri>"` (or pastes it into a second Manager's "Enroll with
     a controller" dialog).
   - The managed install generates a fresh private key locally in `.workspace/tls/node.key` (mode `0600`; **the private
     key never leaves the managed install**).
   - The managed install connects to the controller gateway's `url`, **verifies the gateway's TLS certificate SHA-256
     fingerprint against `fp` from the enrollment URI** (refusing the connection on any mismatch), and sends
     `POST /__node/enroll` with:
     - `token` (the one-time secret);
     - `node` and `region`;
     - `public_key_pem` (its generated public key / CSR);
     - `gateway_url` and `gateway_cert_fp` (if the managed install has a reachable gateway for direct push).
3. **Issuing the certificate**:
   - The controller's gateway forwards `POST /__node/enroll` (rate-limited at the gateway's `register` bucket: 5 burst,
     1 per 5 minutes per IP) to the controller's `remote` service.
   - The controller checks that `sha256(token)` exists, is unexpired, and has `used == false`, marks it `used = true`
     atomically, signs the managed install's certificate (`node.crt`) with `ClusterCa` for `node` and `region`, adds the
     install to its managed-install table, and returns `EnrollAnswer { controller_node, controller_cert_fp, ca_cert_pem,
     node_cert_pem }`.
   - The managed install writes `node.crt` and `cluster-ca.crt` to `.workspace/tls/`, records `[remote]` in `host.toml`,
     and immediately performs its first mTLS `POST /__node/remote` (`Sync`) call.

### 5.3 Revocation and offline behaviour

- **Revocation**:
  - When the operator clicks **Revoke** in the Manager (or runs `reforged-host remote revoke <node>`), the controller:
    1. Marks `<node>` as `Revoked` in its managed-install store and adds `<node>` (and its certificate SHA-256
       fingerprint) to the controller's revoked-peer set;
    2. Immediately drops `<node>` from `TlsPeerLink`'s allowed peers so any subsequent mTLS connection or
       `POST /__node/remote` call from `<node>` is rejected with `403 not_a_peer` / `revoked`;
    3. If `<node>` is reachable at revocation time, sends a best-effort `RemoteCommand::Detached` notification so the
       install logs that it was revoked and stops polling the controller (while keeping its own local server running).
- **When the controller is offline during revocation**:
  - Because the controller itself is the authority that accepts `/__node/remote` syncs and issues commands, revoking an
    install on the controller takes effect on the controller immediately. If the managed install is also a peer in a
    multi-node game network (`DD16`), the controller propagates the peer revocation to the remaining enrolled nodes on
    their next sync so they also remove the revoked node from their `[[network.nodes]]` peer list.

### 5.4 Ownership: one controller per managed install

- A managed install has **at most one controller** at a time (`[remote] controller_url` and `controller_node` in
  `host.toml`). Allowing two controllers to push conflicting `host.toml` revisions, presets, or module states to the
  same install would create split-brain configuration.
- Multiple operators can still manage the same fleet by signing into the controller with their per-person admin keys
  (`DH2`), or by running `reforged-host remote leave` on an install before enrolling it with a different controller.

### 5.5 Exposure: the gateway remains the only listener off loopback

- **"The gateway is the only listener off loopback" (`AGENTS.md`, `DD3`, `DD16`) holds without exception.**
- Enrollment uses `POST /__node/enroll` on the controller's existing gateway listener (`61000` / `443`).
- All ongoing remote-management RPCs use `POST /__node/remote` (`Service::Remote`) on the existing mTLS `/__node/*`
  gateway listener.
- Neither the controller nor any managed install opens a new public TCP or UDP port.

---

## 6. What never leaves an install

The trust boundary between a controller and a managed install is strict: **the controller sends settings and commands,
never secrets, and a managed install reports status and summaries, never secrets or raw databases.**

1. **Never sent from a managed install to the controller (or vice versa)**:
   - `.workspace/config/secrets.env` or any variable in `SECRET_KEYS` (`REFORGED_ADMIN_API_KEY`, `JWT_SECRET`,
     `SESSION_SECRET`, `TURN_STATIC_AUTH_SECRET`, `AUTH_SIGNING_PRIVKEY_B64`, S3 credentials, Postgres passwords,
     MongoDB URI passwords);
   - Player account keys (`RRK_...`), OAuth tokens, refresh tokens, or password/key hashes;
   - Raw SQLite database files (`dainty-lass.db`, `chat.db`), Postgres dumps, or legacy MongoDB BSON documents;
   - The install's TLS private key (`node.key`).
2. **How a managed install gets the secrets it needs**:
   - During initial `setup` (or when a new secret key is required after an update), the managed install generates its
     own cryptographic secrets locally into `.workspace/config/secrets.env` using the OS CSPRNG (`setup::secrets`),
     exactly as a standalone install does today.
3. **Where backups and change-log archives go**:
   - Running `Backup` remotely writes the snapshot to the managed install's own `.workspace/backups/` folder, its own
     configured S3-compatible object-locked bucket (`DH4`), or its designated log-ship peer (`DH7`), and returns only
     metadata (`backup_name`, `files_copied`, `bytes_written`) to the controller.

---

## 7. Failure behaviour (controller unreachable)

When a managed install cannot reach its controller (controller PC powered off, network partition, or controller
restarting):

1. **Keep running on last good settings**: The managed install starts and continues running all game modules, gateway,
   content, and admin services using the last verified configuration (`host.toml`), presets, runtime, and game-data
   pack stored in its local `.workspace/`.
2. **Never stop or wipe**: Unreachability of the controller **never** stops a running server, disconnects players,
   revokes invites, or deletes any local file or database row.
3. **Bounded backoff with jitter**: The background sync task logs a single warning on transition to unreachable
   (`controller unreachable; running on cached revision <N>`) and retries `RemoteRequest::Sync` with exponential
   backoff (5s -> 10s -> 20s -> 40s -> max 60s, plus ±20% jitter) until the controller answers again.
4. **Local escape hatch**: At any time, the local machine owner can run `reforged-host remote leave` to detach the
   install from the unreachable controller and manage it standalone on its current settings.

---

## 8. Licensing (`MIT` from the first line)

- All remote-management protocol types, enrollment logic, sync engine, and command dispatch live in a new crate:
  `rust/crates/remote` (`package.name = "remote"`, `license = "MIT"`, `Copyright (c) 2026 Silent Spud`).
- `rust/crates/remote` depends only on MIT workspace crates (`module-api`, `network`, and—once `L2`/`L3` land—the MIT
  `protocol`, `service-kit`, `store`, `server-controller`, `admin`, and `hostkit`) plus standard third-party MIT/Apache
  crates (`serde`, `serde_json`, `sha2`, `base64`, `rcgen`, `rustls`, `thiserror`, `tokio`, `tracing`).
- Where `crates/remote` invokes host-kit operations (`setup`, `doctor`, `backup`, `update`, `db`, `invite`), it defines
  a clean `RemoteHostHandler` trait in `crates/remote` so `crates/remote` never depends on `services/host`.
- `cargo tree -e normal -i <AGPL-crate>` from `crates/remote` must return no path to any AGPL crate.

---

## 9. User interface and CLI surface

### 9.1 `DaintyLass Manager` UI (`ui/apps/web`)

1. **Managed installs view & target switcher**:
   - On the Manager's **Host** screen (and navigation rail), a **Managed installs** panel lists every enrolled install
     with its label, `NodeId`, region, live state badge (`Online`, `Degraded`, `Offline`, `Revoked`), player count,
     version, and config sync badge (`Revision #N`).
   - Clicking **Enroll install** opens a dialog that mints a one-time `dainty-lass-remote://enroll?...` URI with a
     one-click **Copy enrollment command** (`dainty-lass-host remote enroll "..."`) button and expiry countdown.
2. **Per-install inspection and control (reusing existing tabs)**:
   - Selecting any enrolled install in the Manager switches the target context (`selectedInstallId`) so the existing
     **Server**, **Status**, **Modules**, **Admin**, and **History** tabs display that remote install's status,
     doctor checks, migration state, module cards, per-module logs, active invites, player lookup/ban/kick controls,
     and change-log history/revert controls.
3. **Group actions**:
   - Selecting multiple installs (or **All online installs**) enables group actions: **Push settings now**, **Start
     all**, **Stop all**, **Restart all**, **Run doctor on all**, **Backup all**, and **Broadcast ban/unban**, showing a
     per-install outcome table as each install replies or queues the action.

### 9.2 Headless CLI (`dainty-lass-host` / `reforged-host remote`)

```text
reforged-host remote init-controller [--listen-url <https://host:61000>]
reforged-host remote invite --node <node-id> [--region <region>] [--ttl-minutes 15]
reforged-host remote enroll <enrollment-uri-or-token> [--label <name>]
reforged-host remote status
reforged-host remote list
reforged-host remote revoke <node-id>
reforged-host remote leave
```

### 9.3 Plan rows (`RM1`–`RM5` in `docs/dainty-lass-plan.md`)

| Row | Summary | Verify |
| --- | --- | --- |
| **RM1** | Design (`docs/remote-management.md`), security checklist (`docs/security/remote-management.md`), `SECURITY_REVIEW.md` entry, and fuzz target `remote_wire` (`R0`, `R1`) | Checklist covers enrollment token pinning/single-use, mTLS peer identity, secret non-export, guarded actions, and payload bounds; `cargo check --manifest-path fuzz/Cargo.toml --bins --locked` passes |
| **RM2** | `crates/remote` (`license = "MIT"`): enrollment token mint/redeem over gateway `POST /__node/enroll`, `Service::Remote` (`POST /__node/remote`) over `plugins/network` mTLS links, config revision sync + local override merger, `InstallReport` telemetry, remote command dispatch with `DH6` guarded approval and `DD23` remote operator actor, and `RemoteHostHandler` trait (`R2`) | Unit and integration tests for enrollment, token replay refusal, cert fingerprint mismatch refusal, config merge preserving per-host overrides, guarded two-person approval, and offline fallback; `cargo tree -e normal` shows zero AGPL dependencies |
| **RM3** | Wire `crates/remote` into `controller` (`remote` feature), `host::serve`, `host::commands::setup`, `reforged-host remote` CLI subcommands, Docker entrypoint (`REFORGED_REMOTE_ENROLL_TOKEN`), Manager bridge commands (`spec.rs` + `api.ts`), and `ui/` Managed Installs UI with `en`/`pt-BR` catalog keys (`R2`) | Bridge staleness tests, `npm test`, `bun run test` (including new Managed Installs UI tests in `en` and `pt-BR`), and CLI round-trip tests pass |
| **RM4** | Docker multi-install harness (`docker/remote/test.sh` and `docker/run-tests.sh`) + Windows Manager E2E verification and screenshots in `en` and `pt-BR` (`R3`) | 1 controller + 2 managed installs enroll, sync a setting change, apply a remote ban, survive controller stop, enforce revocation, and prove by wire inspection that zero secrets/account keys/DB files crossed; Manager desktop screenshots captured in both languages |
| **RM5** | Documentation sweep (`docs/hosting.md`, `docs/hosting-linux.md`, `docs/hosting-docker.md`, `docs/architecture.md`, `README.md`, `AGENTS.md`, `NOTICE.md`, and handoff ledger) (`R4`) | All hosting guides include "Managing several installs", `NOTICE.md` lists `crates/remote` under MIT, and all workspace checks pass |

---

## 10. Resolved maintainer decisions (`R0` gate)

1. **Client launcher scope**: **Confirmed (`Recommended: no remote management in client launcher`)** — remote
   management applies strictly to server and `DaintyLass Manager` installs; `dainty-lass-client` has no remote
   management surface.
2. **Single controller ownership**: **Confirmed (`true`)** — each managed install has at most one active controller at a
   time (`remote leave` is required before enrolling with a different controller), while multiple human operators share
   the controller via per-person `DH2` admin keys.
3. **Hybrid push + pull sync**: **Confirmed (`yes`)** — managed installs run a 15-second outbound mTLS heartbeat pull
   combined with immediate mTLS push when reachable, preserving per-host local overrides (`[server] host`, `ports`,
   `paths`, `database`, and `network` identity).
