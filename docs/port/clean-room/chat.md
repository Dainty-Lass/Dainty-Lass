# Clean-Room Provenance Record: `crates/chat` (RD27)

- **Component**: `crates/chat` (crate `chat`, `license = "MIT"`, `Copyright (c) 2026 Silent Spud`)
- **Behaviour specification**: `docs/port/spec-rd27-chat.md` at dainty-lass `ced0d55` (branch `docs/rd27-spec-chat`)
- **Clean-room worktree**: `R:\source\dauntless\.wt\chat-mit` (branch `feat/rd27-chat-mit`, cut from `ced0d55`)
- **Implementer**: one Claude Code subagent session, briefed by the RD27 coordinator with the rules below

## 1. Isolation

### 1.1 Rules the implementer worked under

It never read, listed, grepped or `git show`ed anything under `dainty-restricted`, `legacy-revived`, `rustic-*`,
`dainty-dashboard`, `dainty-legacy` or `dainty-lass-ui`, any other `R:\source\dauntless\.wt\*` worktree, or any
historical version of `crates/chat` / `rust/plugins/chat` in dainty-lass's history. It searched nothing on the web.
Its only inputs were the spec, the MIT code already in dainty-lass, and this folder's `hostkit.md` and
`similarity.md` for the record's format. The session's working directory was set by its launcher to another
worktree (`.wt\both\dainty-restricted`); no tool call read from it.

### 1.2 Worktree (`git sparse-checkout list`)

The worktree was added with `--no-checkout`, made sparse before the first checkout, then checked out:

```text
/*
!/ui/
```

`ui/` is the `dainty-lass-ui` submodule. Before checkout, `git ls-tree -r --name-only HEAD | grep -c '^crates/chat'`
gave `0`: `ced0d55` carries no chat source, so there was nothing of it to exclude. `Test-Path crates/chat` was false
until the implementer created the folder. The only `git ls-tree` listings made were of the top level, `plugins/`,
`services/`, `crates/`, `docs/port/`, `docs/port/clean-room/`, `crates/controller/tests`, `crates/service-plugin` and
`crates/controller/src/service_library`.

## 2. Files read

- `docs/port/spec-rd27-chat.md` (whole), `docs/port/clean-room/hostkit.md` (§1-§4), `docs/port/clean-room/similarity.md`
  (§1)
- `Cargo.toml`, `Cargo.lock` (versions of `rand`, `axum`, `tungstenite` only), `.forgejo/workflows/ci.yml` (the "Every
  package is MIT" step), `crates/hostkit/Cargo.toml` (header), `crates/network/Cargo.toml`
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
- `crates/protocol/src/account.rs` (`ACCOUNT_KEY_PREFIX`), `crates/network/src/{lib.rs (exports), link.rs (Service,
  LinkError, PeerLink, NodeService)}`
- Two lines each of `crates/remote/src/enroll.rs` and `crates/admin/src/guard.rs` (how `rand` 0.10 is called)

## 3. Files written

### 3.1 Category B, rewritten from the spec

| File | Spec |
| --- | --- |
| `crates/chat/src/xmpp.rs` | §2.1 (`xmpp`), §2.15-§2.17 |
| `crates/chat/src/nick.rs` | §2.1 (`nick`), §2.14 |
| `crates/chat/src/redact.rs` | §2.18 |
| `crates/chat/src/settings.rs` | §2.1 (`settings`), §2.2 |
| `crates/chat/src/server.rs` | §2.1 (`server`), §2.6-§2.12 |
| `crates/chat/src/muc.rs` | §2.1 (`muc`), §2.13 |
| `crates/chat/src/connection.rs` | §2.1 (`connection`), §2.4, §2.5 |
| `crates/chat/src/lib.rs` | §2.1 (crate root), §2.3 |

### 3.2 New, asked for by the RD27 brief

- `crates/chat/src/service.rs`: `ChatService`, chat as a `service_plugin::Service`, so the compiled-in build
  (`service_plugin::in_process`) and the library (`export_service!`) run one implementation; with `LibraryChatHost`
  (§3.4) and the node link over `ChatLink::peer_call` / `serve_peers`. Written from §3.4 and the MIT
  `service_plugin::chat` API. It overlaps `chat-plugin/src/lib.rs` (Category A): see §5. **Dropped at integration**
  (§7.3).
- `crates/chat/tests/clean_room.rs`: three black-box tests over real sockets (tokio-tungstenite), on the stubs. Kept
  at integration (§7.4).

### 3.3 Stubs for Category A (replaced at integration, §7.1)

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

Also changed: the workspace `Cargo.toml` (`crates/chat` in `members`, `chat` in `[workspace.dependencies]`) and
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

    **Resolved at integration:** `src/service.rs` dropped (§7.3).

## 6. Similarity

Measured by the integration pass (which may read the AGPL code) with `similarity.md` §1's method, on the files as
committed after integration: each Category B file against its namesake in restricted `849f45e` `crates/chat/src/`,
and against every DR file chat came from at legacy-revived `cfcad3eb` (`rust/services/metagame/src/chat/{connection,
mod,muc,nick,presence,xmpp}.rs`, `redact.rs`, `moderation.rs`, `config.rs`, `limits.rs`, `server.rs`, `db.rs`), the
best one named. "Any" counts a line matched by a 3+-line run in any file of that tree. The rows are also in
`similarity.md` §2.7.

| File | New lines | Restricted namesake (old lines) | Matched | % | Any restricted chat file, % | Best DR file | Matched | % | Any DR, % |
| --- | ---: | --- | ---: | ---: | ---: | --- | ---: | ---: | ---: |
| `src/xmpp.rs` | 524 | `xmpp.rs` (482) | 166 | **31.7** | 33.0 | `chat/xmpp.rs` | 164 | 31.3 | 31.9 |
| `src/nick.rs` | 137 | `nick.rs` (125) | 56 | **40.9** | 41.6 | `chat/nick.rs` | 56 | 40.9 | 41.6 |
| `src/redact.rs` | 130 | `redact.rs` (132) | 26 | **20.0** | 23.1 | `redact.rs` | 26 | 20.0 | 23.1 |
| `src/settings.rs` | 91 | `settings.rs` (80) | 58 | **63.7** | 63.7 | `chat/mod.rs` | 24 | 26.4 | 26.4 |
| `src/server.rs` | 1676 | `server.rs` (1659) | 423 | **25.2** | 26.2 | `chat/mod.rs` | 296 | 17.7 | 18.0 |
| `src/muc.rs` | 723 | `muc.rs` (749) | 113 | **15.6** | 15.8 | `chat/muc.rs` | 66 | 9.1 | 9.7 |
| `src/connection.rs` | 757 | `connection.rs` (848) | 85 | **11.2** | 14.7 | `chat/connection.rs` | 33 | 4.4 | 5.8 |
| `src/lib.rs` | 180 | `lib.rs` (219) | 110 | **61.1** | 62.8 | `chat/xmpp.rs` | 3 | 1.7 | 3.3 |
| `tests/clean_room.rs` | 380 | `tests/chat.rs` (629) | 39 | 10.3 | 11.1 | `chat/connection.rs` | 3 | 0.8 | 0.8 |
| The eight `src` files | 4218 | | 1037 | **24.6** | | | | | |

For scale, the spec (§1.1) had the replaced files at 99.6 % (`xmpp`), 100 % (`nick`), 96.2 % (`redact`), 66.2 %
(`server`) and 48.9 % (`muc`) against DR. At the implementer's commit `0d5d4a5` the figures are the same but for
`server.rs` 24.0 %, `muc.rs` 15.4 %, `connection.rs` 9.8 % and `lib.rs` 60.4 %: fitting the callers to the real
Category A signatures (§7.2) gives them restricted's shape (`sender: message.from.to_string()`, the report tuple's
`category, description`), which adds a few matches.

The test listener the integration added to `tests/federation.rs` (59 code lines, §7.5) matches 0 lines of restricted's
`crates/gateway/src/{nodelink,gateway}.rs`.

**What the matches are.** Every run of 6 lines or more was read.

- `lib.rs` (61.1 %): the 13 `pub mod` lines and the re-exports, `StartError` with its two `#[error]` texts, the
  `ChatPlugin` and `Running` signatures, and the `CorePlugin` impl (`name` `"chat"`, `serves` `Chat`, `attach`
  recording a chat module, `detach`). §2.1's table gives every name, field type, derive and text; after rustfmt two
  writers produce the same lines. The bodies of `start` and `stop` don't match.
- `settings.rs` (63.7 %): one 48-line run (`DEFAULT_PORT`, `WS_PATHS`, `NickCheck` with `as_str`, `Settings` with its
  seven fields in §2.1's order and §2.2's defaults) and a 10-line run of `problems`' tail and the head of the
  `chat_stays_on_loopback` test. `problems` itself, the one piece of logic, differs.
- `nick.rs` (40.9 %): a 22-line run of `FALLBACK_NAME` and `NickReason` with `as_str` (§2.1), the head of the test §5.2
  names, and two 8-line runs in the percent-encoding helpers (`out.push_str(&format!("%{b:02X}"))`, `i += 1`,
  `String::from_utf8(out).ok()`), which §2.14 specifies byte by byte.
- `xmpp.rs` (31.7 %): the namespace constants (§2.1 gives every value), `Node`, `Element`, `Jid` and `Bucket` as §2.1
  declares them, the end of `Bucket::take`, `escape`'s five entities, `has_control_character`, and the trace-redaction test cases (§2.17's
  examples). The one run inside the parser, 6 lines of locals (`attrs`, `end_tag`, `self_closing`, `quote`,
  `attr_name`, `pos`), follows §2.15's list of the parser's state, in its order; the state machine itself has no run
  over 5 lines.
- `server.rs` (25.2 %): 90 runs, almost all 3 to 9 lines. The longest, 27 lines, is `EndReason` and `as_str`, whose ten
  reasons and texts §2.7 lists in that order. Next come the `Options` struct and `Default` (§2.1), the `Event` variants
  §2.1 requires, the base64 alphabet table §2.8.3 gives (`-` as `+`, `_` as `/`), the full JID's `undefined` (§2.0),
  the `/report` answers (§4's texts) and the `NewDirect` literals whose fields `history.rs` fixes. The rest is
  `let Some(session) = self.sessions.get_mut(&id) else { return; };` and `if session.ended { return; }`, guards that a
  task holding a session map repeats everywhere.
- `muc.rs`, `connection.rs`, `redact.rs` and `clean_room.rs`, all under 25 %: no run over 9 lines; constants, error
  texts, `json!` shapes and guards.

**Verdict.** Nothing is similar beyond what the spec fixes. Every file above 25 % gets there through §2.1's public API,
given down to field order and derives, and §2.2-§2.17's constants, error texts and wire templates. The logic between
them (the parser, the task, login, rooms, the listener, the loopback API) shares only short idiomatic guards with
restricted and DR. Against DR the rewrite is at 1.7-41 % where the files it replaces were at 20-100 %.

## 7. Integration

Done by the RD27 integration pass in `R:\source\dauntless\.wt\chat-int` (detached at `0d5d4a5`, pushed to
`feat/rd27-chat-mit`), which, unlike the implementer, read restricted `849f45e` and DR.

### 7.1 Category A, copied in

From restricted `849f45e` by `git show`, byte for byte, with only `// SPDX-License-Identifier: MIT` and
`// Copyright (c) 2026 Silent Spud` prepended to each Rust file under `plugins/` (spec §6.1): the eight migrations,
`src/{env,federation,history,hooks,moderation,naming}.rs`, `tests/{chat,federation,history,dashboard_live}.rs`,
`crates/chat-plugin/{Cargo.toml,src/lib.rs,tests/load.rs}`, `fuzz/fuzz_targets/chat_{stanza,federation}.rs` (with
their `[[bin]]` entries and `chat` in `fuzz/Cargo.toml`) and `docs/security/chat.md`. Each copied Rust file, less its
first three lines, is identical to `849f45e`, except `tests/federation.rs` (§7.5) and the fuzz target (§7.6).

Manifests: `crates/chat/Cargo.toml` is restricted's with the spec's three edits (licence `MIT`, the description
without "from Dauntless Revived", no `gateway` dev-dependency), plus `base64` and tokio's `io-util` (the rewrite's
loopback client) and a `tokio-rustls` dev-dependency (§7.5) pinned as `crates/network` pins it.
`crates/chat-plugin/Cargo.toml` takes licence `MIT` and `network = { path = "../network", default-features = false }`,
since dainty-lass has no `network` workspace entry. `crates/chat-plugin` joins the workspace `members`.

### 7.2 The B callers fitted to the real signatures

No Category A file was edited to fit. In the B files:

| Where | Assumed (§3.3) | Real | Edit |
| --- | --- | --- | --- |
| `lib.rs`, `connection.rs`, `server.rs` | a bounded `mpsc::Sender<Event>` (4096) | `Federation::attach` takes `mpsc::UnboundedSender<Event>` | the queue is `mpsc::unbounded_channel()`, `QUEUE` gone, sends no longer `.await` |
| `server.rs` | `Login { authcid, token, domain }` owned | `&'a str` fields | `Login` built from references |
| `server.rs`, `connection.rs`, `muc.rs` | `History` takes `&GlobalId`; `NewDirect` / `NewRoom` hold `GlobalId` | text (`&str`, `String`) | `.to_string()` at `remember_account` (module as `as_str()`), `pending_for`, `mark_delivered` (now `Vec<i64>` by value), `direct_between`, and in every `NewDirect` / `NewRoom` |
| `server.rs`, `muc.rs` | `Moderation::rate -> (Verdict, String)` | `-> Rated { verdict, shown }` | the two fields read |
| `server.rs` | `async report -> bool`; `Report::source: String`; `parse_report -> Option<Result<ParsedReport, String>>` | sync `report`; `&'static str`; `Option<Result<(String, &'static str, String), ()>>` | `report` takes the tuple and doesn't await |

### 7.3 `ChatService`: chat-plugin's kept, `src/service.rs` dropped

`src/service.rs` didn't cover what `chat-plugin/src/lib.rs` did. It opened the node database itself rather than
`History::beside` it (chat's tables would have gone into the game's database), skipped the `REFORGED_DB_ENGINE` check
and the database folder's creation, sent no `stopping` / `stopped` status, answered `BadToken` instead of `Unavailable`
when no module is loaded, and tried a player homed elsewhere through the first module only. Nothing in dainty-lass
called it. `chat-plugin` keeps its own `ChatService` and `export_service!`, and `chat` no longer depends on
`service-plugin`. Building chat into the core later means moving chat-plugin's `ChatService` (Category A, so already
MIT) into `chat`, not reviving `service.rs`.

### 7.4 `tests/clean_room.rs` kept

Its ghost-replacement test and its `/__chat/*` checks reach code no Category A test does. Edits: `History` opened on a
SQLite file in a temporary folder instead of the stub's `History::memory`; `ChatHost::login` with the real lifetime;
the party room spelled `Party-{naming::room_id_text}`, as the real `naming` reads it; account ids as text for
`History`; and a `Running::tick` before reading delivery marks, which the bind writes after it sends the waiting
stanzas (spec §2.11; the stub wrote them before the client could read).

### 7.5 The mutual-TLS test without the gateway (spec §6.3)

Rewritten, not left to restricted. `federation_works_end_to_end_over_mutual_tls_gateways` is now
`..._over_mutual_tls_node_links`. Each node's `/__node` listener is a test helper (`node_link_server`) built from
`network`'s MIT API only: `NodeCredentials::server_config` for mutual TLS, `cluster::peer_identity` for the caller,
`NodeEndpoint::{admit, serve}` for the answer, and hyper's HTTP/1 server. The client side (`TlsPeerLink`), the
federation, both chats and the assertions are unchanged. It was written from `network`'s source, not the gateway's, and
shares no line with it (§6). What it no longer covers is the gateway's own `/__node` route, which restricted's gateway
tests keep.

### 7.6 The `chat_federation` fuzz target (spec §7 item 2)

A commit of its own. The target asserted `!mid.is_empty()` and `body.len() <= BODY_LIMIT`, while `validate_envelope`
accepts an empty `mid` (it checks one only when present) and bounds the body as 1 to `BODY_LIMIT` chars, so a valid
envelope without a mid, or with 2048 multi-byte chars, would have crashed it. It now asserts `mid.len() <= 128` and
`(1..=BODY_LIMIT).contains(&body.chars().count())`. The validator is unchanged.

### 7.7 Verification

- `cargo test -p chat`: 34 unit tests, `tests/chat.rs` 8, `tests/federation.rs` 6 (the mutual-TLS one included),
  `tests/history.rs` 2 (the Postgres half skipped: no `REFORGED_TEST_POSTGRES_URL`), `tests/clean_room.rs` 3 (three
  runs in a row), `tests/dashboard_live.rs` 1 ignored (needs a running dainty-dashboard); 0 failed
- `cargo build -p chat-plugin`, then `cargo test -p chat-plugin`: `players_chat_through_the_library_the_core_loads`
  passed, through the built library and `controller`
- `cargo check --bin chat_stanza --bin chat_federation` in `fuzz/`: clean (not fuzzed)
- `cargo fmt --all --check`: clean; `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean
- `cargo test --workspace --locked --no-fail-fast`: 669 passed, 0 failed, 2 ignored (76 test binaries)
- "Every package is MIT" (`cargo metadata --no-deps --locked`, the CI step): 20 packages, 0 not MIT
- `cargo tree -e normal -p chat --depth 1`: `module-api`, `network`, `protocol`, `service-kit` and `store` are the only
  workspace crates; no AGPL crate
- Not run against the real 1.14.7 client.
