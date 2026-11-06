# Architecture

What runs where, what each part trusts, and the few contracts everything depends on. The security checklists
under [security/](security) go through each trust boundary line by line; the AGPL components' checklists are in
`restricted/docs/security/`. Code paths are from the repository root; the AGPL ones start with `restricted/`.

## The parts

| Part | Runs on | Code | Listens on |
| --- | --- | --- | --- |
| Launcher & Manager | each player's or host's PC | `restricted/launcher/src-tauri` (Tauri: `dainty-launcher` and `dainty-manager`) over `crates/launcher-kit`, with `launcher/` (the isolation hook and scripts) and pages from `ui/apps/web` (dainty-dash, built with `DAINTY_MODE=launcher` or `manager`) | relay `127.0.0.1:61100` for the game (`dainty-launcher`) |
| Controller | the server | `crates/controller` over `crates/module-api` | the one process tree `start`, the console and `dainty-manager` (`server-gui`) run |
| Gateway (core plugin) | the server | `crates/gateway` | public: TLS 443 (or `public_port`); private: HTTP 61000 on the Tailscale address; node links `/__node/*` with mTLS |
| Allowlist helper (core plugin) | the server, public mode | `crates/gateway`, bin `allowlist` | loopback; edits nftables (Linux) or Windows Firewall for the union of loaded modules' UDP port ranges |
| Content server (core plugin) | the server | `crates/content` | `127.0.0.1:61002`; keys runtime manifests, client profiles and Epic manifests by `GameVersion` |
| Admin API (core plugin) | the server | `crates/admin` | loopback, host-only; per-operator role keys, guarded actions, history and revert |
| Network (core plugin) | the server | `crates/network` | no listener of its own; rides gateway `/__node/*` (directory, home API, session tickets, peer log shipping, remote management) |
| Remote management | controller & managed installs | `crates/remote` (`Service::Remote`) | no listener of its own; rides gateway `/__node/enroll` (rate-limited token redemption) and `/__node/remote` (mTLS under the controller's CA) |
| Chat (core plugin) | the server | `crates/chat` | XMPP over WebSocket routed through the gateway; federated server-to-server over `/__node/chat` |
| Store & history shipping | the server | `crates/store`, `crates/history-ship` | SQLite (default) or Postgres on loopback, with a hash-chained change log shipped to S3 object lock or a peer node |
| Game modules | the server (in-process or child process) | `crates/rustic-isles-1-14-7` composing the `rustic-isles-*` areas (MIT), with `restricted/`'s version data | loopback ports assigned by the controller (`61000`/`61001` by default, shifted on collision) |
| Realms & TURN relay | the server & realm hosts' PCs | `crates/rifts-*` (`rifts-admission`, `rifts-relay`, `rifts-realms`, `rifts-guard`, `rifts-host`, `rifts-transport`), `restricted/modules/rustic-rifts` (`ParadoxTransport.dll`) and `turn-rs` | `turn-rs` on UDP/TCP 3478, TLS 5349 and its UDP relay range (the one listener exception, DD24) |
| Game servers | the server | the game executable, with the host's runtime | UDP 8770-8777 (down to 8766 with ten hunts), guarded by the allowlist |
| Host kit | the server | `crates/hostkit` and `restricted/crates/host`, bin `dainty-host` | nothing; writes `.workspace/config`, manages modules, certs and history, runs `start` |

**The gateway is the only listener off loopback, with one written exception for the TURN relay (DD24).**
Every HTTP and control service binds `127.0.0.1`, and the gateway refuses the host-only routes (`/admin`,
`/launcher`, `/internal`, `/health`, and `/__node/*` without a valid cluster client certificate) and headers
(`x-rustic-isles-gameserver-apikey`, `x-dainty-admin-key`, `x-rustic-isles-metrics-token`, and their `x-mysticparadox-` forms) before anything
reaches an upstream. The game servers' UDP ports are opened per player address by the allowlist only after that
player logs in. When realms are enabled on 1.14.7, `turn-rs` (or `coturn`) listens on UDP/TCP 3478 (and optional
TLS 5349 plus its UDP relay port range) because STUN/TURN traffic cannot sit behind an HTTP gateway
(`restricted/docs/security/turn-relay.md`).

## Controller, core plugins and game modules

DaintyLass separates **core plugins** from **game modules** ([`crates/module-api`](../crates/module-api)):

- **Core plugins (`CorePlugin`)** live in this repository (`gateway` with `allowlist`, `content`, `admin`,
  `network`, `chat`) and provide shared infrastructure for every game version on the host; `restricted/crates/host`
  composes them.
- **Game modules (`GameModule`)** sit beside them. The one there is, `crates/rustic-isles-1-14-7`, is
  `"Rustic Isles 1.14.7"`. A module declares a `ModuleManifest` with an exact `GameVersion` (`1.14.7`), invite
  `version` aliases (`reforged`), default ports, client profile (executable SHA-256, command line, Epic manifest
  SHA-1) and capabilities (`realms`, `network`, `chat_rules`, `patch_set`, `theme`). Dauntless Revived's 1.4.4 is
  known to the invite parser only to be refused (RD5).
- **Registration (`Registrar` and `ModuleContext`).** During `register`, a game module registers its HTTP route
  prefixes (which the gateway routes by `GameVersion`), UDP port ranges (which the allowlist unions across loaded
  modules), static and runtime content files, console/admin commands, status panels, admin actions (`kick`, `ban`,
  `lookup`) and `host.toml` config sections. The controller refuses a second game module for a `GameVersion` that
  is already loaded and shifts colliding loopback ports automatically.
- **In-process and out-of-process loading.** The same `GameModule` trait runs compiled into the server binaries
  (`server-minimal`, `server-console`, `dainty-manager` with `server-gui`) or as an external child process (`module_api::child::main`)
  speaking length-prefixed JSON frames over stdin/stdout ([writing-a-module.md](writing-a-module.md),
  [security/module-host.md](security/module-host.md)). Only external executables listed in `host.toml` with their
  pinned SHA-256 are ever started. A module can also be a library (`[[modules.external]]` with a `.dll` or `.so`
  path), loaded by `crates/plugin-host` over the C ABI in `module_api::plugin` (RD27).
- **Services as libraries (RD27).** A service `host.toml` names in `[services.<name>]` (its `library` path and
  `sha256`) runs as a supervised module of the controller in place of the compiled-in one
  (`controller::service_library`, `hostkit::serve::compose`); `content`, `chat` and `gateway` are known by name and
  what they serve as core plugins. The MIT core, `crates/dainty-lass-server`, has only the admin API and remote
  management built in and takes every other service this way; `restricted/`'s server binaries still compile
  them in unless `host.toml` lists them.

## Joining a server

1. **Invite.** The host makes one with `dainty-host invite`:
   `dainty://join?v=1&host=<Tailscale IP>&port=61000&code=..&name=..[&share=..]` for a private
   server, `dainty://join?v=2&mode=public&host=<IP or DNS>&port=443&fp=<certificate SHA-256>&code=..&name=..`
   for a public one. The `dauntless://` scheme carries the same fields plus `version` (`reforged` for 1.14.7;
   `revived`, a Dauntless Revived 1.4.4 server, is refused with its own error). The parser accepts nothing it does
   not recognise (`crates/protocol`, `tests/cases/invite.json`).
2. **Registration.** The launcher posts the chosen name and the code to `POST /dainty/api/Register` and gets
   an **account key** (`RRK_` and 48 hex digits). The key is stored in the OS keystore and never shown again
   unless the player backs it up. In public mode the launcher connects only over TLS pinned to the `fp` from
   the invite, and the key goes only to that server and certificate.
3. **Status and identity.** `GET /dainty/api/ServerStatus` (with the key, the full view: players and
   instances; without it, the limited one) and `GET /dainty/api/GetUserInfo` (the account's id and name, or
   a ban with its reason).
4. **The runtime.** The content server lists the host's build of `Rustic-Isles.dll` and `winmm.dll` with sizes
   and SHA-256s (`/content/v1/runtime/manifest`). The launcher shows them, the player accepts them once, and
   every launch checks the files again. The launcher never injects: the DLL reaches the game only through the
   loader's `mystic_loader.ini`, and PLAY refuses to start while a leftover ini or an unknown DLL sits beside
   the loader.
5. **Play.** The launcher starts Dauntless 1.14.7 with `-AUTH_PASSWORD=<key>` and `REFORGED_BACKEND_URL`
   pointing at its relay. The relay carries the game's HTTP and chat to the gateway and answers the game's
   region ping (`/QoS`) locally; the game only ever sees `127.0.0.1:61100`. The backend runs Rustic Isles
   with `AUTH_MODE=APIKEY`, so the key is the game's login.

## The launcher API

The backend's `/dainty/api` is the one contract between the launcher, the content server, the admin API and
the host kit (`crates/rustic-isles-reforged/src/reforged_api.rs`, written clean-room from the behaviour of
`restricted/modules/rustic-isles/reference/1.14.7/ParadoxBackend/src/routes/reforgedApi.ts`).

| Route | Who calls it | Through the gateway |
| --- | --- | --- |
| `POST /dainty/api/Register` | launcher | yes |
| `GET /dainty/api/GetUserInfo` | launcher, content server (`x-dainty-user-api-key`) | yes |
| `GET /dainty/api/ServerStatus` | launcher, admin API | yes |
| `GET /dainty/api/RegistrationStatus` | launcher | yes |
| `POST /dainty/api/CreateInvite`, `GET …/InviteCodes`, `DELETE …/InviteCode/:code` | host kit (`x-dainty-admin-key`, loopback only) | refused |
| `POST /dainty/api/SetAccountStatus`, `POST /dainty/api/Kick` | host kit, admin API | refused |

Everything else under `/dainty/api` is refused at the gateway with 403, admin key or not: the backend
accepts those calls only from its own machine anyway. The gateway's tests read the route file, so a route
added there without a policy decision fails CI.

## What each part trusts

- **The launcher trusts nothing from the network.** Every answer is parsed field by field
  (`net/hostapi/parse.rs`), file names in a manifest are the two it knows, and a runtime is run only after the
  player accepted its hashes. A malicious host can still harm a player who accepts their runtime; the
  launcher says so.
- **The gateway trusts its policy and nothing in a request.** Hop-by-hop headers are stripped, the host-only
  ones dropped, rate limits applied per class, and only `POST /heartbeat` feeds the allowlist.
- **The content server trusts the backend's answer about a key**, cached briefly, and its own folders.
- **The backend trusts the game servers' key** (`x-rustic-isles-gameserver-apikey`, loopback only) and the
  admin key on loopback. It stores account keys only as HMACs.
- **The host kit trusts `host.toml` and `secrets.env`** in `.workspace/config`, which it wrote. Every service
  reads one env file and nothing from its working directory. `start` passes the shell's environment on with the
  file on top, so a server setting the file doesn't name can still come from the shell.
- **A managed install trusts its enrolled controller only for shared settings and remote admin actions**
  ([remote-management.md](remote-management.md), [security/remote-management.md](security/remote-management.md)).
  Enrollment redeems a single-use token over TLS pinned to the controller's SHA-256 certificate fingerprint;
  subsequent syncs ride mTLS on `/__node/remote` with the controller's cluster CA. Secrets (`secrets.env`),
  account keys (`RRK_*`), private keys and database files never cross the wire in either direction, local
  identity keys and `[remote].local_overrides` are preserved locally, and remote actions run through the same
  `DH2` role checks, `DH6` guarded approval rules and `DD23` change-log attribution (`remote:<controller>:<key>`)
  as local ones.

## Where the design is going

[MIGRATION_PLAN.md](../MIGRATION_PLAN.md)'s "What's left" table says what is still to do, among it RB15, which
removes the Node backend and MongoDB rollback path. [dainty-lass-plan.md](dainty-lass-plan.md) is the plan behind the controller, the game
modules, the multi-region network, installing the game from a player's Epic library, and change history with
rollback.
