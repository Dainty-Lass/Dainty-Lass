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

> **Note on Planned Module Names in `dainty-lass-plan.md`:** Row DL3a in `docs/dainty-lass-plan.md` also lists `commands/modules.rs`, `postgres.rs`, and `turn.rs` as MIT-eligible modules if present. In the current tree, module console commands live in `repl.rs` (`parse_module`, `module_table`, `run_module`) and `hostconfig/modules.rs`, database setup lives in `commands/setup/database.rs` and `hostconfig.rs` (`DatabaseSettings` / `node_database_config`), and relay/network configuration lives in `commands/network.rs` and `hostconfig.rs` (`NetworkConfig`). All of those are Category A original DaintyLass code and are MIT-eligible.

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
pub const RUNTIME_FILE_NAMES: [&str; 2] = ["MysticParadox.dll", "winmm.dll"];
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
pub struct BothLegacyPorts {
    pub metagame: u16,
    pub deploy: u16,
    pub content: u16,
    pub allowlist: u16,
    pub public_gateway: u16,
    pub private_gateway: u16,
    pub game_first: u16,
    pub game_last: u16,
    pub game_lowest: u16,
}

pub const BOTH_LEGACY_PORTS: BothLegacyPorts = BothLegacyPorts {
    metagame: 61010,
    deploy: 61011,
    content: 61012,
    allowlist: 61015,
    public_gateway: DEFAULT_LEGACY_PUBLIC_PORT,
    private_gateway: 61010,
    game_first: 8780,
    game_last: 8787,
    game_lowest: 8778,
};
```

#### Public Functions & `Paths` Layout

- `pub fn game_exe_relative() -> PathBuf`:
  Returns `Archon/Binaries/Win64/DauntlessServer-Win64-Shipping.exe` joined using platform path separators.
- `pub fn game_port_range(max_hunt_servers: u32) -> (u16, u16)`:
  - Let `n = max_hunt_servers.clamp(1, MAX_HUNTS) as u16`.
  - There are 5 hub instances plus `n` hunt servers (`total = 5 + n` ports).
  - The top port is always `PORTS.game_last` (`8777`).
  - The first port is `PORTS.game_last - total + 1` (e.g. for `n = 3`, `8777 - 8 + 1 = 8770` -> `(8770, 8777)`; for `n = 7`, `(8766, 8777)`).
- `pub fn find_repo_root(start: &Path) -> Option<PathBuf>`:
  Walks `start` and up to 8 parent directories (`0..=8`). Returns the first directory `dir` containing both a directory `modules/rustic-isles` and a file `rust/Cargo.toml`.
- `pub fn locate_repo_root() -> Result<PathBuf, HostError>`:
  Checks `std::env::current_dir()` via `find_repo_root`, then `std::env::current_exe()`'s parent directory via `find_repo_root`. If neither matches, returns an error instructing the user to run from the DaintyLass checkout.
- `#[derive(Debug, Clone, PartialEq, Eq)] pub struct Paths`:
  Constructed by `Paths::new(repo: impl Into<PathBuf>) -> Self` with exact paths relative to `repo`:
  - `repo`: `<repo>`
  - `workspace`: `<repo>/.workspace`
  - `config`: `<repo>/.workspace/config`
  - `logs`: `<repo>/.workspace/logs`
  - `gameserver_logs`: `<repo>/.workspace/logs/gameservers`
  - `tls`: `<repo>/.workspace/tls`
  - `runtime_publish`: `<repo>/.workspace/runtime`
  - `branding`: `<repo>/.workspace/branding`
  - `news_file`: `<repo>/.workspace/branding/news.json`
  - `banner_image`: `<repo>/.workspace/branding/banner.png`
  - `mongo_data`: `<repo>/.workspace/mongodb`
  - `wine_prefix`: `<repo>/.workspace/wine`
  - `gameserver_wrapper`: `<repo>/.workspace/bin/dauntless-gameserver`
  - `systemd`: `<repo>/.workspace/systemd`
  - `preset`: `<repo>/preset`
  - `preset_manifest`: `<repo>/preset/preset.json`
  - `host_toml`: `<repo>/.workspace/config/host.toml`
  - `host_yaml`: `<repo>/.workspace/config/host.yaml`
  - `host_yml`: `<repo>/.workspace/config/host.yml`
  - `host_json`: `<repo>/.workspace/config/host.json`
  - `cert_file`: `<repo>/.workspace/tls/gateway-cert.pem`
  - `key_file`: `<repo>/.workspace/tls/gateway-key.pem`
  - `rustic_isles`: `<repo>/modules/rustic-isles`
  - `revived`: `<repo>/modules/legacy-revived`
  - `paradox`: `<repo>/modules/rustic-isles/reference/1.14.7`
  - `old_upstream`: `<repo>/.workspace/Mystic-Paradox`
  - `backend_dir`: `<repo>/modules/rustic-isles/reference/1.14.7/ParadoxBackend`
  - `director_dir`: `<repo>/modules/rustic-isles/reference/1.14.7/ParadoxDirector`
  - `gateway_dir`: `<repo>/services/gateway`
  - `content_dir`: `<repo>/services/content`
  - `runtime_source`: `<repo>/modules/rustic-isles/native/runtime`
  - `sdk_header`: `<repo>/modules/rustic-isles/native/runtime/SDK.hpp`
  - `sdk_dir`: `<repo>/modules/rustic-isles/native/runtime/SDK`
  - `runtime_dll`: `<repo>/modules/rustic-isles/native/runtime/x64/Release/MysticParadox.dll`
  - `loader_dir`: `<repo>/native/runtime-loader`
  - `loader_dll`: `<repo>/native/runtime-loader/target/release/winmm.dll`
  - `exporter_dir`: `<repo>/modules/rustic-isles/native/catalog-exporter`
  - `exporter_dll`: `<repo>/modules/rustic-isles/native/catalog-exporter/x64/Release/CatalogExporter.dll`
  - `backend_game_data`: `<repo>/modules/rustic-isles/reference/1.14.7/ParadoxBackend/game-data`
  - `director_game_data`: `<repo>/modules/rustic-isles/reference/1.14.7/ParadoxDirector/game-data`
  - `raw_game_data`: `<repo>/modules/rustic-isles/reference/1.14.7/ParadoxBackend/game-data/raw`
  - `exports`: `<repo>/modules/rustic-isles/reference/1.14.7/Items_Analysis`
  - `store_captures`: `<repo>/modules/rustic-isles/reference/1.14.7/DauntlessEndpointDocumentation`
  - `captures_pin`: `<repo>/modules/rustic-isles/reference/1.14.7/captures.pin.json`

---

### 2.2 Module 2: `envfile`

#### Responsibility
Parses, formats, and updates `.env` files while preserving comments, blank lines, line endings (`\n` vs `\r\n`), and UTF-8 BOM (`\u{feff}`) when editing in-place.

#### Public Types & Functions

```rust
pub type EnvMap = BTreeMap<String, String>;

pub fn valid_key(key: &str) -> bool;
pub fn parse_env(text: &str) -> EnvMap;
pub fn format_value(key: &str, value: &str) -> Result<String, HostError>;
pub fn format_env<'a>(
    entries: impl IntoIterator<Item = (&'a str, &'a str)>,
    header: Option<&str>,
) -> Result<String, HostError>;

#[derive(Debug, Clone)]
pub struct EnvDocument { /* preserves lines, line ending, BOM */ }

impl EnvDocument {
    pub fn parse(text: &str) -> Self;
    pub fn values(&self) -> EnvMap;
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), HostError>;
    pub fn render(&self) -> String;
}
```

#### Behavioral Contract & Invariants
1. **Key Validation (`valid_key`):** Non-empty ASCII string starting with `A-Z`, `a-z`, or `_`, followed by zero or more `A-Z`, `a-z`, `0-9`, or `_` (`/^[A-Za-z_][A-Za-z0-9_]*$/`). Invalid keys produce `HostError::message(format!("invalid env key: {key}"))`.
2. **Parsing (`parse_env`):**
   - Strips leading `\u{feff}` BOM if present.
   - Splits on `\n`, trimming trailing `\r`.
   - Ignores empty/whitespace-only lines and lines whose trimmed text starts with `#`.
   - Strips optional `export ` prefix (when followed by whitespace).
   - Splits on the first `=`. Trims key; if `!valid_key(key)`, ignores the line.
   - Value unquoting:
     - Trims leading/trailing whitespace.
     - If wrapped in `"..."` (length >= 2): strips outer quotes and sequentially unescapes `\n` -> newline, `\"` -> `"`, `\\` -> `\` (in that exact order, or single-pass equivalent where `\\n` becomes `\n` literal backslash+n? **Important compatibility invariant:** the Node kit applied `.replace(/\\n/g, "\n").replace(/\\"/g, '"').replace(/\\\\/g, "\\")` sequentially!).
     - If wrapped in `'...'` (length >= 2): strips outer single quotes with no escape processing.
     - If unquoted: strips inline comments starting at the first ` #` (space followed by `#`) and trims trailing whitespace.
   - Later duplicate keys overwrite earlier keys.
3. **Formatting (`format_value`, `format_env`):**
   - If `value` contains `\n` or `\r`, returns `Err(HostError::message(format!("env value for {key} must not contain newlines")))`.
   - Plain values: if `value` is non-empty and every byte is in `A-Z`, `a-z`, `0-9`, `_`, `.`, `/`, `:`, `@`, `,`, `+`, `=`, `%`, `-` (`/^[A-Za-z0-9_./:@,+=%-]+$/`) — **wait**: empty string `""` is also matched by `*` in `/^[A-Za-z0-9_./:@,+=%-]*$/`, so empty string formats as empty `KEY=`!
   - Otherwise wraps in double quotes `"..."`, escaping `\` as `\\` and `"` as `\"`.
   - `format_env` prepends `# <line>\n` for each line of `header` if `Some`, followed by `KEY=formatted_value\n` for each entry, ending with `\n`.
4. **Round-trip `EnvDocument`:**
   - Detects whether input starts with `\u{feff}` (`bom`) and whether input contains `\r\n` (`crlf`).
   - `set(key, value)` validates `key` and formats `value`; if `key` already exists in the document, replaces the **last** occurrence in-place (`KEY=formatted`); otherwise appends a new line at the end (inserting before a trailing empty line if the file ended with a newline).
   - `render()` joins lines with `\r\n` (if `crlf`) or `\n`, prepending `\u{feff}` if `bom` was present.

---

### 2.3 Module 3: `hostconfig`

#### Responsibility
Defines, validates, loads, and saves the server configuration (`host.toml`, `host.yaml` / `host.yml`, `host.json`), manages `secrets.env`, and generates the per-service `.env` files (`backend.env`, `director.env`, `content.env`, `gateway.env`, `admin.env`, `allowlist.env`).

#### Constants

```rust
pub const DEFAULT_MONGO_URI: &str = "mongodb://127.0.0.1:27017/?replicaSet=rs0";
pub const MAX_MULTIPLIER: f64 = 100.0;
pub const DATABASE_FILE: &str = "dainty-lass.db";
pub const NODE_CERT_FILE: &str = "node-cert.pem";
pub const NODE_KEY_FILE: &str = "node-key.pem";
pub const CLUSTER_CA_FILE: &str = "cluster-ca-cert.pem";
pub const SESSION_KEY_FILE: &str = "network-session.key";
pub const LEGACY_RULE_NAME: &str = "Dauntless Reforged Revival 1.4.4 game ports (allowlist)";
pub const LEGACY_UNSUPPORTED: &str = "this build does not include the Dauntless 1.4.4 stack (built without the `legacy` feature): rebuild with `--features legacy`, or set `edition = \"reborn\"`";
pub const SECRET_FILE: &str = "secrets.env";
pub const SECRET_KEYS: [&str; 11] = [
    "GATEWAY_SECRET",
    "ALLOWLIST_SECRET",
    "REFORGED_ADMIN_KEY",
    "REFORGED_GAMESERVER_KEY",
    "API_KEY_HASH_SECRET",
    "METRICS_TOKEN",
    "JWT_PRIVATE_KEY_B64",
    "JWT_PUBLIC_KEY_B64",
    "ADMIN_API_KEY",
    "OPERATOR_SECRET",
    "DATABASE_PASSWORD",
];
pub const RESERVED_PORTS: [u16; 9] = [61000, 61001, 61002, 61003, 61005, 61010, 61011, 61012, 61015];
pub const QUIET_GAME_LOG_CMDS: &str = "LogCore off, LogInit off, LogLinker off, LogPackageLocalizationCache off, LogStreaming off, LogConfig off, LogEnum off, LogOnline off, LogOnlineGame off, LogAnalytics off, LogHttp off, LogTemp off, LogArchon off, LogEOS off, LogEOSAntiCheat off";
```

#### Public Types

- `ServerMode`: `Private` (`"private"`), `Public` (`"public"`).
- `Registration`: `Invitecode` (`"INVITECODE"`), `Open` (`"OPEN"`), `None` (`"NONE"`).
- `Edition`: `Reborn` (`"reborn"`, default), `Legacy` (`"legacy"`), `Both` (`"both"`).
  - Methods: `as_str`, `parse`, `runs_reborn`, `runs_legacy`, `runs(&GameVersion)`, `primary() -> GameVersion`, `versions() -> Vec<GameVersion>`, `supported() -> bool`.
- `Platform`: `Windows`, `Linux`, with `Platform::current()`.
- `MongoConfig`: `{ pub uri: String, pub manage: bool, pub mongod: Option<String> }`.
- `Multipliers`: `{ pub xp: f64, pub rewards: f64 }`.
- `LinuxOptions`: `{ pub wine: Option<String>, pub proton: Option<String>, pub xvfb: bool, pub firewall_interface: Option<String> }` (`DEFAULT_LINUX` has `xvfb: true`, others `None`).
- `DatabaseSettings`: `Sqlite { path: Option<String> }` or `Postgres { url: String }`.
- `PerformanceOptions`:
  - `server_fps: u32` (default `30`, valid `0` or `10..=240`)
  - `idle_fps: u32` (default `10`, valid `0` or `1..=240`)
  - `training_on_demand: bool` (default `true`)
  - `training_idle_minutes: u32` (default `5`, valid `0..=1440`)
  - `verbose_game_logs: bool` (default `false`)
  - `memory_guard: bool` (default `true`)
  - `min_free_memory_mb: u32` (default `1024`, valid `>= 0`)
  - `hunt_memory_mb: u32` (default `1536`, valid `>= 256`)
- `SwitchesConfig`: `{ pub lantern_tap: bool, pub glider_chain_flings: bool, pub cell_swap_in_loadout: bool, pub pangar_lantern_aspect: bool, pub founders_flare: bool }` (all default `false`; serialized to `[switches]` and `REFORGED_SWITCH_*` = `"1"`/`"0"`).
- `HistoryConfig`: `{ pub retention_days: Option<u32>, pub ship_to_peer: Option<NodeId> }` (`retention_days` valid `1..=3650`).
- `NetworkConfig`: `{ pub node: NodeId, pub region: RegionId, pub leader: Option<NodeId>, pub nodes: Vec<Peer> }` (`effective_leader(&self) -> NodeId`).
- `FileForm`: tracks original key order (`keys: Vec<String>`) and nested sub-key orders (`nested: BTreeMap<String, Vec<String>>`) so saving `host.toml` / `host.json` preserves existing field ordering.
- `HostConfig`:
  - `version: u32` (always `1`)
  - `mode: ServerMode`
  - `edition: Edition`
  - `host: String`
  - `game_address: String`
  - `name: String`
  - `game_dir: Option<String>`
  - `registration: Registration`
  - `share_url: Option<String>`
  - `mongo: MongoConfig`
  - `max_hunt_servers: u32`
  - `multipliers: Option<Multipliers>`
  - `public_port: Option<u16>`
  - `legacy_game_dir: Option<String>`
  - `legacy_public_port: Option<u16>`
  - `performance: Option<PerformanceOptions>`
  - `linux: Option<LinuxOptions>`
  - `database: Option<DatabaseSettings>`
  - `guard: Option<GuardConfig>`
  - `history: Option<HistoryConfig>`
  - `modules: Option<ModulesConfig>`
  - `network: Option<NetworkConfig>`
  - `switches: Option<SwitchesConfig>`
  - `file: FileForm`
- `Secrets`: 11 `String` fields corresponding to `SECRET_KEYS`.
- `EnvEntries = Vec<(String, String)>` and `EnvFiles`: `{ pub backend: EnvEntries, pub director: EnvEntries, pub content: EnvEntries, pub gateway: EnvEntries, pub admin: EnvEntries, pub allowlist: Option<EnvEntries> }`.

#### Validation Rules (`validate`, `HostConfig::from_json`, `HostConfig::from_toml`, `HostConfig::from_yaml`)

1. **`mode`:** Must be `"private"` or `"public"`; otherwise `mode must be "private" or "public"`.
2. **`edition`:** Optional; defaults to `"reborn"`. Must be `"reborn"`, `"legacy"`, or `"both"`.
3. **`host`:**
   - In `private` mode: must be a Tailscale CGNAT IPv4 (`100.64.0.0/10`, i.e., `100.(64..=127).x.y`) or a `.ts.net` DNS name (`is_tailscale_host`). Otherwise: `private mode requires a Tailscale IPv4 address (100.64.0.0/10) or .ts.net name for host (got ...)`.
   - In `public` mode: must be a public IPv4 (`is_public_ipv4`: not `0.*`, `10.*`, `127.*`, `169.254.*`, `172.16..=31.*`, `192.168.*`, `100.64..=127.*`, `>= 224.*`) or a DNS name with at least one dot (`is_dns_name`, not ending in `.local` or `.ts.net`). Otherwise: `public mode requires a public IPv4 address or DNS name for host (got ...)`.
4. **`gameAddress` / `game_address`:**
   - In `private` mode: must satisfy `is_tailscale_ipv4`. Otherwise: `private mode requires a Tailscale IPv4 address (100.64.0.0/10) for gameAddress (got ...)`.
   - In `public` mode: must satisfy `is_public_ipv4`. Otherwise: `public mode requires a public IPv4 address for gameAddress (the game server announces an IP, not a DNS name; got ...)`.
5. **`name`:** Control characters (`<= 0x1f` or `0x7f`) stripped and whitespace trimmed; must be `1..=80` characters. Otherwise: `name must be 1-80 characters`.
6. **`registration`:** Uppercased; must be `"INVITECODE"`, `"OPEN"`, or `"NONE"`.
7. **`shareUrl` / `share_url`:** Optional (`null` or omitted). When present, must match `https://login.tailscale.com/admin/invite/<code_of_A-Za-z0-9_->`. Otherwise: `shareUrl must be a https://login.tailscale.com/admin/invite/... link`.
8. **`mongo`:** Object with `uri` starting with `mongodb://` or `mongodb+srv://`, `manage` boolean, `mongod` optional non-empty string.
9. **`maxHuntServers` / `max_hunt_servers`:** Integer `1..=10`. Otherwise: `maxHuntServers must be 1 to 10`.
10. **`multipliers`:** Optional. When present, `xp` and `rewards` must be finite numbers in `1.0..=100.0`. If `mode != Private` and `(xp != 1.0 || rewards != 1.0)`, returns `XP and reward multipliers are for private (Tailscale) test servers only`. If both are `1.0`, normalized to `None`.
11. **`publicPort` / `public_port`:** Optional. If `mode != Public`, returns `publicPort is for public servers only`. Must be an integer TCP port `1..=65535` and not in `RESERVED_PORTS` (`61000, 61001, 61002, 61003, 61005, 61010, 61011, 61012, 61015`). If `443`, normalized to `None`.
12. **`legacyGameDir` / `legacy_game_dir` and `legacyPublicPort` / `legacy_public_port`:** Only allowed when `edition == Both` (`legacyGameDir is for edition = "both" (a legacy-only server uses gameDir)`). `legacyPublicPort` requires `mode == Public`, integer `1..=65535`, not in `RESERVED_PORTS`, and not equal to `gateway_port(config)`. If `4443`, normalized to `None`.
13. **`linux.firewallInterface` / `linux.firewall_interface`:** When present, must match `/^[A-Za-z0-9_.:-]{1,15}$/` and not `"."` or `".."`.
14. **TOML / YAML Error Key Rewriting:** When loading from `host.toml`, `host.yaml`, or `host.yml`, validation error messages have camelCase field names rewritten to `snake_case` (`key_in_toml`) and are prefixed with `<filename>: `. Unknown keys in TOML/YAML check whether converting camelCase -> snake_case matches a known key and append `(did you mean <snake_key>?)`.

#### Config File Resolution (`host_config_file`, `load_host_config`, `save_host_config`)
- Precedence order in `.workspace/config/`: `host.toml`, `host.yaml`, `host.yml`, `host.json`.
- `save_host_config(paths, config)` always writes `.workspace/config/host.toml`. If `.workspace/config/host.json` exists, renames it to `.workspace/config/host.json.bak` and returns `Ok(true)`.

#### Generated `.env` Files (`build_env_files`)
Given `(config, secrets, paths, source_commit, platform)`:
- **`backend.env`:**
  - `NODE_ENV=production`, `PORT=61000`, `BIND_HOST=127.0.0.1`
  - `PUBLIC_HOST=<config.host>`, `PUBLIC_PORT=<gateway_port(config)>`
  - `TLS_ENABLED=<"true" if Public else "false">`
  - `DEPLOYMENT_MODE=<"public" | "private">`
  - `SERVER_NAME=<config.name>`
  - `REGISTRATION_MODE=<config.registration>`
  - `MONGODB_URI=<config.mongo.uri>`
  - `DIRECTOR_HOST=127.0.0.1`, `DIRECTOR_PORT=61001`
  - `CONTENT_SERVER_URL=<Public: https://host[:port]/content/v1/manifest/ ; Private: http://host:61000/content/v1/manifest/>`
  - `REFORGED_GATEWAY_SECRET=<secrets.gateway_secret>`
  - `REFORGED_ADMIN_KEY=<secrets.admin_key>`
  - `REFORGED_GAMESERVER_KEY=<secrets.gameserver_key>`
  - `API_KEY_HASH_SECRET=<secrets.api_key_hash_secret>`
  - `METRICS_TOKEN=<secrets.metrics_token>`
  - `JWT_PRIVATE_KEY_B64=<secrets.jwt_private_key_b64>`
  - `JWT_PUBLIC_KEY_B64=<secrets.jwt_public_key_b64>`
  - `REFORGED_SOURCE_COMMIT=<source_commit>`
  - `REFORGED_RUNTIME_DIR=<paths.runtime_publish>`
  - `REFORGED_XP_MULTIPLIER=<xp if multipliers_on else "1">`
  - `REFORGED_REWARD_MULTIPLIER=<rewards if multipliers_on else "1">`
  - If `[switches]` present: `REFORGED_SWITCH_LANTERN_TAP`, `REFORGED_SWITCH_GLIDER_CHAIN_FLINGS`, `REFORGED_SWITCH_CELL_SWAP_IN_LOADOUT`, `REFORGED_SWITCH_PANGAR_LANTERN_ASPECT`, `REFORGED_SWITCH_FOUNDERS_FLARE` (`"1"` or `"0"`).
- **`director.env`:**
  - `NODE_ENV=production`, `DIRECTOR_BIND_HOST=127.0.0.1`, `DIRECTOR_PORT=61001`
  - `SERVER_IP=<config.game_address>`
  - `GAMESERVER_BINARY_PATH=<Linux: paths.gameserver_wrapper ; Windows with game_dir: <game_dir>/Archon/Binaries/Win64/DauntlessServer-Win64-Shipping.exe ; Windows without game_dir: "">`
  - If `platform == Linux` and `config.game_dir` is `Some(dir)`:
    - `REFORGED_GAME_EXE=<dir>/Archon/Binaries/Win64/DauntlessServer-Win64-Shipping.exe`
    - `REFORGED_WINE_PREFIX=<paths.wine_prefix>`
    - `REFORGED_XVFB=<"1" if linux.xvfb else "0">`
    - `REFORGED_WINE=<linux.wine>` (if `Some`)
    - `REFORGED_PROTON=<linux.proton>` and `REFORGED_PROTON_DATA=<paths.wine_prefix>` (if `linux.proton` is `Some`)
  - `BACKEND_HOST=127.0.0.1`, `BACKEND_PORT=61000`
  - `REFORGED_GAMESERVER_KEY=<secrets.gameserver_key>`
  - `REFORGED_MAX_HUNT_SERVERS=<config.max_hunt_servers>`
  - `REFORGED_SERVER_MAX_FPS=<perf.server_fps>`, `REFORGED_SERVER_IDLE_FPS=<perf.idle_fps>`
  - `REFORGED_TRAINING_ON_DEMAND=<"1"|"0">`, `REFORGED_TRAINING_IDLE_MINUTES=<perf.training_idle_minutes>`
  - `REFORGED_GAMESERVER_LOG_DIR=<paths.gameserver_logs>`
  - `REFORGED_GAMESERVER_LOG_CMDS=<"" if verbose_game_logs else QUIET_GAME_LOG_CMDS>`
  - `REFORGED_MEMORY_GUARD=<"1"|"0">`, `REFORGED_MIN_FREE_MEMORY_MB=<perf.min_free_memory_mb>`, `REFORGED_HUNT_MEMORY_MB=<perf.hunt_memory_mb>`
- **`content.env`:**
  - `PORT=61002`, `BIND_HOST=127.0.0.1`, `SERVER_NAME=<config.name>`, `DEPLOYMENT_MODE=<mode>`, `REGISTRATION_MODE=<registration>`, `SOURCE_COMMIT=<source_commit>`, `RUNTIME_PUBLISH_DIR=<paths.runtime_publish>`, `NEWS_FILE=<paths.news_file>`, `BANNER_IMAGE=<paths.banner_image>`
- **`gateway.env`:**
  - `GATEWAY_MODE=<mode>`
  - `GATEWAY_BIND_HOST=<"0.0.0.0" if Public else config.game_address>`
  - `GATEWAY_PORT=<gateway_port(config) if Public else 61000>`
  - `GATEWAY_TLS=<"1" if Public else "0">`
  - `GATEWAY_CERT_FILE=<paths.cert_file if Public else "">`
  - `GATEWAY_KEY_FILE=<paths.key_file if Public else "">`
  - `GATEWAY_SECRET=<secrets.gateway_secret>`
  - `GATEWAY_BACKEND_URL=http://127.0.0.1:61000`
  - `GATEWAY_CONTENT_URL=http://127.0.0.1:61002`
  - `GATEWAY_ADMIN_URL=http://127.0.0.1:61003`
  - `ALLOWLIST_URL=<"http://127.0.0.1:61005" if Public else "">`
  - `ALLOWLIST_SECRET=<secrets.allowlist_secret if Public else "">`
  - Plus optional `[guard]` (`GATEWAY_TRUST_PROXY`, `GATEWAY_TRUSTED_PROXIES`, `GATEWAY_RATE_LIMIT_*`, `GATEWAY_BAN_FILE`) and `[network]` (`GATEWAY_NODE_ID`, `GATEWAY_REGION`, `GATEWAY_NODE_CERT`, `GATEWAY_NODE_KEY`, `GATEWAY_NODE_CA`, `GATEWAY_SESSION_KEY_FILE`, `GATEWAY_NETWORK_NODES`) variables.
- **`admin.env`:**
  - `ADMIN_BIND_HOST=127.0.0.1`, `ADMIN_PORT=61003`, `ADMIN_API_KEY=<secrets.admin_api_key>`, `OPERATOR_SECRET=<secrets.operator_secret>`, `GATEWAY_SECRET=<secrets.gateway_secret>`, `MONGODB_URI=<config.mongo.uri>`, `DIRECTOR_URL=http://127.0.0.1:61001`, `DIRECTOR_ADMIN_KEY=<secrets.gameserver_key>`, `NEWS_FILE=<paths.news_file>`, `BANNER_IMAGE=<paths.banner_image>`, `SERVER_NAME=<config.name>`, `DEPLOYMENT_MODE=<mode>`, plus `DATABASE_BACKEND` (`sqlite`/`postgres`), `DATABASE_PATH`/`DATABASE_URL`, and `HISTORY_RETENTION_DAYS`.
- **`allowlist.env` (`Some` only when `mode == Public`):**
  - `ALLOWLIST_BIND_HOST=127.0.0.1`, `ALLOWLIST_PORT=61005`, `ALLOWLIST_SECRET=<secrets.allowlist_secret>`, `ALLOWLIST_PORTS=<first>-<last>`, `ALLOWLIST_STATE_FILE=<paths.config/allowlist-state.json>`, and on Linux with `firewall_interface`: `ALLOWLIST_INTERFACE=<iface>`.

---

### 2.4 Module 4: `run`

#### Responsibility
Runs short-lived external commands (`npm`, `git`, `netsh`, `tailscale`, `ss`, `gh`, etc.) either streaming output to a log file (`run`) or capturing stdout (`capture`) with a timeout.

#### Public Types & Functions

```rust
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    pub cwd: PathBuf,
    pub env: Option<BTreeMap<String, String>>,
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
```

#### Behavioral Contract & Invariants
1. **Windows `.cmd` Resolution:** On Windows, bare command names `"npm"` and `"npx"` are invoked as `"npm.cmd"` and `"npx.cmd"`.
2. **Error Line Detection (`is_error_line`):** Returns `true` if `line` matches case-insensitively `npm err!`, `npm error`, `error ts`, or `: error `, or starts with `error:` / `fatal:`.
3. **Output Cap & Last Line (`run`):**
   - Appends a header `\n$ <command> <args...>\n` to `options.log_file` if specified.
   - Merges stdout and stderr line-by-line.
   - Updates `last_line` with the latest non-empty trimmed line.
   - Retains at most `64 * 1024` bytes (64 KiB) in `RunResult.output` (dropping leading bytes when exceeded).
   - When `options.quiet` is `false`, prints all lines; when `options.quiet` is `true`, prints only lines where `is_error_line(line)` is `true`.
4. **Timeout (`capture`):** Runs with a 20-second deadline, returning `Ok(Some(trimmed_stdout))` when exit code is `0` and `Ok(None)` when the command is not found, exits non-zero, or times out.

---

### 2.5 Module 5 & 6: `services` and `services/tree`

#### Responsibility
Supervises child processes (`mongod`, `backend`, `director`, and unported front-door services), isolates their environment variables, probes HTTP/TCP readiness, captures logs, restarts unexpectedly exited children with exponential backoff, and guarantees full process-tree termination (including grandchildren such as `DauntlessServer-Win64-Shipping.exe` spawned by the Director).

#### Public Types & Functions

```rust
pub const STOP_GRACE: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ready {
    Http { host: String, port: u16, path: String },
    Tcp { host: String, port: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceSpec {
    pub name: String,
    pub cwd: PathBuf,
    pub command: PathBuf,
    pub args: Vec<String>,
    pub env_file: Option<PathBuf>,
    pub extra_env: Vec<(String, String)>,
    pub ready: Option<Ready>,
}

pub type Printer = Arc<dyn Fn(&str, &str) + Send + Sync>;

pub fn service_environment<I, F, E, K1, V1, K2, V2, K3, V3>(
    base: I,
    from_file: F,
    extra: E,
) -> Vec<(String, String)>;

pub fn answers(ready: &Ready, timeout: Duration) -> bool;
pub fn wait_until_ready(ready: &Ready, timeout: Duration, cancelled: &dyn Fn() -> bool) -> bool;

pub struct Supervisor { /* ... */ }
impl Supervisor {
    pub fn new(log_dir: impl Into<PathBuf>, print: Printer) -> Self;
    pub fn with_cancel(self, cancelled: Arc<AtomicBool>) -> Self;
    pub fn log_dir(&self) -> &Path;
    pub fn start(&self, spec: ServiceSpec, ready_timeout: Duration) -> Result<bool, HostError>;
    pub fn stop_all(&self);
    pub fn all_stopped(&self) -> bool;
}
```

#### Behavioral Contract & Invariants
1. **Environment Isolation (`service_environment`):**
   - Filters out any ambient variable from `base` whose name starts with `REFORGED_`, `GATEWAY_`, `ALLOWLIST_`, `DIRECTOR_`, or `CONTENT_`, or whose exact name is in:
     `PORT`, `BIND_HOST`, `PUBLIC_HOST`, `PUBLIC_PORT`, `TLS_ENABLED`, `DEPLOYMENT_MODE`, `SERVER_NAME`, `REGISTRATION_MODE`, `MONGODB_URI`, `BACKEND_HOST`, `BACKEND_PORT`, `SERVER_IP`, `GAMESERVER_BINARY_PATH`, `API_KEY_HASH_SECRET`, `METRICS_TOKEN`, `JWT_PRIVATE_KEY_B64`, `JWT_PUBLIC_KEY_B64`, `SOURCE_COMMIT`, `RUNTIME_PUBLISH_DIR`, `NEWS_FILE`, `BANNER_IMAGE`, `ADMIN_BIND_HOST`, `ADMIN_PORT`, `ADMIN_API_KEY`, `OPERATOR_SECRET`, `DIRECTOR_URL`, `DIRECTOR_ADMIN_KEY`, `DATABASE_BACKEND`, `DATABASE_PATH`, `DATABASE_URL`, `HISTORY_RETENTION_DAYS`.
   - Overlays `from_file` (matching keys case-insensitively on Windows), then overlays `extra`.
2. **Readiness Probing (`answers`, `wait_until_ready`):**
   - `Ready::Tcp { host, port }`: TCP connect succeeds within timeout.
   - `Ready::Http { host, port, path }`: TCP connect to `host:port`, sends `GET <path> HTTP/1.1\r\nHost: <host>:<port>\r\nConnection: close\r\n\r\n`, and receives a response starting with `HTTP/1.` whose status code is `< 500`.
   - `wait_until_ready` polls every `250 ms` until `timeout` elapses or `cancelled()` returns `true` (or the child process exits early).
3. **Crash-Loop Restart & Backoff (`Supervisor`):**
   - If a started child exits unexpectedly while `stopping == false`:
     - If it ran for `>= 30 s`, its `restarts` counter resets to `0`.
     - Increments `restarts`. If `restarts > 5`, prints `<name> | stopped too many times; not restarting` and stops restarting.
     - Otherwise waits `min(2000 << (restarts - 1), 30000)` ms (2s, 4s, 8s, 16s, 30s) while polling `stopping`, then respawns.
4. **Reverse-Order Stop & Process Tree (`Supervisor::stop_all`, `services/tree`):**
   - Stops services in **reverse start order** (gateway first, database last).
   - **Windows (`ProcessTree`):** Creates an anonymous Win32 Job Object configured with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK` and assigns the child process to it. On stop, sends graceful termination / waits up to `STOP_GRACE` (`8 s`), then calls `TerminateJobObject` (killing the service and any child game servers it spawned) and closes the job handle.
   - **Unix (`ProcessTree`):** Spawns the child in its own process group (`process_group(0)`) and on Linux sets `PR_SET_PDEATHSIG(SIGTERM)` in `pre_exec`. On stop, sends `SIGTERM` to `-pgid`, waits up to `STOP_GRACE` (`8 s`), then sends `SIGKILL` to `-pgid`.

---

### 2.6 Module 7: `invite`

#### Responsibility
Formats `dauntless://invite?...` links for the launcher, computes and probes gateway X.509 certificate SHA-256 fingerprints, and calls the backend's local admin endpoints (`/reforged/api/CreateInvite` and `/reforged/api/InviteCodes`).

#### Public Types & Functions

```rust
#[derive(Debug, Clone)]
pub struct InviteParts {
    pub mode: ServerMode,
    pub host: String,
    pub port: u16,
    pub code: String,
    pub name: String,
    pub fp: Option<String>,
    pub share: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CodeOptions {
    pub uses: f64,
    pub infinite: bool,
    pub note: Option<String>,
}

pub fn format_invite_parts(parts: &InviteParts) -> Result<String, HostError>;
pub fn format_game_invite(parts: &InviteParts, game: &GameVersion) -> Result<String, HostError>;
pub fn invite_for(config: &HostConfig, code: &str, cert_fingerprint: Option<&str>) -> Result<String, HostError>;
pub fn invite_for_game(
    config: &HostConfig,
    code: &str,
    cert_fingerprint: Option<&str>,
    game: &GameVersion,
) -> Result<String, HostError>;
pub fn certificate_fingerprint(cert_file: &Path) -> Result<String, HostError>;
pub fn served_fingerprint(host: &str, port: u16, timeout_ms: u64) -> Option<String>;
pub fn create_invite_code(admin_key: &str, options: &CodeOptions, backend_port: u16) -> Result<String, HostError>;
pub fn list_invite_codes(admin_key: &str, backend_port: u16) -> Result<Value, HostError>;
```

#### Behavioral Contract & Invariants
1. **Invite Link Formatting (`format_invite_parts`, `format_game_invite`, `invite_for`, `invite_for_game`):**
   - Delegates to `protocol::invite::format_invite` after validating:
     - Public mode requires `fp` (`Some` 64-char hex SHA-256); if missing, returns `HostError::message("public invites require the gateway certificate SHA-256 fingerprint")`.
     - Private mode sets `fp = None` and passes `config.share_url` as `share`.
     - Port: for Reborn (`1.14.7`), `gateway_port(config)` in public mode or `PORTS.private_gateway` (`61000`) in private mode. For Legacy (`1.4.4`) when `config.edition == Both`, uses `legacy_gateway_port(config)` (`4443` default) in public mode or `BOTH_LEGACY_PORTS.private_gateway` (`61010`) in private mode.
     - `game`: omitted from query string when `1.14.7` (default), included as `game=1.4.4` for Legacy.
2. **Certificate Fingerprint (`certificate_fingerprint`, `served_fingerprint`):**
   - `certificate_fingerprint(cert_file)` parses the first `-----BEGIN CERTIFICATE-----...-----END CERTIFICATE-----` PEM block in `cert_file`, base64-decodes the DER bytes, and returns the lowercase 64-character hex SHA-256 digest.
   - `served_fingerprint(host, port, timeout_ms)` opens a TLS connection (using `rustls` with a custom certificate verifier that accepts any server cert and captures the leaf DER certificate) to `127.0.0.1:port` (with SNI `host` if DNS or `127.0.0.1`), returning the lowercase 64-character hex SHA-256 of the presented leaf certificate, or `None` if connection/handshake fails.
3. **Backend Admin API (`create_invite_code`, `list_invite_codes`):**
   - Connects to `http://127.0.0.1:<backend_port>` with header `x-reforged-admin-key: <admin_key>`.
   - `create_invite_code`: `POST /reforged/api/CreateInvite` with JSON body `{"uses": <1 if infinite else uses>, "infinite": <bool>, "name": <note>}` (omitting `"name"` when `note` is `None`).
     - Connection failure -> `"the backend is not running on port <port>: start the server first (reforged-host start)"`.
     - HTTP 401/403 -> `"the backend refused the admin key (was secrets.env changed since the server started?): restart the server"`.
     - Extracts `"code"` string from 200 JSON response.
   - `list_invite_codes`: `GET /reforged/api/InviteCodes`, returns parsed JSON value.

---

### 2.7 Module 8: `doctor`

#### Responsibility
Runs comprehensive diagnostic checks across tools, configuration, Mystic Paradox sources/builds, game data, published runtime, game installation, performance/RAM, Linux/Wine/systemd/firewall, network/TLS certificates, database replica set, and running services.

#### Public Types & Functions

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level { Ok, Warn, Fail, Info }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub area: &'static str,
    pub level: Level,
    pub text: String,
    pub fix: Option<String>,
}

#[derive(Debug, Clone)]
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
    pub label: &'a str,
    pub version_text: Option<&'a str>,
    pub exe_sha256: &'a str,
    pub setup_flag: &'a str,
}

pub const REBORN_BUILD: GameBuild<'static> = GameBuild {
    label: "1.14.7",
    version_text: Some(GAME_VERSION_TEXT),
    exe_sha256: PINNED_EXE_SHA256,
    setup_flag: "--game",
};
pub const SERVICES_MEMORY_MB: u32 = 1536;

pub fn tailscale_ipv4() -> Result<Option<String>, HostError>;
pub fn port_in_use(port: u16, host: Option<&str>) -> Result<bool, HostError>;
pub fn port_owner_hint(port: u16) -> Result<String, HostError>;
pub fn wait_for_http(port: u16, path: &str, timeout_ms: u64) -> bool;
pub fn sha256_file(file: &Path) -> Result<String, HostError>;
pub fn node_version_ok(version: &str) -> bool;
pub fn count_function_files(sdk_dir: &Path) -> usize;
pub fn game_checks(game_dir: Option<&str>, hash_exe: bool) -> Result<Vec<Check>, HostError>;
pub fn game_exe_checks(game_dir: Option<&str>, hash_exe: bool, build: GameBuild<'_>) -> Result<Vec<Check>, HostError>;
pub fn runtime_cpu_patches(dll_path: &Path) -> Option<CpuPatches>;
pub fn hunts_that_fit(total_mb: u32, perf: &PerformanceOptions) -> u32;
pub fn total_memory_mb() -> Option<u32>;
pub fn performance_checks(
    config: &HostConfig,
    runtime_dll: Option<&Path>,
    total_mb: Option<u32>,
    platform: Platform,
) -> Vec<Check>;
pub fn linux_checks(paths: &Paths, config: &HostConfig) -> Result<Vec<Check>, HostError>;
pub fn network_checks(paths: &Paths, config: &HostConfig, gateway_up: bool) -> Result<Vec<Check>, HostError>;
pub fn database_check(mongo: &MongoCheck, uri: &str, managed: bool) -> Check;
pub fn run_doctor(paths: &Paths, options: &DoctorOptions) -> Result<Vec<Check>, HostError>;
pub fn print_checks(checks: &[Check], say: &mut dyn FnMut(&str)) -> (usize, usize);
```

#### Behavioral Contract & Invariants
1. **`node_version_ok(version)`:** Parses `v<major>.<minor>...`; returns `true` iff `major > 20 || (major == 20 && minor >= 19)`.
2. **`count_function_files(sdk_dir)`:** Counts files in `sdk_dir` ending with `_functions.cpp`. (`>= 100` is considered a complete Dumper-7 SDK dump).
3. **`runtime_cpu_patches(dll_path)`:** Reads `dll_path` bytes:
   - `cpu_fixes`: `true` if the UTF-16LE encoding of `"LEGACY_EXPECTED_PLAYER_WALK.flag"` occurs in the binary.
   - `frame_cap`: `true` if the ASCII bytes `b"REFORGED_SERVER_MAX_FPS"` occur in the binary.
   - If either is missing, `performance_checks` emits a `Level::Warn` check containing `"0019 (CPU fixes) and 0020 (frame cap)"`.
4. **`hunts_that_fit(total_mb, perf)`:**
   - Base overhead = `SERVICES_MEMORY_MB` (`1536`) + `(2 * perf.hunt_memory_mb)` (for the 2 hubs) + `perf.min_free_memory_mb`.
   - If `total_mb <= overhead` or `perf.hunt_memory_mb == 0`, returns `0`.
   - Else `((total_mb - overhead) / perf.hunt_memory_mb).min(MAX_HUNTS)`.
   - If `config.max_hunt_servers > fit`: emits `Level::Info` when `perf.memory_guard` is `true`, or `Level::Warn` with fix `"reforged-host setup --memory-guard, or --max-hunts <fit>"` when `perf.memory_guard` is `false`.
5. **`game_checks` / `game_exe_checks`:**
   - Area `"game"`.
   - If `game_dir` is `None`: `Level::Fail` (`"no game folder in host.toml"`).
   - Checks `<game_dir>/Archon/Binaries/Win64/DauntlessServer-Win64-Shipping.exe` exists.
   - Checks `<game_dir>/Version.txt` (stripping UTF-8 BOM and trimming whitespace) equals `GAME_VERSION_TEXT` (`dauntless_rel-1.14.7_Shipping_2024-04-01_14-59-21_647472`); if empty, reports `Version.txt says "nothing", not ...`.
   - When `hash_exe == true`: computes SHA-256 of the `.exe` and compares against `PINNED_EXE_SHA256`; if different, emits `Level::Fail` (`"the executable is another build (SHA-256 <hash>)"`).
6. **`print_checks(checks, say)`:**
   - Groups checks by `check.area` (printing `\n<area>` whenever `area` changes from the previous check).
   - Formats badges: `Level::Ok` -> `"  [ ok ] "`, `Level::Warn` -> `"  [warn] "`, `Level::Fail` -> `"  [FAIL] "`, `Level::Info` -> `"  [info] "`.
   - When `check.level != Level::Ok` and `check.fix` is `Some(fix)`, prints `"         -> {fix}"`.
   - Returns `(fail_count, warn_count)`.

---

### 2.8 Module 9 & 10: `mongo` and `mongo/wire`

#### Responsibility
Finds `mongod` on Windows (`PATH` or newest version under `%ProgramFiles%\MongoDB\Server\<ver>\bin\mongod.exe`) and Linux (`PATH`), builds `mongod` command-line arguments for a loopback 1-member replica set (`rs0`), and speaks the MongoDB `OP_MSG` binary wire protocol directly over TCP to inspect or initiate the replica set without requiring `mongosh` or external drivers.

#### Constants, Types & Functions

```rust
pub const REPLICA_SET: &str = "rs0";
pub const MONGO_CACHE_GB: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MongoProblem { Unreachable, Standalone, Error }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MongoCheck {
    Ok { set_name: String, primary: bool, initiated: bool },
    Failed { reason: MongoProblem, detail: String },
}

#[derive(Debug, Clone, Default)]
pub struct ReplicaSetOptions {
    pub initiate: bool,
    pub host: Option<String>,
    pub timeout_ms: Option<u64>,
}

pub fn mongod_args(data_dir: &Path, log_file: &Path, port: u16) -> Result<Vec<String>, HostError>;
pub fn find_mongod() -> Result<Option<String>, HostError>;
pub fn find_mongod_in(path: Option<&OsStr>, program_files: Option<&Path>) -> Option<String>;
pub fn direct_uri(uri: &str) -> String;
pub fn first_host(uri: &str) -> Result<(String, u16), HostError>;
pub fn ensure_replica_set(uri: &str, options: &ReplicaSetOptions) -> Result<MongoCheck, HostError>;
```

#### Behavioral Contract & Invariants
1. **`mongod_args(data_dir, log_file, port)`:**
   Returns exact arguments:
   `["--replSet", "rs0", "--bind_ip", "127.0.0.1", "--port", "<port>", "--dbpath", "<data_dir>", "--logpath", "<log_file>", "--logappend", "--wiredTigerCacheSizeGB", "1"]`.
2. **`direct_uri(uri)`:**
   Replaces `replicaSet=[^&]+` in the query string with `directConnection=true` (or appends `?directConnection=true` / `&directConnection=true` if no `replicaSet=` parameter is present).
3. **`first_host(uri)`:**
   - Rejects `mongodb+srv://` (`"mongodb+srv:// URIs are not supported for local replica-set initiation"`).
   - Requires `mongodb://` prefix, strips optional `user:pass@`, takes the first comma-separated host entry before `/` or `?`, supports `[ipv6]:port` and `host:port`, defaulting port to `27017`.
4. **Wire Protocol (`mongo/wire` & `ensure_replica_set`):**
   - `mongo/wire` implements minimal BSON serialization/deserialization (`Double`, `String`, `Document`, `Array`, `Bool`, `Null`, `Int32`, `Int64`) and `OP_MSG` framing (`op_code = 2013`, 16-byte header `[total_len, request_id, response_to, 2013]`, 4-byte flags `0`, section kind `0` followed by BSON document).
   - `ensure_replica_set` connects to `options.host` or `first_host(uri)`:
     - Sends `{"hello": 1, "$db": "admin"}`. If `isWritablePrimary == true` (or `ismaster == true`) and `setName` is present, returns `MongoCheck::Ok { set_name, primary: true, initiated: false }`.
     - Sends `{"replSetGetStatus": 1, "$db": "admin"}`:
       - If error code is `76` (`NoReplicationEnabled`) or `errmsg` contains `"--replSet"`, returns `MongoCheck::Failed { reason: MongoProblem::Standalone, detail: "mongod is running without --replSet rs0..." }`. Never attempts `replSetInitiate` on a standalone `mongod`.
       - If error code is `94` (`NotYetInitialized`) or `errmsg` contains `"no replset config"`:
         - If `!options.initiate`, returns `MongoCheck::Failed { reason: MongoProblem::Error, detail: "the replica set is not initiated yet" }`.
         - If `options.initiate`, sends `{"replSetInitiate": {"_id": "rs0", "members": [{"_id": 0, "host": "<host>:<port>"}]}, "$db": "admin"}`, then polls `hello` every `250 ms` up to `timeout_ms` until `isWritablePrimary == true`, returning `MongoCheck::Ok { set_name: "rs0", primary: true, initiated: true }`.

---

### 2.9 Module 11: `gamedata`

#### Responsibility
Tracks the state (`Missing`, `Synthetic`, `Real`) of the 7 backend and 7 Director game-data JSON files, copies `.example.json` files when requested, and orchestrates the 8 game-data generation steps.

#### Constants, Types & Functions

```rust
pub const BACKEND_GAME_DATA: [&str; 7] = [
    "equipment_catalog.json",
    "items.json",
    "ladyluck_store.json",
    "platinum_store.json",
    "progression.json",
    "quests.json",
    "store.json",
];

pub const DIRECTOR_GAME_DATA: [&str; 7] = [
    "behemoths.json",
    "climb_nodes.json",
    "esca_amps.json",
    "fauna.json",
    "gatherables.json",
    "hunts.json",
    "trials_configs.json",
];

pub const SUPPLIED_SOURCES: [(&str, &str); 2] = [
    ("ladyluck_store.source.json", "ladyluck_store.json"),
    ("platinum_store.source.json", "platinum_store.json"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameDataKind { Missing, Synthetic, Real }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataState {
    pub state: GameDataKind,
    pub present: Vec<String>,
    pub missing: Vec<String>,
    pub synthetic: Vec<String>,
}

pub fn game_data_state(dir: &Path, files: &[&str]) -> Result<GameDataState, HostError>;
pub fn use_examples(dir: &Path, files: &[&str]) -> Result<Vec<String>, HostError>;
pub fn game_data_steps(paths: &Paths, examples_for_missing: bool) -> Vec<Step>;
pub fn generate_game_data(
    paths: &Paths,
    options: &GenerateOptions,
    out: &mut dyn FnMut(&str),
) -> Result<Generated, HostError>;
pub fn generate_game_data_with(
    paths: &Paths,
    options: &GenerateOptions,
    generators: &dyn Generators,
    out: &mut dyn FnMut(&str),
) -> Result<Generated, HostError>;
```

#### Behavioral Contract & Invariants
1. **`game_data_state(dir, files)`:**
   - For each `file` in `files`:
     - If `<dir>/<file>` does not exist -> pushed to `missing`.
     - Else -> pushed to `present`. Reads `<dir>/<file>` and compares (after stripping `\r`) to `<dir>/<stem>.example.json` (if it exists), or checks whether the parsed JSON top-level object has `"_synthetic": true` or `"synthetic": true`. If either matches -> pushed to `synthetic`.
   - `state`: `Missing` if `!missing.is_empty()`, else `Synthetic` if `!synthetic.is_empty()`, else `Real`.
2. **`use_examples(dir, files)`:**
   - For each `file` in `files` where `<dir>/<file>` does not exist and `<dir>/<stem>.example.json` exists, copies the example file to `<dir>/<file>` and returns the list of copied filenames.
3. **Generation Pipeline (`generate_game_data_with`):**
   - Step 0a: Ensures pinned store captures (`captures::ensure_captures`) if any capture files are missing.
   - Step 0b: Builds `progression.source.json` from the player's progression export (`progression::write_progression_source`) if the progression export is present.
   - Step 0c: For each `(source, target)` in `SUPPLIED_SOURCES`, if `<raw_game_data>/<source>` is missing and `<backend_game_data>/<target>` exists and is not identical to `<stem>.example.json`, copies `<target>` to `<raw_game_data>/<source>`.
   - Runs the 8 `game_data_steps` (filtered by `options.only` if `Some`):
     1. `"Progression config"` (`backend`, `npm run progression:generate`)
     2. `"Items & equipment catalog"` (`backend`, `npm run catalog:generate`)
     3. `"Quests"` (`backend`, `npm run quests:generate`)
     4. `"Store"` (`backend`, `npm run store:generate`)
     5. `"Platinum store"` (`backend`, `npm run store:enrich-platinum` if `platinum_store.source.json` exists, else `npm run platinum:generate`)
     6. `"Lady Luck store"` (`backend`, `npm run ladyluck:enrich` if `ladyluck_store.source.json` exists, else `npm run ladyluck:generate`)
     7. `"Hunts & behemoths"` (`director`, `npm run gamedata:generate`)
     8. `"Escalations, trials & island objects"` (`director`, `npm run worlddata:generate`)

---

### 2.10 Module 12: `gamepack`

#### Responsibility
Exports and imports portable gzipped JSON game-data packs (`dauntless-reforged-revival-gamedata`, version 1) so a host can generate game data on a Windows PC and import it onto a Linux server.

#### Constants, Types & Functions

```rust
pub const PACK_FORMAT: &str = "dauntless-reforged-revival-gamedata";
pub const PACK_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackFile {
    pub data: Value,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pack {
    pub format: String,
    pub version: u32,
    pub game: String,
    pub changelist: u32,
    pub created: String,
    pub files: BTreeMap<String, PackFile>,
}

pub fn export_game_data_pack(paths: &Paths, out_file: &Path, now: DateTime<Utc>) -> Result<Exported, HostError>;
pub fn read_game_data_pack(file: &Path) -> Result<Pack, HostError>;
pub fn parse_game_data_pack(raw: &[u8]) -> Result<Pack, HostError>;
pub fn import_game_data_pack(paths: &Paths, pack_file: &Path, now: DateTime<Utc>) -> Result<Imported, HostError>;
```

#### Behavioral Contract & Invariants
1. **Export (`export_game_data_pack`):**
   - Collects all 7 `backend/<file>` and 7 `director/<file>` entries that exist and are **not** synthetic/example data.
   - If none are real, returns `Err("no generated game data to export (only the example files, or nothing at all): run reforged-host gamedata first")`.
   - For each included file, normalizes CRLF to LF (`read_text`), computes `sha256_hex`, parses JSON `data`, and writes a gzip-compressed pretty-printed JSON `Pack` (`format: PACK_FORMAT`, `version: 1`, `game: "1.14.7"`, `changelist: 647472`, `created: iso_string(now)`).
   - Default filename (`default_pack_file`): `<workspace>/gamedata-1.14.7-<YYYY-MM-DD>.json.gz`.
2. **Import (`parse_game_data_pack`, `import_game_data_pack`):**
   - Accepts either gzip-compressed (`0x1f, 0x8b` magic) or plain UTF-8 JSON.
   - Validates `format == PACK_FORMAT`, `version == 1`, non-empty `files` object, every key in the allowed 14 `backend/...` or `director/...` paths, and every `sha256` matching `stringify_pretty(&entry.data) + "\n"`.
   - For each file in the pack:
     - Renders `stringify_pretty(&entry.data) + "\n"`.
     - If target file already has the exact same normalized content -> adds to `unchanged`.
     - If target file exists, differs, and is not an `.example.json` copy -> backs up existing file to `<file>.bak.<YYYYMMDDTHHMMSSZ>` and records in `backups`.
     - Writes new file and records in `written`.

---

### 2.11 Module 13: `captures`

#### Responsibility
Reads `captures.pin.json` and fetches pinned store-capture JSON files from a git repository at an exact 40-character commit SHA into `DauntlessEndpointDocumentation`, verifying every file's SHA-256 before writing.

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

pub fn read_capture_pin(pin_file: &Path) -> Result<CapturePin, HostError>;
pub fn parse_capture_pin(text: &str, label: &str) -> Result<CapturePin, HostError>;
pub fn missing_captures(paths: &Paths, pin: &CapturePin) -> Vec<String>;
pub fn ensure_captures(paths: &Paths, pin: &CapturePin, log_file: Option<&Path>) -> Result<CaptureFetch, HostError>;
```

#### Behavioral Contract & Invariants
1. **Pin Validation (`parse_capture_pin`):**
   - `repository`: must be a non-empty `https://` URL or an existing local directory path.
   - `commit`: must be a 40-character hex SHA-1 string (`/^[0-9a-fA-F]{40}$/`).
   - `files`: non-empty map of `relative_path -> 64_char_lowercase_hex_sha256`.
   - Path safety: rejects any path that is empty, starts with `/`, contains `\`, `:`, `..`, or `.` segments (`"plain paths"` error).
2. **Fetch & Verification (`ensure_captures`):**
   - `missing_captures` returns any file in `pin.files` that is missing under `paths.store_captures` or whose SHA-256 does not equal the pinned digest.
   - If `missing_captures` is empty, returns `Ok(CaptureFetch::AlreadyReady)`.
   - Otherwise initializes a bare git repository in `<workspace>/captures/<sanitized_name>.git`, runs `git fetch --depth 1 <repository> <commit>` (if `<commit>^{commit}` is not already present), extracts each file with `git show <commit>:<path>`, verifies its SHA-256 against the pin **before** writing any file to `paths.store_captures`, and returns `Ok(CaptureFetch::Fetched)`.

---

### 2.12 Module 14: `preset`

#### Responsibility
Creates, inspects, and applies the repository-level `preset/` directory (`preset/preset.json` + game-data files) so servers set up from a checkout with a preset start with working game data.

#### Constants, Types & Functions

```rust
pub const PRESET_FORMAT: &str = "dauntless-reforged-revival-preset";
pub const PRESET_VERSION: u32 = 1;
pub const PRESET_MANIFEST: &str = "preset.json";
pub const PRESET_EXCLUDED: [&str; 2] = ["backend/ladyluck_store.json", "backend/platinum_store.json"];

pub fn read_preset_manifest(paths: &Paths) -> Result<Option<PresetManifest>, HostError>;
pub fn create_preset(paths: &Paths, options: &CreateOptions, now: DateTime<Utc>) -> Result<PresetCreated, HostError>;
pub fn apply_preset(paths: &Paths, force: bool) -> Result<Option<PresetApplied>, HostError>;
pub fn apply_preset_at(paths: &Paths, force: bool, now: DateTime<Utc>) -> Result<Option<PresetApplied>, HostError>;
```

#### Behavioral Contract & Invariants
1. **Exclusions:** `backend/ladyluck_store.json` and `backend/platinum_store.json` (`PRESET_EXCLUDED`) are **never** included in a preset (they are built from the pinned store captures on each server).
2. **`create_preset`:**
   - Collects all real (non-missing, non-example) game-data files except `PRESET_EXCLUDED`.
   - If none exist, returns `Err("no generated game data to make a preset from: run reforged-host gamedata first")`.
   - Records `executable_sha256` from `options.executable_sha256` or by hashing `options.executable` if the file exists.
   - Writes each file into `preset/backend/<name>` or `preset/director/<name>`, removes any stale files not in the new manifest, and writes `preset/preset.json`.
3. **`apply_preset` / `apply_preset_at`:**
   - If `preset/preset.json` does not exist, returns `Ok(None)`.
   - Verifies every file listed in `preset.json` exists under `preset/` and matches its recorded `sha256` and `bytes` before writing anything.
   - For each file:
     - If destination exists, differs from the preset, and is **not** example/synthetic data:
       - When `force == false`: leaves the host's own file untouched and adds its path to `kept`.
       - When `force == true`: backs up the host's file to `<file>.bak.<YYYYMMDDTHHMMSSZ>` (`backups`) and overwrites it (`written`).
     - If destination is missing or is still example/synthetic data: writes the preset file (`written`).
     - If destination already matches the preset: adds to `unchanged`.

---

### 2.13 Module 15: `progression`

#### Responsibility
Reads the 5 JSONL files exported from Dauntless 1.14.7 under `Items_Analysis/progression_1_14_7/` (`slayers_path.jsonl`, `escalation_progress.jsonl`, `weapon_mastery.jsonl`, `behemoth_mastery.jsonl`, `huntpass-season-table.jsonl`) and builds `progression.source.json` for the backend's progression generator.

#### Constants, Types & Functions

```rust
pub const PROGRESSION_EXPORT_DIR: &str = "progression_1_14_7";
pub const TRACK_EXPORTS: [&str; 4] = [
    "slayers_path.jsonl",
    "escalation_progress.jsonl",
    "weapon_mastery.jsonl",
    "behemoth_mastery.jsonl",
];
pub const HUNT_PASS_EXPORT: &str = "huntpass-season-table.jsonl";
pub const ACTIVE_HUNT_PASS: &str = "season43";
pub const GENERATED_BY: &str = "reforged-host gamedata (from progression_1_14_7/*.jsonl)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceWritten {
    Written { file: PathBuf, tracks: usize, hunt_pass_levels: usize },
    Skipped { reason: String },
}

pub fn read_jsonl(file: &Path) -> Result<Vec<Value>, HostError>;
pub fn parse_jsonl(text: &str, label: &str) -> Result<Vec<Value>, HostError>;
pub fn build_progression_config(export_dir: &Path) -> Result<ProgressionBuild, HostError>;
pub fn build_from_rows(track_sets: &[Vec<Value>], hunt_passes: &[Value]) -> Result<ProgressionBuild, HostError>;
pub fn progression_text(config: &Value) -> String;
pub fn write_progression_source(paths: &Paths) -> Result<SourceWritten, HostError>;
```

#### Behavioral Contract & Invariants
1. **JSONL Parsing (`parse_jsonl`):** Strips UTF-8 BOM, ignores blank lines and `"_meta"` header objects, and returns all row objects. Errors if no data rows exist.
2. **Track & Grant Transformation (`build_from_rows`):**
   - Selects the hunt pass row where `row["name"] == "season43"` (`ACTIVE_HUNT_PASS`), falling back to the last row.
   - Builds `huntPass` object (`seasonId`, `maxLevel`, `xpPerLevel`, `premiumCost`, `eliteTrack`, `prestigeStartLevel`, `prestigeLoopLength`, `levels`).
   - Sorts all track rows by `track_id` in byte/code-unit order (`cmp`), transforming each level's `grants` (`grant_type == "Talent"` -> `{"type": "talent", "id": ...}`, `"Item"` -> `{"type": "item", "id": ..., "amount": ...}`, `"LoadoutSlot"` -> `{"type": "loadoutSlot", ...}`, `"LanternCapacitor"` -> `{"type": "lanternCapacitor", "amount": ...}`).
   - `progression_text(config)` formats the resulting JSON with 2-space indentation, compacting single-primitive arrays (`levels: [100, 200, ...]`) onto one line to match the Node kit's exact output byte-for-byte.

---

### 2.14 Module 16: `publish`

#### Responsibility
Publishes `MysticParadox.dll` and `winmm.dll` into `.workspace/runtime/`, writes `.workspace/runtime/runtime.json`, installs the DLLs beside the server's own `DauntlessServer-Win64-Shipping.exe` (safely moving locked DLLs aside on Windows), and creates required server flag files (`CALL_HUNTPASS_QUERY.flag`).

#### Constants, Types & Functions

```rust
pub const ASIDE_MARK: &str = ".rr-old-";
pub const SERVER_RUNTIME_FLAGS: [&str; 1] = ["CALL_HUNTPASS_QUERY.flag"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeFile {
    pub name: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedRuntime {
    pub version: String,
    pub game_version: String,
    pub changelist: u32,
    pub published_at: String,
    pub files: Vec<RuntimeFile>,
}

pub fn runtime_version(now: DateTime<Utc>, dll_sha256: &str) -> String;
pub fn replace_loaded_file(src: &Path, dst: &Path) -> Result<Replaced, HostError>;
pub fn default_sources(paths: &Paths) -> [(&'static str, PathBuf); 2];
pub fn publish_runtime(
    paths: &Paths,
    now: DateTime<Utc>,
    sources: &[(&str, PathBuf)],
) -> Result<PublishedRuntime, HostError>;
pub fn install_server_runtime(paths: &Paths, win64_dir: &Path) -> InstalledRuntime;
pub fn ensure_server_flags(win64_dir: &Path) -> Result<Vec<String>, HostError>;
```

#### Behavioral Contract & Invariants
1. **Runtime Version Format (`runtime_version`):**
   `rt-<YYYYMMDD>-<first 8 chars of MysticParadox.dll sha256>` (e.g. `rt-20260417-a1b2c3d4`).
2. **Publishing (`publish_runtime`):**
   - Verifies both source files (`MysticParadox.dll` and `winmm.dll`) exist; if missing, returns `Err("<name> is not built yet (expected at <path>)")`.
   - Copies both files into `.workspace/runtime/`, computes their sizes and lowercase hex SHA-256 digests, and writes `.workspace/runtime/runtime.json` with 2-space pretty JSON + trailing `\n`.
3. **Locked DLL Replacement (`replace_loaded_file`, `install_server_runtime`):**
   - Cleans up any stale `<name>.rr-old-*` files in the destination directory first.
   - If `fs::copy(src, dst)` fails (e.g. `ERROR_SHARING_VIOLATION` on Windows because a game server is running), renames `dst` to `<dst>.rr-old-<timestamp>` and copies `src` to `dst`, returning `Replaced::InUse`.

---

### 2.15 Module 17: `linux`

#### Responsibility
Implements Linux-specific host capabilities: checking root and POSIX capabilities (`CAP_NET_BIND_SERVICE`, `CAP_NET_ADMIN` from `/proc/self/status`), locating Wine or Proton, initializing the Wine prefix (`wineboot --init`), writing `.workspace/bin/dauntless-gameserver`, generating `.workspace/systemd/{reforged.service,reforged-allowlist.service,reforged-allowlist-legacy.service,install.sh}`, and generating `.workspace/config/firewall.sh` (`ufw`).

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

pub fn effective_capabilities(status_text: &str) -> Option<u64>;
pub fn has_capability(cap: u32) -> Result<bool, HostError>;
pub fn is_root() -> Result<bool, HostError>;
pub fn can_bind_port_with(port: u16, root: bool, cap_bind: bool, unprivileged_start: Option<u16>) -> bool;
pub fn can_bind_port(port: u16) -> Result<bool, HostError>;
pub fn can_change_firewall() -> Result<bool, HostError>;
pub fn sh_quote(s: &str) -> String;
pub fn gameserver_wrapper_script(host_bin: &Path) -> String;
pub fn write_gameserver_wrapper(paths: &Paths) -> Result<PathBuf, HostError>;
pub fn game_runtime(options: &LinuxOptions) -> Result<Option<GameRuntime>, HostError>;
pub fn ensure_wine_prefix(paths: &Paths, options: &LinuxOptions, log_file: &Path) -> Result<WinePrefix, HostError>;
pub fn systemd_units(config: &HostConfig, paths: &Paths, ctx: &UnitContext) -> Result<BTreeMap<String, String>, HostError>;
pub fn write_systemd_units(config: &HostConfig, paths: &Paths) -> Result<Vec<PathBuf>, HostError>;
pub fn systemd_installed(config: &HostConfig) -> bool;
pub fn linux_firewall_script(config: &HostConfig) -> Result<String, HostError>;
```

#### Behavioral Contract & Invariants
1. **Capabilities & Port Binding:**
   - `effective_capabilities` parses the hex `CapEff:\t<hex>` line from `/proc/self/status`.
   - `can_bind_port_with(port, root, cap_bind, unprivileged_start)`: returns `true` if `port >= 1024`, `root`, `cap_bind`, or `unprivileged_start.is_some_and(|start| port >= start)` (reading `/proc/sys/net/ipv4/ip_unprivileged_port_start`).
2. **Wrapper Script (`gameserver_wrapper_script`):**
   Writes `#!/bin/sh\nexec '<host_bin>' gameserver "$@"\n` with mode `0o755` at `.workspace/bin/dauntless-gameserver`.
3. **Systemd Units (`systemd_units`):**
   - Refuses `ctx.user == "root"`.
   - Generates `reforged.service` (with `AmbientCapabilities=CAP_NET_BIND_SERVICE` when public gateway port `< 1024`, and `Environment=REFORGED_ALLOWLIST=external` when public), `reforged-allowlist.service` (in public mode, with `AmbientCapabilities=CAP_NET_ADMIN`), `reforged-allowlist-legacy.service` (when `edition == Both` and public), and `install.sh`.
4. **Linux Firewall Script (`linux_firewall_script`):**
   Generates a `#!/bin/sh` script configuring `ufw` rules:
   - In public mode: allows TCP `<gateway_port>` (and `<legacy_gateway_port>` when `Both`) and deletes any wide-open UDP rule on the game port range (since `reforged-allowlist` manages `nftables` directly).
   - In private mode: allows TCP `61000` and UDP `<first>:<last>` from `100.64.0.0/10` (`Tailscale`).

---

### 2.16 Module 18: `selfupdate`

#### Responsibility
Implements release discovery, SemVer comparison, `SHA256SUMS.txt` verification, GitHub build-provenance attestation verification (`gh attestation verify`), `.zip` / `.tar.gz` extraction with zip-slip/symlink protection, local-modification detection, and atomic in-place installation with `.workspace/update/installing` crash-recovery markers.

#### Constants, Types & Functions

```rust
pub const RELEASE_FILE: &str = "release.json";
pub const SUMS_FILE: &str = "SHA256SUMS.txt";
pub const ARCHIVE_PREFIX: &str = "ReforgedRevivalServer-";
pub const MANIFEST_NAME: &str = "reforged-revival-server";
pub const DEFAULT_REPOSITORY: &str = "https://github.com/astridotte/dainty-lass";
pub const SOURCE_ENV: &str = "REFORGED_UPDATE_URL";
pub const ATTESTATION_ENV: &str = "REFORGED_UPDATE_ATTESTATION";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    NoRelease(String),
    DownloadFailed(String),
    NotInSums(String),
    Checksum { file: String, expected: String, actual: String },
    AttestationFailed(String),
    BadArchive(String),
    LocalChanges(Vec<String>),
}

pub fn this_platform() -> Result<&'static str, HostError>;
pub fn archive_name(version: &str, platform: &str) -> String;
pub fn read_manifest(root: &Path) -> Result<Option<Manifest>, HostError>;
pub fn damaged_files(root: &Path, manifest: &Manifest) -> Result<Vec<String>, HostError>;
pub fn find_release(source: &Source, platform: &str, wanted: Wanted<'_>) -> UpdateResult<Candidate>;
pub fn fetch(candidate: &Candidate, work_dir: &Path) -> UpdateResult<PathBuf>;
pub fn verify_attestation(archive: &Path, candidate: &Candidate, log_file: &Path) -> UpdateResult<Attestation>;
pub fn extract(archive: &Path, staging: &Path) -> UpdateResult<PathBuf>;
pub fn runtime_changed(previous: &Manifest, next: &Manifest) -> bool;
pub fn local_changes(root: &Path, previous: Option<&Manifest>, next: &Manifest) -> Result<Vec<String>, HostError>;
pub fn remove_moved_aside(root: &Path);
pub fn installing_marker(root: &Path) -> PathBuf;
pub fn install(staged: &Path, root: &Path, previous: Option<&Manifest>) -> UpdateResult<(Manifest, Installed)>;
```

#### Behavioral Contract & Invariants
1. **Archive Naming (`archive_name`):**
   - `windows-x64`: `ReforgedRevivalServer-<version>-windows-x64.zip`
   - `linux-x64`: `ReforgedRevivalServer-<version>-linux-x64.tar.gz`
2. **Download & Checksum (`fetch`):**
   - Downloads `SHA256SUMS.txt` first, looks up the exact archive filename, downloads the archive to `<work_dir>/<archive>.part`, verifies that the full `Content-Length` was received and that the SHA-256 equals the digest in `SHA256SUMS.txt`.
   - On any checksum mismatch or interrupted download, deletes the partial file immediately and returns `Problem::Checksum` or `Problem::DownloadFailed`.
3. **Archive Extraction (`extract`):**
   - Rejects any archive entry with `..`, leading `/`, `\`, `:`, or symlinks/hardlinks (`Problem::BadArchive`).
   - Strips a single shared top-level directory (`reforged-revival-server-<version>/`) if all entries share one.
4. **Installation (`install`):**
   - Verifies every file in `staged` against `staged/release.json` (`damaged_files`).
   - Checks `local_changes(root, previous, &next)`: refuses with `Problem::LocalChanges` if any release-tracked file on disk was modified by the user (ignoring `package-lock.json` on a git checkout).
   - Writes `.workspace/update/installing` marker before modifying `root`, replaces files via `<dst>.reforged-new` (moving locked `bin/` executables aside to `<dst>.old-<n>` on Windows), removes files present in `previous` but absent in `next`, writes `release.json` **last**, and deletes the `installing` marker.

---

### 2.17 Module 19: `wine`

#### Responsibility
Implements the hidden `reforged-host gameserver [args...]` command on Linux, wrapping `DauntlessServer-Win64-Shipping.exe` inside Wine or Proton (optionally under `xvfb-run -a`).

#### Constants, Types & Functions

```rust
pub const ENV_EXE: &str = "REFORGED_GAME_EXE";
pub const ENV_WINE: &str = "REFORGED_WINE";
pub const ENV_PROTON: &str = "REFORGED_PROTON";
pub const ENV_PREFIX: &str = "REFORGED_WINE_PREFIX";
pub const ENV_PROTON_DATA: &str = "REFORGED_PROTON_DATA";
pub const ENV_XVFB: &str = "REFORGED_XVFB";
pub const ENV_XVFB_RUN: &str = "REFORGED_XVFB_RUN";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: PathBuf,
}

pub fn is_executable(file: &Path) -> bool;
pub fn find_on_path_in(names: &[&str], path: Option<&OsStr>) -> Option<String>;
pub fn find_on_path(names: &[&str]) -> Result<Option<String>, HostError>;
pub fn find_wine(configured: Option<&str>) -> Result<Option<String>, HostError>;
pub fn dll_overrides(existing: Option<&str>) -> String;
pub fn build_launch(env: &BTreeMap<String, String>, game_args: &[String]) -> Result<LaunchSpec, HostError>;
pub fn wine_prefix_ready(prefix: &Path) -> bool;
pub fn run_game_server(game_args: &[String]) -> Result<i32, HostError>;
```

#### Behavioral Contract & Invariants
1. **DLL Overrides (`dll_overrides`):**
   Always appends `"winmm=n,b;mscoree=d;mshtml=d"` (after `<existing>;` if `existing` is non-empty) so Wine loads the server's `winmm.dll` runtime loader natively while disabling Mono and Gecko prompts.
2. **Launch Construction (`build_launch`):**
   - Requires `REFORGED_GAME_EXE` (`ENV_EXE`) to be set and exist on disk.
   - Ensures `-nullrhi` is present in the game server arguments (prepending it if not already in `game_args`).
   - Sets `WINEDEBUG=-all` (unless already set in `env`) and `WINEDLLOVERRIDES`.
   - When `REFORGED_PROTON` is set: launches `<proton> run <exe> <args...>` with `STEAM_COMPAT_DATA_PATH` and `STEAM_COMPAT_CLIENT_INSTALL_PATH`.
   - Otherwise locates Wine (`REFORGED_WINE`, `wine64`, or `wine`) and launches `<wine> <exe> <args...>` with `WINEPREFIX`.
   - When `REFORGED_XVFB == "1"` and `DISPLAY` is unset/empty: wraps the command in `<xvfb-run> -a -s "-screen 0 640x480x24 -nolisten tcp" <cmd> <args...>`.

---

### 2.18 Module 20: `cli`

#### Responsibility
Defines the `clap` CLI argument parser (`Cli`, `Command`, and per-command `*Args` structs) and custom error rendering (`parse_from`) matching the host kit's CLI surface.

#### Public Commands & Flags (`Command`)
1. `setup` (`SetupArgs`):
   - `--mode <private|public>`, `--edition <reborn|legacy|both>`, `--host <addr>`, `--game-address <ip>`, `--name <name>`, `--game <dir>`, `--legacy-game <dir>`, `--registration <INVITECODE|OPEN|NONE>`, `--share <url>`, `--mongo-uri <uri>`, `--manage-mongo` / `--no-manage-mongo`, `--mongod <path>`, `--max-hunts <1-10>`, `--xp-multiplier <1-100>`, `--reward-multiplier <1-100>`, `--no-multipliers` / `--off`, `--public-port <port>`, `--legacy-public-port <port>`, `--server-fps <fps>`, `--idle-fps <fps>`, `--training-on-demand` / `--training-at-boot`, `--training-idle-minutes <min>`, `--verbose-game-logs` / `--quiet-game-logs`, `--memory-guard` / `--no-memory-guard`, `--min-free-ram <mb>`, `--hunt-ram <mb>`, `--wine <path>`, `--proton <path>` / `--no-proton`, `--xvfb` / `--no-xvfb`, `--firewall-interface <iface>` / `--no-firewall-interface`, `--skip-install`, plus trailing `unexpected: Vec<String>` to catch unquoted paths with spaces.
2. `start`: No flags (runs `serve` in non-interactive `ServeOptions::start()` mode).
3. `serve` (`ServeArgs`): `--config <file>` (`-c`), `--tui`, `--log-level <filter>` (default `"info"`).
4. `invite` (`InviteArgs`): `--for <whom>`, `--uses <1-1000>` (default `"1"`), `--unlimited`, `--list`, `--json`, `--game <version>`.
5. `doctor` (`DoctorArgs`): `--no-hash`.
6. `gamedata` (`GamedataArgs`): `--examples`, `--example-sources`, `--progression`, `--export [file]`, `--import [file]`.
7. `publish-runtime` (`PublishRuntimeArgs`): `--from [dir]`.
8. `install-game` (`InstallGameArgs`): `--dir <folder>`, `--version <ver>`, `--code-file <file>`, `--sign-out`.
9. `preset` (`PresetArgs`): `--create`, `--generate`, `--name <name>`, `--apply`, `--force`.
10. `multipliers` (`MultipliersArgs`): `--xp <n>` / `--xp-multiplier <n>`, `--rewards <n>` / `--reward-multiplier <n>`, `--off` / `--no-multipliers`.
11. `update` (`UpdateArgs`): `--check`, `--json`, `--no-setup`.
12. `backup`: No flags.
13. `network` (`NetworkArgs`): subcommands `ca-init`, `issue-node-cert`, `list`, `status`, `join`.
14. `history` (`HistoryArgs`) & `revert` (`RevertArgs`): `--account`, `--actor`, `--since`, `--until`, `--limit`, `--cursor`, `verify`, `--dry-run`, `--apply <plan_id>`.
15. Hidden service/wrapper subcommands: `gateway`, `allowlist`, `content`, `admin`, `gameserver`.

---

### 2.19 Module 21: `commands/*`

#### Responsibility
Implements the CLI command handlers and their dependency-injection traits (`SetupEnv`, `UpdateEnv`, `BackupEnv`, `Control`) for `setup`, `start`, `doctor`, `invite`, `multipliers`, `gamedata`, `preset`, `publish-runtime`, and `update`.

#### Key Submodule Specifications
1. **`commands/setup` (`setup.rs`, `setup/options.rs`, `setup/firewall.rs`, `setup/install.rs`):**
   - Rejects unquoted extra positional words (`unexpected "..."`).
   - On Linux, refuses to run as root (`NOT_AS_ROOT`).
   - Builds `HostConfig` via `options::host_config_from_args(args, existing, env)` preserving existing `host.toml` settings for any flag not passed, validates, and writes `.workspace/config/host.toml` (migrating `host.json` -> `host.json.bak` if present).
   - Step 1: Checks `paradox::missing_paradox`, migrates any files from `.workspace/Mystic-Paradox` or pre-submodule `paradox/` & `native/` (`paradox::migrate_old_workspace`), and applies `preset/` if present (`apply_preset(paths, false)`).
   - Step 2: Unless `--skip-install`, runs `install::install_services`:
     - If `release.json` exists (`ServiceSource::Release`), verifies release files and re-fetches/repairs damaged files from the release source.
     - Otherwise (`ServiceSource::Checkout`), runs `npm ci --no-audit --no-fund` and `npm run build` in `ParadoxBackend`, `ParadoxDirector`, `services/gateway`, and `services/content`.
   - Step 3: Loads or creates `.workspace/config/secrets.env`, writes service `.env` files, creates `.workspace/branding/news.json` (`{"items": []}`) if missing, writes `.workspace/config/firewall.ps1` (Windows, CRLF line endings) or `.workspace/config/firewall.sh` + `.workspace/bin/dauntless-gameserver` + `.workspace/systemd/*` (Linux), and in `public` mode generates `.workspace/tls/gateway-{cert,key}.pem` if missing and prints its SHA-256 fingerprint.
   - Step 4 (Linux): Checks Wine/Proton and initializes `.workspace/wine`.
2. **`commands/start` (`start/mod.rs`, `start/control.rs`):**
   - `control.rs`:
     - `CONTROL_ENV = "REFORGED_CONTROL"`, `SERVER_READY_LINE = "REFORGED_SERVER_READY"`.
     - `controlled_by_stdin()` returns `true` when `REFORGED_CONTROL=stdin`.
     - `watch_stop_input(input, on_stop)` reads lines until a line whose trimmed text equals `"stop"` (case-insensitive) or EOF, then invokes `on_stop` exactly once.
     - `Control::begin(stdin)` registers OS signal handlers (`Ctrl+C`, `Ctrl+Break`, console close on Windows; `SIGINT`, `SIGTERM`, `SIGHUP` on Unix) and optionally spawns the stdin watcher thread.
   - `start/mod.rs`:
     - `service_plan` plans `backend` (port 61000, `/dauntless-status`, required, 120s timeout), `director` (port 61001, `/api/instances`, optional, 60s), `content` (port 61002, `/content/v1/news`, optional, 60s), `admin` (when built-in and `admin.env` exists, port 61003, `/health`, optional, 60s), `allowlist` (when public and not external, port 61005, `/status`, optional, 60s), and `gateway` (TCP `127.0.0.1:<public_port>` or `<game_address>:61000`, required, 60s).
     - `start_services` checks `port_taken` **before** spawning each service so an already-occupied port fails fast with `port_owner_hint` instead of falsely passing readiness.
     - `finish` prints the server URL line, prints `REFORGED_SERVER_READY` when `controlled_by_stdin()`, waits on `control.wait()`, and calls `supervisor.stop_all()`.
3. **`commands/doctor` (`doctor.rs`):**
   - Calls `run_doctor(paths, &DoctorOptions { hash_exe: !args.no_hash })`, prints grouped checks via `print_checks`, prints `\n<N> problem(s), <M> warning(s).`, and exits with `ExitCode::FAILURE` (`1`) if `fails > 0`, else `ExitCode::SUCCESS` (`0`).
4. **`commands/invite` (`invite.rs`):**
   - Validates `--uses` (`1..=1000` integer) and `--game` (`reborn`/`reforged`/`1.14.7` or `legacy`/`revived`/`1.4.4`).
   - Calls backend `create_invite_code` (or legacy invite hook for `1.4.4`), reads `certificate_fingerprint` in public mode, formats the `dauntless://invite?...` link, and prints either human-readable instructions or a single JSON line `{"link": ..., "for": ..., "uses": ..., "shareUrl": ...}` when `--json` is passed.
5. **`commands/multipliers` (`multipliers.rs`):**
   - Rejects `Edition::Legacy`.
   - When `--xp`, `--rewards`, or `--off` is passed: updates `config.multipliers` (enforcing `1..=100` and `ServerMode::Private`), saves `host.toml` preserving all other keys/comments, and rewrites the `.env` files.
6. **`commands/gamedata`, `commands/preset`, `commands/publish_runtime`, `commands/update`:**
   - Thin CLI wrappers around `gamedata`/`gamepack`, `preset`, `publish`, and `selfupdate` respectively, formatting console/JSON output and returning `ExitCode::SUCCESS` or `ExitCode::FAILURE`.

---

### 2.20 Module 22: `setup/paradox` (`commands/setup/paradox.rs`)

#### Responsibility
Verifies that the required source directories exist in the checkout (`missing_paradox`) and performs one-time non-destructive migration (`migrate_old_workspace`) of a host's own generated files (Dumper-7 SDK headers/CPP files, built `MysticParadox.dll` / `winmm.dll` / `CatalogExporter.dll`, `Items_Analysis` exports, `DauntlessEndpointDocumentation` captures, and non-example `game-data/*.json` + `raw/`) from older workspace layouts (`.workspace/Mystic-Paradox` or pre-submodule `paradox/` and `native/`).

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
pub struct Layout { /* paths for current and legacy workspace locations */ }

impl Layout {
    pub fn new(p: &Paths) -> Self;
}

pub fn missing_paradox(l: &Layout) -> Vec<&'static str>;
pub fn old_workspace_items(l: &Layout) -> Vec<(PathBuf, PathBuf)>;
pub fn pre_submodule_items(l: &Layout) -> Vec<(PathBuf, PathBuf)>;
pub fn old_folders(l: &Layout) -> Vec<PathBuf>;
pub fn migrate_old_workspace(l: &Layout, repo: &Path) -> Result<Vec<String>, HostError>;
```

#### Behavioral Contract & Invariants
1. **`missing_paradox(l)`:**
   Checks the existence of 5 sentinel files:
   - `modules/rustic-isles/reference/1.14.7/ParadoxBackend/package.json`
   - `modules/rustic-isles/reference/1.14.7/ParadoxDirector/package.json`
   - `modules/rustic-isles/native/runtime/MysticParadox.vcxproj`
   - `native/runtime-loader/Cargo.toml`
   - `modules/rustic-isles/native/catalog-exporter/CatalogExporter.vcxproj`
   Returns the relative directory names of any that are missing.
2. **`migrate_old_workspace(l, repo)`:**
   - Inspects `.workspace/Mystic-Paradox` (`old_workspace_items`) and pre-submodule `paradox/`, `native/paradox-runtime`, `native/catalog-exporter` (`pre_submodule_items`).
   - Copies only items that exist in the old location and do **not** yet exist in the new location (an existing empty directory at destination is treated as non-existent so `raw/` or `SDK/` can be populated).
   - Never copies `*.example.json` files and never deletes the old directories.
   - Returns the list of migrated destination paths relative to `repo` using `/` separators.

---

## 3. Target Crate Architecture for DL3b (`crates/hostkit` vs `services/host`)

1. **`rust/crates/hostkit` (`license = "MIT"`):**
   - Contains all Category A (MIT-eligible original modules) and Category B (clean-room reimplemented modules from Section 2).
   - Exposes a clean hook/trait (`LegacySupport` or equivalent) for the few places where `setup`, `invite`, `doctor`, and `serve` optionally delegate to the 1.4.4 (`Edition::Legacy` / `Edition::Both`) stack, and a service-registration callback for in-process front-door services.
   - Has **zero** dependencies on `gateway`, `allowlist`, `content`, `admin`, or `legacy-revived` crates.
2. **`rust/services/host` (`license = "AGPL-3.0-or-later"`):**
   - Thin binary/library shell that depends on `hostkit` and the optional feature-gated service crates (`gateway`, `allowlist`, `content`, `admin`) plus `legacy/` (`Edition::Legacy` / `Edition::Both` support).
   - Re-exports `hostkit` modules so all existing integration tests in `rust/services/host/tests/*.rs` and workspace binaries (`reforged-host`, `dainty-lass-server-console`, `dainty-lass-server-minimal`, `launcher`) continue to compile and pass unchanged.
