# Clean-Room Behaviour Specification: `crates/hostkit` (DL3a)

> **Scope & Purpose (DD26 / DL3a):** This document is the clean-room behaviour specification for extracting and reimplementing the MIT-licensed `rust/crates/hostkit` crate in row **DL3b**, leaving a thin AGPL-3.0-or-later shell in `rust/services/host` for features that link or invoke Dauntless Revived (`DR`) services.
>
> All types, function signatures, CLI flags, configuration schemas (`host.toml`, `host.yaml`, `host.json`, `secrets.env`, `<service>.env`), file paths, wire formats, exit codes, error strings, and test invariants are specified below from observable contracts (`docs/hosting.md`, `docs/hosting-linux.md`, `docs/app-structure-plan.md`, and the integration/unit test suite) without copying implementation code or referencing prior Node host-kit internals.

---

## 1. Module Provenance & Licensing Classification

Every source file currently in `rust/services/host/src/` falls into one of three categories under **DD26**:

### 1.1 Category A — Original DaintyLass Modules (Already MIT-Eligible; Move or Keep as-is)

These modules were written originally for DaintyLass in Rust with no `ReforgedHost` or `Dauntless Revived` (`DR`) derivation. They may move into `crates/hostkit` (or remain shared) without clean-room reimplementation:

| Current Path in `services/host/src/` | Purpose & Provenance | Destination in DL3b |
| --- | --- | --- |
| `error.rs` | `HostError` (`Message`, `Io`, `NotImplemented`) and `Result<T, HostError>` wrapper | `crates/hostkit` |
| `hosttoml.rs` | Round-trip TOML <-> JSON structural converter (`document`, `to_toml`, `key_in_toml`, `key_to_toml`, `check_keys`) | `crates/hostkit` |
| `hostconfig/yaml.rs` | YAML <-> JSON structural converter (`document`, `to_yaml`) for `host.yaml` / `host.yml` | `crates/hostkit` |
| `hostconfig/guard.rs` | `[guard]` configuration section (`GuardConfig`) for rate-limiting, bans, and trusted proxies (DH4) | `crates/hostkit` |
| `hostconfig/modules.rs` | `[modules]` and `[[modules.external]]` configuration section (`ModulesConfig`, `ExternalModuleConfig`) (DF3) | `crates/hostkit` |
| `epicinstall.rs` | Epic Games Store Dauntless 1.14.7 manifest downloader, chunk verifier, and resume state (`Plan`, `Files`, `Env`, `install_game`, `sign_out`) (DE6 / DD22) | `crates/hostkit` |
| `guarded.rs` | Helper checking whether the in-process allowlist guards game module UDP ports (`allowlist_guards_modules`) (DF4d) | `crates/hostkit` |
| `repl.rs` | Interactive console parser and executor (`ConsoleCommand`, `parse_command`, `execute_command`, `run_repl`, `plain`) including `module ...` commands | `crates/hostkit` |
| `tui.rs` | `ratatui` terminal dashboard (`TuiState`, `KeyAction`, `on_key`, `run_tui`) | `crates/hostkit` |
| `serve.rs` | Unified in-process `ServerController` composition (`ServeOptions`, `compose`, `new_registry`, `list_modules`, `ensure_server_files`, `run`, `prepare_embedded`, `minimal_main`) — *Note: `legacy` hooks and AGPL feature-gated service constructors (`gateway`, `allowlist`, `content`, `admin`) cross the shell boundary via traits/callbacks* | Split: controller/module orchestration in `crates/hostkit`; AGPL service wiring in `services/host` |
| `commands/backup.rs` | `backup` subcommand (`BackupEnv`, `find_mongodump_in`, `run`, `run_with`) | `crates/hostkit` |
| `commands/history.rs` | `history` and `revert` subcommands (`Select`, `Job`, `Surface`, `Done`, `parse_console`, `run`, `run_history`, `run_revert`) (DD23 / DH3b) | `crates/hostkit` (or wired via `admin` feature) |
| `commands/install_game.rs` | `install-game` subcommand (`Configured`, `Deps`, `run`, `run_with`, `configure`) (DE6) | `crates/hostkit` |
| `commands/network.rs` | `network` subcommand (`ca-init`, `issue-node-cert`, `list`, `status`, `join`) (DN3 / DN13) | `crates/hostkit` |
| `commands/setup/database.rs` | Database seam (`Database::Mongo`, `database`, `database_notes`) | `crates/hostkit` |
| `commands/setup/secrets.rs` | Cryptographic secret generator (`new_secrets`, `pem`) using `aws-lc-rs` | `crates/hostkit` |
| `commands/start/binaries.rs` | Service binary locator and Rust/Node switch (`FrontService`, `rust_ported`, `Launch`, `Programs`, `find_node`) | `crates/hostkit` |

> **Note on Planned Module Names in `dainty-lass-plan.md`:** Row DL3a in `docs/dainty-lass-plan.md` also lists `commands/modules.rs`, `postgres.rs`, and `turn.rs` as MIT-eligible modules if present. In the current tree, module console commands live in `repl.rs` and `hostconfig/modules.rs`, database setup lives in `commands/setup/database.rs` and `hostconfig.rs` (`DatabaseSettings` / `node_database_config`), and relay/network configuration lives in `commands/network.rs` and `hostconfig.rs` (`NetworkConfig`). All of those are Category A original DaintyLass code and are MIT-eligible.

---

### 1.2 Category B — The 22 Ported Host-Kit Modules (Subject to DL2 Relicensing or DL3b Clean-Room Reimplementation)

These 22 modules were ported during rows RH1–RH5 and must be reimplemented from this clean-room specification in `rust/crates/hostkit` under `MIT` (unless re-licensed upstream under DL2):

1. `paths` (`src/paths.rs`)
2. `envfile` (`src/envfile.rs`)
3. `hostconfig` (`src/hostconfig.rs`)
4. `run` (`src/run.rs`)
5. `services` (`src/services.rs`)
6. `services/tree` (`src/services/tree.rs`)
7. `invite` (`src/invite.rs`)
8. `doctor` (`src/doctor.rs`)
9. `mongo` (`src/mongo.rs`)
10. `mongo/wire` (`src/mongo/wire.rs`)
11. `gamedata` (`src/gamedata.rs`)
12. `gamepack` (`src/gamepack.rs`)
13. `captures` (`src/captures.rs`)
14. `preset` (`src/preset.rs`)
15. `progression` (`src/progression.rs`)
16. `publish` (`src/publish.rs`)
17. `linux` (`src/linux.rs`)
18. `selfupdate` (`src/selfupdate.rs`)
19. `wine` (`src/wine.rs`)
20. `cli` (`src/cli.rs`)
21. `commands/*` (`src/commands/{mod,doctor,gamedata,invite,multipliers,preset,publish_runtime,setup,update}.rs`, `src/commands/setup/{firewall,install,options}.rs`, `src/commands/start/{mod,control}.rs`)
22. `setup/paradox` (`src/commands/setup/paradox.rs`)

---

### 1.3 Category C — AGPL-3.0-or-later Shell Modules (Cite `DR` / Link AGPL Crates; Remain in `rust/services/host`)

These files directly cite Dauntless Revived (`DR`) or link AGPL-3.0-or-later service crates and therefore **remain in `rust/services/host`** under `AGPL-3.0-or-later`:

| Path in `services/host/src/` | Why it Remains in the AGPL Shell (`services/host`) |
| --- | --- |
| `legacy/mod.rs` & `legacy/off.rs` | Implements the Dauntless 1.4.4 (`legacy` / `both` edition) stack: generates `metagame.env` and `deploy.env` for `DR`'s `metagame` and `deploy` servers, runs `DR` SQLite migrations (`modules/legacy-revived/rust/crates/database/migrations`), inserts 1.4.4 invite codes into `metagame.sqlite`'s `invite_codes` table, checks `UndauntedInternalServer.dll` and `mystic_loader.ini`, and plans the 1.4.4 supervised processes. |
| `lib.rs` (service entrypoints & `BUILT_IN_SERVICES`) | Links the `gateway`, `allowlist`, `content`, and `admin` service crates (`service_main`, `console_main`, `minimal_main`) and dispatches to `hostkit`. |

---

## 2. Clean-Room Specifications for the 22 Ported Modules

### 2.1 Module 1: `paths`

#### Responsibility
Defines static port assignments, pinned game build constants for Dauntless 1.14.7, repository root discovery, and canonical filesystem paths (`Paths`) relative to the server root directory.

#### Constants & Types

```rust
pub const RUNTIME_FILE_NAMES: [&str; 2] = ["Rustic-Isles.dll", "winmm.dll"];
pub const GAME_CHANGELIST: u32 = 647472;
pub const GAME_VERSION_TEXT: &str = "dauntless_rel-1.14.7_Shipping_2024-04-01_14-59-21_647472";
pub const PINNED_EXE_SHA256: &str = "f3f95fbf22709ef077a11632ed95d675b7209b7e0172d9ecb9ba7d02a1fbb66b";
pub const MAX_HUNTS: u32 = 10;
pub const DEFAULT_LEGACY_PUBLIC_PORT: u16 = 4443;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ports {
    pub backend: u16,
    pub director: u16,
    pub content: u16,
    pub admin: u16,
    pub allowlist: u16,
    pub public_gateway: u16,
    pub private_gateway: u16,
    pub relay: u16,
    pub mongo: u16,
    pub game_first: u16,
    pub game_last: u16,
    pub game_lowest: u16,
}

pub const PORTS: Ports = Ports {
    backend: 61000,
    director: 61001,
    content: 61002,
    admin: 61003,
    allowlist: 61005,
    public_gateway: 443,
    private_gateway: 61000,
    relay: 61100,
    mongo: 27017,
    game_first: 8770,
    game_last: 8777,
    game_lowest: 8766,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyPorts {
    pub metagame: u16,
    pub deploy: u16,
    pub content: u16,
    pub allowlist: u16,
    pub private_gateway: u16,
    pub chat: u16,
    pub game_first: u16,
    pub game_last: u16,
}

pub const BOTH_LEGACY_PORTS: LegacyPorts = LegacyPorts {
    metagame: 61010,
    deploy: 61011,
    content: 61012,
    allowlist: 61015,
    private_gateway: 61010,
    chat: 61099,
    game_first: 8780,
    game_last: 8787,
};
```

#### Public Functions & `Paths` Layout

- `pub fn game_exe_relative() -> PathBuf`:
  Returns `Archon/Binaries/Win64/Dauntless-Win64-Shipping.exe` joined using platform path separators.
- `pub fn game_port_range(max_hunts: u32) -> (u16, u16)`:
  - Let `hunts = max_hunts.clamp(1, MAX_HUNTS) as u16`.
  - Returns `(PORTS.game_first.min(PORTS.game_last - 1 - hunts), PORTS.game_last)` (i.e. `(8770, 8777)` for `1..=6` hunts, and `(8776 - hunts, 8777)` for `7..=10` hunts, down to `(8766, 8777)` at `10`).
- `pub fn find_repo_root(start: &Path) -> Result<PathBuf, HostError>`:
  Walks `start` (canonicalized if possible) and up to 8 parent directories (`0..=8`). Returns `Ok(dir)` for the first directory containing either `rust/services/host/Cargo.toml` or `release.json`. If none matches, returns `Err(HostError::message(format!("could not find the project root from {}", start.display())))`.
- `pub fn locate_repo_root() -> Result<PathBuf, HostError>`:
  Tries `std::env::current_dir()` via `find_repo_root`, then `std::env::current_exe()`'s parent directory via `find_repo_root`.
- `#[derive(Debug, Clone, PartialEq, Eq)] pub struct Paths`:
  Constructed by `Paths::new(repo: impl Into<PathBuf>) -> Self` with exact fields relative to `repo`:
  - `repo`: `<repo>`
  - `preset`: `<repo>/preset`
  - `host_kit`: `<repo>/rust/services/host`
  - `gateway_dir`: `<repo>/services/gateway`
  - `content_dir`: `<repo>/services/content`
  - `workspace`: `<repo>/.workspace`
  - `paradox`: `<repo>/modules/rustic-isles/reference/1.14.7`
  - `isles`: `<repo>/modules/rustic-isles`
  - `revived`: `<repo>/modules/legacy-revived`
  - `old_upstream`: `<repo>/.workspace/Mystic-Paradox`
  - `backend_dir`: `<repo>/modules/rustic-isles/reference/1.14.7/ParadoxBackend`
  - `director_dir`: `<repo>/modules/rustic-isles/reference/1.14.7/ParadoxDirector`
  - `runtime_source`: `<repo>/modules/rustic-isles/native/runtime`
  - `sdk_header`: `<repo>/modules/rustic-isles/native/runtime/SDK.hpp`
  - `sdk_dir`: `<repo>/modules/rustic-isles/native/runtime/SDK`
  - `runtime_dll`: `<repo>/modules/rustic-isles/native/runtime/x64/Release/Rustic-Isles.dll`
  - `loader_dir`: `<repo>/native/runtime-loader`
  - `loader_dll`: `<repo>/native/runtime-loader/target/x86_64-pc-windows-msvc/release/winmm.dll`
  - `exporter_dir`: `<repo>/modules/rustic-isles/native/catalog-exporter`
  - `exporter_dll`: `<repo>/modules/rustic-isles/native/catalog-exporter/x64/Release/CatalogExporter.dll`
  - `backend_game_data`: `<repo>/modules/rustic-isles/reference/1.14.7/ParadoxBackend/game-data`
  - `director_game_data`: `<repo>/modules/rustic-isles/reference/1.14.7/ParadoxDirector/game-data`
  - `raw_game_data`: `<repo>/modules/rustic-isles/reference/1.14.7/ParadoxBackend/game-data/raw`
  - `exports`: `<repo>/modules/rustic-isles/reference/1.14.7/Items_Analysis`
  - `store_captures`: `<repo>/modules/rustic-isles/reference/1.14.7/DauntlessEndpointDocumentation`
  - `captures_pin`: `<repo>/modules/rustic-isles/reference/1.14.7/ParadoxBackend/store-captures.json`
  - `runtime_publish`: `<repo>/.workspace/runtime`
  - `config`: `<repo>/.workspace/config`
  - `host_toml`: `<repo>/.workspace/config/host.toml`
  - `host_json`: `<repo>/.workspace/config/host.json`
  - `tls`: `<repo>/.workspace/tls`
  - `cert_file`: `<repo>/.workspace/tls/gateway-cert.pem`
  - `key_file`: `<repo>/.workspace/tls/gateway-key.pem`
  - `mongo_data`: `<repo>/.workspace/mongodb`
  - `logs`: `<repo>/.workspace/logs`
  - `gameserver_logs`: `<repo>/.workspace/logs/gameservers`
  - `branding`: `<repo>/.workspace/branding`
  - `news_file`: `<repo>/.workspace/config/news.json`
  - `bin`: `<repo>/.workspace/bin`
  - `gameserver_wrapper`: `<repo>/.workspace/bin/reforged-gameserver`
  - `wine_prefix`: `<repo>/.workspace/wine`
  - `proton_data`: `<repo>/.workspace/proton`
  - `systemd`: `<repo>/.workspace/systemd`

---

### 2.2 Module 2: `envfile`

#### Responsibility
Parses, formats, and updates `.env` files while preserving insertion order, comments, blank lines, line endings (`\n` vs `\r\n`), and UTF-8 BOM (`\u{feff}`) when editing in-place.

#### Public Types & Functions

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvMap(Vec<(String, String)>);

impl EnvMap {
    pub fn new() -> Self;
    pub fn get(&self, key: &str) -> Option<&str>;
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>);
    pub fn iter(&self) -> std::slice::Iter<'_, (String, String)>;
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
}
// Also implements IntoIterator for EnvMap and &EnvMap, and FromIterator<(String, String)>.

pub fn valid_key(key: &str) -> bool;
pub fn parse_env(text: &str) -> EnvMap;
pub fn format_value(value: &str) -> Result<String, HostError>;
pub fn format_env<K: AsRef<str>, V: AsRef<str>>(header: &[&str], entries: &[(K, V)]) -> Result<String, HostError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvDocument;

impl EnvDocument {
    pub fn parse(text: &str) -> Self;
    pub fn values(&self) -> EnvMap;
    pub fn get(&self, key: &str) -> Option<String>;
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), HostError>;
    pub fn to_text(&self) -> String;
}
```

#### Behavioral Contract & Invariants
1. **Key Validation (`valid_key`):** Non-empty ASCII string matching `/^[A-Za-z_][A-Za-z0-9_]*$/`. Invalid keys produce `HostError::message(format!("invalid env key: {}", serde_json::Value::String(key.into())))`.
2. **Parsing (`parse_env`):**
   - Strips leading `\u{feff}` BOM if present.
   - Splits on `\n`, stripping trailing `\r`.
   - Uses JavaScript whitespace trimming (`service_kit::js::trim`). Ignores empty lines and lines starting with `#`.
   - Strips optional `export` prefix when followed by at least one whitespace character (`service_kit::js::is_space`).
   - Splits on the first `=`; trims key with `js::trim`; if `!valid_key(key)`, ignores the line.
   - Value unquoting (after `js::trim`):
     - If wrapped in `"..."` (length >= 2): strips outer quotes and sequentially replaces `\n` -> newline, `\"` -> `"`, `\\` -> `\`.
     - If wrapped in `'...'` (length >= 2): strips outer single quotes with no escape processing.
     - If unquoted: strips inline comments starting at the first `<whitespace>#` (any char satisfying `js::is_space` immediately followed by `#`) and trims trailing whitespace.
   - `EnvMap::insert` updates the existing entry in-place if the key is already present, preserving insertion order.
3. **Formatting (`format_value`, `format_env`):**
   - If `value` contains `\n` or `\r`, returns `Err(HostError::message("env values cannot contain line breaks"))`.
   - Plain values: if `value` is non-empty and every byte is in `A-Z`, `a-z`, `0-9`, `_`, `.`, `/`, `:`, `@`, `,`, `+`, `=`, `%`, `-` (`/^[A-Za-z0-9_./:@,+=%-]+$/`), returns `value` unquoted.
   - Otherwise (including empty string `""`), wraps in double quotes `"..."`, escaping `\` as `\\` and `"` as `\"`.
   - `format_env(header, entries)`: writes `# <line>` (or `#` if `<line>` is empty) for each line in `header`, followed by an empty line `""` if `!header.is_empty()`, followed by `KEY=formatted_value` for each entry, joined by `\n` with a trailing `\n`.
4. **Round-trip `EnvDocument`:**
   - Preserves the original UTF-8 BOM (`\u{feff}`), line ending (`\r\n` if present in the input, else `\n`), comments, and blank lines.
   - `set(key, value)` validates `key` and formats `value`; if `key` already exists in the document, replaces the **last** matching entry line in-place; otherwise inserts a new entry line before the trailing newline, ensuring the rendered document ends with a trailing newline.
   - `to_text()` joins lines with the preserved line ending, prepending `\u{feff}` if a BOM was present in the input.

---

### 2.3 Module 3: `hostconfig`

#### Responsibility
Defines, validates, loads, and saves the server configuration (`host.toml`, `host.yaml` / `host.yml`, `host.json`), manages `secrets.env`, and generates the per-service `.env` files (`backend.env`, `director.env`, `content.env`, `gateway.env`, `admin.env`, `allowlist.env`).

#### Constants & Submodules

```rust
pub mod guard;
pub mod modules;
pub mod yaml;

pub use guard::GuardConfig;
pub use modules::{ExternalModuleEntry, ModulesConfig, NetworkConfig};

pub const DATABASE_FILE: &str = "dainty-lass.db";
pub const DEFAULT_LINUX: LinuxOptions = LinuxOptions {
    wine: None,
    proton: None,
    xvfb: true,
    firewall_interface: None,
};
pub const DEFAULT_PERFORMANCE: PerformanceOptions = PerformanceOptions {
    server_fps: 30,
    idle_fps: 10,
    training_on_demand: true,
    training_idle_minutes: 5,
    verbose_game_logs: false,
    memory_guard: true,
    min_free_memory_mb: 1024,
    hunt_memory_mb: 1536,
};
pub const QUIET_GAME_LOG_CMDS: &str = "LogPhoenixCharacter Warning, LogOnline Warning, LogLoad Off";
pub const MAX_MULTIPLIER: f64 = 100.0;
pub const DEFAULT_MONGO_URI: &str = "mongodb://127.0.0.1:27017/?replicaSet=rs0";
pub const RESERVED_PORTS: [u16; 8] = [
    PORTS.backend,
    PORTS.director,
    PORTS.content,
    PORTS.allowlist,
    PORTS.relay,
    PORTS.mongo,
    61003,
    61004,
];
pub const LEGACY_UNSUPPORTED: &str = "this server runs Dauntless Revived's 1.4.4 (edition = \"legacy\" or \"both\"), and this reforged-host was built without it: use a build with the legacy feature (cargo build --release -p server-console --features legacy), or setup --edition reborn";
pub const LEGACY_RULE_NAME: &str = "DaintyLass 1.4.4 game ports (allowlist)";
pub const MAX_HOST_CONFIG_BYTES: u64 = 256 * 1024;
pub const SECRET_FILE: &str = "secrets.env";
pub const SECRET_KEYS: [&str; 11] = [
    "GATEWAY_SECRET",
    "ALLOWLIST_SECRET",
    "REFORGED_ADMIN_KEY",
    "GAMESERVER_API_KEY",
    "API_KEY_HASH_SECRET",
    "RUSTIC_ISLES_METRICS_TOKEN",
    "AUTH_SIGNING_PRIVKEY_B64",
    "AUTH_SIGNING_PUBKEY_B64",
    "REFORGED_ADMIN_API_KEY",
    "RUSTIC_ISLES_MANAGER_OPERATOR_SECRET",
    "REFORGED_DB_PASSWORD",
];
pub const NODE_CERT_FILE: &str = "node-cert.pem";
pub const NODE_KEY_FILE: &str = "node-key.pem";
pub const CLUSTER_CA_FILE: &str = "cluster-ca-cert.pem";
pub const SESSION_KEY_FILE: &str = "session-key";
pub const ENV_FILE_HEADER: [&str; 2] = [
    "Written by ReforgedHost (`npm run setup`) from host.json and secrets.env. SECRET: never share or commit.",
    "Run setup again instead of editing: it rewrites this file.",
];
pub const LEGACY_ENV_FILES: [&str; 2] = ["metagame", "deploy"];
```

#### Public Types & Functions

- `ServerMode`: `Private` (`"private"`), `Public` (`"public"`), with `as_str(self) -> &'static str`.
- `Registration`: `Invitecode` (`"INVITECODE"`), `Open` (`"OPEN"`), `None` (`"NONE"`), with `as_str(self)` and `parse(text: &str) -> Option<Self>`.
- `MongoConfig`: `{ pub uri: String, pub manage: bool, pub mongod: Option<String> }`.
- `Multipliers`: `{ pub xp: f64, pub rewards: f64 }` (`Copy`).
- `LinuxOptions`: `{ pub wine: Option<String>, pub proton: Option<String>, pub xvfb: bool, pub firewall_interface: Option<String> }`.
- `DatabaseSettings`:
  ```rust
  #[derive(Debug, Clone, Default, PartialEq, Eq)]
  pub struct DatabaseSettings {
      pub engine: Option<String>,
      pub postgres_host: Option<String>,
      pub postgres_port: Option<u16>,
      pub postgres_user: Option<String>,
      pub postgres_database: Option<String>,
      pub allow_remote: Option<bool>,
      pub fresh_start: Option<bool>,
  }
  ```
  with `pub fn postgres(&self) -> bool` (`self.engine.as_deref() == Some("postgres")`) and `pub fn to_json(&self) -> Value`.
- `PerformanceOptions`: `{ pub server_fps: u32, pub idle_fps: u32, pub training_on_demand: bool, pub training_idle_minutes: u32, pub verbose_game_logs: bool, pub memory_guard: bool, pub min_free_memory_mb: u32, pub hunt_memory_mb: u32 }`.
- `Edition`: `Reborn` (default), `Legacy`, `Both`, with `as_str`, `parse`, `versions() -> Vec<GameVersion>`, `runs(&GameVersion) -> bool`, `runs_reborn() -> bool`, `runs_legacy() -> bool`, `supported() -> bool` (`protocol::invite::played_versions()`), `primary() -> GameVersion`.
- `SwitchesConfig`:
  ```rust
  #[derive(Debug, Clone, Default, PartialEq, Eq)]
  pub struct SwitchesConfig {
      pub reforge: Option<String>,
      pub mastery_persistence: Option<String>,
      pub bounty_persistence: Option<String>,
      pub cooldown_persistence: Option<String>,
      pub account_erasure: Option<String>,
      pub log_purge: Option<String>,
      pub rate_limit: Option<String>,
      pub rate_limit_per_minute: Option<u32>,
      pub gameserver_rate_limit_per_minute: Option<u32>,
      pub host_rate_limit_per_minute: Option<u32>,
  }
  ```
  with `from_json`, `to_json`, `env() -> Vec<(&'static str, String)>`, `module_switches() -> Vec<(&'static str, Value)>`.
- `HistoryConfig`: `{ pub ship_to_peer: Option<module_api::network::NodeId> }`, with `from_json` and `to_json`.
- `FileForm`: opaque round-trip state carried on `HostConfig` so saving preserves unmodified keys and comments from the loaded TOML/YAML/JSON file.
- `HostConfig`:
  ```rust
  #[derive(Debug, Clone, PartialEq)]
  pub struct HostConfig {
      pub version: u32,
      pub mode: ServerMode,
      pub edition: Edition,
      pub host: String,
      pub game_address: String,
      pub name: String,
      pub game_dir: Option<String>,
      pub registration: Registration,
      pub share_url: Option<String>,
      pub mongo: MongoConfig,
      pub max_hunt_servers: u32,
      pub multipliers: Option<Multipliers>,
      pub public_port: Option<u16>,
      pub legacy_game_dir: Option<String>,
      pub legacy_public_port: Option<u16>,
      pub performance: Option<PerformanceOptions>,
      pub linux: Option<LinuxOptions>,
      pub database: Option<DatabaseSettings>,
      pub guard: Option<GuardConfig>,
      pub history: Option<HistoryConfig>,
      pub switches: Option<SwitchesConfig>,
      pub modules: Option<ModulesConfig>,
      pub network: Option<NetworkConfig>,
      pub file: FileForm,
  }
  ```
  with `from_json(&Value) -> Result<Self, HostError>`, `from_toml(&str) -> Result<Self, HostError>`, `written_over(self, previous: Option<&HostConfig>) -> Self`, `fresh_start(&self) -> bool`, `to_toml(&self) -> Result<String, HostError>`, `to_json(&self) -> Result<Value, HostError>`.
- `Secrets`: `{ pub gateway_secret: String, pub allowlist_secret: String, pub admin_key: String, pub gameserver_key: String, pub api_key_hash_secret: String, pub metrics_token: String, pub jwt_private_key_b64: String, pub jwt_public_key_b64: String, pub admin_api_key: String, pub operator_secret: String, pub database_password: String }`.
- `Platform`: `Windows`, `Linux`, `Other`, with `Platform::current() -> Self`.
- `EnvEntries = Vec<(String, String)>` and `EnvFiles { pub backend: EnvEntries, pub director: EnvEntries, pub content: EnvEntries, pub gateway: EnvEntries, pub admin: EnvEntries, pub allowlist: Option<EnvEntries> }` with `files(&self) -> [(&'static str, Option<&EnvEntries>); 6]`.
- Functions:
  - `pub fn is_local_database_host(host: &str) -> bool`
  - `pub fn check_edition(c: &HostConfig) -> Result<(), HostError>`
  - `pub fn performance_options(c: &HostConfig) -> PerformanceOptions`
  - `pub fn valid_minutes(value: f64) -> bool` (`value.fract() == 0.0 && (0.0..=1440.0).contains(&value)`)
  - `pub fn valid_megabytes(value: f64, min: f64) -> bool` (`value.fract() == 0.0 && value >= min && value <= 1_048_576.0`)
  - `pub fn valid_fps(value: f64, min: f64) -> bool` (`value.fract() == 0.0 && (value == 0.0 || (value >= min && value <= 240.0))`)
  - `pub fn game_ports(c: &HostConfig) -> (u16, u16)`
  - `pub fn game_ports_text(c: &HostConfig, separator: &str) -> String`
  - `pub fn gateway_port(c: &HostConfig) -> u16`
  - `pub fn legacy_gateway_port(c: &HostConfig) -> u16`
  - `pub fn legacy_game_ports(c: &HostConfig) -> (u16, u16)`
  - `pub fn multipliers_on(c: &HostConfig) -> bool`
  - `pub fn linux_options(c: &HostConfig) -> LinuxOptions`
  - `pub fn is_ipv4(host: &str) -> bool`
  - `pub fn is_tailscale_ipv4(host: &str) -> bool`
  - `pub fn is_public_ipv4(host: &str) -> bool`
  - `pub fn is_dns_name(host: &str) -> bool`
  - `pub fn is_linux_interface(name: &str) -> bool`
  - `pub fn clean_server_name(name: &str) -> String` (control chars `<= 0x1f` and `0x7f..=0x9f` replaced by `' '`, runs of `js::is_space` collapsed to single `' '`, `js::trim`, truncated to at most 48 UTF-16 code units)
  - `pub fn validate(document: &Value) -> Result<Value, HostError>`
  - `pub fn stringify_pretty(value: &Value) -> String` (`JSON.stringify(value, null, 2)` via `service_kit::jsjson`)
  - `pub fn read_host_config(file: &Path) -> Result<Option<HostConfig>, HostError>`
  - `pub fn load_host_config(p: &Paths) -> Result<Option<HostConfig>, HostError>`
  - `pub fn has_host_config(p: &Paths) -> bool`
  - `pub fn save_host_config(p: &Paths, c: &HostConfig) -> Result<bool, HostError>`
  - `pub fn host_config_text(c: &HostConfig) -> Result<String, HostError>`
  - `pub fn write_host_config(file: &Path, c: &HostConfig) -> Result<(), HostError>`
  - `pub fn new_secrets() -> Result<Secrets, HostError>`
  - `pub fn load_or_create_secrets(config_dir: &Path, make: impl FnOnce() -> Result<Secrets, HostError>) -> Result<(Secrets, bool), HostError>`
  - `pub fn write_secret_file(file: &Path, text: &str) -> Result<(), HostError>`
  - `pub fn build_env_files(c: &HostConfig, s: &Secrets, p: &Paths, source_commit: &str, platform: Platform) -> EnvFiles`
  - `pub fn node_database_config(c: &HostConfig, p: &Paths) -> Result<store::DatabaseConfig, HostError>`
  - `pub fn write_env_files(p: &Paths, files: &EnvFiles) -> Result<Vec<PathBuf>, HostError>`
  - `pub fn write_named_env_files(p: &Paths, files: &[(&str, Option<&EnvEntries>)]) -> Result<Vec<PathBuf>, HostError>`
  - `pub fn read_env_file(file: &Path) -> Result<EnvMap, HostError>`
  - Crate-internal helpers used by `hostconfig/{guard,modules}.rs`: `pub(crate) fn bad(text: impl Into<String>) -> HostError`, `pub(crate) fn shown(value: Option<&Value>) -> String`, `pub(crate) fn integer(value: Option<&Value>) -> Option<f64>`, `pub(crate) fn path_text(path: &Path) -> String`.

#### Validation & `.env` Generation Contract
1. **`validate(document)` Error Messages:**
   - Not an object -> `"host.json must hold a JSON object"`.
   - `mode` not `"private"` or `"public"` -> `"mode must be private or public"`.
   - `edition` present and not `"reborn"`, `"legacy"`, or `"both"` -> `"edition must be reborn, legacy or both"`.
   - `host`: must be a string (`"host must be a string"`), trimmed with `js::trim` and lowercased:
     - Private: `!is_tailscale_ipv4(&host) && !host.ends_with(".ts.net")` -> `"a private server's host must be this PC's Tailscale address (100.64.0.0/10) or MagicDNS name, not {raw_host}"`.
     - Public: `!is_public_ipv4(&host) && !is_dns_name(&host)` -> `"a public server's host must be a public IPv4 address or a DNS name, not {raw_host}"`.
   - `gameAddress`: must be an IPv4 string (`"gameAddress must be an IPv4 address (the one players' game traffic reaches), not {shown}"`), and in private mode must satisfy `is_tailscale_ipv4` (`"a private server's game address must be its Tailscale IPv4"`).
   - `name`: must be a string; after `clean_server_name`, if empty -> `"the server needs a name"`.
   - `registration`: must be `"INVITECODE"`, `"OPEN"`, or `"NONE"` (`"registration must be INVITECODE, OPEN or NONE"`).
   - `shareUrl`: must be `Null` or a string matching `/^https:\/\/login\.tailscale\.com\/[A-Za-z0-9/_?=&.%-]+$/` (`"shareUrl must be a https://login.tailscale.com/ link"`).
   - `mongo`: if present, must be an object (`"mongo must be an object"`) with `uri` starting with `"mongodb://"` or `"mongodb+srv://"` (`"mongo.uri must be a mongodb:// connection string"`).
   - `maxHuntServers`: integer in `1..=10` (`"maxHuntServers must be 1 to 10 (UDP 8770-8777 up to 6 hunts, 8766-8777 at 10)"`).
   - `multipliers`: if present, must be an object (`"multipliers must be an object with xp and rewards"`); each value must be in `1.0..=100.0` (`"the {what} multiplier must be a number from 1 to 100, not {shown}"`); in public mode, unless both `xp == 1.0` and `rewards == 1.0`, fails with `"XP and reward multipliers are for private (Tailscale) test servers only"`.
   - `publicPort`: if present, requires `mode == Public` (`"publicPort is for public servers only"`), integer `1..=65535` (`"publicPort must be a TCP port from 1 to 65535, not {shown}"`), not in `RESERVED_PORTS` or `8766..=8777` (`"publicPort {n} is one the server uses itself: pick another (4443, for example)"`), and when `edition == Both && mode == Public` not in `BOTH_LEGACY_PORTS` (`"publicPort {reborn_port} is one the 1.4.4 stack uses on a server running both games: pick another"`).
   - `legacyGameDir`: if non-null string when `edition != Both`, fails with `"legacyGameDir is for a server running both games (edition both)"`; if neither null nor string, fails with `"legacyGameDir must be a folder, not {shown}"`.
   - `legacyPublicPort`: requires `both && mode == Public` (`"legacyPublicPort is for a public server running both games (edition both)"`), integer `1..=65535` (`"legacyPublicPort must be a TCP port from 1 to 65535, not {shown}"`), not in reserved/game/legacy ports (`"legacyPublicPort {n} is one the server uses itself: pick another (4443, for example)"`), and when `both && mode == Public` must not equal `reborn_port` (`"both gateways would listen on {reborn_port}: give the 1.4.4 one another port (legacyPublicPort)"`).
   - `performance`: validates `serverFps` (`0` or `10..=240`), `idleFps` (`0` or `1..=240`), `trainingOnDemand` (bool), `trainingIdleMinutes` (`0..=1440`), `verboseGameLogs` (bool), `memoryGuard` (bool), `minFreeMemoryMb` (`0..=1048576`), `huntMemoryMb` (`256..=1048576`).
   - `linux`: object with `wine` and `proton` (`null` or string starting with `'/'`: `"linux.{what} must be the full path of the program, or null"`), `xvfb` (`bool`: `"linux.xvfb must be true or false"`), `firewallInterface` (`null` or `is_linux_interface`: `"linux.firewallInterface must be a network interface name like eth0, not {shown}"`).
   - `database`: object with `engine` (`"sqlite"` or `"postgres"`), `postgresHost`/`postgresUser`/`postgresDatabase` (non-empty strings with no control chars), `postgresPort` (`1..=65535`), `allowRemote` (bool), `freshStart` (bool); if `engine == "postgres" && !allowRemote && !is_local_database_host(host)`, fails with `"database.postgresHost {host} is not on this machine, and connections to Postgres are not encrypted: use 127.0.0.1, or set database.allowRemote = true if the network between them is private"`.
   - `guard`, `switches`, `modules`, `network`, `history`: validated via their respective `validate` / `from_json` methods; if `network.nodes` is non-empty and `mode != "public"`, fails with `"network.nodes needs mode = \"public\": node links run over the gateway's TLS port"`; if `history.shipToPeer` is set, requires `network` with `peer != network.node` and `peer` listed in `network.nodes`.
2. **`build_env_files(c, s, p, source_commit, platform)` Exact Entries:**
   - **`backend`:**
     - `NODE_ENV=production`, `RUSTIC_ISLES_SERVICE_ROLE=api`, `RUSTIC_ISLES_METRICS_TOKEN=<s.metrics_token>`, `PORT=61000`, `BIND_HOST=127.0.0.1`, `REFORGED_HTTP_ONLY=true`, `QOS_PORT=0`, `QOS_TARGET_URL=http://127.0.0.1:61100/QoS`, `AUTH_MODE=APIKEY`, `ALLOW_NO_AUTH_DEV_MODE=false`, `AUTH_SIGNING_PRIVKEY_B64=<s.jwt_private_key_b64>`, `AUTH_SIGNING_PUBKEY_B64=<s.jwt_public_key_b64>`, `API_KEY_HASH_SECRET=<s.api_key_hash_secret>`, `GAMESERVER_API_KEYS=<s.gameserver_key>`.
     - When `cfg!(feature = "paradox")`: `MONGODB_URI=<c.mongo.uri>`, `MONGODB_DB=reforged`, `MONGODB_APP_NAME=reforged-backend`, `MONGODB_TRANSACTION_MAX_COMMIT_TIME_MS=15000`.
     - `TARGET_CHANGELIST=647472`, `APPROVED_EXECUTABLE_SHA256=<PINNED_EXE_SHA256>`, `MATCHMAKING_MODE=DEPLOYSERVER`, `DEPLOYSERVER_URL=127.0.0.1:61001`, `HUNT_GO_NOW_MIN_DELAY_MS=1`, `HUNT_GO_NOW_MAX_DELAY_MS=1000`, `REALTIME_XMPP_ENABLED=true`, `REALTIME_XMPP_CAPTURE=false`, `REALTIME_XMPP_ALLOWED_HOSTS=<c.host>`, `REALTIME_XMPP_DEV_CITY_MUC=false`, `REGISTRATION_MODE=<c.registration>`, `REFORGED_ADMIN_KEY=<s.admin_key>`, `RUSTIC_ISLES_MANAGER_ADMIN=on`, `RUSTIC_ISLES_MANAGER_OPERATOR_SECRET=<s.operator_secret>`, `REFORGED_SERVER_NAME=<c.name>`, `REFORGED_SERVER_VERSION=0.1.0`, `REFORGED_SOURCE_URL=https://github.com/FreezaMotoqueiro/Dauntless-Reforged-Revival`, `REFORGED_SOURCE_COMMIT=<source_commit>`, `REFORGED_SERVER_MODE=<c.mode>`, `REFORGED_SERVER_HOST=<c.host>`.
     - If `multipliers_on(c)`: `REFORGED_XP_MULTIPLIER=<jsjson::number(m.xp)>`, `REFORGED_REWARD_MULTIPLIER=<jsjson::number(m.rewards)>`.
     - Switch env entries from `c.modules` (`isles-1-14-7` `switches` section) overlaid by `c.switches.env()`.
     - `REFORGED_SESSION_SUPPLIES_FILE=<p.config/session-supplies.json>`, `LOG_LEVEL=info`.
   - **`director`:**
     - `NODE_ENV=production`, `PORT=61001`, `BIND_HOST=127.0.0.1`, `MY_IP=<c.game_address>`, `PORT_RANGE_BEGIN=<first>`, `PORT_RANGE_END=<last>`, `GAMESERVER_BINARY_PATH=<linux: p.gameserver_wrapper ; else exe_text>`.
     - If `linux`: `REFORGED_GAME_EXE=<exe_text>`, optional `REFORGED_WINE`, optional `REFORGED_PROTON`, `REFORGED_WINEPREFIX=<p.wine_prefix>`, `REFORGED_PROTON_DATA=<p.proton_data>`, `REFORGED_XVFB=<"1"|"0">`.
     - `METAGAME_API_KEY=<s.gameserver_key>`, `SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP=3`, `GAMESERVER_READY_TIMEOUT_MS=<"90000" if linux else "30000">`, `MAX_CONCURRENT_HUNT_SERVERS=<c.max_hunt_servers>`, `TRAINING_ON_DEMAND=<"1"|"0">`, `TRAINING_IDLE_STOP_MINUTES=<perf.training_idle_minutes if training_on_demand else 0>`, optional `GAMESERVER_RUNTIME_LOG_DIR=<exe.parent()>` (when `c.game_dir` is `Some`), `GAMESERVER_MEMORY_GUARD=<"1"|"0">`, `GAMESERVER_MIN_FREE_MB=<perf.min_free_memory_mb>`, `GAMESERVER_HUNT_MEMORY_MB=<perf.hunt_memory_mb>`, `REFORGED_SERVER_MAX_FPS=<perf.server_fps>`, `REFORGED_SERVER_IDLE_FPS=<perf.idle_fps>`, optional `GAMESERVER_LOG_CMDS=<QUIET_GAME_LOG_CMDS>` (when `!perf.verbose_game_logs`), `SERVER_RUNTIME_AUTO_UPDATE=false`, `SERVER_RUNTIME_UPDATE_REQUIRED=false`, `GAMESERVER_LOG_DIR=<p.gameserver_logs>`, `REFORGED_BACKEND_URL=http://127.0.0.1:61000`, `RUSTIC_ISLES_RUNTIME_PROFILE=production`, `LOG_LEVEL=info`.
   - **`content`:**
     - `PORT=61002`, `BIND_HOST=127.0.0.1`, `BACKEND_URL=http://127.0.0.1:61000`, `CONTENT_RUNTIME_DIR=<p.runtime_publish>`, `CONTENT_BRANDING_DIR=<p.branding>`, `CONTENT_NEWS_FILE=<p.news_file>`.
   - **`admin`:**
     - `PORT=61003`, `BIND_HOST=127.0.0.1`, `BACKEND_URL=http://127.0.0.1:61000`, `REFORGED_DIRECTOR_URL=http://127.0.0.1:61001`, `REFORGED_ADMIN_API_KEY=<s.admin_api_key>`, `REFORGED_ADMIN_KEY=<s.admin_key>`, `RUSTIC_ISLES_MANAGER_OPERATOR_SECRET=<s.operator_secret>`, `REFORGED_ADMIN_API_TIMEOUTS_FILE=<p.config/admin-timeouts.json>`, `REFORGED_MAX_HUNT_SERVERS=<c.max_hunt_servers>`, `REFORGED_SERVER_MODE=<c.mode>`, followed by database environment variables (`REFORGED_DB_ENGINE=sqlite`, `REFORGED_DB_FILE=<p.config/dainty-lass.db>` or `REFORGED_DB_ENGINE=postgres`, `REFORGED_DB_HOST`, `REFORGED_DB_PORT`, `REFORGED_DB_USER`, `REFORGED_DB_NAME`, `REFORGED_DB_PASSWORD`, optional `REFORGED_DB_ALLOW_REMOTE=1`) and `c.guard.env()`.
   - **`gateway` & `allowlist`:**
     - Common gateway entries: `GATEWAY_SECRET=<s.gateway_secret>`, `GATEWAY_BACKEND_URL=http://127.0.0.1:61000`, `GATEWAY_CONTENT_URL=http://127.0.0.1:61002`, `GATEWAY_WS_URL=http://127.0.0.1:61000`.
     - In `Public` mode:
       - `gateway`: `GATEWAY_TLS=1`, `GATEWAY_BIND=0.0.0.0`, `GATEWAY_PORT=<gateway_port(c)>`, `GATEWAY_CERT=<p.cert_file>`, `GATEWAY_KEY=<p.key_file>`, optional node-link environment variables (when `c.network` has non-empty `nodes`: `GATEWAY_NODE_ID`, `GATEWAY_NODE_REGION`, `GATEWAY_NODE_CERT`, `GATEWAY_NODE_KEY`, `GATEWAY_NODE_CA`, `GATEWAY_NETWORK_NODES`), common entries, `ALLOWLIST_URL=http://127.0.0.1:61005`, `ALLOWLIST_SECRET=<s.allowlist_secret>`.
       - `allowlist`: `ALLOWLIST_DRY_RUN=<"1" if Platform::Other else "0">`, `ALLOWLIST_FIREWALL=<"nftables" if linux else "windows">`, (if `linux`: `ALLOWLIST_NFT_TABLE=reforged_revival` and optional `ALLOWLIST_INTERFACE=<iface>`), `ALLOWLIST_BIND=127.0.0.1`, `ALLOWLIST_PORT=61005`, `ALLOWLIST_SECRET=<s.allowlist_secret>`, `ALLOWLIST_PORTS=<first>-<last>`, `ALLOWLIST_AUDIT_LOG=<p.logs/allowlist-audit.log>`, `ALLOWLIST_STATE_FILE=<p.config/allowlist-state.json>`.
     - In `Private` mode:
       - `gateway`: `GATEWAY_TLS=0`, `GATEWAY_BIND=<c.game_address>`, `GATEWAY_PORT=61000`, common entries, `GATEWAY_ALLOWLIST=0`.
       - `allowlist`: `None`.

---

### 2.4 Module 4: `run` & `tls`

#### Responsibility
`run` executes short-lived external commands (`npm`, `git`, `netsh`, `tailscale`, `ss`, `gh`, etc.) either streaming output to a log file (`run`) or capturing stdout (`capture`). `tls` re-exports `service_kit::tls` and generates `.workspace/tls/gateway-{cert,key}.pem` when missing.

#### Public Types & Functions (`run.rs` and `tls.rs`)

```rust
// run.rs
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    pub cwd: PathBuf,
    pub env: Option<Vec<(String, String)>>,
    pub log_file: Option<PathBuf>,
    pub quiet: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunResult {
    pub code: i32,
    pub last_line: Option<String>,
    pub output: String,
}

pub fn is_error_line(line: &str) -> bool;
pub fn run(command: &str, args: &[&str], options: &RunOptions) -> Result<RunResult, HostError>;
pub fn capture(command: &str, args: &[&str], cwd: Option<&Path>) -> Result<Option<String>, HostError>;

// tls.rs
pub use service_kit::tls::{
    Cert, CertRequest, Validity, generate_self_signed_cert, generate_self_signed_cert_at,
};
pub fn write_gateway_cert_if_missing(paths: &Paths, config: &HostConfig) -> Result<bool, HostError>;
```

#### Behavioral Contract & Invariants
1. **Error Line Detection (`is_error_line`):**
   - Trims `line` with `service_kit::js::trim`.
   - Strips optional `"Uncaught "` prefix, takes the leading ASCII alphabetic word (`A-Za-z`), and returns `true` iff that word ends with `"Error"` (matching `/^(Uncaught )?[A-Za-z]*Error\b/`).
2. **Command Execution (`run`):**
   - Appends `\n> <command> <args...>\n` to `options.log_file` if `Some`.
   - On Windows, if `command` has no path separator (`/` or `\`), resolves `<command>.cmd` or `<command>.exe` on `PATH` (from `options.env` or `std::env::var_os("PATH")`), and sets `CREATE_NO_WINDOW` (`0x0800_0000`).
   - Merges stdout and stderr, tracking `last_line` (latest non-empty trimmed line) and retaining at most `64 * 1024` bytes (`64 KiB`) in `output`.
   - When `options.quiet` is `false`, echoes output to stdout; when `options.quiet` is `true`, prints `  ! <trimmed>` to stderr for any line where `is_error_line(line)` is `true`.
3. **Gateway Certificate Generation (`tls::write_gateway_cert_if_missing`):**
   - If both `paths.cert_file` and `paths.key_file` exist, returns `Ok(false)`.
   - Otherwise calls `generate_self_signed_cert(&CertRequest { common_name: config.host.clone(), dns_names, ip_addresses })` (putting `config.host` in `ip_addresses` if `is_ipv4(&config.host)` else in `dns_names`, plus `config.game_address` in `ip_addresses` if different from `config.host`), writes `paths.cert_file` and `paths.key_file` (via `write_secret_file`), and returns `Ok(true)`.

---

### 2.5 Module 5 & 6: `services` and `services/tree`

Re-exported directly from `service_kit::services` (`pub use service_kit::services;`).

---

### 2.6 Module 7: `invite`

#### Responsibility
Formats `dauntless://invite?...` links for the launcher, computes and probes gateway X.509 certificate SHA-256 fingerprints, and calls the backend's local admin endpoints (`/reforged/api/CreateInvite` and `/reforged/api/InviteCodes`).

#### Public Types & Functions

```rust
pub use protocol::invite::{
    DEFAULT_GAME_VERSION, GameVersion, InviteParts, format_game_invite, format_invite_parts,
    parse_game_version, played_versions, reforged_version, revived_version,
};

#[derive(Debug, Clone, Default)]
pub struct CodeOptions {
    pub uses: f64,
    pub infinite: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

pub fn invite_for(config: &HostConfig, code: &str, cert_fingerprint: Option<&str>) -> Result<String, HostError>;
pub fn invite_for_game(
    config: &HostConfig,
    code: &str,
    cert_fingerprint: Option<&str>,
    game: &GameVersion,
) -> Result<String, HostError>;
pub fn certificate_fingerprint(cert_file: &Path) -> Result<String, HostError>;
pub fn served_fingerprint(port: u16, host: &str, timeout: Duration) -> Option<String>;
pub fn http_request(
    method: &str,
    port: u16,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
    timeout: Duration,
) -> Result<HttpResponse, HostError>;
pub fn create_invite_code(port: u16, admin_key: &str, opts: &CodeOptions) -> Result<String, HostError>;
pub fn list_invite_codes(port: u16, admin_key: &str) -> Result<Vec<Value>, HostError>;
```

#### Behavioral Contract & Invariants
1. **Invite Link Formatting (`invite_for`, `invite_for_game`):**
   - For Legacy (`*game == revived_version()`), uses `legacy_gateway_port(config)` as port; for Reborn, uses `gateway_port(config)`.
   - Delegates to `format_game_invite(&InviteParts { mode: config.mode == ServerMode::Public, host: config.host.clone(), port, code: code.to_owned(), name: config.name.clone(), fp: cert_fingerprint.map(str::to_owned), share: config.share_url.clone() }, game)`.
2. **Certificate Fingerprint (`certificate_fingerprint`, `served_fingerprint`):**
   - `certificate_fingerprint(cert_file)` parses the first `-----BEGIN CERTIFICATE-----...-----END CERTIFICATE-----` PEM block in `cert_file`, base64-decodes the DER bytes, and returns the lowercase 64-character hex SHA-256 digest (`sha256: <64_hex>` or `<64_hex>` as `protocol::invite` expects: 64 lowercase hex characters).
   - `served_fingerprint(port, host, timeout)` opens a TLS connection to `127.0.0.1:port` (with SNI `host`) using `rustls` with a custom certificate verifier that captures the leaf DER certificate, returning its lowercase 64-character hex SHA-256 digest, or `None` if connection/handshake fails within `timeout`.
3. **Backend Admin API (`http_request`, `create_invite_code`, `list_invite_codes`):**
   - `http_request` connects to `127.0.0.1:<port>` over plain HTTP/1.1 with `timeout`, sends `<method> <path> HTTP/1.1` with `Host: 127.0.0.1:<port>`, `Connection: close`, `Content-Length` (when `body.is_some()`), and custom `headers`, and parses status + response body (handling `Transfer-Encoding: chunked` or `Content-Length`/EOF).
   - `create_invite_code(port, admin_key, opts)`:
     - Validates `!admin_key.trim().is_empty()` (`"REFORGED_ADMIN_KEY is missing from secrets.env"`).
     - `POST /reforged/api/CreateInvite` with `X-Reforged-Admin-Key: <admin_key>`, `Content-Type: application/json`, and body `{"maxUses": <opts.uses as i64>, "infinite": <opts.infinite>, "note": <note_or_null>}`.
     - Connection error -> `"could not reach the backend on 127.0.0.1:{port} ({e}). Is the server running? (npm start)"`.
     - Non-200 status -> `"backend refused invite creation (HTTP {status}): {err}"`.
     - Extracts `"code"` string from response JSON.
   - `list_invite_codes(port, admin_key)`:
     - `GET /reforged/api/InviteCodes` with `X-Reforged-Admin-Key: <admin_key>`, returning the `"invites"` array from the 200 JSON response.

---

### 2.7 Module 8: `doctor`

#### Responsibility
Runs comprehensive diagnostic checks across tools, configuration, Rustic Isles sources/builds, game data, published runtime, game installation, performance/RAM, Linux/Wine/systemd/firewall, network/TLS certificates, database state, and running services.

#### Public Types & Functions

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level { Ok, Warn, Fail, Info }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub area: String,
    pub level: Level,
    pub text: String,
    pub fix: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DoctorOptions {
    pub hash_exe: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuPatches {
    pub cpu_fixes: bool,
    pub frame_cap: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct GameBuild<'a> {
    pub version_text: &'a str,
    pub exe_sha256: &'a str,
    pub name: &'a str,
    pub get_it: &'a str,
}

pub const REBORN_BUILD: GameBuild<'static> = GameBuild {
    version_text: GAME_VERSION_TEXT,
    exe_sha256: PINNED_EXE_SHA256,
    name: "1.14.7",
    get_it: "run reforged-host install-game",
};
pub const SERVICES_MEMORY_MB: i64 = 1536;

pub fn tailscale_ipv4() -> Option<String>;
pub fn port_in_use(port: u16, host: &str) -> bool;
pub fn port_owner_hint(port: u16) -> Option<String>;
pub fn port_owner_hint_for(
    port: u16,
    platform: Platform,
    run_cmd: impl Fn(&str, &[&str]) -> Option<String>,
) -> Option<String>;
pub fn wait_for_http(port: u16, path: &str, timeout: Duration) -> bool;
pub fn sha256_file(file: &Path) -> Result<String, HostError>;
pub fn node_version_ok(version: &str) -> bool;
pub fn count_function_files(sdk_dir: &Path) -> usize;
pub fn game_checks(game_dir: Option<&str>, hash_exe: bool) -> Vec<Check>;
pub fn game_exe_checks(game_dir: Option<&str>, hash_exe: bool, build: &GameBuild<'_>) -> Vec<Check>;
pub fn runtime_cpu_patches(dll_path: &Path) -> Option<CpuPatches>;
pub fn hunts_that_fit(total_mb: u64, perf: &PerformanceOptions) -> u64;
pub fn total_memory_mb() -> Option<u64>;
pub fn performance_checks(
    config: &HostConfig,
    runtime_dll: &Path,
    total_mb: Option<u64>,
    platform: Platform,
) -> Vec<Check>;
pub fn linux_checks(p: &Paths, config: &HostConfig) -> Result<Vec<Check>, HostError>;
pub fn network_checks(p: &Paths, config: &HostConfig, platform: Platform) -> Result<Vec<Check>, HostError>;
pub fn database_check(mongo: &MongoCheck, managed: bool) -> Check;
pub fn store_database_check(p: &Paths, config: &HostConfig) -> Check;
pub fn run_doctor(p: &Paths, opts: &DoctorOptions) -> Result<Vec<Check>, HostError>;
pub fn print_checks(checks: &[Check], say: impl FnMut(&str)) -> (usize, usize);
```

#### Behavioral Contract & Invariants
1. **`node_version_ok(version)`:** Strips optional `'v'`, parses `<major>.<minor>...`; returns `true` iff `major > 20 || (major == 20 && minor >= 19)`.
2. **`count_function_files(sdk_dir)`:** Counts files in `sdk_dir` whose name ends with `_functions.cpp`.
3. **`runtime_cpu_patches(dll_path)`:** Reads `dll_path` bytes:
   - `cpu_fixes`: `true` if UTF-16LE bytes of `"LEGACY_EXPECTED_PLAYER_WALK.flag"` occur in the file.
   - `frame_cap`: `true` if ASCII bytes `b"REFORGED_SERVER_MAX_FPS"` occur in the file.
4. **`hunts_that_fit(total_mb, perf)`:**
   - Base overhead = `SERVICES_MEMORY_MB` (`1536`) + `2 * i64::from(perf.hunt_memory_mb)` + `i64::from(perf.min_free_memory_mb)`.
   - Available = `total_mb as i64 - overhead`. If `available <= 0` or `perf.hunt_memory_mb == 0`, returns `0`; else `(available / i64::from(perf.hunt_memory_mb)) as u64`.
5. **`game_checks` / `game_exe_checks`:**
   - Area `"game"` (or `"legacy-game"` when `build.name != "1.14.7"`).
   - If `game_dir` is `None` or empty: `Level::Fail` (`"no game folder in host.toml"`).
   - Checks `<game_dir>/Archon/Binaries/Win64/Dauntless-Win64-Shipping.exe` exists.
   - Checks `<game_dir>/Version.txt` (stripping UTF-8 BOM and trimming whitespace) equals `build.version_text`; if empty, reports `"nothing"`.
   - When `hash_exe == true`: computes SHA-256 of the `.exe` and compares against `build.exe_sha256`.
6. **`store_database_check(p, config)`:**
   - Area `"database"`. Checks `commands::update::unmigrated_mongo_doctor_problem(p, config)` first (reporting `Level::Fail` if unmigrated MongoDB data is present); otherwise opens `node_database_config(config, p)` via `store::Database::open_sync` and runs `health_check_sync()`.
7. **`print_checks(checks, say)`:**
   - Groups checks by `check.area` (printing `\n<area>` whenever `area` changes).
   - Formats badges: `Level::Ok` -> `"  [ ok ] "`, `Level::Warn` -> `"  [warn] "`, `Level::Fail` -> `"  [FAIL] "`, `Level::Info` -> `"  [info] "`.
   - When `check.level != Level::Ok` and `check.fix` is `Some(fix)`, prints `"         -> {fix}"`.
   - Returns `(fail_count, warn_count)`.

---

### 2.8 Module 9 & 10: `mongo` and `mongo/wire`

#### Responsibility
Finds `mongod` on Windows (`PATH` or newest version under `%ProgramFiles%\MongoDB\Server\<ver>\bin\mongod.exe`) and Linux (`PATH`), builds `mongod` command-line arguments for a loopback 1-member replica set (`rs0`), and speaks the MongoDB `OP_MSG` binary wire protocol directly over TCP to inspect or initiate the replica set.

#### Constants, Types & Functions

```rust
pub mod wire;

pub const REPLICA_SET: &str = "rs0";
pub const MONGO_CACHE_GB: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MongoProblem { NoDriver, Unreachable, Standalone, Error }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MongoCheck {
    Ok { set_name: String, primary: bool, initiated: bool },
    Failed { reason: MongoProblem, detail: String },
}

#[derive(Debug, Clone, Copy)]
pub struct ReplicaSetOptions {
    pub initiate: bool,
    pub timeout: Duration,
}

impl Default for ReplicaSetOptions {
    fn default() -> Self {
        Self { initiate: true, timeout: Duration::from_millis(8000) }
    }
}

pub fn mongod_args(data_dir: &Path, log_file: &Path, port: u16) -> Vec<String>;
pub fn find_mongod(configured: Option<&str>) -> Result<Option<String>, HostError>;
pub fn newest_installed(base: &Path) -> Option<PathBuf>;
pub fn program_files() -> Vec<PathBuf>;
pub fn search_path(names: &[&str], path: Option<&OsStr>) -> Option<PathBuf>;
pub fn direct_uri(uri: &str) -> String;
pub fn first_host(uri: &str) -> Result<(String, u16), String>;
pub fn ensure_replica_set(uri: &str, opts: &ReplicaSetOptions) -> MongoCheck;
```

#### Behavioral Contract & Invariants
1. **`mongod_args(data_dir, log_file, port)`:**
   Returns `["--replSet", "rs0", "--bind_ip", "127.0.0.1", "--port", "<port>", "--dbpath", "<data_dir>", "--logpath", "<log_file>", "--logappend", "--wiredTigerCacheSizeGB", "1"]`.
2. **`direct_uri(uri)`:**
   Replaces `replicaSet=[^&]+` in the query string with `directConnection=true` (or appends `?directConnection=true` / `&directConnection=true` if no `replicaSet=` parameter is present).
3. **`first_host(uri)`:**
   - Rejects `mongodb+srv://` (`"mongodb+srv:// is not supported for a local replica set"`).
   - Requires `mongodb://` prefix, strips optional `user:pass@`, takes the first comma-separated host entry before `/` or `?`, supports `[ipv6]:port` and `host:port`, defaulting port to `27017`.
4. **Wire Protocol (`mongo/wire` & `ensure_replica_set`):**
   - `mongo::wire` provides `Value` (`Double`, `Str`, `Doc`, `Array`, `Bool`, `Null`, `Int32`, `Int64`, `Other`), `Doc = Vec<(String, Value)>`, `find`, `get`, `as_str`, `as_f64`, `as_bool`, `encode_doc`, `decode_doc`, `op_msg`, `parse_op_msg`, and `command(addr: SocketAddr, doc: &Doc, timeout: Duration) -> io::Result<Doc>`.
   - `ensure_replica_set(uri, opts)` connects to the first host parsed from `uri` and sends `hello` / `replSetGetStatus` / `replSetInitiate` as needed, returning `MongoCheck::Ok` or `MongoCheck::Failed`.

---

### 2.9 Module 11: `gamedata`

#### Responsibility
Tracks the state (`Missing`, `Synthetic`, `Real`) of the 7 backend and 7 Director game-data JSON files, copies `.example.json` files when requested (falling back to embedded minimal JSON seeds when `.example.json` files are absent from disk), and orchestrates the 8 game-data generation steps.

#### Constants, Types & Functions

```rust
pub const BACKEND_GAME_DATA: [&str; 7] = [
    "progression_config.json",
    "slayers_path.json",
    "slayers_path_definitions.json",
    "ladyluck_store.json",
    "platinum_store.json",
    "reward_cache_store.json",
    "inventory_storage_policy.json",
];

pub const DIRECTOR_GAME_DATA: [&str; 7] = [
    "player_hunts_table.json",
    "matchmaker_hunts_table.json",
    "arena_easy_matchmaker_hunts.json",
    "arena_hard_matchmaker_hunts.json",
    "arena_elite_matchmaker_hunts.json",
    "arena_hard_matchmaker_hunts_new.json",
    "arena_elite_matchmaker_hunts_new.json",
];

pub const SUPPLIED_SOURCES: [(&str, &str); 2] = [
    ("progression_config.json", "progression_config.source.json"),
    ("ladyluck_store.json", "ladyluck_store.source.json"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameDataKind { Missing, Synthetic, Real }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataState {
    pub state: GameDataKind,
    pub missing: Vec<String>,
    pub synthetic: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepDir { Backend, Director }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratorKind {
    Catalog,
    Progression,
    Quests,
    Store,
    PlatinumEnrich,
    PlatinumGenerate,
    LadyluckEnrich,
    LadyluckGenerate,
    DirectorGamedata,
    DirectorWorlddata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step<'a> {
    pub label: &'a str,
    pub script: &'a str,
    pub dir: StepDir,
    pub kind: GeneratorKind,
}

#[derive(Debug, Clone, Default)]
pub struct GenerateOptions {
    pub examples_for_missing: bool,
    pub log_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Generated {
    pub copied_sources: Vec<String>,
    pub captures: Option<CaptureFetch>,
    pub progression: Option<SourceWritten>,
    pub steps: Vec<String>,
    pub examples: Vec<String>,
}

pub struct Generators<'a> {
    pub captures: &'a dyn Fn(&Paths, &CapturePin, Option<&Path>) -> Result<CaptureFetch, HostError>,
    pub step: &'a dyn Fn(&Paths, &Step<'_>, Option<&Path>) -> Result<(), HostError>,
}

pub fn game_data_state(dir: &Path, files: &[&str]) -> Result<GameDataState, HostError>;
pub fn use_examples(dir: &Path, files: &[&str]) -> Result<Vec<String>, HostError>;
pub fn seed_embedded_examples(paths: &Paths) -> Result<Vec<String>, HostError>;
pub fn game_data_steps(p: &Paths) -> Vec<Step<'static>>;
pub fn run_generator(p: &Paths, step: &Step<'_>, log_file: Option<&Path>) -> Result<(), HostError>;
pub fn generate_game_data(
    p: &Paths,
    opts: &GenerateOptions,
    print: impl FnMut(&str),
) -> Result<Generated, HostError>;
pub fn generate_game_data_with(
    p: &Paths,
    opts: &GenerateOptions,
    print: impl FnMut(&str),
    hooks: &Generators<'_>,
) -> Result<Generated, HostError>;
```

#### Behavioral Contract & Invariants
1. **`game_data_state(dir, files)`:**
   - For each `file` in `files`:
     - If `<dir>/<file>` does not exist -> pushed to `missing`.
     - Else if `<dir>/<stem>.example.json` exists and has identical LF-normalized text -> pushed to `synthetic`.
   - `state`: `Missing` if `!missing.is_empty()`, else `Synthetic` if `!synthetic.is_empty()`, else `Real`.
2. **`use_examples(dir, files)` & `seed_embedded_examples(paths)`:**
   - `use_examples`: copies `<dir>/<stem>.example.json` to `<dir>/<file>` for any missing file in `files` (falling back to minimal embedded JSON when `<stem>.example.json` does not exist on disk).
   - `seed_embedded_examples(paths)`: seeds all missing `BACKEND_GAME_DATA` and `DIRECTOR_GAME_DATA` files and `quests_definitions.json`, `bounty_templates.json`, `EscalationData.json`, `TrialsData.json`, `sanctum_lore_nodes.json`, `ramsgate_gatherables.json`, `island_fauna.json`.
3. **`run_generator(p, step, log_file)`:**
   - When `cfg!(feature = "paradox")` is enabled: runs `npm run <step.script> -- --exports <p.exports> --captures <p.store_captures>` in `p.backend_dir` or `p.director_dir`.
   - By default (without `paradox`): delegates to `crate::hooks::get().run_generator(p, step)`.

---

### 2.10 Module 12: `gamepack`

#### Responsibility
Exports and imports portable gzipped JSON game-data packs (`dauntless-reforged-revival-gamedata`, version 1) so a host can generate game data on a Windows PC and import it onto a Linux server.

#### Constants, Types & Functions

```rust
pub const PACK_FORMAT: &str = "dauntless-reforged-revival-gamedata";
pub const PACK_VERSION: u64 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackFile {
    pub path: String,
    pub sha256: String,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pack {
    pub format: String,
    pub version: u64,
    pub game: String,
    pub created: String,
    pub files: Vec<PackFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exported {
    pub file: PathBuf,
    pub files: Vec<String>,
    pub left: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    pub written: Vec<String>,
    pub unchanged: Vec<String>,
    pub backups: Vec<String>,
}

pub fn pack_entries(p: &Paths) -> Vec<(String, PathBuf)>;
pub fn default_pack_file(p: &Paths, now: DateTime<Utc>) -> PathBuf;
pub fn pack_json(pack: &Pack) -> String;
pub fn export_game_data_pack(p: &Paths, out_file: &Path, now: DateTime<Utc>) -> Result<Exported, HostError>;
pub fn read_game_data_pack(p: &Paths, file: &Path) -> Result<Pack, HostError>;
pub fn parse_game_data_pack(p: &Paths, file: &Path, compressed: &[u8]) -> Result<Pack, HostError>;
pub fn import_game_data_pack(p: &Paths, file: &Path) -> Result<Imported, HostError>;
pub fn import_game_data_pack_at(p: &Paths, file: &Path, now: DateTime<Utc>) -> Result<Imported, HostError>;
```

#### Behavioral Contract & Invariants
1. **Export (`export_game_data_pack`):**
   - Collects all 7 `backend/<file>` (`BACKEND_GAME_DATA`), `backend/raw/progression_config.source.json`, `backend/raw/ladyluck_store.source.json`, and 7 `director/<file>` (`DIRECTOR_GAME_DATA`) entries (`pack_entries`) that exist and are **not** synthetic/example data (i.e. differ from the corresponding `.example.json` file).
   - If none are real, returns `Err("no generated game data to export (only the example files, or nothing at all): run reforged-host gamedata first")`.
   - Writes gzip-compressed `pack_json(&pack)` (`format: PACK_FORMAT`, `version: 1`, `game: "1.14.7-647472"`, `created: iso_string(now)`, `files: [{path, sha256, data}]`).
   - Default filename (`default_pack_file`): `<workspace>/gamedata-1.14.7-647472-<YYYYMMDD>.json.gz`.
2. **Import (`parse_game_data_pack`, `import_game_data_pack_at`):**
   - Gunzips `compressed`, validates `format == PACK_FORMAT`, `version == 1`, `game == "1.14.7-647472"`, every `f.path` in `pack_entries(p)`, valid JSON in `f.data`, and `sha256_hex(&f.data) == f.sha256`.
   - For each file in the pack:
     - If target file already has identical content -> adds to `unchanged`.
     - If target file exists, differs, and is not an `.example.json` copy -> backs up existing file to `<file>.bak.<YYYYMMDDTHHMMSSZ>` and records in `backups`.
     - Writes new file and records in `written`.

---

### 2.11 Module 13: `captures`

#### Responsibility
Reads `ParadoxBackend/store-captures.json` and fetches pinned store-capture JSON files from a git repository at an exact 40-character commit SHA into `DauntlessEndpointDocumentation`, verifying every file's SHA-256 before writing.

#### Public Types & Functions

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturePin {
    pub name: String,
    pub repository: String,
    pub commit: String,
    pub files: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureFetch { AlreadyReady, Fetched }

pub fn read_capture_pin(file: &Path) -> Result<CapturePin, HostError>;
pub fn parse_capture_pin(file: &Path, text: &str) -> Result<CapturePin, HostError>;
pub fn missing_captures(p: &Paths, pin: &CapturePin) -> Vec<String>;
pub fn ensure_captures(p: &Paths, pin: &CapturePin, log_file: Option<&Path>) -> Result<CaptureFetch, HostError>;
```

#### Behavioral Contract & Invariants
1. **Pin Validation (`parse_capture_pin(file, text)`):**
   - `repository`: must start with `https://`, `file://`, `/`, or `<A-Za-z>:\` (`"{file}: repository must be an https URL"`).
   - `commit`: must be a 40-character lowercase hex SHA-1 string (`"{file}: commit must be a full 40-character commit id"`).
   - `files`: non-empty JSON object mapping relative `/`-separated plain paths to 64-character lowercase hex SHA-256 strings (`"{file}: files must map plain paths to SHA-256 hashes"`, `"{file}: no files"`).
   - `name`: defaults to `"DauntlessEndpointDocumentation"`.
2. **Fetch & Verification (`ensure_captures`):**
   - `missing_captures` returns any file in `pin.files` that is missing under `p.store_captures` or whose SHA-256 does not equal the pinned digest.
   - If `missing_captures` is empty, returns `Ok(CaptureFetch::AlreadyReady)`.
   - Otherwise initializes a bare git repository in `<workspace>/captures/<pin.name>.git`, runs `git -c protocol.version=2 fetch --quiet --depth 1 <repository> <commit>`, extracts each file with `git show <commit>:<path>`, verifies its SHA-256 against the pin **before** writing any file to `p.store_captures`, and returns `Ok(CaptureFetch::Fetched)`.

---

### 2.12 Module 14: `preset`

#### Responsibility
Creates, inspects, and applies the repository-level `preset/` directory (`preset/preset.json` + game-data files) so servers set up from a checkout with a preset start with working game data.

#### Constants, Types & Functions

```rust
pub const PRESET_FORMAT: &str = "dauntless-reforged-revival-preset";
pub const PRESET_MANIFEST: &str = "preset.json";
pub const PRESET_EXCLUDED: [&str; 2] = ["backend/ladyluck_store.json", "backend/platinum_store.json"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetFile {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetManifest {
    pub comment: Vec<String>,
    pub name: String,
    pub game: String,
    pub executable_sha256: Option<String>,
    pub created: String,
    pub sources: Vec<String>,
    pub files: Vec<PresetFile>,
}

impl PresetManifest {
    pub fn to_json(&self) -> String;
}

#[derive(Debug, Clone, Default)]
pub struct CreateOptions {
    pub name: String,
    pub executable: Option<PathBuf>,
    pub executable_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetCreated {
    pub manifest: PresetManifest,
    pub left: Vec<String>,
    pub excluded: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetApplied {
    pub name: String,
    pub written: Vec<String>,
    pub unchanged: Vec<String>,
    pub kept: Vec<String>,
    pub backups: Vec<String>,
}

pub fn read_preset_manifest(p: &Paths) -> Result<Option<PresetManifest>, HostError>;
pub fn create_preset(p: &Paths, opts: &CreateOptions, now: DateTime<Utc>) -> Result<PresetCreated, HostError>;
pub fn apply_preset(paths: &Paths, force: bool) -> Result<Option<PresetApplied>, HostError>;
pub fn apply_preset_at(p: &Paths, force: bool, now: DateTime<Utc>) -> Result<Option<PresetApplied>, HostError>;
```

#### Behavioral Contract & Invariants
1. **Exclusions:** `backend/ladyluck_store.json` and `backend/platinum_store.json` (`PRESET_EXCLUDED`) are **never** included in a preset.
2. **`create_preset`:**
   - Collects all real (non-missing, non-example) game-data files in `pack_entries(p)` except `PRESET_EXCLUDED`.
   - If none exist, returns `Err("no generated game data to make a preset from yet (reforged-host gamedata first, or pass --generate)")`.
   - Records `executable_sha256` from hashing `opts.executable` (if it exists) or `opts.executable_sha256`.
   - Removes any stale files in `preset/` not in `[PRESET_MANIFEST, "README.md"]` or the new manifest's `files`, writes each file into `preset/<pack_path>`, and writes `preset/preset.json`.
3. **`apply_preset` / `apply_preset_at`:**
   - If `preset/preset.json` does not exist, returns `Ok(None)`.
   - Verifies `manifest.game == "1.14.7-647472"`, every file is a known non-excluded `pack_entries` path, exists under `preset/`, and its LF-normalized text matches `f.sha256` before writing anything.
   - For each file:
     - If destination exists, differs from the preset, and is **not** example/synthetic data:
       - When `force == false`: leaves the host's own file untouched and adds its path to `kept`.
       - When `force == true`: backs up the host's file to `<file>.bak.<YYYYMMDDTHHMMSSZ>` (`backups`) and overwrites it (`written`).
     - If destination is missing or is still example/synthetic data: writes the preset file (`written`).
     - If destination already matches the preset: adds to `unchanged`.

---

### 2.13 Module 15: `progression`

#### Responsibility
Reads the 5 JSONL files exported from Dauntless 1.14.7 under `Items_Analysis/progression_1_14_7/` (`player-experience-tracks.jsonl`, `weapon-experience-tracks.jsonl`, `experience-tracks.jsonl`, `mastery-tracks.jsonl`, `huntpass-season-table.jsonl`) and builds `progression_config.source.json` for the backend's progression generator.

#### Constants, Types & Functions

```rust
pub const PROGRESSION_EXPORT_DIR: &str = "progression_1_14_7";
pub const TRACK_EXPORTS: [&str; 4] = [
    "player-experience-tracks.jsonl",
    "weapon-experience-tracks.jsonl",
    "experience-tracks.jsonl",
    "mastery-tracks.jsonl",
];
pub const HUNT_PASS_EXPORT: &str = "huntpass-season-table.jsonl";
pub const ACTIVE_HUNT_PASS: &str = "season43";
pub const GENERATED_BY: &str = "DaintyLass host kit, from the CatalogExporter progression export";

#[derive(Debug, Clone, PartialEq)]
pub struct ProgressionBuild {
    pub config: Value,
    pub paths: usize,
    pub tracks: Vec<String>,
    pub skipped: Vec<String>,
    pub derived: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceWritten {
    Supplied,
    NoExport,
    Written { written: usize, derived: usize },
}

pub fn read_jsonl(file: &Path) -> Result<Vec<Map<String, Value>>, HostError>;
pub fn parse_jsonl(text: &str) -> Vec<Map<String, Value>>;
pub fn build_progression_config(exports_dir: &Path, active_hunt_pass: &str) -> Result<ProgressionBuild, HostError>;
pub fn build_from_rows(
    tracks: &[Vec<Map<String, Value>>],
    seasons: &[Map<String, Value>],
    active_hunt_pass: &str,
) -> Result<ProgressionBuild, HostError>;
pub fn progression_text(build: &ProgressionBuild) -> String;
pub fn write_progression_source(exports_dir: &Path, source_file: &Path) -> Result<SourceWritten, HostError>;
```

#### Behavioral Contract & Invariants
1. **JSONL Parsing (`parse_jsonl`):** Strips UTF-8 BOM, splits on `\n`, trims lines, and collects all lines that parse as JSON objects (`Value::Object`).
2. **Track & Prestige Transformation (`build_from_rows`):**
   - Iterates `tracks` in order, keeping the first occurrence of each track id (`base.progressionTrack` or `rowName`).
   - Sorts `base.requirements` (`rankId >= 0`, `requiredXP >= 0`) by `rankId` into `[{"rank_id": rank, "xp_required": xp}]`.
   - Records prestige tracks from `row.prestigeTrack` (when non-empty and `!= "None"`) and synthesizes `PrestigeTrack_Weapon_<weapon>` for every `ExperienceTrack_Weapon_<weapon>` with the same requirements and empty rewards (`derived`).
   - Requires `ExperienceTrack_PlayerLevel` to be present (`"the progression export has no ExperienceTrack_PlayerLevel..."`).
   - Appends the `active_hunt_pass` path (`0..=50` ranks, rank 0 `0` XP, ranks `1..=50` `100` XP, empty rewards, `"prestige": null`) if not already present.
   - `config` has shape `{"generated_by": GENERATED_BY, "code": null, "message": "OK", "payload": {"paths": paths}}`.
   - `write_progression_source` leaves `source_file` untouched (`SourceWritten::Supplied`) if `source_file` exists and its `"generated_by"` is not `GENERATED_BY`.

---

### 2.14 Module 16: `publish`

#### Responsibility
Publishes `Rustic-Isles.dll` and `winmm.dll` into `.workspace/runtime/`, writes `.workspace/runtime/runtime.json`, installs the DLLs beside the server's own `Dauntless-Win64-Shipping.exe` (safely moving locked DLLs aside on Windows), and creates required server flag files (`CALL_HUNTPASS_QUERY.flag`).

#### Constants, Types & Functions

```rust
pub const ASIDE_MARK: &str = ".rr-old-";
pub const SERVER_RUNTIME_FLAGS: [&str; 1] = ["CALL_HUNTPASS_QUERY.flag"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeFile {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedRuntime {
    pub version: String,
    pub changelist: u32,
    pub files: Vec<RuntimeFile>,
}

impl PublishedRuntime {
    pub fn to_json(&self) -> String;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Replaced { Replaced, InUse }

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstalledRuntime {
    pub installed: Vec<String>,
    pub unchanged: Vec<String>,
    pub in_use: Vec<String>,
    pub failed: Vec<(String, String)>,
}

pub fn runtime_version(now: DateTime<Utc>, hashes: &[&str]) -> String;
pub fn sha256_file(file: &Path) -> Result<String, HostError>;
pub fn replace_loaded_file(tmp: &Path, target: &Path) -> Result<Replaced, HostError>;
pub fn default_sources(p: &Paths) -> [(&'static str, PathBuf); 2];
pub fn publish_runtime(
    p: &Paths,
    now: DateTime<Utc>,
    sources: &[(&str, PathBuf)],
) -> Result<PublishedRuntime, HostError>;
pub fn install_server_runtime(p: &Paths, win64_dir: &Path) -> InstalledRuntime;
pub fn ensure_server_flags(win64_dir: &Path) -> Result<Vec<String>, HostError>;
```

#### Behavioral Contract & Invariants
1. **Runtime Version Format (`runtime_version(now, hashes)`):**
   `<YYYYMMDD>-<first 8 hex of SHA-256(hashes.join(":"))>` (e.g. `20260417-a1b2c3d4`), and `PublishedRuntime::to_json` formats `{"version": ..., "build": {"changelist": 647472}, "files": [{"name", "size", "sha256"}]}` with `stringify_pretty` + `"\n"`.
2. **Publishing (`publish_runtime`):**
   - Verifies both source files (`Rustic-Isles.dll` and `winmm.dll`) exist; if missing, returns `Err("<name> is not built yet (<path>)")`.
   - Copies both files into `.workspace/runtime/`, computes their sizes and lowercase hex SHA-256 digests, and atomically writes `.workspace/runtime/runtime.json` last.
3. **Locked DLL Replacement (`replace_loaded_file`, `install_server_runtime`):**
   - Sweeps any stale `<name>.rr-old-*` files in `win64_dir` first.
   - If `fs::rename(tmp, target)` fails, renames `target` to `<target>.rr-old-<millis>`, renames `tmp` to `target`, attempts to remove the aside file, and returns `Replaced::InUse`.

---

### 2.15 Module 17: `linux`

#### Responsibility
Implements Linux-specific host capabilities: checking root and POSIX capabilities (`CAP_NET_BIND_SERVICE`, `CAP_NET_ADMIN` from `/proc/self/status`), locating Wine or Proton, initializing the Wine prefix (`wineboot --init`), writing `.workspace/bin/reforged-gameserver`, generating `.workspace/systemd/{reforged.service,reforged-allowlist.service,reforged-allowlist-legacy.service,install.sh}`, and generating `.workspace/config/firewall.sh` (`ufw`).

#### Constants, Types & Functions

```rust
pub const UNIT_SERVER: &str = "reforged.service";
pub const UNIT_ALLOWLIST: &str = "reforged-allowlist.service";
pub const UNIT_ALLOWLIST_LEGACY: &str = "reforged-allowlist-legacy.service";
pub const CAP_NET_BIND_SERVICE: u32 = 10;
pub const CAP_NET_ADMIN: u32 = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeKind { Wine, Proton }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameRuntime { pub kind: RuntimeKind, pub path: String }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WinePrefix { pub ok: bool, pub created: bool, pub detail: Option<String> }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitContext { pub user: String, pub host_bin: PathBuf }

pub fn effective_capabilities(status_text: &str) -> u64;
pub fn has_capability(bit: u32, status_text: Option<&str>) -> bool;
pub fn is_root() -> Result<bool, HostError>;
pub fn can_bind_port_with(port: u16, root: bool, status: Option<&str>, unprivileged_start: Option<&str>) -> bool;
pub fn can_bind_port(port: u16) -> Result<bool, HostError>;
pub fn can_change_firewall() -> Result<bool, HostError>;
pub fn sh_quote(s: &str) -> String;
pub fn gameserver_wrapper_script(host_bin: &Path) -> String;
pub fn write_gameserver_wrapper(paths: &Paths) -> Result<PathBuf, HostError>;
pub fn game_runtime(options: &LinuxOptions) -> Result<Option<GameRuntime>, HostError>;
pub fn ensure_wine_prefix(paths: &Paths, options: &LinuxOptions, log_file: &Path) -> Result<WinePrefix, HostError>;
pub fn unit_word(s: &str) -> Result<String, HostError>;
pub fn unit_path(s: &str) -> Result<String, HostError>;
pub fn unit_context(paths: &Paths) -> Result<UnitContext, HostError>;
pub fn systemd_units(c: &HostConfig, p: &Paths, ctx: &UnitContext) -> Result<Vec<(String, String)>, HostError>;
pub fn write_systemd_units(config: &HostConfig, paths: &Paths) -> Result<Vec<PathBuf>, HostError>;
pub fn write_systemd_units_with(config: &HostConfig, paths: &Paths, ctx: &UnitContext) -> Result<Vec<PathBuf>, HostError>;
pub fn systemd_installed(unit: &str) -> bool;
pub fn linux_firewall_script(c: &HostConfig) -> Result<String, HostError>;
```

#### Behavioral Contract & Invariants
1. **Capabilities & Port Binding:**
   - `effective_capabilities(status_text)` parses hex `CapEff:` from `/proc/self/status` (or returns `0`).
   - `can_bind_port_with(port, root, status, unprivileged_start)`: returns `true` if `port >= 1024 || root`, or parsed `unprivileged_start <= port`, or `has_capability(CAP_NET_BIND_SERVICE, status)`.
2. **Wrapper Script (`gameserver_wrapper_script`):**
   Writes `#!/bin/sh` header and `exec '<host_bin>' gameserver "$@"\n` with mode `0o755` at `.workspace/bin/reforged-gameserver`.
3. **Systemd Units (`systemd_units`):**
   - Refuses `ctx.user == "root" || ctx.user.is_empty()`.
   - Returns `Vec<(String, String)>` containing `reforged-allowlist.service` (in public mode), `reforged-allowlist-legacy.service` (when `edition == Both` and public), `reforged.service` (setting `Environment=REFORGED_ALLOWLIST=external` in public mode so `start` delegates firewall changes to `reforged-allowlist.service`), and `install.sh`.
4. **Linux Firewall Script (`linux_firewall_script`):**
   Generates a `#!/bin/sh` script configuring `ufw` rules for public mode (`ufw allow <port>/tcp`, `ufw allow <first>:<last>/udp`) or private mode (`ufw allow in on tailscale0 to any port 61000 proto tcp`, `ufw allow in on tailscale0 to any port <first>:<last> proto udp`).

---

### 2.16 Module 18: `selfupdate`

#### Responsibility
Implements release discovery, SemVer comparison, `SHA256SUMS.txt` verification, GitHub build-provenance attestation verification (`gh attestation verify`), `.zip` / `.tar.gz` extraction with zip-slip/symlink protection, local-modification detection, and atomic in-place installation with `.workspace/update/installing` crash-recovery markers.

#### Constants, Types & Functions

```rust
pub const RELEASE_FILE: &str = "release.json";
pub const SUMS_FILE: &str = "SHA256SUMS.txt";
pub const SOURCE_ENV: &str = "REFORGED_SERVER_RELEASE";
pub const TAG_PREFIX: &str = "server-v";
pub const ARCHIVE_PREFIX: &str = "ReforgedRevivalServer-";
pub const DEFAULT_REPOSITORY: &str = env!("REFORGED_RELEASE_REPOSITORY");

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    NoRelease(String),
    DownloadFailed(String),
    NotInSums(String),
    Checksum { file: String, expected: String, actual: String },
    Attestation(String),
    BadArchive(String),
    LocalChanges(Vec<String>),
}

impl Problem {
    pub fn code(&self) -> &'static str;
    pub fn detail(&self) -> Option<String>;
    pub fn message(&self) -> String;
}

#[derive(Debug)]
pub enum UpdateError {
    Problem(Problem),
    Host(HostError),
}

pub type UpdateResult<T> = Result<T, UpdateError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub core: [u64; 3],
    pub pre: Vec<String>,
}
impl Version {
    pub fn parse(text: &str) -> Option<Self>;
    pub fn is_prerelease(&self) -> bool;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestFile { pub path: String, pub sha256: String, pub size: u64 }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub version: String,
    pub commit: String,
    pub target: String,
    pub platform: String,
    pub repository: Option<String>,
    pub files: Vec<ManifestFile>,
}
impl Manifest {
    pub fn parse(text: &str) -> Result<Self, String>;
    pub fn file(&self, path: &str) -> Option<&ManifestFile>;
    pub fn short_commit(&self) -> &str;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location { File(PathBuf), Url(String) }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source { GitHub(String), Folder(PathBuf), Url(String) }
impl Source {
    pub fn from_env(installed: Option<&Manifest>) -> Self;
    pub fn parse(value: &str) -> Self;
    pub fn describe(&self) -> String;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub version: String,
    pub archive: String,
    pub archive_at: Location,
    pub sums_at: Location,
    pub repository: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wanted<'a> { Newest(Option<&'a str>), Exactly(&'a str) }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attestation { Verified, Skipped(String) }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub from: Option<String>,
    pub to: String,
    pub files: usize,
    pub removed: usize,
    pub runtime_changed: Option<bool>,
}

pub fn platform() -> Option<&'static str>;
pub fn this_platform() -> Result<&'static str, HostError>;
pub fn archive_name(version: &str, platform: &str) -> String;
pub fn archive_version<'a>(name: &'a str, platform: &str) -> Option<&'a str>;
pub fn pick_newest<'a>(versions: &[&'a str], installed: Option<&str>) -> Option<&'a str>;
pub fn read_manifest(root: &Path) -> Result<Option<Manifest>, HostError>;
pub fn sha256_file(file: &Path) -> Result<String, HostError>;
pub fn parse_sums(text: &str) -> Vec<(String, String)>;
pub fn find_release(source: &Source, platform: &str, wanted: Wanted<'_>) -> UpdateResult<Candidate>;
pub fn candidates_from_github(releases: &Value, repo: &str, platform: &str, wanted: Wanted<'_>) -> UpdateResult<Candidate>;
pub fn fetch(candidate: &Candidate, dir: &Path) -> UpdateResult<PathBuf>;
pub fn verify_attestation(archive: &Path, candidate: &Candidate, log_file: &Path) -> UpdateResult<Attestation>;
pub fn extract(archive: &Path, dest: &Path) -> UpdateResult<PathBuf>;
pub fn damaged_files(root: &Path, manifest: &Manifest) -> Result<Vec<String>, HostError>;
pub fn runtime_changed(previous: &Manifest, next: &Manifest) -> bool;
pub fn local_changes(root: &Path, previous: Option<&Manifest>, next: &Manifest) -> Result<Vec<String>, HostError>;
pub fn remove_moved_aside(root: &Path);
pub fn installing_marker(root: &Path) -> PathBuf;
pub fn install(staged: &Path, root: &Path, previous: Option<&Manifest>) -> UpdateResult<(Manifest, Installed)>;
pub mod http {
    pub fn get_text(url: &str, api: bool) -> Result<String, String>;
    pub fn download(url: &str, file: &Path) -> Result<(), String>;
}
```

---

### 2.17 Module 19: `wine`

#### Responsibility
Implements the hidden `reforged-host gameserver [args...]` command on Linux, wrapping `Dauntless-Win64-Shipping.exe` inside Wine or Proton (optionally under `xvfb-run -a`).

#### Constants, Types & Functions

```rust
pub const ENV_EXE: &str = "REFORGED_GAME_EXE";
pub const ENV_WINE: &str = "REFORGED_WINE";
pub const ENV_PROTON: &str = "REFORGED_PROTON";
pub const ENV_PREFIX: &str = "REFORGED_WINEPREFIX";
pub const ENV_PROTON_DATA: &str = "REFORGED_PROTON_DATA";
pub const ENV_XVFB: &str = "REFORGED_XVFB";
pub const ENV_XVFB_RUN: &str = "REFORGED_XVFB_RUN";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub runtime: String,
}

pub fn is_executable(file: &Path) -> bool;
pub fn find_on_path_in(names: &[&str], path_var: &str) -> Option<String>;
pub fn find_on_path(names: &[&str]) -> Result<Option<String>, HostError>;
pub fn find_wine(path_var: &str) -> Result<Option<String>, HostError>;
pub fn dll_overrides(existing: Option<&str>) -> String;
pub fn build_launch(game_args: &[String], env: &BTreeMap<String, String>) -> Result<LaunchSpec, HostError>;
pub fn wine_prefix_ready(prefix: &Path) -> bool;
pub fn run_game_server(args: &[String]) -> Result<i32, HostError>;
```

#### Behavioral Contract & Invariants
1. **DLL Overrides (`dll_overrides`):**
   Replaces any existing `winmm`, `mscoree`, or `mshtml` override entries in `existing` and appends `"winmm=n,b;mscoree=d;mshtml=d"`.
2. **Launch Construction (`build_launch(game_args, env)`):**
   - Requires `REFORGED_GAME_EXE` (`ENV_EXE`) to be an absolute path that exists on disk.
   - When `REFORGED_PROTON` is set: requires `REFORGED_PROTON` to be executable and `REFORGED_PROTON_DATA` to be an absolute path; sets `STEAM_COMPAT_DATA_PATH`, `STEAM_COMPAT_CLIENT_INSTALL_PATH` (if under `compatibilitytools.d` or `steamapps/common`), `STEAM_COMPAT_APP_ID=0`, `PROTON_LOG=0`, `WINEDLLOVERRIDES`; launches `<proton> run <exe> <game_args...>` with `runtime = "Proton (<build>)"`.
   - Otherwise locates Wine (`REFORGED_WINE` or `find_wine(PATH)`), requires `REFORGED_WINEPREFIX` to be an absolute path, sets `WINEPREFIX`, `WINEDEBUG=-all` (if unset), `WINEDLLOVERRIDES`; launches `<wine> <exe> <game_args...>` with `runtime = "Wine"`.
   - When `REFORGED_XVFB != "0"` and `DISPLAY` is unset/empty: wraps in `<xvfb-run> -a -s "-screen 0 1024x768x24" <command> <args...>` with `runtime = "<runtime> on xvfb-run"`.

---

### 2.18 Module 20: `cli`

#### Responsibility
Defines the `clap` CLI argument parser (`Cli`, `Command`, and per-command `*Args` structs) and custom error rendering (`parse_from`) matching the host kit's CLI surface.

#### Public Commands & Flags (`Command`)
1. `Setup(SetupArgs)`:
   - `--mode`, `--edition`, `--host`, `--game-address`, `--name`, `--game`, `--legacy-game`, `--registration`, `--share`, `--mongo-uri`, `--manage-mongo`, `--no-manage-mongo`, `--mongod`, `--max-hunts`, `--xp-multiplier`, `--reward-multiplier`, `--no-multipliers`, `--public-port`, `--legacy-public-port`, `--server-fps`, `--idle-fps`, `--training-on-demand`, `--training-at-boot`, `--training-idle-minutes`, `--verbose-game-logs`, `--quiet-game-logs`, `--memory-guard`, `--no-memory-guard`, `--min-free-ram`, `--hunt-ram`, `--wine`, `--proton`, `--no-proton`, `--xvfb`, `--no-xvfb`, `--firewall-interface`, `--no-firewall-interface`, `--fresh-start`, `--skip-install`, `unexpected: Vec<String>`.
2. `Start`: No flags.
3. `Serve(ServeArgs)`: `--config` (`-c`), `--tui`, `--log-level` (default `"info"`).
4. `Invite(InviteArgs)`: `--for`, `--uses` (default `"1"`), `--unlimited`, `--list`, `--json`, `--game`.
5. `Doctor(DoctorArgs)`: `--no-hash`.
6. `Gamedata(GamedataArgs)`: `--examples`, `--example-sources`, `--progression`, `--export`, `--import`.
7. `PublishRuntime(PublishRuntimeArgs)`: `--from`.
8. `InstallGame(InstallGameArgs)`: `--dir`, `--version`, `--code-file`, `--sign-out`.
9. `Preset(PresetArgs)`: `--create`, `--generate`, `--name`, `--apply`, `--force`.
10. `Multipliers(MultipliersArgs)`: `--xp`, `--rewards`, `--off`.
11. `Update(UpdateArgs)`: `--check`, `--json`, `--no-setup`.
12. `Backup`: No flags.
13. `Db(DbArgs)`: `DbCommand::{Migrate(DbMigrateArgs), Verify(DbVerifyArgs), ExportMongo(DbExportMongoArgs), Replay(DbReplayArgs)}`.
14. `Network(NetworkArgs)`: `NetworkCommand::{CaInit(CaInitArgs), IssueNodeCert(IssueNodeCertArgs), List, Status(NetworkStatusArgs), Join(JoinArgs)}`.
15. `History(HistoryArgs)` & `Revert(RevertArgs)`: `--account`, `--actor`, `--since`, `--until`, `--limit`, `--cursor`, `HistoryCommand::Verify`, `--dry-run`, `--apply`.
16. Hidden service/wrapper subcommands: `Gateway`, `Allowlist`, `Content`, `Admin`, `Backend`, `Director`, `Gameserver(GameserverArgs)`.

---

### 2.19 Module 21: `commands/*`

#### Responsibility
Implements the CLI command handlers and their dependency-injection traits (`SetupEnv`, `UpdateEnv`, `BackupEnv`, `Control`) for `setup`, `start`, `doctor`, `invite`, `multipliers`, `gamedata`, `preset`, `publish-runtime`, `update`, `backup`, `db`, `history`, `install-game`, and `network`.

#### Key Submodule Specifications
1. **`commands/setup` (`setup.rs`, `setup/options.rs`, `setup/firewall.rs`, `setup/install.rs`, `setup/database.rs`, `setup/secrets.rs`, `setup/paradox.rs`):**
   - `options.rs`: `SetupPromptEnv`, `resolve_options(args: &SetupArgs, existing: Option<&HostConfig>, env: &dyn SetupPromptEnv) -> Result<HostConfig, HostError>`.
   - `firewall.rs`: `ALLOWLIST_RULE_NAME`, `FirewallRules { pub applied: Vec<String>, pub failed: Vec<String> }`, `firewall_script(config: &HostConfig) -> String`, `apply_windows_firewall_rules(config: &HostConfig, log_file: &Path) -> Result<FirewallRules, HostError>`, `write_firewall_script(p: &Paths, config: &HostConfig) -> Result<PathBuf, HostError>`.
   - `install.rs`: `ServiceSource { Checkout, Release(Manifest) }`, `ServiceInstall { pub name: &'static str, pub dir: PathBuf, pub build_output: Option<PathBuf> }`, `service_source(p: &Paths) -> Result<ServiceSource, HostError>`, `services_to_install(p: &Paths) -> Vec<ServiceInstall>`, `install_service(svc: &ServiceInstall, log_file: &Path) -> Result<(), HostError>`, `repair_release(p: &Paths, manifest: &Manifest, log_file: &Path) -> Result<usize, HostError>`.
   - `setup.rs`: `NOT_AS_ROOT`, `SetupEnv` trait, `RealSetupEnv`, `run(paths: &Paths, args: &SetupArgs) -> ExitCode`, `run_with(p: &Paths, args: &SetupArgs, env: &mut dyn SetupEnv) -> Result<(), HostError>`.
2. **`commands/start` (`start/mod.rs`, `start/control.rs`, `start/binaries.rs`):**
   - `control.rs`: `CONTROL_ENV = "REFORGED_CONTROL"`, `SERVER_READY_LINE = "REFORGED_SERVER_READY"`, `controlled_by_stdin() -> bool`, `watch_stop_input(input, on_stop)`, `Control::begin(stdin) -> Result<Self, HostError>`:
     - `Control::begin(stdin)` synchronously installs OS termination handlers before returning: on Windows it clears any inherited ignore-Ctrl+C state (`SetConsoleCtrlHandler(None, false)`) so non-interactive parent shells do not suppress `CTRL_C_EVENT` and registers a console control handler for `CTRL_C_EVENT`, `CTRL_BREAK_EVENT`, and `CTRL_CLOSE_EVENT`; on Unix it registers handlers for `SIGINT` and `SIGTERM`.
     - When `stdin` is `true`, `Control::begin` also spawns a reader thread on standard input via `watch_stop_input`, which triggers shutdown when a trimmed `"stop"` line (case-insensitive) or EOF/read error is encountered.
     - When shutdown is triggered (by signal, console event, stdin `"stop"`/EOF, or `Control::stop()`), `Control::asked()` returns `true`, the shared `AtomicBool` returned by `Control::flag()` becomes `true`, and all threads blocked in `Control::wait()` or `Control::wait_timeout(timeout)` wake immediately.
   - `start/mod.rs`: `PlannedService`, `StartEnv` trait, `service_plan(p: &Paths, config: &HostConfig, programs: &Programs, external_allowlist: bool) -> Result<Vec<PlannedService>, HostError>`, `start_services`, `finish`, `run(paths: &Paths) -> ExitCode`:
     - `finish(started, supervisor, control, controlled, up, after_stop)`: when `started && !control.asked()`, prints `up`, prints `SERVER_READY_LINE` (if `controlled`) or `"Ctrl+C stops everything."` (if `!controlled`), flushes stdout, and blocks in `control.wait()`; then prints `"Stopping..."`, stops all supervised service process trees (`supervisor.stop_all()`, killing each service's process tree/JobObject and waiting for exit), invokes `after_stop()`, and prints `"Stopped."`.
3. **`commands/update` (`update.rs`):**
   - `MONGO_MIGRATED_MARKER = ".mongo-migrated"`, `MONGO_COLLECTIONS: [&str; 15]`, `UpdateEnv` trait, `RealUpdateEnv`, `mongo_migrated_marker(p: &Paths) -> PathBuf`, `has_unmigrated_mongo_dir(p: &Paths) -> bool`, `has_external_mongo_evidence(p: &Paths, config: &HostConfig) -> bool`, `check_unmigrated_mongo_before_start(p: &Paths, config: &HostConfig) -> Result<(), HostError>`, `unmigrated_mongo_doctor_problem(p: &Paths, config: &HostConfig) -> Option<(String, String)>`, `migrate_existing_mongo_data(p: &Paths, config: &HostConfig, log_file: &Path, say: &mut dyn FnMut(&str)) -> Result<bool, HostError>`, `run(paths: &Paths, args: &UpdateArgs) -> ExitCode`, `run_with(p: &Paths, args: &UpdateArgs, env: &mut dyn UpdateEnv) -> Result<ExitCode, HostError>`.
4. **`commands/doctor`, `commands/invite`, `commands/multipliers`, `commands/gamedata`, `commands/preset`, `commands/publish_runtime`:**
   - Thin CLI wrappers around `doctor`, `invite`, `hostconfig`, `gamedata`/`gamepack`, `preset`, and `publish` returning `ExitCode`.

---

### 2.20 Module 22: `setup/paradox` (`commands/setup/paradox.rs`)

#### Responsibility
Verifies that the required source directories exist in the checkout (`missing_paradox`) and performs one-time non-destructive migration (`migrate_old_workspace`) of a host's own generated files from older workspace layouts (`.workspace/Mystic-Paradox` or pre-submodule `paradox/` and `native/`).

#### Constants, Types & Functions

```rust
pub const SDK_FILES: [&str; 6] = [
    "SDK.hpp",
    "Assertions.inl",
    "NameCollisions.inl",
    "PropertyFixup.hpp",
    "UnrealContainers.hpp",
    "UtfN.hpp",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout;

impl Layout {
    pub fn new(p: &Paths) -> Self;
}

pub fn missing_paradox(l: &Layout) -> Vec<&'static str>;
pub fn old_workspace_items(l: &Layout) -> Vec<(PathBuf, PathBuf)>;
pub fn pre_submodule_items(l: &Layout) -> Vec<(PathBuf, PathBuf)>;
pub fn old_folders(l: &Layout) -> Vec<PathBuf>;
pub fn migrate_old_workspace(l: &Layout, repo: &Path) -> Result<Vec<String>, HostError>;
```

---

## 3. Target Crate Architecture for DL3b (`crates/hostkit` vs `services/host`)

1. **`rust/crates/hostkit` (`license = "MIT"`, `Copyright (c) 2026 Silent Spud`):**
   - Contains all Category A (MIT-eligible original modules) and Category B (clean-room reimplemented modules from Section 2 and Section 4).
   - Exposes `pub mod hooks` with `pub trait HostHooks: Send + Sync` and `set_hooks` / `get`:
     ```rust
     pub trait HostHooks: Send + Sync {
         fn run_generator(&self, paths: &Paths, step: &Step<'_>) -> Result<(), HostError>;
         fn db_migrate(
             &self,
             from: &str,
             db_cfg: &store::DatabaseConfig,
             dry_run: bool,
         ) -> Result<(Vec<(String, usize)>, Vec<(String, usize, usize)>), HostError>;
         fn db_verify(
             &self,
             from: &str,
             db_cfg: &store::DatabaseConfig,
         ) -> Result<Vec<(String, usize, usize)>, HostError>;
         fn db_export_mongo(&self, uri: &str, out: &Path) -> Result<Vec<(String, usize)>, HostError>;
         fn register_default_modules(
             &self,
             registry: &mut controller::ModuleRegistry,
             paths: &Paths,
             config: &HostConfig,
             secrets: &Secrets,
             env: &EnvFiles,
             db_cfg: &store::DatabaseConfig,
         ) -> Result<(), HostError>;
         fn spawn_front_services(
             &self,
             paths: &Paths,
             config: &HostConfig,
             env: &EnvFiles,
             allowlist_external: bool,
             legacy_Too: bool,
         ) -> Result<Vec<Box<dyn controller::ManagedTask>>, HostError>;
         fn run_service(&self, name: &str, args: &[String]) -> Option<ExitCode>;
         fn legacy_write_env_files(
             &self,
             paths: &Paths,
             config: &HostConfig,
             secrets: &Secrets,
             platform: Platform,
         ) -> Result<Vec<PathBuf>, HostError>;
         fn legacy_prepare(&self, paths: &Paths, config: &HostConfig) -> Result<(), HostError>;
         fn legacy_create_invite_code(
             &self,
             paths: &Paths,
             opts: &CodeOptions,
         ) -> Result<String, HostError>;
         fn legacy_list_invite_codes(&self, paths: &Paths) -> Result<Vec<Value>, HostError>;
         fn legacy_doctor_checks(
             &self,
             paths: &Paths,
             config: &HostConfig,
             opts: &DoctorOptions,
         ) -> Result<Vec<Check>, HostError>;
     }
     ```
   - Has **zero** normal or dev dependencies on `gateway`, `allowlist`, `content`, `chat`, `isles-core`, `isles-db`, `isles-director`, `isles-1-14-7`, `isles-1-12-0`, `rifts-host`, `rifts-seam`, or `revived-1-4-4`.
2. **`rust/services/host` (`license = "AGPL-3.0-only"`):**
   - Thin binary/library shell that depends on `hostkit` and the AGPL module/service crates (`isles-core`, `isles-db`, `isles-director`, `isles-1-14-7`, `gateway`, `allowlist`, `content`, `revived-1-4-4`).
   - Implements `HostHooks`, registers it via `Once`, and re-exports `hostkit` modules so all existing integration tests in `rust/services/host/tests/*.rs` and workspace binaries (`reforged-host`, `dainty-lass-server-console`, `dainty-lass-server-minimal`, `launcher`) continue to compile and pass unchanged.

---

## 4. Post-cutover Updates (`C1`–`C7`, `M1`–`M6`, `A1`)

### 4.1 In-process Rust Backend and Director by Default (`C1`–`C6`)

1. **Default Build vs `--features paradox`:**
   - By default (`paradox` feature disabled), the 1.14.7 backend and Director run in-process in Rust (using SQLite/Postgres via `store::Database` instead of Node.js and MongoDB), and the 8 game-data generation steps (`gamedata`) run in-process via `HostHooks::run_generator` instead of spawning `npm run ...`.
   - When `--features paradox` is enabled, the legacy Node.js (`ParadoxBackend`, `ParadoxDirector`) and MongoDB (`mongod`) paths remain available for rollback.
2. **Doctor Checks without `paradox`:**
   - When `paradox` is disabled, `doctor` does **not** require `node`, `npm`, `mongod`, or `git`, and does not check `ParadoxBackend/node_modules` or `ParadoxDirector/node_modules`.
   - Instead, `doctor` calls `store_database_check(paths, config)` which reports `[FAIL]` in area `"database"` if unmigrated legacy MongoDB data is detected or if the SQLite/Postgres store fails its health check.
3. **Hidden Service Subcommands (`backend`, `director`):**
   - `Cli::Command` includes hidden subcommands `Backend` and `Director` dispatched through `HostHooks::run_service`.

### 4.2 Legacy MongoDB-to-SQLite Migration & Refusal Guards (`M1`–`M6`, `A1`)

1. **Migration Marker and Config Escape Hatch:**
   - Marker file: `.workspace/config/.mongo-migrated` (`mongo_migrated_marker(paths)`).
   - `host.toml` `[database]` field: `fresh_start: Option<bool>` (`config.fresh_start()`), CLI flag `reforged-host setup --fresh-start` (sets `database.fresh_start = Some(true)`).
2. **Detecting Unmigrated Data (`has_unmigrated_mongo_dir`, `has_external_mongo_evidence`, `collection_has_documents`):**
   - If `.workspace/config/.mongo-migrated` exists or `config.fresh_start()` is `true`, no migration or refusal is triggered.
   - Managed MongoDB directory (`.workspace/mongodb`, `paths.mongo_data`): considered non-empty if it contains files other than `mongod.lock`.
   - External MongoDB URI (`has_external_mongo_evidence`):
     - Treats a URI as explicitly configured by a host when `mongo.uri` differs from the default host/port (`127.0.0.1:27017` / `localhost:27017` with no credentials) **or** when `.workspace/config/backend.env` or `.workspace/logs/mongo.log` already exists on disk (`A1`: a fresh Docker `apply` whose URI only changes the database name on `127.0.0.1:27017` without prior workspace artifacts is **not** treated as unmigrated legacy data).
     - `collection_has_documents(uri)`: connects via the MongoDB wire protocol (`mongo::wire`) and checks `count` on the 15 collections in `MONGO_COLLECTIONS`.
3. **Automatic Migration in `setup` and `update` (`M1`):**
   - During `reforged-host setup` and `reforged-host update`, calls `migrate_existing_mongo_data`:
     - If unmigrated MongoDB data is present, starts a temporary `mongod` if needed, invokes `HostHooks::db_migrate`, and writes `.workspace/config/.mongo-migrated` only after verification confirms zero mismatches.
     - Never modifies or deletes the MongoDB data directory.
4. **Startup Refusal (`M1`, `M4`):**
   - `start`, `serve`, and `backend` perform the unmigrated-MongoDB check before requiring `backend.env` or `host.toml`, and refuse to start if unmigrated MongoDB data in `.workspace/mongodb` or unreachable external MongoDB evidence exists without `.mongo-migrated`.

### 4.3 `db` Subcommands (`M2`, `M3`, `M5`, `RU8`, `RU9`)

`reforged-host db <subcommand>` (`commands/db.rs`) delegates to `HostHooks::run_db_command` (or `HostHooks::db_migrate`, `HostHooks::db_verify`, and `HostHooks::db_export_mongo`):
1. `reforged-host db migrate [--from-mongo <uri_or_archive>] [--to <db_url>] [--dry-run]`
2. `reforged-host db verify --from-mongo <uri_or_archive> [--to <db_url>]`
3. `reforged-host db export-mongo [--from-mongo <uri>] --out <file.tgz>`
4. `reforged-host db replay --corpus <path> [--left <url|recorded>] [--right <url>] [--key-map <file.json>] [--out <report.json>] [--json]`:
   Replays a gateway JSONL capture corpus against recorded responses or live backend/Director/chat targets via `isles_db::run_replay_cli`, automatically defaulting `--signing-key`, `--jwt-secret`, `--gs-key`, and `--gateway-secret` from `.workspace/config/secrets.env` when omitted on the CLI. Returns exit code `0` on zero unexplained diffs and `2` when unexplained diffs are found.


