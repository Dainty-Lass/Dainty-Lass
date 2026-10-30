# Architecture

What runs where, what each part trusts, and the few contracts everything depends on. The security checklists
under [security/](security) go through each trust boundary line by line.

## The parts

| Part | Runs on | Code | Listens on |
| --- | --- | --- | --- |
| Launcher & Manager | each player's or host's PC | `rust/launcher` (Tauri: `dainty-lass-client` and `dainty-lass-manager`), page from `ui/` (`dainty-lass-ui`) | relay `127.0.0.1:61100` for the game (`dainty-lass-client`) |
| Controller | the server | `rust/crates/controller` over `rust/crates/module-api` | the one process tree `start`, the console and `dainty-lass-manager` (`server-gui`) run |
| Gateway (core plugin) | the server | `rust/services/gateway` | public: TLS 443 (or `public_port`); private: HTTP 61000 on the Tailscale address; node links `/__node/*` with mTLS |
| Allowlist helper (core plugin) | the server, public mode | `rust/services/gateway`, bin `allowlist` | loopback; edits nftables (Linux) or Windows Firewall for the union of loaded modules' UDP port ranges |
| Content server (core plugin) | the server | `rust/services/content` | `127.0.0.1:61002`; keys runtime manifests, client profiles and Epic manifests by `GameVersion` |
| Admin API (core plugin) | the server | `rust/services/admin` | loopback, host-only; per-operator role keys, guarded actions, history and revert |
| Network (core plugin) | the server | `rust/plugins/network` | no listener of its own; rides gateway `/__node/*` (directory, home API, session tickets, peer log shipping) |
| Chat (core plugin) | the server | `rust/plugins/chat` | XMPP over WebSocket routed through the gateway; federated server-to-server over `/__node/chat` |
| Store & history shipping | the server | `rust/crates/store`, `rust/crates/history-ship` | SQLite (default) or Postgres on loopback, with a hash-chained change log shipped to S3 object lock or a peer node |
| Game modules | the server (in-process or child process) | `modules/rustic-isles` (`isles-1-14-7`, `isles-1-12-0`), `modules/legacy-revived` (`revived-1-4-4`) | loopback ports assigned by the controller (`61000`/`61001` by default, shifted on collision) |
| Realms & TURN relay | the server & realm hosts' PCs | `modules/rustic-rifts` (`rifts-admission`, `rifts-relay`, `rifts-realms`, `rifts-guard`, `rifts-host`, `ParadoxTransport.dll`) and `turn-rs` | `turn-rs` on UDP/TCP 3478, TLS 5349 and its UDP relay range (the one listener exception, DD24) |
| Game servers | the server | the game executable, with the host's runtime | UDP 8770-8777 (down to 8766 with ten hunts), guarded by the allowlist |
| Host kit | the server | `rust/services/host`, bin `reforged-host` | nothing; writes `.workspace/config`, manages modules, certs and history, runs `start` |

**The gateway is the only listener off loopback, with one written exception for the TURN relay (DD24).**
Every HTTP and control service binds `127.0.0.1`, and the gateway refuses the host-only routes (`/admin`,
`/launcher`, `/internal`, `/health`, and `/__node/*` without a valid cluster client certificate) and headers
(`x-mysticparadox-gameserver-apikey`, `x-reforged-admin-key`, `x-mysticparadox-metrics-token`) before anything
reaches an upstream. The game servers' UDP ports are opened per player address by the allowlist only after that
player logs in. When realms are enabled on 1.14.7, `turn-rs` (or `coturn`) listens on UDP/TCP 3478 (and optional
TLS 5349 plus its UDP relay port range) because STUN/TURN traffic cannot sit behind an HTTP gateway
([security/turn-relay.md](security/turn-relay.md)).

## Controller, core plugins and game modules

DaintyLass separates **core plugins** from **game modules** ([`rust/crates/module-api`](../rust/crates/module-api)):

- **Core plugins (`CorePlugin`)** live in this repository (`gateway` with `allowlist`, `content`, `admin`, `host`,
  `network`, `chat`) and provide shared infrastructure for every game version on the host.
- **Game modules (`GameModule`)** live in their own repositories (`modules/rustic-isles` for `"Rustic Isles 1.14.7"`
  and `"Rustic Isles 1.12"`, `modules/legacy-revived` for `"Dauntless Revived"` 1.4.4) and declare a
  `ModuleManifest` with an exact `GameVersion` (`1.14.7`, `1.12.0`, `1.4.4`), invite `version` aliases
  (`reforged`, `revived`), default ports, client profile (executable SHA-256, command line, Epic manifest SHA-1)
  and capabilities (`realms`, `network`, `chat_rules`, `patch_set`, `theme`).
- **Registration (`Registrar` and `ModuleContext`).** During `register`, a game module registers its HTTP route
  prefixes (which the gateway routes by `GameVersion`), UDP port ranges (which the allowlist unions across loaded
  modules), static and runtime content files, console/admin commands, status panels, admin actions (`kick`, `ban`,
  `lookup`) and `host.toml` config sections. The controller refuses a second game module for a `GameVersion` that
  is already loaded and shifts colliding loopback ports automatically.
- **In-process and out-of-process loading.** The same `GameModule` trait runs compiled into the server binaries
  (`server-minimal`, `server-console`, `dainty-lass-manager` with `server-gui`) or as an external child process (`module_api::child::main`)
  speaking length-prefixed JSON frames over stdin/stdout ([writing-a-module.md](writing-a-module.md),
  [security/module-host.md](security/module-host.md)). Only external executables listed in `host.toml` with their
  pinned SHA-256 are ever started.

## Joining a server

1. **Invite.** The host makes one with `reforged-host invite`:
   `reforged-revival://join?v=1&host=<Tailscale IP>&port=61000&code=..&name=..[&share=..]` for a private
   server, `reforged-revival://join?v=2&mode=public&host=<IP or DNS>&port=443&fp=<certificate SHA-256>&code=..&name=..`
   for a public one. The `dauntless://` scheme carries the same fields plus `version` (`reforged` for 1.14.7,
   `revived` for a Dauntless Revived 1.4.4 server). The parser accepts nothing it does not recognise
   (`rust/crates/protocol`, `tests/cases/invite.json`).
2. **Registration.** The launcher posts the chosen name and the code to `POST /reforged/api/Register` and gets
   an **account key** (`RRK_` and 48 hex digits). The key is stored in the OS keystore and never shown again
   unless the player backs it up. In public mode the launcher connects only over TLS pinned to the `fp` from
   the invite, and the key goes only to that server and certificate.
3. **Status and identity.** `GET /reforged/api/ServerStatus` (with the key, the full view: players and
   instances; without it, the limited one) and `GET /reforged/api/GetUserInfo` (the account's id and name, or
   a ban with its reason).
4. **The runtime.** The content server lists the host's build of `MysticParadox.dll` and `winmm.dll` with sizes
   and SHA-256s (`/content/v1/runtime/manifest`). The launcher shows them, the player accepts them once, and
   every launch checks the files again. The launcher never injects: the DLL reaches the game only through the
   loader's `mystic_loader.ini`, and PLAY refuses to start while a leftover ini or an unknown DLL sits beside
   the loader.
5. **Play.** The launcher starts Dauntless 1.14.7 with `-AUTH_PASSWORD=<key>` and `REFORGED_BACKEND_URL`
   pointing at its relay. The relay carries the game's HTTP and chat to the gateway and answers the game's
   region ping (`/QoS`) locally; the game only ever sees `127.0.0.1:61100`. The backend runs Mystic Paradox
   with `AUTH_MODE=APIKEY`, so the key is the game's login.

## The launcher API

The backend's `/reforged/api` is the one contract between the launcher, the content server, the admin API and
the host kit (`modules/rustic-isles/crates/isles-core/src/areas/reforged_api/`, ported from
[modules/rustic-isles/reference/1.14.7/ParadoxBackend/src/routes/reforgedApi.ts](../modules/rustic-isles/reference/1.14.7/ParadoxBackend/src/routes/reforgedApi.ts)).

| Route | Who calls it | Through the gateway |
| --- | --- | --- |
| `POST /reforged/api/Register` | launcher | yes |
| `GET /reforged/api/GetUserInfo` | launcher, content server (`x-reforged-user-api-key`) | yes |
| `GET /reforged/api/ServerStatus` | launcher, admin API | yes |
| `GET /reforged/api/RegistrationStatus` | launcher | yes |
| `POST /reforged/api/CreateInvite`, `GET …/InviteCodes`, `DELETE …/InviteCode/:code` | host kit (`x-reforged-admin-key`, loopback only) | refused |
| `POST /reforged/api/SetAccountStatus`, `POST /reforged/api/Kick` | host kit, admin API | refused |

Everything else under `/reforged/api` is refused at the gateway with 403, admin key or not: the backend
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
- **The backend trusts the game servers' key** (`x-mysticparadox-gameserver-apikey`, loopback only) and the
  admin key on loopback. It stores account keys only as HMACs.
- **The host kit trusts `host.toml` and `secrets.env`** in `.workspace/config`, which it wrote. Every service
  reads one env file and nothing from its working directory; `start` clears the environment of anything that
  looks like a server setting before passing the file on.

## Where the design is going

[MIGRATION_PLAN.md](../MIGRATION_PLAN.md) ports the backend and Director to Rust and replaces MongoDB with
`store`. [dainty-lass-plan.md](dainty-lass-plan.md) turns the server into a controller with game modules (one
per game version, in repositories of their own), moves the launcher page to `dainty-lass-ui`, and adds the
multi-region network, installing the game from a player's Epic library, and change history with rollback.
