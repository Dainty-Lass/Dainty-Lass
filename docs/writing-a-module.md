# Writing a game module or plugin

Every game version Dainty Lass serves (today `1.14.7`) is packaged as a **`GameModule`** on
[`crates/dainty-module-api`](../crates/dainty-module-api). The same `GameModule` implementation runs compiled into
`restricted/`'s server binaries (`server-minimal`, `server-console`, and the launcher's `server-gui`) or out of
process as a standalone executable speaking length-prefixed JSON frames over stdin and stdout (`module_api::child`).

## 1. Crate layout

A game module depends on `module-api`, `protocol`, `service-kit` and `store`: by path inside this repository, as
`crates/rustic-isles-1-14-7` does, or as git dependencies on dainty-lass at one rev from a repository of its own. Give
the crate both a `[lib]` target (for compiled-in server builds) and a `[[bin]]` target (for out-of-process loading):

```toml
[package]
name = "my-game-module"
version = "0.1.0"
edition = "2024"
license = "AGPL-3.0-only" # or "MIT OR Apache-2.0" if written from scratch with no AGPL upstream

[lib]
name = "my_game_module"
path = "src/lib.rs"

[[bin]]
name = "my-game-module"
path = "src/main.rs"

[dependencies]
module-api = { version = "0.1.0", features = ["child"] }
```

## 2. Declaring the manifest (`ModuleManifest`)

Each game module declares one exact `GameVersion` (dot-separated integers, compared exactly, no ranges). The
controller keys loaded game modules by `GameVersion` and refuses to load a second module for a version that is
already active:

```rust
use module_api::{
    Capabilities, ClientProfile, GameVersion, ModuleApiVersion, ModuleId, ModuleKind,
    ModuleManifest, PortSpec, VersionAlias,
};

pub fn manifest() -> ModuleManifest {
    let version = GameVersion::parse("1.14.7").expect("valid game version");
    ModuleManifest {
        id: ModuleId::new("my-1-14-7").expect("valid module id"),
        display_name: "My 1.14.7 Module".to_owned(),
        kind: ModuleKind::GameModule {
            game_version: version.clone(),
            aliases: vec![VersionAlias::new("my-alias").expect("valid alias")],
            unverified: false,
        },
        api_version: ModuleApiVersion::CURRENT,
        default_ports: vec![
            PortSpec::tcp("backend", 41000),
            PortSpec::tcp("director", 41001),
            PortSpec::udp_range("game", 8770, 8777),
        ],
        capabilities: Capabilities::default(),
        client_profile: Some(ClientProfile::new(
            version,
            "My 1.14.7 Module",
            "f3f95fbf00000000000000000000000000000000000000000000000000000000",
            "Archon/Binaries/Win64/Dauntless-Win64-Shipping.exe",
        )),
    }
}
```

- **Ports.** `default_ports` are requests, not hard bindings: if another loaded module already holds TCP `41000`,
  the controller assigns the next free port and passes the resolved map into `start` via `ModuleContext`. Never
  open a public listener inside a module; bind `127.0.0.1` on the controller-assigned TCP port and register your
  UDP game-server range so the allowlist opens it only for logged-in players.
- **Unverified mark.** Set `unverified: true` when a port has not yet been replayed against live traffic (like
  `"Rustic Isles 1.12"`). The server GUI and status pages surface the badge automatically.

## 3. Implementing `GameModule` and `Registrar`

Implement `module_api::GameModule` on your module type. In `register`, declare every route prefix, UDP range,
content file, command, event, config section and admin handler the module needs through `Registrar`:

```rust
use module_api::{
    AdminAction, AdminOutcome, AdminRequest, CommandCall, CommandOutcome, CommandSpec,
    ConfigField, ConfigFieldKind, ConfigSectionSpec, GameModule, ModuleContext, ModuleError,
    ModuleManifest, ModuleStatus, Registrar, RouteGroup, UdpRange,
};

pub struct MyModule {
    manifest: ModuleManifest,
}

impl GameModule for MyModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    fn register(&mut self, reg: &mut Registrar) -> Result<(), ModuleError> {
        reg.add_route_group(RouteGroup::new("backend", vec!["/dainty/api".into(), "/account".into()]))?;
        reg.add_udp_range(UdpRange::new("game", 8770, 8777)?)?;
        reg.add_command(CommandSpec::new("status", "Print module status"))?;
        reg.add_config_section(ConfigSectionSpec::new(
            "switches",
            vec![ConfigField::new("reforge", ConfigFieldKind::String, "on")],
        ))?;
        Ok(())
    }

    fn start(&mut self, ctx: ModuleContext) -> Result<(), ModuleError> {
        // Read assigned loopback ports from ctx.ports(), config from ctx.config_section("switches"),
        // and start your loopback HTTP server.
        let _backend_port = ctx.port("backend");
        Ok(())
    }

    fn stop(&mut self) -> Result<(), ModuleError> {
        Ok(())
    }

    fn status(&self) -> ModuleStatus {
        ModuleStatus::running("ready")
    }

    fn command(&mut self, call: CommandCall) -> Result<CommandOutcome, ModuleError> {
        Ok(CommandOutcome::text(format!("ran {}", call.name)))
    }

    fn admin(&mut self, req: AdminRequest) -> Result<AdminOutcome, ModuleError> {
        match req.action {
            AdminAction::Lookup { account_id } => Ok(AdminOutcome::ok(format!("account {account_id}"))),
            _ => Err(ModuleError::Unsupported("action not supported".into())),
        }
    }
}
```

- **Database access.** Every database write must go through `store`'s write path (`store::Database`) with an actor
  from the request context so the hash-chained change log (`DH1`–`DH5`) records the before and after images. Do
  not open an untracked SQLite or Postgres connection inside a module.
- **Declarative UI panels.** The server GUI (dainty-dash, `ui/`) renders each module's commands, status fields and
  config sections straight from its `Registrar` declarations, so no module ships UI code.

### Answering game calls (API 1.2)

Other modules (the admin API, the Discord bot, `remote`) learn about your game through `GameModule::game`, routed by
the registry like `admin` and `chat` (`crates/dainty-module-api/src/game.rs`, the module-sharing plan's A1). Register
each call you answer with `registrar.game_call(GameCallKind::...)`; a call you don't override answers `unsupported`.

| Call | Answer | Notes |
| --- | --- | --- |
| `GameCall::Availability` | `GameAnswer::Availability(Availability)` | `up` only when the hub is running and ready; otherwise `reason` is an `UnavailableReason` code, never a sentence (the reader translates it) |
| `GameCall::Servers` | `GameAnswer::Servers(Vec<GameServer>)` | Every game server your Director runs: hub, hunt, dojo, tutorial |
| `GameCall::CreateInvite { uses, expires_in_secs, note, require_up }` | `GameAnswer::Invite { code }` | With `require_up`, check availability again inside the write and refuse with `reason.error()`; `uses` is at least 1 |
| `GameCall::RevokeInvite { code }` | `GameAnswer::Done` | `ErrorKind::NotFound` when there is no such code |

An invite code is a credential: never log it or put it in an event. Callers read a refusal back with
`UnavailableReason::from_error`. A module that tracks its hub emits `game::SERVER_UP_EVENT`,
`SERVER_DOWN_EVENT` (payload: a `GameServer`) and `AVAILABILITY_CHANGED_EVENT` (payload: an `Availability`),
declared with `registrar.emits` like any other event.

## 4. Out-of-process binary (`src/main.rs`)

Wrap the same `GameModule` in `module_api::child::run_stdio` so it can be launched as an external executable:

```rust
fn main() -> std::process::ExitCode {
    module_api::child::run_stdio(my_game_module::MyModule::new())
}
```

When the controller starts the child process (`crates/dainty-module-api/src/host.rs`), it performs the `Hello` /
`Welcome` handshake, checks `ModuleApiVersion::CURRENT` and the manifest's `id` and `sha256`, replays `register`,
and forwards `start`, `command`, `admin`, `chat`, `game`, `status` and `stop` over length-prefixed frames (capped at `MAX_FRAME_LEN`,
4 MiB; see [security/module-host.md](security/module-host.md)).

## 5. Loading the module in `settings.toml`

- **Compiled-in modules** are enabled via Cargo features on `restricted/`'s `server-console`, `server-minimal`
  and `launcher` ([server-modules.md](server-modules.md)) and loaded by default unless `[modules] compiled = [...]`
  restricts the list.
- **External modules** are listed in `.workspace/config/settings.toml` with a relative path under the server root and
  their lowercase hex SHA-256:

```toml
[[modules.external]]
id = "my-1-14-7"
path = "modules/my-game-module.exe"
sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
autoload = true

[modules.my-1-14-7.switches]
reforge = "on"
```

Use the server console's `modules`, `module load <id>` and `module unload <id>`, or the server GUI's Modules screen,
to inspect, start, stop or reload the module at runtime.
