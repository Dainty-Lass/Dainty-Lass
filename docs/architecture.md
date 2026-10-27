# Architecture

What runs where, what each part trusts, and the few contracts everything depends on. The security checklists
under [security/](security) go through each trust boundary line by line.

## The parts

| Part | Runs on | Code | Listens on |
| --- | --- | --- | --- |
| Launcher | each player's PC | `rust/launcher` (Tauri), page from `dainty-lass-ui` | relay `127.0.0.1:61100` for the game |
| Gateway | the server | `rust/services/gateway` | public: TLS 443 (or `public_port`); private: HTTP 61000 on the Tailscale address |
| Allowlist helper | the server, public mode | `rust/services/gateway`, bin `allowlist` | loopback; edits nftables (Linux) or Windows Firewall |
| Content server | the server | `rust/services/content` | `127.0.0.1:61002` |
| Admin API | the server | `rust/services/admin` | loopback, host-only |
| Backend | the server | `modules/rustic-isles/reference/1.14.7/ParadoxBackend` (Node, Mystic Paradox) until the Rust port | `127.0.0.1:61000` |
| Director | the server | `modules/rustic-isles/reference/1.14.7/ParadoxDirector` (Node, Mystic Paradox) until the Rust port | `127.0.0.1:61001`; starts the game servers |
| Game servers | the server | the game, with the host's runtime | UDP 8770-8777 (down to 8766 with ten hunts) |
| Host kit | the server | `rust/services/host`, bin `reforged-host` | nothing; writes the env files, runs `start` |
| Controller | the server | `rust/crates/controller` | the one process tree `start`, the console and the server GUI run |
| MongoDB | the server | managed by the host kit, or the host's own | loopback, until the Rust port moves to `store` |

**The gateway is the only listener off loopback.** Everything else binds `127.0.0.1`, and the gateway refuses
the host-only routes (`/admin`, `/launcher`, `/internal`, `/health`) and headers
(`x-mysticparadox-gameserver-apikey`, `x-reforged-admin-key`, `x-mysticparadox-metrics-token`) before anything
reaches them. The game servers are the exception the allowlist exists for: their UDP ports are opened per
player address, only after that player logged in.

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
the host kit ([modules/rustic-isles/reference/1.14.7/ParadoxBackend/src/routes/reforgedApi.ts](../modules/rustic-isles/reference/1.14.7/ParadoxBackend/src/routes/reforgedApi.ts)).

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
