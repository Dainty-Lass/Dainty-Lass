<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# DaintyLass plan: one controller, game modules in their own repositories

Status: draft, written 2026-10-26. The maintainer chose the decisions in section 3 the same day, over several
rounds of questions; section 8 lists what is still open.

This plan renames the app and splits this repository. `dauntless-eternal` (this repository, Reforged Revival)
becomes **DaintyLass**: the controller, the module framework, the core plugins and the admin API. The game code
leaves for repositories of its own: **rustic-isles** (Mystic Paradox's 1.14.7 and 1.12.0, ported to Rust, as two
game modules), **rustic-rifts** (Mystic Paradox's player-hosted realms, ported to Rust, as libraries) and
**dauntless-revived** (Dauntless Revived's 1.4.4, one game module). The launcher page is replaced by an Astro and
shadcn UI in **dainty-lass-ui**. DaintyLass mounts the other four as git submodules.

On top of that, servers in several regions (Oregon, New York, the UK, Germany) join one **network**: global
accounts owned by a home region, one matchmaking pool that widens from the lowest-ping region outwards, a shared
Ramsgate per region, chat and direct messages across servers, and hunt ids that are the same on every server.

It amends [`MIGRATION_PLAN.md`](../MIGRATION_PLAN.md) where section 7 says so and leaves the rest of it standing.
The integration pass folds the amendments in; a row here never edits that file (AGENTS.md).

Row ids: **DS** = repositories, split and rename, **DF** = module framework and controller, **DU** = UI,
**RU** = rustic-isles, **RF** = rustic-rifts, **RV** = dauntless-revived, **DN** = the network, **DX** =
cross-cutting, **DD** = decisions. "DR" still means Dauntless Revived at `e1c5fec`, and the RB, RA, RS, RH and
RD ids point into `MIGRATION_PLAN.md`.

## 1. Where things stand

Measured 2026-10-26.

| What | State |
| --- | --- |
| This repository | `main` at `c57f157`: one cargo workspace with the controller (`crates/controller`), service libraries (gateway with the allowlist, content, admin, host kit; backend and Director are stubs), DR's 1.4.4 `metagame` and `deploy` behind the `legacy` feature, 12 edition binary crates (`Refined` = 1.4.4, `Reforged` = 1.14.7). Paradox's Node backend and Director at `paradox/` (`00d3be8` + 33 patches + fixes since, such as RD25's kick route). C++ in `native/`: `paradox-runtime`, `runtime-loader`, `catalog-exporter`, DR's `internal-server` and `client-mods` |
| Forgejo | `git.silentspud.com`, Forgejo 15.0.10, with access to the instance restricted. One repository, `silentspud/dauntless-eternal` (public on the instance), reached as `origin` over SSH on port 222 |
| Mystic Paradox | Upstream `main` at `21b9ff5` (`mystic` remote, `pranav158/Mystic-Paradox`). The local `replay` branch is `21b9ff5` plus Reforged's patches 0001–0039 as one commit each and their follow-ups (`ce1c42e`, 41 commits). The 1.12.0 release is the tag `dauntless-1.12.0` (`4d5dfaa`), an ancestor of `main` 91 commits back |
| Paradox's size (TS, JS and C++, tests included) | 1.12.0: backend 16.8k lines, Director 1.7k, runtime 11.4k. `replay`: backend 103.5k, Director 4.5k, runtime 29.5k, `ParadoxTransport` 8.8k |
| Realms on `replay` | Players host hubs and hunts on their own PCs; joiners connect through `ParadoxTransport.dll` (ICE on libjuice: direct, or through a TURN relay). A self-hosted backend brings its own relay (Cloudflare TURN or a `turn-rest` server such as coturn) and an admission key pair. Mystic's anti-cheat and Steam transport are private, so Guard runs on stubs in observe mode. In the backend, `p2p/`, `realms/` and Guard are about 2.0 of its 5.1 MiB of source (210 files mention realms). A "local realm" is something else: the whole backend on one PC with no database server (`PERSISTENCE_PROVIDER=sqlite`, every repository on `node:sqlite`, a file under `realm-data/`, daily `VACUUM INTO` backups); player-hosted hubs keep their state on the central backend |
| Dauntless Revived | `SilentSpud/dauntless-revived` on GitHub: `plan` at `24b7624` is upstream `main` (`5eb74ff`, `mixutin/dauntless-revived`) plus 468 commits, DR's Rust migration among them. This repository carries pieces of it with Reforged's changes: merge `54e271a` (commits `c1be7f7`…`18ac587`) |
| Rust Director | `feat/rb10-director-single-binary` (`cdc0a87`): RB10, the Director in Rust, never merged; based on the old `plan` layout |
| Getting the game | Epic still lists Dauntless (app name `Jackal`) for accounts that added it before it left the store in May 2025, but serves only 2.1.1's manifest. Older builds install from an archived manifest plus Epic's chunk CDN (`.../Builds/Org/o-krlzxj88qrtb69fredeuaf887bl5az/Jackal/default` on three hosts), which asks for no login. The community [Dauntless Manifest Archive](https://github.com/addison2k/DauntlessManifestArchive) holds 1.5.4, 1.6.2, 1.10.5, 1.12.0, 1.14.1, 1.14.5, 1.14.6, 1.14.7 and 2.1.1 with SHA-1 sums; **1.4.4 is not archived**, so it can't come from Epic. Mystic's guide pins 1.14.7's manifest by SHA-1 (`aa70748e…31476`) and its executable by SHA-256, through Legendary. Epic can delete an old build's chunks at any time. [`egs-api`](https://docs.rs/egs-api/) 0.14.0 (MIT, September 2026) signs in with an authorization code, lists the library and parses download manifests; whether it downloads chunks itself is not documented |

## 2. End state

```text
silentspud/dainty-lass            (renamed from dauntless-eternal)
  rust/
    crates/module-api             the framework: GameVersion, manifests, traits, the out-of-process protocol
    crates/controller             the registry, lifecycle, commands and events
    crates/protocol, proxy, service-kit, store (SQLite or Postgres, change history)
    plugins/gateway (allowlist), content, admin, host, launcher (Tauri shell)
    plugins/network               node links, directory, instance registry, matchmaking ring
    plugins/chat                  XMPP over WebSocket, federated between nodes
    bins/server, server-console, server-gui, client
  native/runtime-loader           winmm.dll, shared by every game's client payload
  ui/                             submodule: silentspud/dainty-lass-ui
  modules/rustic-isles/           submodule: silentspud/rustic-isles
  modules/rustic-rifts/           submodule: silentspud/rustic-rifts
  modules/dauntless-revived/      submodule: silentspud/dauntless-revived
  docs/ docker/ brand/

silentspud/rustic-isles           Mystic Paradox's history, `replay` as main
  crates/isles-*                  one Rust backend and Director for both versions
  modules/isles-1-14-7            game module "Rustic Isles 1.14.7", links rustic-rifts' server libraries
  modules/isles-1-12-0            game module "Rustic Isles 1.12"
  native/runtime/                 MysticParadox.dll per version; catalog-exporter
  reference/                      the Node services each version is ported from, until the replay passes
  preset/                         1.14.7 game data (from dainty-lass)

silentspud/rustic-rifts           realms, one library per component
  crates/rifts-admission, rifts-relay, rifts-realms, rifts-guard   server side, used by isles-1-14-7
  crates/rifts-host               client side, used by DaintyLass's launcher
  native/transport/               ParadoxTransport.dll, with its history

silentspud/dauntless-revived      DR's history, SilentSpud's `plan` as main
  modules/revived-1-4-4           game module "Dauntless Revived": metagame, deploy
  native/internal-server, client-mods

silentspud/dainty-lass-ui         Astro + shadcn monorepo from the preset
```

## 3. Decisions

| # | Topic | Decision | Consequence |
| --- | --- | --- | --- |
| DD1 | Name | The app is **DaintyLass**; the repository `dainty-lass`, renamed on Forgejo so the old URL redirects. Names people read change: README, docs, window titles, both languages' strings, crate and binary names. **Names an install depends on stay as they are**, with no migration planned: the `dauntless://` invite scheme and the `reforged-revival://` refusal, `RRK_` keys, the launcher's identifier and `userData` folder (the stored keys live there), env prefixes (`REFORGED_` and the rest), the `x-reforged-*` headers, the `/reforged/api` routes, the nftables table, the certificate and every invite's `fp`, and the file names the game loads (`MysticParadox.dll`, `ParadoxTransport.dll`, `winmm.dll`) | No host or player redoes anything on update. Players and hosts never read those names; renaming one would need a migration for no visible gain |
| DD2 | Repositories | **Five**, on `git.silentspud.com` under `silentspud`, all public on the instance. DaintyLass pins the other four as submodules at `ui/` and `modules/<name>/`. Module repositories never depend on DaintyLass's git history: they depend on `module-api`, `protocol`, `service-kit` and `store` by version from Forgejo's Cargo registry, and DaintyLass's workspace `[patch]`es those to its own paths, so a checkout with submodules builds as one tree. The module repositories sit under `exclude` in DaintyLass's workspace and keep workspaces of their own. rustic-isles takes rustic-rifts the same way | No circular git dependency; a module builds alone against a released API, or inside DaintyLass against the working copy. DF1 proves the cargo layout before anything depends on it. Public repositories also let CI and clones fetch submodules without a token |
| DD3 | Module kinds | **Core plugins** add capabilities other modules use: gateway with the allowlist, content, store, admin API, host kit commands, the Tauri shell, the console and TUI, chat and the network (DD16, DD21). They live in DaintyLass and are compiled in, picked by each binary's features. **Game modules** carry everything a server for one game version needs and use the core plugins only through `ModuleContext`: they register routes behind the gateway, UDP ranges with the allowlist, files with content, commands, events, admin actions (kick, ban, lookup) and config sections | One gateway, one allowlist and one content server per host, whatever games it runs. A game module never opens a public listener (AGENTS.md: the gateway is the only one off loopback) |
| DD4 | Game module identity | Each game module declares an **exact game version** (`GameVersion`: dot-separated integers, compared exactly, no ranges), a display name, a module id, the invite `version` aliases it answers to (`revived`, `reforged`), its module API version, its default ports and its capabilities (realms, for one). The controller keys modules by game version and **refuses a second module for a version that is already loaded**, compiled in or not. The display names: **"Rustic Isles 1.14.7"**, **"Rustic Isles 1.12"** (Rustic Isles' names always carry the version) and **"Dauntless Revived"** | Replaces `Edition`, `EditionSelection` and `Game` (`protocol`): an invite's `version` already names a game version, so it maps straight to a module. Port defaults come from manifests, and the controller moves a clash instead of hard-coding 61000/61010 |
| DD5 | Loading at runtime | **Out of process.** A game module crate is a library (compiled in) and, through `module-api`'s adapter, an executable the controller starts and talks to over the child's stdin and stdout: length-prefixed frames, a handshake carrying the manifest and API version, then calls both ways. The same `GameModule` trait backs both, so a module is written once. Unload: a stop request, a grace period, then the process tree is killed (job object on Windows, process group on Linux, as `start` already does). Only modules named in `host.toml` with their SHA-256 can be loaded; the admin API and the console can load and unload those, never an arbitrary path | A crashing module doesn't take the controller down, and no Rust ABI is involved. Every call crosses a pipe, which costs nothing next to the game's HTTP. The frame parser is a trust boundary with a fuzz target (DX2) |
| DD6 | Paradox's backend and Director | **Ported to Rust now, inside rustic-isles** (realms in rustic-rifts, DD14). The RB rows move there as RU rows, and RD17's gate no longer holds for them. Until the port passes its replay, today's `main` keeps shipping and hosts keep running Node Paradox; the 1.14.7 module ships only once its Rust services pass | The largest part of this plan: about 108k lines of TS and JS for 1.14.7 against 18.5k for 1.12.0, of which realms are about 40% |
| DD7 | rustic-isles' base and name | **The `replay` branch**: Mystic `main` at `21b9ff5` with Reforged's 39 patches, pushed with its history as rustic-isles' `main`; the `dauntless-1.12.0` tag comes with it. Upstream `main` is kept as a branch for later cherry-picks. Fixes made on this repository's `paradox/` since RP2 that `replay` lacks are listed (DS5) and applied first. **No new name borrows from "Mystic Paradox"** (its additional terms, sections 3 and 4): repositories, crates and display names are Rustic Isles' and Rustic Rifts'. Mystic Paradox is named only in truthful attribution (DX4) | The latest Mystic, nothing Reforged fixed lost, and no name that could read as affiliated with Mystic Paradox |
| DD8 | One codebase for 1.14.7 and 1.12.0 | The Rust port is written once, from the 1.14.7 Node code. 1.12.0's differences come from diffing the tag's Node services against the same files on `replay`, and each becomes a version hook or a data table in the shared crates. The per-version module crates stay thin: manifest, hooks, tables, Director config. The C++ runtime is built per version from each host's SDK; folding the two runtime trees into one source is a spike (RU10), not a promise | 1.12.0 costs a route inventory, a diff and its hooks, not a second port |
| DD9 | 1.12.0's verification | **Built, verified later.** "Rustic Isles 1.12" is ported against its Node tests and the route diff only, and its manifest marks it unverified until someone records 1.12.0 traffic for a replay. It never gates the 1.14.7 cutover | The UI shows it as unverified; hosts can still load it |
| DD10 | dauntless-revived's base | **SilentSpud's fork** (`plan`, `24b7624`) pushed with its history, upstream `main` kept as a branch. Then what this repository changed in its copy of DR (DS5) is brought across. The module is DR's `metagame`, `deploy`, `internal-server` and `client-mods`; DR's own gateway, content, launcher, `discord-bot` and `friend-kit` are dropped from the build, since DaintyLass's core plugins and admin API replace them | DR's 1.4.4 server keeps its history and its Reforged changes; one gateway serves every game |
| DD11 | The UI | **A new repository, `dainty-lass-ui`**, made by `bunx --bun shadcn@latest init --preset b1cYwfJGNu --template astro --monorepo` and mounted at `ui/`. Tauri stays the shell and serves Astro's static build. The bridge's generated TS types are written into the submodule (`RR_UPDATE_GENERATED=1`), and DaintyLass's staleness test reads them there. Both language catalogs move to the UI repository, and `gen/strings.json` is generated from them. Module-specific screens are data: the UI draws a game module's commands, status and settings from its manifest, so no module ships UI code. Realm hosting is the one UI that is code, shown for modules with the realms capability | Section 6's "the page's look word for word" no longer holds; both languages and every string do. Bun joins the build. The Electron launcher in `legacy/` is untouched until RA16 |
| DD12 | Binaries | The 12 edition crates **collapse to four**: `server`, `server-console`, `server-gui`, `client`, each with one Cargo feature per compiled-in game module and one for realms. The release builds every module in. The player `client` links no game server code: what it needs per game (the executable hash, the command line, the ini edits, which files to fetch) comes from the module's client profile, served by content. With the realms feature it links `rifts-host` | Fewer crates and one feature list. The launcher's legacy 1.4.4 paths become a client profile from the revived module |
| DD13 | Native code | **`runtime-loader` stays in DaintyLass** at `native/runtime-loader`, with Paradox's attribution: the launcher's PLAY check (RD9) and every game's client DLLs go through it, 1.4.4's included (`920a411`). `paradox-runtime`, `catalog-exporter` and `preset/` go to rustic-isles, `ParadoxTransport` to rustic-rifts, `internal-server` and `client-mods` to dauntless-revived. RD16 stands: until its cut-off Freeza's Paradox patches land in rustic-isles, after it only runtime patches, still there | No module depends on another game module's repository. The runtime patch method (`docs/paradox-runtime-patches.md`) moves with the runtime |
| DD14 | Realms | **In scope, in their own repository, `rustic-rifts`**: a library monorepo, one crate per component, ported from `replay`'s `p2p/`, `realms/` and Guard code and its `ParadoxTransport`. Rifts is neither a core plugin nor a game module: realms exist only on 1.14.7 and share its accounts, sessions and hunt data, so `isles-1-14-7` links the server crates and declares the realms capability, and DaintyLass's launcher links `rifts-host` for hosting and joining. Admission, relay credentials and leases are admin actions and commands of the 1.14.7 module. Guard keeps observe mode on stubs, since Mystic's anti-cheat is private. The relay is DD24's | Realms ride the module framework without a third module kind. About 40% of the backend port moves here, and the UI gains a hosting flow. Hosts that turn realms on need a TURN relay; the rest need nothing new |
| DD15 | Database | **SQLite by default, Postgres where concurrency pays.** A server on a PC and a dedicated community host run SQLite (WAL, one file, no service); a dedicated host may pick Postgres in `host.toml`; network region nodes (DD16) run Postgres, since a home database takes every account homed there plus writes forwarded from other regions. `store` offers both through Diesel's `MultiConnection`, so every repository is written once in SQL both accept, with migrations in both dialects and every repository's tests run on both in CI. `store db move` copies a database from one engine to the other, for a host that switches. Within a node, chat (DD21) keeps its own SQLite file or Postgres schema so its writes don't queue behind the game's; the network directory (DD17) and admin timeouts are tables in the node's database; matchmaking tickets, presence and the instance registry stay in memory. The launcher keeps no database (settings file, keystore, a resume file per download). libSQL is not needed; Turso's Rust engine is watched, and could take over the SQLite side without a file change once it leaves beta with concurrent writes | Nobody installs a database to run a server on a PC or a small host, and DR's metagame and `main`'s `store` stay on the SQLite they have. The cost is two dialects: portable queries, two migration sets and a doubled test matrix, far less than Mystic's separate implementation per database. Amends RD15, which put the whole Paradox port on Postgres |
| DD16 | The network | **One operator's cluster** of regional nodes (Oregon, New York, the UK, Germany), trusted with each other. Each node is an ordinary DaintyLass server whose `host.toml` names its region and its peers. Nodes talk through each other's gateways on host-only routes with mutual TLS under a cluster CA, so the gateway stays the only public listener. The network is a core plugin (`plugins/network`) with game hooks; **1.14.7 only** at first, through a network capability in its manifest, and **built alongside the port**, on the Rust backend only | No federation with other people's servers yet; the protocol is versioned so it can come later. 1.12 and 1.4.4 can opt in later through the same capability |
| DD17 | Accounts | **Global accounts, owned by a home region.** An account's data lives only in its home node's database; another node that needs it (a hunt it hosts, a hub the player is in) calls the home's internal API. A small directory, replicated to every node with one leader node for writes, maps account to home and holds username claims, so names are unique network-wide. **The account key never leaves its home** (AGENTS.md's rule holds): another node gets a short-lived session ticket the home signs | No distributed writes and no shared database, so the engine is a per-node choice (DD15). Registration (rare) round-trips to the directory leader. A far region pays a cross-region call per account write it makes, not per read the home makes |
| DD18 | Global ids | **Every hunt, hub instance, party and matchmaking ticket gets a network-wide id** (a ULID tagged with the owning node), and every node can resolve one to its owner through the instance registry. A player whose join reaches the wrong node gets a redirect, and the launcher's relay (port 61100) re-points to the owner's gateway with a home-signed ticket; the game client only ever sees `127.0.0.1:61100`, so it doesn't notice. Accounts get global ids from the start (RU4's seams) | The Rust port uses these ids from its first commit, so no id migration later. Whether the 1.14.7 client survives the relay switching mid-session is DN1's spike |
| DD19 | Matchmaking | **One pool that widens by ping.** The launcher measures round-trip time to every node's gateway and reports the regions in order. A ticket is owned by the player's home node and published to the nearest region first, then the next after a delay, and so on until every region sees it. A region that forms a match claims each ticket from its home (compare-and-set), so a ticket lands in one match only. The hosting node's Director starts the game server, its allowlist admits the matched players' addresses, and the game server's XP and objective posts are forwarded to each player's home | Ring delays are settings. A match forms where most of its players have the lowest ping, and nobody waits forever for their own region |
| DD20 | Ramsgate | **One shared Ramsgate per region**, hosted by that region's node, split into shards when an instance is full. A player goes to the Ramsgate of their lowest-ping region, whichever node is their home | Players from every server meet in their region's hub; the hub's game server talks to many homes through the network |
| DD21 | Chat | **A chat core plugin** (`plugins/chat`) from DR's XMPP-over-WebSocket code, with each game module giving its JID and room rules, and nodes federated server-to-server as XMPP does. A network-wide room, one room per region, and party and hunt rooms owned by the instance's node. Direct messages route to the recipient's current node and are stored at both players' homes, delivered at login when the recipient was offline. Room history lives with the room's owner and is replicated to the nodes that read it. Mutes and bans apply network-wide through the home | RU5's chat area becomes this plugin's 1.14.7 rules instead of a chat of its own. Paradox's City rooms (patch 0025) become per-region rooms |
| DD22 | Getting the game from Epic | **The launcher installs the game from the player's own Epic library.** It signs in to Epic (`egs-api`, the authorization-code flow Legendary uses, in a webview with a paste-the-code fallback), checks that `Jackal` is in the library, and lists the versions it can install: those whose game module's client profile names an Epic build (version, manifest SHA-1, manifest sources, the executable's SHA-256). It downloads the chosen one from Epic's CDN, checks every file against the manifest and the executable's hash, and installs each version in a folder of its own, so several sit side by side. Nothing is downloaded for an account without Dauntless, even though the CDN would serve it. DaintyLass never hosts or mirrors game files. **Manifests are committed** into the repository of the game module that pins them (by SHA-1) and served by content; the launcher takes a joined server's copy first, then the module's own, then the archive, and checks the SHA-1 whichever it got. The Epic session is kept like the account key: in the keystore, never across the bridge, never in a log. The host kit gets the same install as a command, with the pasted-code sign-in, for the game servers the Director starts | Players stop needing Legendary. 1.12.0 and 1.14.7 can be installed; **1.4.4 cannot** (no archived manifest), so "Dauntless Revived" keeps locating an existing folder only. A version whose chunks Epic has deleted shows as unavailable. Signing in with Epic's own launcher client id is what Legendary and Heroic do, not something Epic sanctions. DR's resumable downloader (`net/downloader.rs`, which RD22 left out) comes back for this |
| DD23 | Change history and rollback | **Every change is recorded and can be undone**, because maintainers have lost data to partners who turned on them. Four layers. (1) `store` writes an append-only change log in the same transaction as each change: time, actor, table, row key, operation, the row before and after. Player-facing records (accounts, characters, inventory, bans) are soft-deleted and purged only after a retention period. `history` and `revert --actor <who> --since <time>` write compensating changes, which can themselves be reverted, and report rows changed since instead of overwriting them. (2) The log and periodic snapshots ship continuously off the server to S3-compatible storage with object lock (S3, Backblaze B2, MinIO, Cloudflare R2), with write-only credentials on the server and the account held by the owner only; a restore replays a snapshot and the log up to a chosen moment, on either engine. (3) Each log entry carries the hash of the one before it and the head ships with the log, so an edited or truncated history shows where it breaks. (4) The admin API gets **one key per person** with a role (viewer, moderator, admin, owner); the existing `REFORGED_ADMIN_API_KEY` becomes the owner's key. Destructive and bulk actions wait out a delay or need a second operator's approval. In a network, each node can also ship its log to another region | A partner with access to the admin API or the host tools can't erase anything for good. One with root on the server can stop the shipping and then delete, and costs the owner only what changed since the last shipped entry, seconds to minutes; the status page and the admin API alert when shipping falls behind. Every write must go through `store`'s write path for the log to be whole, so modules get no raw connection |
| DD24 | The TURN relay | **turn-rs, as its own process**, one per server (one per region in a network), started, stopped and configured by the host kit with `static-auth-secret` set to the backend's `turn-rest` secret (`secrets.env`), UDP and TCP on 3478, TLS on 5349 when the host has a certificate for it, and its relay `port-range` opened by the allowlist's firewall code. The backend's provider stays `turn-rest`, so coturn is a drop-in fallback and Cloudflare TURN stays a setting. A TURN relay can't sit behind the gateway, so it is the **one written exception** to "the gateway is the only listener off loopback" (AGENTS.md, through the integration pass) | Realms need nothing from a third party, and a relay crash doesn't touch the game server. turn-rs has no tagged releases and doesn't claim to be production-ready, so RF8 is also its trial; if it falls short, the host kit runs coturn instead with the same secret |

## 4. Work

### 4.1 Repositories, split and rename (DS)

| # | What | Verify |
| --- | --- | --- |
| DS1 | Rename `silentspud/dauntless-eternal` to `dainty-lass` on Forgejo; point every clone's `origin` at the new URL (maintainer) | The old URL redirects; `git ls-remote` works on both |
| DS2 | Create `silentspud/rustic-isles` (public): push `replay` as `main`, the `dauntless-1.12.0` and `v1.12-final` tags, Mystic's `main` as `upstream/main`; `mystic` stays a fetch-only remote | `git log` shows Mystic's history; the tag resolves to `4d5dfaa` |
| DS3 | Create `silentspud/dauntless-revived` (public): push SilentSpud's `plan` as `main`, `mixutin`'s `main` as `upstream/main` | `git log` shows DR's history to `24b7624` |
| DS4 | Create `silentspud/dainty-lass-ui` (public) from the shadcn command (DD11); commit the generated tree untouched as the first commit | `bun install` and `bun run build` pass |
| DS5 | Catch-up lists, read-only: this repository's commits touching `paradox/`, `native/paradox-runtime`, `native/catalog-exporter` and `preset/` that `replay` lacks (`docs/port/catchup-isles.md`); its DR-derived commits (`c1be7f7`…`18ac587` and later ones touching `metagame`, `deploy`, `native/internal-server`, `native/client-mods`) as `docs/port/catchup-revived.md` | Every commit is listed as needed or not applicable, with the reason |
| DS6 | Swap the code for submodules: delete `paradox/`, `native/paradox-runtime`, `native/catalog-exporter`, `native/internal-server`, `native/client-mods`, `preset/`, `rust/services/metagame`, `rust/services/deploy`; add the module submodules; path sweep of the host kit, Docker, CI and docs. Lands once RU1 and RV1 hold that code, and before the Rust port, so hosts keep running what they run today | `cargo test`, `npm test` in the moved Node services, `bash docker/run-tests.sh` pass from a fresh `git clone --recurse-submodules` |
| DS7 | Create `silentspud/rustic-rifts` (public) from rustic-isles with `git subtree split` of `ParadoxTransport`, so the transport keeps its history; the Rust crates start empty | `git log` in the new repository shows the transport's commits |

### 4.2 Module framework and controller (DF)

| # | What | Verify |
| --- | --- | --- |
| DF1 | **Spike**: the cargo layout of DD2. A module workspace under `exclude`, used as a path dependency; a crate on Forgejo's Cargo registry `[patch]`ed to a path; `workspace = true` inheritance inside the submodule; on Windows and Linux. Kill criterion: if the registry or the `[patch]` fails, modules take `module-api` as a git dependency on a tag | A toy module builds both alone and inside DaintyLass |
| DF2 | `crates/module-api`, the seam: `GameVersion`, `ModuleManifest` (capabilities, the unverified mark), the `GameModule` and `CorePlugin` traits, `Registrar` (routes, ports, commands, events, admin actions, config sections), `ModuleContext`; the frame protocol and its adapters for both sides; every capability registered as a stub (RD18's seam rule) | A stub module runs compiled in and out of process and gives the same answers; fuzz target `module_frame` |
| DF3 | Controller: the registry keyed by game version with the duplicate refusal (DD4), load and unload, the `[modules]` section of `host.toml` (compiled-in list, external modules by path and SHA-256), commands into the console, TUI, server GUI and admin API, the event bus, port assignment, logs and status from out-of-process modules into the tracing layers. `Edition`, `EditionSelection` and `Game` removed | Two modules for one version refused in either order; a module killed mid-run shows as failed and restarts on command; nothing left running after stop |
| DF4 | Core plugins on the registry: the gateway routes by game version to the module's upstream (the invite's `version`, then the route), the allowlist guards the union of the modules' UDP ranges, content keys runtime manifests and client profiles by version (`edition=` still accepted, a request without one still gets 1.14.7), the admin API sends kick, ban and lookup to the module of the player's game, the host kit's commands gain `module list`, `load`, `unload` | The gateway and content parity tests pass unchanged; RD25's admin API cases pass against every game |
| DF5 | Bins: four crates with one feature per module and one for realms (DD12); the release workflow builds them (`server-release.yml` owner) | Each bin starts with zero, one and every module |
| DF6 | Rename sweep (DD1): crate and binary names, window titles, docs, both languages' strings; the contract names DD1 keeps are left as they are | No `Reforged Revival` or `dauntless-eternal` left outside history, the catch-up lists and the contract names |
| DF7 | `store` on two engines (DD15): `MultiConnection` over SQLite (as today) and Postgres, RB3's `pq-sys` `bundled` spike on MSVC and Linux, migrations in both dialects, `store db move` between them, the engine choice in `host.toml`; the host kit's managed Postgres (RH3) and Docker's Postgres container for hosts that pick it; CI runs every repository's tests on both | `cargo test -p store` on both OSes against both engines; a database moved to Postgres and back compares equal |

### 4.3 UI (DU)

| # | What | Verify |
| --- | --- | --- |
| DU1 | Tauri serves the UI: `frontendDist` at the Astro build, the CSP and the isolation hook checked against Astro's output (no inline script Tauri's policy refuses), bridge types generated into `ui/` | The empty UI opens in the Tauri debug build and a bridge call round-trips |
| DU2 | Player screens: servers, invites, PLAY with the runtime acceptance, settings, the game folder; both languages moved over with every key | The launcher tests that drive the page pass; `i18n` checks pass on the new catalogs |
| DU3 | Server screens: the Host page, the per-component status page, module panels drawn from manifests (DD11) with the unverified mark, module load and unload | A server GUI build with every module repository runs one of each game |
| DU4 | Drop `rust/launcher/vite.config.ts`'s build of `legacy/ReforgedLauncher` | No Tauri build reads `legacy/` |
| DU5 | Realm screens: host a hub or hunt, join one, the relay status; shown only for modules with the realms capability | Two launchers on two PCs host and join a realm (maintainer, RF8) |

### 4.4 rustic-isles (RU)

The RB rows of `MIGRATION_PLAN.md` section 4.5, moved here (DD6) and done for both versions. Realms are left to the
RF rows: an area that touches them calls through rustic-rifts' crates.

| # | What | From | Verify |
| --- | --- | --- | --- |
| RU1 | Lay out the monorepo with `git mv`: the Node services to `reference/1.14.7/`, `ParadoxRuntime` to `native/runtime/`, `catalog-exporter` and `preset/` brought from DaintyLass; `ParadoxLauncher` deleted, `ParadoxTransport` deleted once DS7 holds it; `ADDITIONAL_TERMS.md`, `NOTICE.md` and the licence kept at the top; DS5's isles list applied | RP3 | The Node tests pass from the new paths |
| RU2 | 1.12.0's Node services and runtime from the tag into `reference/1.12.0/` | — | The tag's own tests pass there |
| RU3 | Route inventory of both versions and their diff (DD8), each route marked isles or rifts | RB1 | Every Express route in each inventory; CI fails on a new route without an entry |
| RU4 | Cargo workspace and seams: `isles-core` (one trait per repository, on `store`'s two engines through its change log), `isles-chat`, `isles-director`, the two module crates with their manifests and hooks; every route answering 501 | RB5 | Both modules load into DaintyLass and answer 501 |
| RU5 | Areas, one agent each, both versions: auth, the Reforged API, game areas (five agents, as RB8), chat (as rules for `plugins/chat`, DD21), game data and scripts; account writes go through the home API's trait (DN2) | RB4, RB6–RB9, RB11 | Each area's Node tests ported, for both versions where 1.12.0 has the area |
| RU6 | Director in Rust, starting from `cdc0a87` | RB10 | Ramsgate, the Training Grounds and a hunt start and stop, on both versions (maintainer) |
| RU7 | `db migrate` from MongoDB and `db verify` | RB4 | Copies of real databases migrate with no difference |
| RU8 | Shadow replay for 1.14.7, realms included | RB2, RB12 | No unexplained diff |
| RU9 | Rollback, cutover and Node removal | RB13–RB15 | As in those rows |
| RU10 | **Spike**: one runtime source tree for both versions, SDK glue apart | — | Kill criterion: if the shared part is under half the code, the two trees stay apart |
| RU11 | 1.12.0's replay, when someone records 1.12.0 traffic (DD9); clears the unverified mark | RB12 | No unexplained diff |

### 4.5 rustic-rifts (RF)

| # | What | Verify |
| --- | --- | --- |
| RF1 | Inventory of realms: the `p2p/`, `realms/` and Guard routes (RU3's rifts marks), the runtime's hosting hooks, the transport's ABI 2 and 3, the `P2P_*` settings, what the host's PC stores | Every route, setting and ABI export listed with its component |
| RF2 | Cargo workspace and seams: `rifts-admission`, `rifts-relay`, `rifts-realms`, `rifts-guard`, `rifts-host`, each a trait surface with stubs; `isles-1-14-7` calls them | The 1.14.7 module builds with rifts stubbed and without them |
| RF3 | Transport build: `ParadoxTransport.dll` and its self-test in CI (MSVC v145, libjuice pinned by SHA-256) | `ParadoxTransportTest.exe all --relay-local` passes in CI |
| RF4 | Admission (tickets, the key pair, its generator as a command) and relay credentials (`turn-rest` for turn-rs and coturn, Cloudflare TURN as a setting); the host kit runs turn-rs (DD24): its config, the shared secret, its ports through the allowlist's firewall code, start and stop with the server | The transport's self-test passes against Rust-minted tickets; credentials from both providers |
| RF5 | Realms, leases and hosting tiers on `store` | `realms/` and `p2p/` Node tests ported |
| RF6 | Guard in observe mode on stubs, with its evidence chain intake | The Guard tests that don't need the private module ported |
| RF7 | `rifts-host`: the launcher's side of hosting and joining, behind DaintyLass's realms feature | A realm hosted from the debug launcher on one PC |
| RF8 | Two-PC check across NAT, direct and relay-only through turn-rs (maintainer); the trial that keeps turn-rs or switches the host kit to coturn (DD24) | A hub and a hunt hosted and joined both ways, through turn-rs with `P2P_FORCE_SDR` on |

### 4.6 dauntless-revived (RV)

| # | What | Verify |
| --- | --- | --- |
| RV1 | DS5's revived list applied; `metagame` and `deploy` on `module-api`; module crate `revived-1-4-4` (display name "Dauntless Revived", game version 1.4.4, alias `revived`); DR's gateway, content, launcher, `discord-bot` and `friend-kit` out of the build | The metagame's tests pass; the module loads into DaintyLass |
| RV2 | The 1.4.4 client profile (DD12): executable hash, command line, the DLL through the runtime loader | The launcher starts 1.4.4 against a DaintyLass server (maintainer) |
| RV3 | The metagame on `store`'s two engines (DD15) and its change log (DD23): its queries made portable, its schema in both dialects; an existing 1.4.4 SQLite database opens as it is | The metagame's tests pass on both engines; a copy of a real 1.4.4 database opens unchanged and moves to Postgres with `store db move` |

### 4.7 The network (DN)

Built alongside the RU rows (DD16), on the Rust backend; nothing here touches the Node services.

| # | What | Verify |
| --- | --- | --- |
| DN1 | **Spike** (maintainer, a 1.14.7 client): the launcher's relay re-pointed to another gateway mid-session, in Ramsgate and between hunts; and a hunt on one server's game server while the player's backend is another's. Kill criterion: if the client drops the session on a switch, wrong-node joins are refused with a message instead of moved | Ramsgate, a hunt and chat keep working across a switch |
| DN2 | Seams: the network capability in `module-api`, node identity and region, global account and instance ids (DD18) in RU4's seams, the internal home API as a trait with stubs, `[network]` in `host.toml` | RU4's modules build against the seams; ids are network-wide from the first row |
| DN3 | Node links: the cluster CA and node certificates (a `network` command to issue them), mutual TLS on host-only gateway routes, peer health, clock skew checks | A node with a foreign certificate is refused; links survive a peer restart |
| DN4 | Directory: account to home, username claims, the leader and its replicas, registration through the leader | A name taken in one region is refused in every other; a replica catches up after a partition |
| DN5 | Home API and session tickets: the account operations other nodes need (session check, character load, progression, inventory and bounty writes), tickets signed by the home | A hunt hosted in another region saves loot and XP at the player's home |
| DN6 | Instance registry and redirects: global ids resolved to owners, the wrong-node answer, the launcher relay's re-point with a ticket (DD18) | A join that reaches the wrong node lands on the right one (or is refused, per DN1) |
| DN7 | Region ping: the launcher's round-trip measurement to every node, reported to the home in order | The order matches measured latency on the canary |
| DN8 | Matchmaking ring (DD19): home-owned tickets, the widening schedule, the claim protocol, placement on the hosting node's Director, the allowlist feed, XP and objective forwarding | Simulated players in four regions: nobody double-booked, wait time bounded by the ring delays |
| DN9 | Regional Ramsgate (DD20): one shared hub per region, sharding, assignment by lowest ping | Players with different homes meet in one region's hub |
| DN10 | Chat plugin and federation (DD21): rooms, direct messages across nodes, offline delivery, history stores, network-wide mutes | A DM between players on two continents arrives once, both histories hold it |
| DN11 | Admin API across the network: kick, timeout and ban sent through the player's home and applied on every node | A ban on one node refuses the player everywhere |
| DN12 | Test harness: four nodes in Docker with injected latency between them (`netem`), a `network` Compose profile | Runs in CI with the region latencies of the canary |
| DN13 | Joining an existing server to the network: its accounts get it as their home, existing invites and keys keep working. Username clashes: the account registered first keeps the name; every other one gets a temporary name with a suffix (`Fox_2`, shortened to fit 16 characters) and is asked at its next login to pick a new name, with that rename free. Friends, chat and bans reference account ids, so a rename breaks none of them; the login reply carries the new name so the launcher updates the one it stores (RD22) | A Reforged host's database joins; only players whose name clashed see the rename prompt |
| DN14 | Canary network: four nodes in the four regions (maintainer) | The smoke list (section 6) across regions, then a week of play |

### 4.8 Installing from Epic (DE)

| # | What | Verify |
| --- | --- | --- |
| DE1 | **Spike** (maintainer's Epic account): `egs-api` signs in, finds `Jackal` in the library, parses the archived 1.14.7 manifest and fetches a few chunks from each CDN host. Kill criterion: if its parser or chunk handling falls short, `egs-api` keeps sign-in and the library and the manifest parser and chunk fetch are written here, from the format Legendary documents | The fetched chunks hash as the manifest says |
| DE2 | Epic sign-in in the launcher: the login webview that captures the authorization code, the paste fallback, the session in the keystore, sign out; new bridge commands (a seam change, through the integration pass), strings appended in both languages | Sign in, restart, still signed in; sign out clears the keystore entry; no token in any log |
| DE3 | Ownership: `Jackal` in the library, cached for a while; a clear refusal when it isn't | An account without Dauntless can't start a download |
| DE4 | Versions: the Epic build fields in the client profile (`module-api`), filled by the 1.14.7 and 1.12 modules; the launcher's list (compiled-in profiles plus those the joined servers' content serves), with each version's download and install size from its manifest; 1.4.4 listed as "locate your folder" | The list matches the profiles; sizes match the manifests |
| DE5 | Download and install: the manifest from content, the module or the archive (DD22) and checked against its SHA-1 before it is parsed; the 1.14.7 and 1.12.0 manifests committed to rustic-isles and served by content; chunks from the three CDN hosts with failover, decompressed and assembled, every file checked, then the executable's SHA-256; resume, pause, parallel chunks, a free-space check, one folder per version; DR's `net/downloader.rs` brought back for it (amends RD22) | 1.14.7 installs and passes Mystic's checks (611 files, `Version.txt`, the executable's hash); a download killed halfway resumes; a corrupted chunk is fetched again |
| DE6 | The host kit's install command with the pasted-code sign-in, and the Docker image's use of it | A fresh Linux host installs 1.14.7 for its game servers |
| DE7 | UI: sign in, the library check, the version list, download progress (with DU2) | Each screen in both languages |

### 4.9 Change history and rollback (DH)

| # | What | Verify |
| --- | --- | --- |
| DH1 | The change log in `store` (DD23), a seam landing with DF7: written in the same transaction as each change, the actor from the request context, soft deletes and the retention purge for player-facing tables; modules get only `store`'s write path | A change made through any repository appears in the log with its before and after; a test fails a repository that writes around `store` |
| DH2 | Per-person admin keys and roles: keys issued and revoked by command (the owner's stays in `secrets.env`), the role checked on every admin route, the key's id recorded as the actor | A moderator's key can't delete; every change names the key that made it |
| DH3 | `history` and `revert` in the host kit, the console and the admin API: by account, actor or time range, a dry run first, compensating changes, conflicts reported | A partner's mass delete is reverted in one command; a row changed since stays as it is and is reported |
| DH4 | Off-server shipping and restore: the log streamed and snapshots taken on a schedule (SQLite's online backup, `pg_dump` or a base backup), written to an object-locked S3-compatible bucket with write-only credentials (`secrets.env`), the bucket in `host.toml`; `restore --to <time>` on either engine; a lag alert on the status page and the admin API | Restore to a minute before a delete on a copy; with shipping stopped, the alert fires |
| DH5 | The hash chain: each entry hashes the one before, the head ships with the log, `history verify` | An entry edited or removed locally is found and named |
| DH6 | Guarded actions: the destructive and bulk admin actions listed, each with a delay or a second operator's approval, cancelable while it waits | A bulk delete waits; a second operator cancels it |
| DH7 | In a network, each node's log also shipped to another region's node (with DN3) | A node's history survives the node |

### 4.10 Cross-cutting (DX)

| # | What |
| --- | --- |
| DX1 | Docs: `architecture.md` for modules and core plugins, the hosting guides (picking SQLite or Postgres, the history bucket and restores, realms and their relay), the migration guide's paths, a "writing a game module" page |
| DX2 | Security (RX3): `docs/security/module-host.md` for the frame protocol, external module loading and the SHA-256 list; fuzz target `module_frame`. Each module repository keeps the checklists for what it carries (`metagame.md`, `deploy.md` move to dauntless-revived; rustic-rifts gets admission tickets, the pair cipher and relay credentials, with fuzz targets for the ticket and control-message parsers). The network gets `docs/security/network.md`: node certificates and the cluster CA, session tickets, the directory, the claim protocol and chat federation, with fuzz targets for node-link messages and federated stanzas. Installing from Epic gets `docs/security/epic.md`: the Epic session in the keystore, manifests pinned before parsing, file names in a manifest kept inside the install folder, CDN TLS with no redirects; fuzz target `epic_manifest`. Change history gets `docs/security/history.md`: the log's completeness, admin keys and roles, the bucket's write-only credentials and object lock, the hash chain, guarded actions; fuzz target `history_record` for log entries read back during a restore. The relay gets `docs/security/turn-relay.md`: the listener exception, the shared secret, credential lifetimes, relay-only mode and the port range |
| DX3 | CI on Forgejo Actions: a runner with MSVC for the native builds (maintainer), submodule checkout, Bun; `.github/workflows` read as Forgejo reads them, or moved to `.forgejo/workflows`. GitHub stays a read-only source (`upstream`, `mystic`) until DX5 |
| DX4 | Licensing: AGPL-3.0-only in every repository; Mystic's `ADDITIONAL_TERMS.md` and its section 1 notice in rustic-isles' and rustic-rifts' README and `NOTICE.md`, which also say they are modified, unofficial versions (its section 2); DR's attribution in dauntless-revived; libjuice's MPL-2.0 notice in rustic-rifts; `NOTICE.md` here points to all four |
| DX5 | Publishing to GitHub, once the launcher and the server are confirmed working (maintainer): each repository pushed public, submodule URLs and the Cargo registry switched or mirrored, release workflows pointed at it |

## 5. Order

| Wave | Rows | Needs | Hands-on |
| --- | --- | --- | --- |
| 0 | DS5 lists; DF1 spike; DF7's `pq-sys` spike | — | — |
| 0 | DN1 spike | — | A 1.14.7 client, two servers |
| 0 | DE1 spike | — | An Epic account with Dauntless |
| 0 | DS1–DS4 | — | Forgejo: rename, create the repositories, push |
| 1 | DF2 seam; RU1, RU2; DS7; RV1's history work; DU1 | DS2–DS4, DF1 | — |
| 2 | DS6 submodule swap | RU1, RV1, DS7 | — |
| 2 | DF3 controller, DF4 core plugins, DF7 store with DH1; RU3 inventories, RF1 | DF2 | — |
| 3 | RU4 and RF2 seams with DN2; RV1's module crate; DF5 bins; DU2, DU3 | DF3, DF4, DF7 | — |
| 3 | DE2–DE6, then DE7 with DU2 | DF2 (client profile fields), DE1 | An Epic account with Dauntless |
| 3 | DF6 rename sweep, by itself, between integration passes | DS6 | — |
| 4 | RU5 (about nine agents), RU6, RU7; RF3–RF6; RV2, RV3 | RU4, RF2 | Game servers, the 1.4.4 client |
| 4 | DN3, DN4, DN5, DN12 | DN2 | — |
| 4 | DH2–DH6 | DH1 | An object-locked bucket |
| 5 | DN6–DN11, DH7 | DN3–DN5, RU5's chat and auth areas, RU6 | — |
| 5 | RF7, DU5, RF8 | RF3–RF6 | Two PCs, turn-rs on the canary |
| 6 | RU8 replay, then RU9 | RU5–RU7, RF5–RF6 | Canary, 1.14.7 captures |
| 7 | DN13, then DN14 | RU9 on single nodes, DN6–DN12 | Four regional servers |
| any | RU10 spike; RU11 when 1.12.0 traffic exists; DX rows with the rows they cover | — | Host SDKs for both versions |
| last | DX5 GitHub publication | The launcher and server confirmed working | Maintainer |

Today's `main` keeps shipping until DS6. After DS6, hosts update to a DaintyLass build that still runs Node Paradox
from the rustic-isles submodule; the Rust 1.14.7 module replaces it at RU9's cutover. The network is built
alongside, but goes live only after RU9 has run on single servers: a node runs alone until DN13 joins it.

## 6. Things that must not change

`MIGRATION_PLAN.md` section 6 holds, with two changes: the launcher page's look is replaced (DD11), its strings in
both languages are not; and "the host-only routes and headers stay refused" now covers every module's routes. Also:

- An invite made before the split joins the same server after it: scheme, `version`, `fp`.
- A host's `.env` files and `host.toml` read unchanged; the module section has defaults that reproduce today's
  server.
- Each game version's routes and JSON, matched as its Node service answers them.
- Every name DD1 keeps.
- A server that isn't in a network works exactly as one that is with no peers.
- The account key goes only to its home server and certificate; other nodes see session tickets (DD17).
- Every change made through a server can be found in its history and reverted, and a revert never overwrites a
  newer change silently (DD23).

## 7. Amendments to `MIGRATION_PLAN.md`

For the integration pass: RD1 (Paradox becomes a submodule, no longer a subtree at `paradox/`), RD11 and section
4.1 (the layout in section 2 here), RD15 (SQLite by default and Postgres where concurrency pays, DD15), RD17 (no
gate for the Paradox port, DD6), RD22 (DR's downloader comes back for installing from Epic, DD22), RD25 (one admin
key per person with a role, the existing key becoming the owner's, DD23), section 4.5 (the RB rows move to RU and
RF), RP rows (their results move to rustic-isles), RR5 (the page moves to `ui/`, not `rust/launcher/ui`), section
5.1's ownership table (the module repositories and `ui/` own their own files; `crates/module-api` is a seam owned
by DF2).

## 8. Open questions

Answered 2026-10-26: display names (DD4), Mystic's name clause (DD7), how deep the rename goes (DD1), the database
engine (DD15), 1.12.0's verification (DD9), realms (DD14), visibility (DD2, DX5), the network (DD16–DD21), where
manifests come from (DD22), change history (DD23), the TURN relay (DD24) and username clashes (DN13). Mystic's
"local realm" turned out to be the whole backend on one PC with no database server, not storage on a realm host's
PC (section 1).

None open.
