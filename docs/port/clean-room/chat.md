# Clean-Room Provenance Record: `plugins/chat` (RD27)

- **Component**: `plugins/chat` (crate `chat`, `license = "MIT"`, `Copyright (c) 2026 Silent Spud`)
- **Behaviour specification**: `docs/port/spec-rd27-chat.md` at dainty-lass `ced0d55` (branch `docs/rd27-spec-chat`)
- **Clean-room worktree**: `R:\source\dauntless\.wt\chat-mit` (branch `feat/rd27-chat-mit`, cut from `ced0d55`)
- **Implementer**: one Claude Code subagent session, briefed by the RD27 coordinator with the rules below

## 1. Isolation

### 1.1 Rules the implementer worked under

It never read, listed, grepped or `git show`ed anything under `dainty-restricted`, `legacy-revived`, `rustic-*`,
`dainty-dashboard`, `dainty-legacy` or `dainty-lass-ui`, any other `R:\source\dauntless\.wt\*` worktree, or any
historical version of `plugins/chat` / `rust/plugins/chat` in dainty-lass's history. It searched nothing on the web.
Its only inputs were the spec, the MIT code already in dainty-lass, and this folder's `hostkit.md` and
`similarity.md` for the record's format. The session's working directory was set by its launcher to another
worktree (`.wt\both\dainty-restricted`); no tool call read from it.

### 1.2 Worktree (`git sparse-checkout list`)

The worktree was added with `--no-checkout`, made sparse before the first checkout, then checked out:

```text
/*
!/ui/
```

`ui/` is the `dainty-lass-ui` submodule. Before checkout, `git ls-tree -r --name-only HEAD | grep -c '^plugins/chat'`
gave `0`: `ced0d55` carries no chat source, so there was nothing of it to exclude. `Test-Path plugins/chat` was false
until the implementer created the folder. The only `git ls-tree` listings made were of the top level, `plugins/`,
`services/`, `crates/`, `docs/port/`, `docs/port/clean-room/`, `crates/controller/tests`, `crates/service-plugin` and
`crates/controller/src/service_library`.

## 2. Files read

- `docs/port/spec-rd27-chat.md` (whole), `docs/port/clean-room/hostkit.md` (§1-§4), `docs/port/clean-room/similarity.md`
  (§1)
- `Cargo.toml`, `Cargo.lock` (versions of `rand`, `axum`, `tungstenite` only), `.forgejo/workflows/ci.yml` (the "Every
  package is MIT" step), `crates/hostkit/Cargo.toml` (header), `plugins/network/Cargo.toml`
- `crates/service-plugin/src/lib.rs`, `crates/service-plugin/src/chat.rs`, `crates/service-plugin/Cargo.toml`
- `crates/controller/src/service_library/chat.rs` (lines 1-80), `crates/controller/tests/chat-service/{Cargo.toml,
  src/lib.rs}` (the MIT test library on the same protocol)
- `crates/module-api/src/{chat.rs, lib.rs}`; from `module.rs`, `manifest.rs`, `network.rs` and `ids.rs` only
  `CorePlugin`, `CoreCapability`, `ModuleError`, `ErrorKind`, `Capabilities`, `ModuleManifest`'s fields,
  `NodeIdentity`, `GlobalId` and the `name_type!` constructors
- `crates/store/src/lib.rs` (exports, `Error`), `crates/store/src/history.rs` (`Actor`), `crates/store/src/engine.rs`
  (`DatabaseConfig::from_lookup`)
- `crates/service-kit/src/{serve.rs, js.rs}`, `crates/service-kit/src/env.rs` (`Env`), `crates/service-kit/src/context.rs`
  (method names), `crates/service-kit/Cargo.toml`
- `crates/protocol/src/account.rs` (`ACCOUNT_KEY_PREFIX`), `plugins/network/src/{lib.rs (exports), link.rs (Service,
  LinkError, PeerLink, NodeService)}`
- Two lines each of `crates/remote/src/enroll.rs` and `services/admin/src/guard.rs` (how `rand` 0.10 is called)

## 3. Files written

### 3.1 Category B, rewritten from the spec

| File | Spec |
| --- | --- |
| `plugins/chat/src/xmpp.rs` | §2.1 (`xmpp`), §2.15-§2.17 |
| `plugins/chat/src/nick.rs` | §2.1 (`nick`), §2.14 |
| `plugins/chat/src/redact.rs` | §2.18 |
| `plugins/chat/src/settings.rs` | §2.1 (`settings`), §2.2 |
| `plugins/chat/src/server.rs` | §2.1 (`server`), §2.6-§2.12 |
| `plugins/chat/src/muc.rs` | §2.1 (`muc`), §2.13 |
| `plugins/chat/src/connection.rs` | §2.1 (`connection`), §2.4, §2.5 |
| `plugins/chat/src/lib.rs` | §2.1 (crate root), §2.3 |

### 3.2 New, asked for by the RD27 brief

- `plugins/chat/src/service.rs`: `ChatService`, chat as a `service_plugin::Service`, so the compiled-in build
  (`service_plugin::in_process`) and the library (`export_service!`) run one implementation; with `LibraryChatHost`
  (§3.4) and the node link over `ChatLink::peer_call` / `serve_peers`. Written from §3.4 and the MIT
  `service_plugin::chat` API. It overlaps `chat-plugin/src/lib.rs` (Category A): see §5.
- `plugins/chat/tests/clean_room.rs`: three black-box tests over real sockets (tokio-tungstenite), on the stubs.

### 3.3 Stubs for Category A (replaced at integration)

Each opens with `CLEAN-ROOM STUB (RD27)`. Their APIs are this rewrite's reading of §1.4 and §2-§3; nothing in them is
the real file.

| Stub | What it assumes |
| --- | --- |
| `src/hooks.rs` | `ChatHost` methods return `BoxFuture`; `Login { authcid, token, domain }`; `Player { account, module, name }`; `default_jid -> Option<module_api::chat::Jid>`; `default_rules(&RoomKind)`; `is_unsupported(&ModuleError)` |
| `src/history.rs` | in memory; `async` methods returning `Result<_, store::Error>`; `account_of -> Option<String>`; `mute_of(&str)`, `set_mute(Mute, Actor)`, `clear_mute(&str, Actor) -> bool`; `pending_for` / `mark_delivered` / `direct_between` take `&GlobalId`; `NewDirect` / `NewRoom` hold `GlobalId`s; `History::open(DatabaseConfig)` async; `History::memory()` is the stub's own |
| `src/federation.rs` | the `Federation` trait's methods and signatures (§5); `Envelope` struct variants; `DirectWire` / `RoomWire` fields; `ChatFederationConfig::{new(NodeIdentity), with_leader(NodeId, Vec<(NodeId, Region)>)}`; `ChatFederation::new(config, Arc<dyn PeerLink>) -> Arc<Self>`; the stub federates nothing |
| `src/moderation.rs` | `rate(&str, Channel) -> (Verdict, String)`; `record(Line)`; `async report(Report) -> bool`; `parse_report -> Option<Result<ParsedReport, String>>`; `Line::channel: &'static str`; `Moderation::start(DashboardSettings)` |
| `src/naming.rs` | `room_kind_of(&str, &NodeId, &Region)`; `Party-<ULID>` / `Hunt-<ULID>` owned by this node |
| `src/env.rs` | `settings(&Env) -> Result<Settings, String>`, `dashboard(&Env) -> Result<Option<DashboardSettings>, String>`, `database(&Env) -> Result<DatabaseConfig, String>` |
| `Cargo.toml` | a placeholder manifest; the A manifest (with §1.2's three edits) replaces it, and must keep `axum`, `base64`, `chrono`, `futures-util`, `network` (no default features), `protocol`, `rand`, `service-kit`, `service-plugin`, `sha2`, `store`, `subtle`, `thiserror`, `tokio` (`net`, `io-util`, `time`, `sync`, `macros`), `tracing`, and `tokio-tungstenite` for tests |

Also changed: the workspace `Cargo.toml` (`plugins/chat` in `members`, `chat` in `[workspace.dependencies]`) and
`Cargo.lock`.

## 4. Verification

- `cargo test -p chat`: 16 unit + 3 black-box tests, 0 failed
- `cargo clippy -p chat --all-targets -- -D warnings`: clean
- `cargo fmt --all --check`: clean; `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean
- `cargo test --workspace --locked --no-fail-fast`: 634 passed, 0 failed, 1 ignored (70 test binaries)
- "Every package is MIT" (`cargo metadata --no-deps --locked`, the CI step): 19 packages, 0 not MIT
- Not run against the real 1.14.7 client, nor against the Category A tests, which are not in this worktree.

## 5. Ambiguities and the assumptions taken

1. **Category A signatures.** §1.4 names the A items B calls but not their signatures; §3.3 above lists each
   assumption. Where the real file differs, the B callers change, never the A file.
2. **`Event` and the queue.** The queue is a bounded `tokio::sync::mpsc::Sender<Event>` (4096); `Federation::attach`
   takes that sender. Besides §2.1's three variants: `Admit`, `Open`, `Frame`, `Gone`, `Tick`, `Shutdown`.
3. **Seen mids for remote and pulled messages.** "Already seen" is checked by remembering: a remote direct message, a
   remote room message and a pulled or waiting message are recorded as seen when handled, so a retry is not shown
   twice.
4. **SASL failure throttle.** The address waits 10 minutes once it has 10 failures inside 10 minutes (at the 10th);
   bind timeouts likewise at the 3rd.
5. **Unbound sessions per address** count non-ended sessions from that address that have not bound.
6. **`/report` grammar** is `moderation::parse_report`'s (A); the stub's is only good enough for the tests.
7. **`/__chat/history/*` parameters**: an empty value counts as missing.
8. **Trace redaction** matches the closing tag ASCII case-insensitively, as the opening one.
9. **`stamp`** gives the epoch's stamp outside JavaScript's Date range (±8.64e15 ms); years past 9999 are written as
   chrono writes them, not JavaScript's `+0YYYYY`.
10. **`parse_frame` text after the root** is appended only while an element is open, per §2.15; a frame deeper than a
    32 KiB message can nest was not tested for drop depth.
11. **Library login** treats a `Timeout` from a module as unavailable, as `Unavailable` is.
12. **`ChatService` vs `chat-plugin`.** The brief asked for the `Service` impl in this crate; §1.2 has
    `chat-plugin/src/lib.rs` (A) already holding `ChatService` and `LibraryChatHost`. The integration pass keeps one:
    either `chat-plugin` becomes `service_plugin::export_service!(chat::service::ChatService::new());` and A's
    `ChatService` is dropped, or `src/service.rs` is dropped. This one was written from §3.4 without seeing A's.

## 6. Similarity

Left for the reviewer: measure `plugins/chat/src/{xmpp,nick,redact,settings,server,muc,connection,lib}.rs` against
restricted `849f45e` `plugins/chat/src/` and DR's `services/metagame/src/chat/` (legacy-revived `cfcad3eb`) with
`similarity.md` §1, and add the rows there.
